//! Pathname-resolution helpers built around borrowed path slices.
//!
//! Path parsing is represented by [`Pathname`], a zero-allocation wrapper over
//! a raw string slice. It skips repeated separators while walking directory
//! components, but keeps the final component as the caller supplied it so
//! name-based lookups see the requested bytes.

use alloc::sync::Arc;
use core::str::Split;

use bitflags::bitflags;

use crate::{
    fs::{
        layout::InodeType,
        minix::{Inode, InodeId},
        mount::MOUNT_TABLE,
        resolve_inode,
    },
    task, time,
};

/// Resolve one pathname to its final inode.
pub fn resolve_path(path: &str) -> Option<Arc<Inode>> {
    let (dir, basename) = resolve_parent(path)?;

    let inode = if basename.is_empty() {
        dir
    } else {
        let inum = dir.lookup(basename).ok()??;
        resolve_inode(InodeId::new(dir.id.device, inum))
    };

    inode.inner.lock().touch_access(time::current_time());

    Some(inode)
}

/// Resolve a pathname to its parent directory inode and the final component name.
///
/// Returns `(parent_directory_inode, basename)` where `basename` is the last
/// path component as a raw string. When the path ends with `/`, `basename` is
/// empty — the caller decides how to handle that case.
///
/// The returned parent directory is guaranteed to be a directory inode with
/// search (execute) permission for the current task.
pub fn resolve_parent(path: &str) -> Option<(Arc<Inode>, &str)> {
    let pathname = Pathname::new(path)?;
    let fs_ctx = task::with_current(|inner| inner.fs.clone());
    let root_inode = fs_ctx.root_directory.clone()?;
    let mut current_inode = if pathname.is_absolute() {
        Arc::clone(&root_inode)
    } else {
        fs_ctx.current_directory.clone()?
    };
    let (parent_components, basename) = pathname.parent_path();

    for component in parent_components {
        if current_inode.file_type() != InodeType::Directory
            || !check_permission(&current_inode, AccessMask::MAY_EXEC)
        {
            return None;
        }

        match component {
            PathComponent::Current => {}
            PathComponent::Parent => {
                current_inode = resolve_dotdot(&current_inode, &root_inode)?;
            }
            PathComponent::Name(name) => {
                let child_inum = current_inode.lookup(name).ok()??;
                current_inode = resolve_inode(InodeId::new(current_inode.id.device, child_inum));
            }
        }
    }

    if !basename.is_empty()
        && (current_inode.file_type() != InodeType::Directory
            || !check_permission(&current_inode, AccessMask::MAY_EXEC))
    {
        return None;
    }

    Some((current_inode, basename))
}

/// Check whether the current task has `mask` access to `inode`.
///
/// Returns `true` when the access is allowed.  The check considers the
/// effective uid/gid of the running process and falls back to superuser
/// override (euid == 0).
///
/// A deleted file (link_count == 0) is inaccessible to everyone, including
/// the superuser.
pub fn check_permission(inode: &Inode, mask: AccessMask) -> bool {
    let (euid, egid) = task::with_current(|inner| (inner.identity.euid, inner.identity.egid));
    check_permission_as(inode, mask, euid, egid)
}

/// Same as [`check_permission`] but with explicitly supplied uid/gid.
///
/// Used by `sys_access` which checks against the real uid/gid rather than
/// the effective ones.
pub fn check_permission_as(inode: &Inode, mask: AccessMask, uid: u16, gid: u16) -> bool {
    let inner = inode.inner.lock();
    let disk = &inner.disk_inode;

    if disk.link_count == 0 {
        return false;
    }

    let mode = if uid == disk.user_id {
        disk.mode.0 >> 6
    } else if gid == disk.group_id as u16 {
        disk.mode.0 >> 3
    } else {
        disk.mode.0
    };

    (mode & mask.bits() & 0o7) == mask.bits() || uid == 0
}

bitflags! {
    /// Permission mask bits.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct AccessMask: u16 {
        /// Execute or directory-search permission.
        const MAY_EXEC  = 1;
        /// Write permission.
        const MAY_WRITE = 2;
        /// Read permission.
        const MAY_READ  = 4;
    }
}

/// Borrowed pathname used to centralize parsing rules during resolution.
#[derive(Clone, Copy)]
struct Pathname<'a> {
    /// Raw pathname string owned by the syscall or kernel caller.
    raw: &'a str,
}

/// One meaningful component from a pathname.
#[derive(Clone, Copy)]
enum PathComponent<'a> {
    /// The current directory component (`.`).
    Current,
    /// The parent directory component (`..`).
    Parent,
    /// A directory entry name that must be looked up as-is.
    Name(&'a str),
}

/// Iterator over non-empty pathname components.
struct PathComponents<'a> {
    /// Raw separator-based splitter; empty pieces are skipped by [`Iterator::next`].
    inner: Split<'a, char>,
}

/// Resolve one `..` step from `current_inode`.
///
/// Pathname rule: task root acts as a pseudo-root,
/// and traversing `..` from a mounted filesystem root first moves back to the
/// covered mount-point inode before reading that directory's `..` entry.
fn resolve_dotdot(current_inode: &Arc<Inode>, root_inode: &Arc<Inode>) -> Option<Arc<Inode>> {
    if current_inode.id == root_inode.id {
        return Some(Arc::clone(root_inode));
    }

    let parent_lookup_base = MOUNT_TABLE
        .lock()
        .mount_point_for_root(current_inode.id)
        .unwrap_or_else(|| Arc::clone(current_inode));

    let parent_inode_number = parent_lookup_base.lookup("..").ok()??;
    Some(resolve_inode(InodeId::new(
        parent_lookup_base.id.device,
        parent_inode_number,
    )))
}

impl<'a> Pathname<'a> {
    /// Wrap a non-empty raw pathname.
    fn new(raw: &'a str) -> Option<Self> {
        (!raw.is_empty()).then_some(Self { raw })
    }

    /// Return true when lookup starts from the task root directory.
    fn is_absolute(self) -> bool {
        self.raw.starts_with('/')
    }

    /// Return true when the final component should be traversed as a directory.
    fn has_trailing_slash(self) -> bool {
        self.raw.ends_with('/')
    }

    /// Split this pathname into traversed parent components and the basename.
    ///
    /// The basename is empty for paths ending in `/`; otherwise it is the
    /// final non-empty component exactly as name lookups should see it.
    fn parent_path(self) -> (PathComponents<'a>, &'a str) {
        if self.has_trailing_slash() {
            return (PathComponents::new(self.raw), "");
        }

        let trimmed_end = self
            .raw
            .rfind(|byte| byte != '/')
            .map(|index| index + 1)
            .unwrap_or(0);
        let without_trailing_separators = &self.raw[..trimmed_end];
        let basename_start = without_trailing_separators
            .rfind('/')
            .map(|index| index + 1)
            .unwrap_or(0);

        let parent = &self.raw[..basename_start];
        let basename = &without_trailing_separators[basename_start..];
        (PathComponents::new(parent), basename)
    }
}

impl<'a> PathComponent<'a> {
    /// Classify a non-empty raw component.
    fn from_raw(component: &'a str) -> Self {
        match component {
            "." => Self::Current,
            ".." => Self::Parent,
            name => Self::Name(name),
        }
    }
}

impl<'a> PathComponents<'a> {
    /// Build a component iterator over `raw`.
    fn new(raw: &'a str) -> Self {
        Self {
            inner: raw.split('/'),
        }
    }
}

impl<'a> Iterator for PathComponents<'a> {
    type Item = PathComponent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .find(|component| !component.is_empty())
            .map(PathComponent::from_raw)
    }
}
