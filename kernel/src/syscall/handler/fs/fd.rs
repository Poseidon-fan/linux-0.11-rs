//! File descriptor syscall handlers (open, read, write, close, dup, fcntl, pipe, etc.).

use alloc::{sync::Arc, vec};
use core::mem;

use user_lib::syscall::{
    NR_OPEN,
    fs::{
        AccessMode, F_DUPFD, F_GETFD, F_GETFL, F_SETFD, F_SETFL, OpenFlags, OpenOptions, Stat,
        Whence,
    },
};

use crate::{
    define_syscall_handler,
    fs::{
        InodeType,
        file::{BlockDeviceFile, CharDeviceFile, File, InodeFile, PipeFile},
        get_inode,
        minix::InodeId,
        path::{self, AccessMask},
    },
    segment::uaccess,
    syscall::{
        EACCES, EBADF, EEXIST, EINVAL, EISDIR, EMFILE, ENOENT, EPERM, SYSCALL_TABLE,
        context::SyscallContext,
    },
    task::{self, TASK_OPEN_FILES_LIMIT},
    time,
};

define_syscall_handler!(
    user_lib::syscall::NR_OPEN = 5,
    fn sys_open(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (path_ptr, raw_flags, mode) = ctx.args();
        let pathname = uaccess::read_pathname(path_ptr);
        let flags = OpenFlags::from_raw(raw_flags);
        let (access_mode, open_options) = flags.into_parts().ok_or(EINVAL)?;
        let effective_access_mode = if access_mode == AccessMode::ReadOnly
            && open_options.contains(OpenOptions::TRUNCATE)
        {
            AccessMode::WriteOnly
        } else {
            access_mode
        };

        let (dir, basename) = path::resolve_parent(&pathname).ok_or(ENOENT)?;

        let inode = if basename.is_empty() {
            if effective_access_mode != AccessMode::ReadOnly
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
                    if file_type == InodeType::Directory
                        && effective_access_mode != AccessMode::ReadOnly
                    {
                        return Err(EPERM);
                    }
                    let required = match effective_access_mode {
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
                Arc::new(InodeFile::new(inode, effective_access_mode, open_options))
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
    user_lib::syscall::NR_CREAT = 8,
    fn sys_creat(ctx: &mut SyscallContext) -> Result<u32, u32> {
        // creat(path, mode) == open(path, O_WRONLY | O_CREAT | O_TRUNC, mode)
        // path_ptr is already in ctx.ebx, just rewrite flags and mode args.
        let (_, mode, _) = ctx.args();
        ctx.ecx = AccessMode::WriteOnly as u32
            | OpenOptions::CREATE.bits()
            | OpenOptions::TRUNCATE.bits();
        ctx.edx = mode;
        SYSCALL_TABLE[NR_OPEN as usize](ctx)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_READ = 3,
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
    user_lib::syscall::NR_WRITE = 4,
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
    user_lib::syscall::NR_CLOSE = 6,
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
    user_lib::syscall::NR_LSEEK = 19,
    fn sys_lseek(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, offset, whence) = ctx.args();
        let whence = Whence::from_raw(whence).ok_or(EINVAL)?;
        let file = get_file(fd)?;
        file.seek(offset as i32, whence).map(|pos| pos as u32)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_DUP = 41,
    fn sys_dup(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, _, _) = ctx.args();
        let file = get_file(fd)?;
        let new_fd = task::with_current(|inner| inner.fs.add_file(file)).ok_or(EMFILE)?;
        Ok(new_fd as u32)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_DUP2 = 63,
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
    user_lib::syscall::NR_FSTAT = 28,
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
    user_lib::syscall::NR_IOCTL = 54,
    fn sys_ioctl(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, cmd, arg) = ctx.args();
        let file = get_file(fd)?;
        file.ioctl(cmd, arg)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_FCNTL = 55,
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
    user_lib::syscall::NR_PIPE = 42,
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
