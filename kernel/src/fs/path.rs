//! Path parsing and pathname-resolution helpers.
//!
//! This module keeps pathname parsing separate from inode traversal so later
//! resolution code can reuse one structured view of the input path.

use alloc::sync::Arc;
use bitflags::bitflags;

use crate::{
    fs::{
        get_inode,
        layout::InodeType,
        minix::{Inode, InodeId},
        mount::MOUNT_TABLE,
    },
    task, time,
};

/// Resolve one pathname to its final inode.
///
/// Equivalent to the original Linux 0.11 `namei()`.
pub fn resolve_path(path: &str) -> Option<Arc<Inode>> {
    let (dir, basename) = resolve_parent(path)?;

    let inode = if basename.is_empty() {
        // Path ended with `/` (e.g. "/usr/") — the directory itself is the target.
        dir
    } else {
        let inum = dir.lookup(basename).ok()??;
        get_inode(InodeId {
            device: dir.id.device,
            inode_number: inum,
        })
    };

    // Update access time on the final inode, matching original namei() behaviour.
    {
        let mut inner = inode.inner.lock();
        inner.access_time = time::current_time();
        inner.is_dirty = true;
    }

    Some(inode)
}

/// Resolve a pathname to its parent directory inode and the final component name.
///
/// Returns `(parent_directory_inode, basename)` where `basename` is the last
/// path component as a raw string.  When the path ends with `/`, `basename` is
/// empty — the caller decides how to handle that case.
///
/// The returned parent directory is guaranteed to be a directory inode with
/// search (execute) permission for the current task.
fn resolve_parent(path: &str) -> Option<(Arc<Inode>, &str)> {
    let parsed_path = ParsedPath::parse(path)?;
    let fs_ctx = task::current_task()
        .pcb
        .inner
        .exclusive(|inner| inner.fs.clone());

    let root_inode = fs_ctx.root_directory.clone()?;

    let mut current_inode = if parsed_path.is_absolute() {
        Arc::clone(&root_inode)
    } else {
        fs_ctx.current_directory.clone()?
    };

    let basename = match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    };

    let mut components = parsed_path.components().peekable();
    while let Some(component) = components.next() {
        // The last component is the basename — don't traverse it.
        if !basename.is_empty() && components.peek().is_none() {
            break;
        }

        if current_inode.inner.lock().disk_inode.mode.file_type() != InodeType::Directory
            || !check_permission(&current_inode, AccessMask::MAY_EXEC)
        {
            return None;
        }

        match component {
            PathComponent::CurrentDirectory => {}
            PathComponent::ParentDirectory => {
                current_inode = resolve_dotdot(&current_inode, &root_inode)?;
            }
            PathComponent::Name(name) => {
                let child_inum = current_inode.lookup(name).ok()??;
                current_inode = get_inode(InodeId {
                    device: current_inode.id.device,
                    inode_number: child_inum,
                });
            }
        }
    }

    // Verify the parent is a searchable directory before returning it.
    if !basename.is_empty()
        && (current_inode.inner.lock().disk_inode.mode.file_type() != InodeType::Directory
            || !check_permission(&current_inode, AccessMask::MAY_EXEC))
    {
        return None;
    }

    Some((current_inode, basename))
}

/// Resolve one `..` step from `current_inode`.
///
/// This follows the Linux 0.11 pathname rule: task root acts as a pseudo-root,
/// and traversing `..` from a mounted filesystem root first moves back to the
/// covered mount-point inode before reading that directory's `..` entry.
fn resolve_dotdot(current_inode: &Arc<Inode>, root_inode: &Arc<Inode>) -> Option<Arc<Inode>> {
    if current_inode.id == root_inode.id {
        return Some(Arc::clone(root_inode));
    }

    let parent_lookup_base = MOUNT_TABLE
        .lock()
        .get_mount_point_by_root(current_inode.id)
        .unwrap_or_else(|| Arc::clone(current_inode));

    let parent_inode_number = parent_lookup_base.lookup("..").ok()??;
    Some(get_inode(InodeId {
        device: parent_lookup_base.id.device,
        inode_number: parent_inode_number,
    }))
}

