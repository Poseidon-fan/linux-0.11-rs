//! Path and inode syscall handlers (stat, link, mkdir, chmod, chdir, etc.).

use core::mem;

use user_lib::syscall::fs::Stat;

use crate::{
    define_syscall_handler,
    error::{EACCES, EEXIST, EISDIR, ENOENT, ENOTDIR, ENOTEMPTY, EPERM, EXDEV, Result},
    fs::{
        InodeMode, InodeType, get_inode,
        minix::InodeId,
        path::{self, AccessMask},
    },
    segment::uaccess,
    syscall::context::SyscallContext,
    task, time,
};

define_syscall_handler!(
    user_lib::syscall::NR_STAT = 18,
    fn sys_stat(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, buf_ptr, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);
        let inode = path::resolve_path(&pathname).ok_or(ENOENT)?;
        let stat = inode.stat();
        let bytes = unsafe {
            core::slice::from_raw_parts(&stat as *const Stat as *const u8, mem::size_of::<Stat>())
        };
        uaccess::write_bytes(bytes, buf_ptr as *mut u8);
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_LINK = 9,
    fn sys_link(ctx: &mut SyscallContext) -> Result<u32> {
        let (oldname_ptr, newname_ptr, _) = ctx.args();
        let oldname = uaccess::read_pathname(oldname_ptr);
        let newname = uaccess::read_pathname(newname_ptr);

        let old_inode = path::resolve_path(&oldname).ok_or(ENOENT)?;
        if old_inode.file_type() == InodeType::Directory {
            return Err(EPERM);
        }

        let (dir, basename) = path::resolve_parent(&newname).ok_or(EACCES)?;
        if basename.is_empty() {
            return Err(EACCES);
        }

        if dir.id.device != old_inode.id.device {
            return Err(EXDEV);
        }

        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(EACCES);
        }

        if dir.lookup(basename)?.is_some() {
            return Err(EEXIST);
        }

        dir.add_entry(basename, old_inode.id.inode_number)?;

        let mut inner = old_inode.inner.lock();
        inner.disk_inode.link_count += 1;
        inner.change_time = time::current_time();
        inner.is_dirty = true;

        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_UNLINK = 10,
    fn sys_unlink(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let (dir, basename) = path::resolve_parent(&pathname).ok_or(ENOENT)?;
        if basename.is_empty() {
            return Err(ENOENT);
        }
        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(EACCES);
        }

        let inum = dir.lookup(basename)?.ok_or(ENOENT)?;
        let inode = get_inode(InodeId {
            device: dir.id.device,
            inode_number: inum,
        });
        if inode.file_type() == InodeType::Directory {
            return Err(EISDIR);
        }

        dir.remove_entry(basename)?;

        let mut inner = inode.inner.lock();
        inner.disk_inode.link_count -= 1;
        inner.change_time = time::current_time();
        inner.is_dirty = true;

        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_CHDIR = 12,
    fn sys_chdir(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(ENOENT)?;
        if inode.file_type() != InodeType::Directory {
            return Err(ENOTDIR);
        }
        if !path::check_permission(&inode, AccessMask::MAY_EXEC) {
            return Err(EACCES);
        }

        task::with_current(|inner| inner.fs.current_directory = Some(inode));
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_CHROOT = 61,
    fn sys_chroot(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(ENOENT)?;
        if inode.file_type() != InodeType::Directory {
            return Err(ENOTDIR);
        }

        task::with_current(|inner| inner.fs.root_directory = Some(inode));
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_MKDIR = 39,
    fn sys_mkdir(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, mode, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let (dir, basename) = path::resolve_parent(&pathname).ok_or(ENOENT)?;
        if basename.is_empty() {
            return Err(ENOENT);
        }
        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(EACCES);
        }
        if dir.lookup(basename)?.is_some() {
            return Err(EEXIST);
        }

        dir.create_directory(basename, mode as u16)?;
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_MKNOD = 14,
    fn sys_mknod(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, mode, dev) = ctx.args();
        if !task::is_superuser() {
            return Err(EPERM);
        }

        let pathname = uaccess::read_pathname(path_ptr);
        let (dir, basename) = path::resolve_parent(&pathname).ok_or(ENOENT)?;
        if basename.is_empty() {
            return Err(ENOENT);
        }
        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(EACCES);
        }
        if dir.lookup(basename)?.is_some() {
            return Err(EEXIST);
        }

        let type_bits = mode as u16 & InodeMode::TYPE_MASK;
        if type_bits != 0o060000 && type_bits != 0o020000 {
            use crate::error::EINVAL;
            return Err(EINVAL);
        }
        let perm_bits = mode as u16 & InodeMode::FLAGS_MASK;
        dir.create_device(basename, type_bits, perm_bits, dev as u16)?;
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_RMDIR = 40,
    fn sys_rmdir(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let (dir, basename) = path::resolve_parent(&pathname).ok_or(ENOENT)?;
        if basename.is_empty() {
            return Err(ENOENT);
        }
        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(EACCES);
        }

        let inum = dir.lookup(basename)?.ok_or(ENOENT)?;
        let inode = get_inode(InodeId {
            device: dir.id.device,
            inode_number: inum,
        });
        if inode.file_type() != InodeType::Directory {
            return Err(ENOTDIR);
        }
        if !inode.is_empty_directory()? {
            return Err(ENOTEMPTY);
        }

        dir.remove_entry(basename)?;

        let now = time::current_time();
        {
            let mut inner = inode.inner.lock();
            inner.disk_inode.link_count = 0;
            inner.change_time = now;
            inner.is_dirty = true;
        }
        {
            let mut inner = dir.inner.lock();
            inner.disk_inode.link_count -= 1;
            inner.change_time = now;
            inner.is_dirty = true;
        }

        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_CHMOD = 15,
    fn sys_chmod(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, mode, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(ENOENT)?;
        let euid = task::with_current(|inner| inner.identity.euid);
        if euid != inode.inner.lock().disk_inode.user_id && !task::is_superuser() {
            return Err(EACCES);
        }

        let mut inner = inode.inner.lock();
        inner.disk_inode.mode = InodeMode(
            (mode as u16 & InodeMode::FLAGS_MASK)
                | (inner.disk_inode.mode.0 & !InodeMode::FLAGS_MASK),
        );
        inner.is_dirty = true;
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_CHOWN = 16,
    fn sys_chown(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, uid, gid) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        if !task::is_superuser() {
            return Err(EACCES);
        }

        let inode = path::resolve_path(&pathname).ok_or(ENOENT)?;
        let mut inner = inode.inner.lock();
        inner.disk_inode.user_id = uid as u16;
        inner.disk_inode.group_id = gid as u8;
        inner.is_dirty = true;
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_ACCESS = 33,
    fn sys_access(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, mode, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(EACCES)?;
        let mask = AccessMask::from_bits_truncate(mode as u16 & 0o7);

        let (uid, gid) = task::with_current(|inner| (inner.identity.uid, inner.identity.gid));

        if path::check_permission_as(&inode, mask, uid, gid) {
            Ok(0)
        } else {
            Err(EACCES)
        }
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_UTIME = 30,
    fn sys_utime(ctx: &mut SyscallContext) -> Result<u32> {
        let (path_ptr, times_ptr, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(ENOENT)?;

        let (actime, modtime) = if times_ptr != 0 {
            let base = times_ptr as *const u32;
            let actime = uaccess::read_u32(base);
            let modtime = uaccess::read_u32(unsafe { base.add(1) });
            (actime, modtime)
        } else {
            let now = time::current_time();
            (now, now)
        };

        let mut inner = inode.inner.lock();
        inner.access_time = actime;
        inner.disk_inode.modification_time = modtime;
        inner.is_dirty = true;
        Ok(0)
    }
);
