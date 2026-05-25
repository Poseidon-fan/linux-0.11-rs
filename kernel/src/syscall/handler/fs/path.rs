//! Path and inode syscall handlers (stat, link, mkdir, chmod, chdir, etc.).

use user_lib::syscall::{
    Syscall,
    fs::{Stat, TimeUpdate},
};

use crate::{
    define_syscall_handler,
    error::{Errno, Result},
    fs::{
        InodeMode, InodeType,
        minix::InodeId,
        path::{self, AccessMask},
        resolve_inode,
    },
    mm,
    segment::uaccess,
    syscall::context::SyscallContext,
    task, time,
};

define_syscall_handler!(
    Syscall::Stat = 18,
    fn sys_stat(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, buf_ptr, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);
        let inode = path::resolve_path(&pathname).ok_or(Errno::NOENT)?;
        let stat = inode.stat();
        mm::ensure_user_area_writable(buf_ptr, core::mem::size_of::<Stat>());
        uaccess::write_struct(&stat, buf_ptr as *mut Stat);
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Link = 9,
    fn sys_link(ctx: &mut SyscallContext) -> Result<u32> {
        let (oldname_ptr, newname_ptr, _) = ctx.args();
        let oldname = uaccess::read_pathname(oldname_ptr);
        let newname = uaccess::read_pathname(newname_ptr);

        let old_inode = path::resolve_path(&oldname).ok_or(Errno::NOENT)?;
        if old_inode.file_type() == InodeType::Directory {
            return Err(Errno::PERM);
        }

        let (dir, basename) = path::resolve_parent(&newname).ok_or(Errno::ACCESS)?;
        if basename.is_empty() {
            return Err(Errno::ACCESS);
        }

        if dir.id.device != old_inode.id.device {
            return Err(Errno::XDEV);
        }

        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(Errno::ACCESS);
        }

        if dir.lookup(basename)?.is_some() {
            return Err(Errno::EXIST);
        }

        dir.add_entry(basename, old_inode.id.inode_number)?;

        let mut inner = old_inode.inner.lock();
        inner.disk_inode.link_count += 1;
        inner.touch_change(time::current_time());

        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Unlink = 10,
    fn sys_unlink(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let (dir, basename) = path::resolve_parent(&pathname).ok_or(Errno::NOENT)?;
        if basename.is_empty() {
            return Err(Errno::NOENT);
        }
        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(Errno::ACCESS);
        }

        let inum = dir.lookup(basename)?.ok_or(Errno::NOENT)?;
        let inode = resolve_inode(InodeId::new(dir.id.device, inum));
        if inode.file_type() == InodeType::Directory {
            return Err(Errno::ISDIR);
        }

        dir.remove_entry(basename)?;

        let mut inner = inode.inner.lock();
        inner.disk_inode.link_count -= 1;
        inner.touch_change(time::current_time());

        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Chdir = 12,
    fn sys_chdir(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(Errno::NOENT)?;
        if inode.file_type() != InodeType::Directory {
            return Err(Errno::NOTDIR);
        }
        if !path::check_permission(&inode, AccessMask::MAY_EXEC) {
            return Err(Errno::ACCESS);
        }

        task::with_current(|inner| inner.fs.current_directory = Some(inode));
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Chroot = 61,
    fn sys_chroot(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(Errno::NOENT)?;
        if inode.file_type() != InodeType::Directory {
            return Err(Errno::NOTDIR);
        }

        task::with_current(|inner| inner.fs.root_directory = Some(inode));
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Mkdir = 39,
    fn sys_mkdir(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, mode, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let (dir, basename) = path::resolve_parent(&pathname).ok_or(Errno::NOENT)?;
        if basename.is_empty() {
            return Err(Errno::NOENT);
        }
        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(Errno::ACCESS);
        }
        if dir.lookup(basename)?.is_some() {
            return Err(Errno::EXIST);
        }

        dir.create_directory(basename, mode as u16)?;
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Mknod = 14,
    fn sys_mknod(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, mode, dev) = ctx.args();
        if !task::is_superuser() {
            return Err(Errno::PERM);
        }

        let pathname = uaccess::read_pathname(path_ptr);
        let (dir, basename) = path::resolve_parent(&pathname).ok_or(Errno::NOENT)?;
        if basename.is_empty() {
            return Err(Errno::NOENT);
        }
        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(Errno::ACCESS);
        }
        if dir.lookup(basename)?.is_some() {
            return Err(Errno::EXIST);
        }

        let type_bits = mode as u16 & InodeMode::TYPE_MASK;
        if type_bits != 0o060000 && type_bits != 0o020000 {
            use crate::error::Errno;
            return Err(Errno::INVAL);
        }
        let perm_bits = mode as u16 & InodeMode::FLAGS_MASK;
        dir.create_device(basename, type_bits, perm_bits, dev as u16)?;
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Rmdir = 40,
    fn sys_rmdir(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let (dir, basename) = path::resolve_parent(&pathname).ok_or(Errno::NOENT)?;
        if basename.is_empty() {
            return Err(Errno::NOENT);
        }
        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(Errno::ACCESS);
        }

        let inum = dir.lookup(basename)?.ok_or(Errno::NOENT)?;
        let inode = resolve_inode(InodeId::new(dir.id.device, inum));
        if inode.file_type() != InodeType::Directory {
            return Err(Errno::NOTDIR);
        }
        if !inode.is_empty_directory()? {
            return Err(Errno::NOTEMPTY);
        }

        dir.remove_entry(basename)?;

        let now = time::current_time();
        {
            let mut inner = inode.inner.lock();
            inner.disk_inode.link_count = 0;
            inner.touch_change(now);
        }
        {
            let mut inner = dir.inner.lock();
            inner.disk_inode.link_count -= 1;
            inner.touch_modified(now);
        }

        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Chmod = 15,
    fn sys_chmod(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, mode, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(Errno::NOENT)?;
        let euid = task::with_current(|inner| inner.identity.euid);
        if euid != inode.inner.lock().disk_inode.user_id && !task::is_superuser() {
            return Err(Errno::ACCESS);
        }

        let mut inner = inode.inner.lock();
        inner.disk_inode.mode = InodeMode(
            (mode as u16 & InodeMode::FLAGS_MASK)
                | (inner.disk_inode.mode.0 & !InodeMode::FLAGS_MASK),
        );
        inner.touch_change(time::current_time());
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Chown = 16,
    fn sys_chown(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, uid, gid) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        if !task::is_superuser() {
            return Err(Errno::ACCESS);
        }

        let inode = path::resolve_path(&pathname).ok_or(Errno::NOENT)?;
        let mut inner = inode.inner.lock();
        inner.disk_inode.user_id = uid as u16;
        inner.disk_inode.group_id = gid as u8;
        inner.touch_change(time::current_time());
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Access = 33,
    fn sys_access(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, mode, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(Errno::ACCESS)?;
        let mask = AccessMask::from_bits_truncate(mode as u16 & 0o7);

        let (uid, gid) = task::with_current(|inner| (inner.identity.uid, inner.identity.gid));

        if path::check_permission_as(&inode, mask, uid, gid) {
            Ok(0)
        } else {
            Err(Errno::ACCESS)
        }
    }
);

define_syscall_handler!(
    Syscall::Utime = 30,
    fn sys_utime(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, times_ptr, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(Errno::NOENT)?;

        let (actime, modtime) = if times_ptr != 0 {
            let times = uaccess::read_struct(times_ptr as *const TimeUpdate);
            (times.access_time, times.modification_time)
        } else {
            let now = time::current_time();
            (now, now)
        };

        let mut inner = inode.inner.lock();
        inner.set_access_and_modified(actime, modtime, time::current_time());
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Rename = 38,
    fn sys_rename(ctx: &mut SyscallContext) -> Result<u32> {
        let (old_ptr, new_ptr, _) = ctx.args();
        let oldname = uaccess::read_pathname(old_ptr);
        let newname = uaccess::read_pathname(new_ptr);

        let (old_dir, old_base) = path::resolve_parent(&oldname).ok_or(Errno::NOENT)?;
        if old_base.is_empty() || old_base == "." || old_base == ".." {
            return Err(Errno::INVAL);
        }
        let (new_dir, new_base) = path::resolve_parent(&newname).ok_or(Errno::NOENT)?;
        if new_base.is_empty() || new_base == "." || new_base == ".." {
            return Err(Errno::INVAL);
        }

        if old_dir.id.device != new_dir.id.device {
            return Err(Errno::XDEV);
        }
        if !path::check_permission(&old_dir, AccessMask::MAY_WRITE) {
            return Err(Errno::ACCESS);
        }
        if !path::check_permission(&new_dir, AccessMask::MAY_WRITE) {
            return Err(Errno::ACCESS);
        }

        let old_inum = old_dir.lookup(old_base)?.ok_or(Errno::NOENT)?;
        let old_inode = resolve_inode(InodeId::new(old_dir.id.device, old_inum));
        let old_is_dir = old_inode.file_type() == InodeType::Directory;
        let same_parent = old_dir.id.inode_number == new_dir.id.inode_number;

        // POSIX: `rename(a, a)` where both names resolve to the same path is
        // a successful no-op. Likewise when both names point at the same
        // inode (hard-linked aliases).
        if same_parent && old_base == new_base {
            return Ok(0);
        }
        if let Some(existing) = new_dir.lookup(new_base)? {
            if existing == old_inum {
                return Ok(0);
            }
        }

        // Refuse to move a directory into itself or any of its descendants.
        if old_is_dir && !same_parent {
            let mut cursor = new_dir.clone();
            for _ in 0..MAX_PATH_DEPTH {
                if cursor.id.device == old_inode.id.device && cursor.id.inode_number == old_inum {
                    return Err(Errno::INVAL);
                }
                let parent_inum = cursor.lookup("..")?.ok_or(Errno::IO)?;
                if parent_inum == cursor.id.inode_number {
                    break;
                }
                cursor = resolve_inode(InodeId::new(cursor.id.device, parent_inum));
            }
        }

        // If the destination exists, displace it first.
        if let Some(new_inum) = new_dir.lookup(new_base)? {
            let new_inode = resolve_inode(InodeId::new(new_dir.id.device, new_inum));
            let new_is_dir = new_inode.file_type() == InodeType::Directory;

            if old_is_dir && !new_is_dir {
                return Err(Errno::NOTDIR);
            }
            if !old_is_dir && new_is_dir {
                return Err(Errno::ISDIR);
            }
            if new_is_dir && !new_inode.is_empty_directory()? {
                return Err(Errno::NOTEMPTY);
            }

            new_dir.remove_entry(new_base)?;

            let now = time::current_time();
            if new_is_dir {
                {
                    let mut inner = new_inode.inner.lock();
                    inner.disk_inode.link_count = 0;
                    inner.touch_change(now);
                }
                {
                    let mut parent = new_dir.inner.lock();
                    parent.disk_inode.link_count -= 1;
                    parent.touch_modified(now);
                }
            } else {
                let mut inner = new_inode.inner.lock();
                inner.disk_inode.link_count -= 1;
                inner.touch_change(now);
            }
        }

        // Splice old into the new directory, then remove the old name.
        new_dir.add_entry(new_base, old_inum)?;
        old_dir.remove_entry(old_base)?;

        let now = time::current_time();

        // Cross-parent directory rename rewires `..` and shifts the
        // per-directory link count from old parent to new.
        if old_is_dir && !same_parent {
            old_inode.remove_entry("..")?;
            old_inode.add_entry("..", new_dir.id.inode_number)?;
            {
                let mut p = old_dir.inner.lock();
                p.disk_inode.link_count -= 1;
                p.touch_modified(now);
            }
            {
                let mut p = new_dir.inner.lock();
                p.disk_inode.link_count += 1;
                p.touch_modified(now);
            }
        }

        old_inode.inner.lock().touch_change(now);
        Ok(0)
    }
);

/// Upper bound used while walking `..` to detect rename loops on a
/// malformed filesystem. 256 is well past any sensible nesting depth.
const MAX_PATH_DEPTH: usize = 256;
