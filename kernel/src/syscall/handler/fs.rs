//! Filesystem-related syscall handlers (open, read, write, close, etc.).

use alloc::{sync::Arc, vec};
use core::mem;

use linkme::distributed_slice;
use user_lib::fs::{
    AccessMode, F_DUPFD, F_GETFD, F_GETFL, F_SETFD, F_SETFL, OpenFlags, OpenOptions, Stat, Whence,
};

use crate::{
    define_syscall_handler,
    driver::{self, blk::hd},
    fs::{
        self, buffer,
        file::{BlockDeviceFile, CharDeviceFile, File, InodeFile, PipeFile},
        get_inode,
        layout::{InodeMode, InodeType, ROOT_INODE_NUMBER},
        minix::{INODE_TABLE, InodeId, MinixFileSystem},
        mount::{MOUNT_TABLE, Mount},
        path::{self, AccessMask},
    },
    segment::uaccess,
    syscall::{
        EACCES, EBADF, EBUSY, EEXIST, EINVAL, EISDIR, EMFILE, ENOENT, ENOTBLK, ENOTDIR, ENOTEMPTY,
        EPERM, EXDEV, SYSCALL_TABLE, context::SyscallContext,
    },
    task::{self, TASK_OPEN_FILES_LIMIT},
    time,
};

define_syscall_handler!(
    user_lib::NR_SETUP = 0,
    fn sys_setup(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (drive_info_addr, _, _) = ctx.args();
        hd::setup_from_bios(drive_info_addr as *const u8).map_err(|()| EPERM)?;
        fs::mount_root();
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::NR_OPEN = 5,
    fn sys_open(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (path_ptr, raw_flags, mode) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);
        let flags = OpenFlags::from_raw(raw_flags);
        let (access_mode, open_options) = flags.into_parts().ok_or(EINVAL)?;

        let (dir, basename) = path::resolve_parent(&pathname).ok_or(ENOENT)?;

        let inode = if basename.is_empty() {
            if access_mode != AccessMode::ReadOnly
                || open_options.intersects(OpenOptions::CREATE | OpenOptions::TRUNCATE)
            {
                return Err(EISDIR);
            }
            dir
        } else {
            match dir.lookup(basename)? {
                None if open_options.contains(OpenOptions::CREATE) => {
                    if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
                        return Err(EACCES);
                    }
                    dir.create_file(basename, mode as u16)?
                }
                None => return Err(ENOENT),
                Some(inum) => {
                    if open_options.contains(OpenOptions::EXCLUSIVE) {
                        return Err(EEXIST);
                    }
                    let inode = get_inode(InodeId {
                        device: dir.id.device,
                        inode_number: inum,
                    });
                    let file_type = inode.file_type();
                    if file_type == InodeType::Directory && access_mode != AccessMode::ReadOnly {
                        return Err(EPERM);
                    }
                    let required = match access_mode {
                        AccessMode::ReadOnly => AccessMask::MAY_READ,
                        AccessMode::WriteOnly => AccessMask::MAY_WRITE,
                        AccessMode::ReadWrite => AccessMask::MAY_READ | AccessMask::MAY_WRITE,
                    };
                    if !path::check_permission(&inode, required) {
                        return Err(EPERM);
                    }
                    inode.inner.lock().access_time = time::current_time();
                    if open_options.contains(OpenOptions::TRUNCATE) {
                        inode.truncate();
                    }
                    inode
                }
            }
        };

        let file_type = inode.file_type();
        let file: Arc<dyn File> = match file_type {
            InodeType::Regular | InodeType::Directory => {
                Arc::new(InodeFile::new(inode, access_mode, open_options))
            }
            InodeType::CharacterDevice => Arc::new(CharDeviceFile::new(inode)),
            InodeType::BlockDevice => Arc::new(BlockDeviceFile::new(inode)),
            _ => return Err(EPERM),
        };

        let fd = task::with_current(|inner| inner.fs.add_file(file)).ok_or(EMFILE)?;

        Ok(fd as u32)
    }
);