bitflags! {
    /// Permission mask bits matching the original kernel's `MAY_*` constants.
    struct AccessMask: u16 {
        const MAY_EXEC  = 1;
        const MAY_WRITE = 2;
        const MAY_READ  = 4;
    }
}

/// Check whether the current task has `mask` access to `inode`.
///
/// Returns `true` when the access is allowed.  The check considers the
/// effective uid/gid of the running process and falls back to superuser
/// override (euid == 0).
///
/// A deleted file (link_count == 0) is inaccessible to everyone, including
/// the superuser, matching the original kernel behaviour.
fn check_permission(inode: &Inode, mask: AccessMask) -> bool {
    let inner = inode.inner.lock();
    let disk = &inner.disk_inode;

    if disk.link_count == 0 {
        return false;
    }

    let (euid, egid) = task::current_task()
        .pcb
        .inner
        .exclusive(|inner| (inner.identity.euid, inner.identity.egid));

    let mut mode = disk.mode.0;
    if euid == disk.user_id {
        mode >>= 6;
    } else if egid == disk.group_id as u16 {
        mode >>= 3;
    }

    (mode & mask.bits() & 0o7) == mask.bits() || euid == 0
}

/// One parsed pathname that preserves high-level path semantics.
///
/// The parser keeps the original borrowed string and exposes structural
/// information through accessors and an iterator over path components.
struct ParsedPath<'a> {
    raw: &'a str,
    is_absolute: bool,
    has_trailing_slash: bool,
}

/// One logical pathname component yielded during parsing.
enum PathComponent<'a> {
    /// One ordinary non-special directory entry name.
    Name(&'a str),
    /// The current-directory marker `.`.
    CurrentDirectory,
    /// The parent-directory marker `..`.
    ParentDirectory,
}

/// Iterator over logical pathname components.
///
/// Empty components caused by repeated `/` are skipped so callers see the same
/// hierarchy the filesystem traversal logic should observe.
struct PathComponents<'a> {
    remaining: &'a str,
}

impl<'a> ParsedPath<'a> {
    /// Parse one pathname string into a reusable structured form.
    ///
    /// Returns `None` when `path` is empty, matching the original kernel's
    /// treatment of an empty pathname as invalid input.
    fn parse(path: &'a str) -> Option<Self> {
        if path.is_empty() {
            return None;
        }

        Some(Self {
            raw: path,
            is_absolute: path.starts_with('/'),
            has_trailing_slash: path.len() > 1 && path.ends_with('/'),
        })
    }

    /// Return whether the pathname designates the root path with no components.
    fn is_root(&self) -> bool {
        self.is_absolute && self.components().next().is_none()
    }

    /// Return whether the pathname starts from the task root directory.
    fn is_absolute(&self) -> bool {
        self.is_absolute
    }

    /// Return whether the pathname ends with a slash beyond the `/` root case.
    fn has_trailing_slash(&self) -> bool {
        self.has_trailing_slash
    }

    /// Iterate over logical pathname components.
    fn components(&self) -> PathComponents<'a> {
        PathComponents {
            remaining: self.raw,
        }
    }
}

impl<'a> PathComponents<'a> {
    /// Extract the next non-empty raw component and advance the iterator.
    fn next_raw_component(&mut self) -> Option<&'a str> {
        while let Some(stripped) = self.remaining.strip_prefix('/') {
            self.remaining = stripped;
        }

        if self.remaining.is_empty() {
            return None;
        }

        match self.remaining.find('/') {
            Some(index) => {
                let component = &self.remaining[..index];
                self.remaining = &self.remaining[index + 1..];
                Some(component)
            }
            None => {
                let component = self.remaining;
                self.remaining = "";
                Some(component)
            }
        }
    }
}

impl<'a> Iterator for PathComponents<'a> {
    type Item = PathComponent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let component = self.next_raw_component()?;
        Some(match component {
            "." => PathComponent::CurrentDirectory,
            ".." => PathComponent::ParentDirectory,
            name => PathComponent::Name(name),
        })
    }
}
