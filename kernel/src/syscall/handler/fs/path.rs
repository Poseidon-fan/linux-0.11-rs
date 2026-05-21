//! Path and inode syscall handlers (stat, link, mkdir, chmod, chdir, etc.).

use core::mem;

use user_lib::syscall::{Syscall, fs::Stat};

use crate::{
    define_syscall_handler,
    error::{Errno, Result},
    fs::{
        InodeMode, InodeType,
        minix::InodeId,
        path::{self, AccessMask},
        resolve_inode,
    },
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
        let bytes = unsafe {
            core::slice::from_raw_parts(&stat as *const Stat as *const u8, mem::size_of::<Stat>())
        };
        uaccess::write_bytes(bytes, buf_ptr as *mut u8);
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
        inner.change_time = time::current_time();
        inner.is_dirty = true;

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
        inner.change_time = time::current_time();
        inner.is_dirty = true;

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
        inner.is_dirty = true;
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
        inner.is_dirty = true;
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