define_syscall_handler!(
    user_lib::NR_READ = 3,
    fn sys_read(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, buf_ptr, count) = ctx.args();
        let file = get_file(fd)?;

        let mut kernel_buf = vec![0u8; count as usize];
        let bytes_read = file.read(&mut kernel_buf)?;
        uaccess::write_bytes(&kernel_buf[..bytes_read], buf_ptr as *mut u8);

        Ok(bytes_read as u32)
    }
);

define_syscall_handler!(
    user_lib::NR_WRITE = 4,
    fn sys_write(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, buf_ptr, count) = ctx.args();
        let file = get_file(fd)?;

        let mut kernel_buf = vec![0u8; count as usize];
        uaccess::read_bytes(buf_ptr as *const u8, &mut kernel_buf);
        let bytes_written = file.write(&kernel_buf)?;

        Ok(bytes_written as u32)
    }
);

define_syscall_handler!(
    user_lib::NR_CLOSE = 6,
    fn sys_close(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, _, _) = ctx.args();
        task::with_current(|inner| {
            let slot = inner.fs.open_files.get_mut(fd as usize).ok_or(EBADF)?;
            if slot.is_none() {
                return Err(EBADF);
            }
            *slot = None;
            Ok(0)
        })
    }
);

define_syscall_handler!(
    user_lib::NR_LINK = 9,
    fn sys_link(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (oldname_ptr, newname_ptr, _) = ctx.args();
        let oldname = uaccess::read_pathname(oldname_ptr);
        let newname = uaccess::read_pathname(newname_ptr);

        // Resolve old file
        let old_inode = path::resolve_path(&oldname).ok_or(ENOENT)?;
        if old_inode.file_type() == InodeType::Directory {
            return Err(EPERM);
        }

        // Resolve parent directory of new path
        let (dir, basename) = path::resolve_parent(&newname).ok_or(EACCES)?;
        if basename.is_empty() {
            return Err(EACCES);
        }

        // Must be same device
        if dir.id.device != old_inode.id.device {
            return Err(EXDEV);
        }

        // Check write permission on parent directory
        if !path::check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(EACCES);
        }

        // New name must not already exist
        if dir.lookup(basename)?.is_some() {
            return Err(EEXIST);
        }

        // Add directory entry pointing to old inode
        dir.add_entry(basename, old_inode.id.inode_number)?;

        // Increment link count
        let mut inner = old_inode.inner.lock();
        inner.disk_inode.link_count += 1;
        inner.change_time = time::current_time();
        inner.is_dirty = true;

        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::NR_UNLINK = 10,
    fn sys_unlink(ctx: &mut SyscallContext) -> Result<u32, u32> {
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
    user_lib::NR_CHDIR = 12,
    fn sys_chdir(ctx: &mut SyscallContext) -> Result<u32, u32> {
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
    user_lib::NR_MKDIR = 39,
    fn sys_mkdir(ctx: &mut SyscallContext) -> Result<u32, u32> {
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
    user_lib::NR_MKNOD = 14,
    fn sys_mknod(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (path_ptr, mode, dev) = ctx.args();
        if !task::is_super() {
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
            return Err(EINVAL);
        }
        let perm_bits = mode as u16 & InodeMode::FLAGS_MASK;
        dir.create_device(basename, type_bits, perm_bits, dev as u16)?;
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::NR_RMDIR = 40,
    fn sys_rmdir(ctx: &mut SyscallContext) -> Result<u32, u32> {
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
    user_lib::NR_STAT = 18,
    fn sys_stat(ctx: &mut SyscallContext) -> Result<u32, u32> {
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
    user_lib::NR_FSTAT = 28,
    fn sys_fstat(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, buf_ptr, _) = ctx.args();
        let file = get_file(fd)?;
        let stat = file.stat()?;
        let bytes = unsafe {
            core::slice::from_raw_parts(&stat as *const Stat as *const u8, mem::size_of::<Stat>())
        };
        uaccess::write_bytes(bytes, buf_ptr as *mut u8);
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::NR_LSEEK = 19,
    fn sys_lseek(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, offset, whence) = ctx.args();
        let whence = Whence::from_raw(whence).ok_or(EINVAL)?;
        let file = get_file(fd)?;
        file.seek(offset as i32, whence).map(|pos| pos as u32)
    }
);

define_syscall_handler!(
    user_lib::NR_DUP = 41,
    fn sys_dup(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, _, _) = ctx.args();
        let file = get_file(fd)?;
        let new_fd = task::with_current(|inner| inner.fs.add_file(file)).ok_or(EMFILE)?;
        Ok(new_fd as u32)
    }
);

define_syscall_handler!(
    user_lib::NR_DUP2 = 63,
    fn sys_dup2(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (oldfd, newfd, _) = ctx.args();
        if oldfd == newfd {
            // Verify oldfd is valid, then return it unchanged.
            get_file(oldfd)?;
            return Ok(newfd);
        }
        let file = get_file(oldfd)?;
        task::with_current(|inner| {
            let slot = inner.fs.open_files.get_mut(newfd as usize).ok_or(EBADF)?;
            *slot = Some(file);
            Ok(newfd)
        })
    }
);

define_syscall_handler!(
    user_lib::NR_CREAT = 8,
    fn sys_creat(ctx: &mut SyscallContext) -> Result<u32, u32> {
        // creat(path, mode) == open(path, O_WRONLY | O_CREAT | O_TRUNC, mode)
        // path_ptr is already in ctx.ebx, just rewrite flags and mode args.
        let (_, mode, _) = ctx.args();
        ctx.ecx = user_lib::fs::AccessMode::WriteOnly as u32
            | user_lib::fs::OpenOptions::CREATE.bits()
            | user_lib::fs::OpenOptions::TRUNCATE.bits();
        ctx.edx = mode;
        SYSCALL_TABLE[user_lib::NR_OPEN as usize](ctx)
    }
);

define_syscall_handler!(
    user_lib::NR_CHROOT = 61,
    fn sys_chroot(ctx: &mut SyscallContext) -> Result<u32, u32> {
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
    user_lib::NR_CHMOD = 15,
    fn sys_chmod(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (path_ptr, mode, _) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        let inode = path::resolve_path(&pathname).ok_or(ENOENT)?;
        let euid = task::with_current(|inner| inner.identity.euid);
        if euid != inode.inner.lock().disk_inode.user_id && !task::is_super() {
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
    user_lib::NR_CHOWN = 16,
    fn sys_chown(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (path_ptr, uid, gid) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);

        if !task::is_super() {
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
    user_lib::NR_SYNC = 36,
    fn sys_sync(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        fs::sync();
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::NR_ACCESS = 33,
    fn sys_access(ctx: &mut SyscallContext) -> Result<u32, u32> {
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
    user_lib::NR_UTIME = 30,
    fn sys_utime(ctx: &mut SyscallContext) -> Result<u32, u32> {
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

define_syscall_handler!(
    user_lib::NR_MOUNT = 21,
    fn sys_mount(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (dev_name_ptr, dir_name_ptr, _rw_flag) = ctx.args();
        let dev_name = uaccess::read_pathname(dev_name_ptr);
        let dir_name = uaccess::read_pathname(dir_name_ptr);

        // Resolve the device node and extract its device number.
        let dev_inode = path::resolve_path(&dev_name).ok_or(ENOENT)?;
        if dev_inode.file_type() != InodeType::BlockDevice {
            return Err(EPERM);
        }
        let dev = dev_inode.device_number();
        drop(dev_inode);

        // Resolve the mount-point directory.
        let dir_inode = path::resolve_path(&dir_name).ok_or(ENOENT)?;
        if dir_inode.file_type() != InodeType::Directory {
            return Err(EPERM);
        }
        if dir_inode.id.inode_number == ROOT_INODE_NUMBER {
            return Err(EBUSY);
        }

        let mut mt = MOUNT_TABLE.lock();
        if mt.get_fs(dev).is_some() {
            return Err(EBUSY);
        }
        if mt.is_mount_point(dir_inode.id) {
            return Err(EPERM);
        }

        // Load the filesystem from the target device.
        let new_fs = MinixFileSystem::open(dev).ok_or(EBUSY)?;
        let root_inode = INODE_TABLE.lock().get_inode_raw(
            InodeId {
                device: dev,
                inode_number: ROOT_INODE_NUMBER,
            },
            &new_fs,
        );

        mt.insert(Arc::new(Mount {
            device: dev,
            file_system: new_fs,
            root_inode,
            mount_point_inode: Some(dir_inode),
        }))
        .ok_or(EBUSY)?;

        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::NR_UMOUNT = 22,
    fn sys_umount(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (dev_name_ptr, _, _) = ctx.args();
        let dev_name = uaccess::read_pathname(dev_name_ptr);

        // Resolve the device node and extract its device number.
        let dev_inode = path::resolve_path(&dev_name).ok_or(ENOENT)?;
        if dev_inode.file_type() != InodeType::BlockDevice {
            return Err(ENOTBLK);
        }
        let dev = dev_inode.device_number();
        drop(dev_inode);

        if dev == driver::root_dev() {
            return Err(EBUSY);
        }
        if MOUNT_TABLE.lock().get_fs(dev).is_none() {
            return Err(ENOENT);
        }

        let mut inode_table = INODE_TABLE.lock();
        if inode_table.has_active_inodes(dev) {
            return Err(EBUSY);
        }
        inode_table.evict_device(dev);
        drop(inode_table);

        buffer::sync_dev(dev);
        MOUNT_TABLE.lock().remove_by_device(dev);
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::NR_IOCTL = 54,
    fn sys_ioctl(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, cmd, arg) = ctx.args();
        let file = get_file(fd)?;
        file.ioctl(cmd, arg)
    }
);

define_syscall_handler!(
    user_lib::NR_FCNTL = 55,
    fn sys_fcntl(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, cmd, arg) = ctx.args();
        let file = get_file(fd)?;

        match cmd {
            F_DUPFD => task::with_current(|inner| {
                let new_fd = (arg as usize..TASK_OPEN_FILES_LIMIT)
                    .find(|&i| inner.fs.open_files[i].is_none())
                    .ok_or(EMFILE)?;
                inner.fs.open_files[new_fd] = Some(Arc::clone(&file));
                inner.fs.close_on_exec &= !(1 << new_fd);
                Ok(new_fd as u32)
            }),

            F_GETFD => {
                let cloexec = task::with_current(|inner| (inner.fs.close_on_exec >> fd) & 1);
                Ok(cloexec)
            }

            F_SETFD => {
                task::with_current(|inner| {
                    if arg & 1 != 0 {
                        inner.fs.close_on_exec |= 1 << fd;
                    } else {
                        inner.fs.close_on_exec &= !(1 << fd);
                    }
                });
                Ok(0)
            }

            F_GETFL | F_SETFL => Ok(0),

            _ => Err(EINVAL),
        }
    }
);

define_syscall_handler!(
    user_lib::NR_PIPE = 42,
    fn sys_pipe(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fildes_ptr, _, _) = ctx.args();
        let (reader, writer) = PipeFile::create_pair()?;

        let (fd0, fd1) = task::with_current(|inner| {
            let fd0 = inner.fs.add_file(reader as Arc<dyn File>).ok_or(EMFILE)?;
            match inner.fs.add_file(writer as Arc<dyn File>) {
                Some(fd1) => Ok((fd0, fd1)),
                None => {
                    inner.fs.open_files[fd0] = None;
                    Err(EMFILE)
                }
            }
        })?;

        uaccess::write_u32(fd0 as u32, fildes_ptr as *mut u32);
        uaccess::write_u32(fd1 as u32, unsafe { (fildes_ptr as *mut u32).add(1) });
        Ok(0)
    }
);

/// Retrieve the file object for a given fd, or `Err(EBADF)`.
fn get_file(fd: u32) -> Result<Arc<dyn File>, u32> {
    task::with_current(|inner| inner.fs.open_files.get(fd as usize).cloned().flatten()).ok_or(EBADF)
}
