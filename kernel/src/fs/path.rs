//! Pathname-resolution helpers built on top of [`std`]-style string splitting.
//!
//! Path components are obtained directly from `path.split('/')`, with `.` and
//! `..` recognised as special cases during traversal. The final component is
//! returned verbatim to the caller so name-based lookups see the raw bytes
//! the user requested.

use alloc::sync::Arc;

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

bitflags! {
    /// Permission mask bits.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct AccessMask: u16 {
        const MAY_EXEC  = 1;
        const MAY_WRITE = 2;
        const MAY_READ  = 4;
    }
}

/// Resolve one pathname to its final inode.
pub fn resolve_path(path: &str) -> Option<Arc<Inode>> {
    let (dir, basename) = resolve_parent(path)?;

    let inode = if basename.is_empty() {
        dir
    } else {
        let inum = dir.lookup(basename).ok()??;
        resolve_inode(InodeId::new(dir.id.device, inum))
    };

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
/// path component as a raw string. When the path ends with `/`, `basename` is
/// empty — the caller decides how to handle that case.
///
/// The returned parent directory is guaranteed to be a directory inode with
/// search (execute) permission for the current task.
pub fn resolve_parent(path: &str) -> Option<(Arc<Inode>, &str)> {
    if path.is_empty() {
        return None;
    }

    let ends_with_slash = path.ends_with('/');
    let fs_ctx = task::with_current(|inner| inner.fs.clone());
    let root_inode = fs_ctx.root_directory.clone()?;

    let mut current_inode = if path.starts_with('/') {
        Arc::clone(&root_inode)
    } else {
        fs_ctx.current_directory.clone()?
    };

    let mut basename = "";
    let mut components = path.split('/').filter(|c| !c.is_empty()).peekable();

    while let Some(name) = components.next() {
        // The final component is the basename unless the path ended with '/',
        // in which case every component (including the last) is traversed.
        if components.peek().is_none() && !ends_with_slash {
            basename = name;
            break;
        }

        if current_inode.file_type() != InodeType::Directory
            || !check_permission(&current_inode, AccessMask::MAY_EXEC)
        {
            return None;
        }

        match name {
            "." => {}
            ".." => current_inode = resolve_dotdot(&current_inode, &root_inode)?,
            _ => {
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
