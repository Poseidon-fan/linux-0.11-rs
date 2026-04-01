use alloc::sync::Arc;
use alloc::vec;
use linkme::distributed_slice;
use user_lib::fs::{AccessMode, OpenFlags, OpenOptions, Whence};

use crate::{
    define_syscall_handler,
    driver::blk::hd,
    fs::{
        self,
        file::{File, InodeFile},
        get_inode,
        layout::InodeType,
        minix::InodeId,
        path::{self, AccessMask, check_permission},
    },
    segment,
    syscall::{
        EACCES, EBADF, EEXIST, EINVAL, EISDIR, EMFILE, ENOENT, ENOTDIR, EPERM, SYSCALL_TABLE,
        context::SyscallContext,
    },
    task, time,
};

define_syscall_handler!(
    user_lib::NR_SETUP = 0,
    fn sys_setup(ctx: &SyscallContext) -> Result<u32, u32> {
        let (drive_info_addr, _, _) = ctx.args();
        hd::setup_from_bios(drive_info_addr as *const u8).map_err(|()| EPERM)?;
        fs::mount_root();
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::NR_OPEN = 5,
    fn sys_open(ctx: &SyscallContext) -> Result<u32, u32> {
        let (path_ptr, raw_flags, mode) = ctx.args();
        let pathname = segment::get_fs_string(path_ptr as *const u8, 256);
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
                    if !check_permission(&dir, AccessMask::MAY_WRITE) {
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
                    let file_type = inode.inner.lock().disk_inode.mode.file_type();
                    if file_type == InodeType::Directory && access_mode != AccessMode::ReadOnly {
                        return Err(EPERM);
                    }
                    let required = match access_mode {
                        AccessMode::ReadOnly => AccessMask::MAY_READ,
                        AccessMode::WriteOnly => AccessMask::MAY_WRITE,
                        AccessMode::ReadWrite => AccessMask::MAY_READ | AccessMask::MAY_WRITE,
                    };
                    if !check_permission(&inode, required) {
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

        let file_type = inode.inner.lock().disk_inode.mode.file_type();
        let file: Arc<dyn File> = match file_type {
            InodeType::Regular | InodeType::Directory => {
                Arc::new(InodeFile::new(inode, access_mode, open_options))
            }
            // TODO: BlockDevice / CharacterDevice / Fifo
            _ => return Err(EPERM),
        };

        let fd = task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.fs.add_file(file))
            .ok_or(EMFILE)?;

        Ok(fd as u32)
    }
);

define_syscall_handler!(
    user_lib::NR_READ = 3,
    fn sys_read(ctx: &SyscallContext) -> Result<u32, u32> {
        let (fd, buf_ptr, count) = ctx.args();
        let file = get_file(fd)?;

        let mut kernel_buf = vec![0u8; count as usize];
        let bytes_read = file.read(&mut kernel_buf)?;
        segment::put_fs_bytes(&kernel_buf[..bytes_read], buf_ptr as *mut u8);

        Ok(bytes_read as u32)
    }
);

define_syscall_handler!(
    user_lib::NR_WRITE = 4,
    fn sys_write(ctx: &SyscallContext) -> Result<u32, u32> {
        let (fd, buf_ptr, count) = ctx.args();
        let file = get_file(fd)?;

        let mut kernel_buf = vec![0u8; count as usize];
        segment::get_fs_bytes(buf_ptr as *const u8, &mut kernel_buf);
        let bytes_written = file.write(&kernel_buf)?;

        Ok(bytes_written as u32)
    }
);

define_syscall_handler!(
    user_lib::NR_CLOSE = 6,
    fn sys_close(ctx: &SyscallContext) -> Result<u32, u32> {
        let (fd, _, _) = ctx.args();
        task::current_task().pcb.inner.exclusive(|inner| {
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
    user_lib::NR_UNLINK = 10,
    fn sys_unlink(ctx: &SyscallContext) -> Result<u32, u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = segment::get_fs_string(path_ptr as *const u8, 256);

        let (dir, basename) = path::resolve_parent(&pathname).ok_or(ENOENT)?;
        if basename.is_empty() {
            return Err(ENOENT);
        }
        if !check_permission(&dir, AccessMask::MAY_WRITE) {
            return Err(EACCES);
        }

        let inum = dir.lookup(basename)?.ok_or(ENOENT)?;
        let inode = get_inode(InodeId {
            device: dir.id.device,
            inode_number: inum,
        });
        if inode.inner.lock().disk_inode.mode.file_type() == InodeType::Directory {
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
    fn sys_chdir(ctx: &SyscallContext) -> Result<u32, u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = segment::get_fs_string(path_ptr as *const u8, 256);

        let inode = path::resolve_path(&pathname).ok_or(ENOENT)?;
        if inode.inner.lock().disk_inode.mode.file_type() != InodeType::Directory {
            return Err(ENOTDIR);
        }
        if !check_permission(&inode, AccessMask::MAY_EXEC) {
            return Err(EACCES);
        }

        task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.fs.current_directory = Some(inode));
        Ok(0)
    }
);

define_syscall_handler!(
    user_lib::NR_LSEEK = 19,
    fn sys_lseek(ctx: &SyscallContext) -> Result<u32, u32> {
        let (fd, offset, whence) = ctx.args();
        let whence = Whence::from_raw(whence).ok_or(EINVAL)?;
        let file = get_file(fd)?;
        file.seek(offset as i32, whence).map(|pos| pos as u32)
    }
);

/// Retrieve the file object for a given fd, or `Err(EBADF)`.
fn get_file(fd: u32) -> Result<Arc<dyn File>, u32> {
    task::current_task()
        .pcb
        .inner
        .exclusive(|inner| inner.fs.open_files.get(fd as usize).cloned().flatten())
        .ok_or(EBADF)
}
