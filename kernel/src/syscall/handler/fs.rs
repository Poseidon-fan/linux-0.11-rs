use alloc::sync::Arc;
use linkme::distributed_slice;
use user_lib::fs::OpenFlags;

use alloc::vec;

use crate::{
    define_syscall_handler,
    driver::blk::hd,
    fs::{
        self,
        file::{File, InodeFile},
        layout::InodeType,
        path,
    },
    segment,
    syscall::{EBADF, EMFILE, EPERM, SYSCALL_TABLE, context::SyscallContext},
    task,
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
        let (access_mode, open_options) = flags.into_parts().ok_or(EPERM)?;

        let inode = path::open_path(&pathname, flags, mode as u16)?;

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

/// Retrieve the file object for a given fd, or `Err(EBADF)`.
fn get_file(fd: u32) -> Result<Arc<dyn File>, u32> {
    task::current_task()
        .pcb
        .inner
        .exclusive(|inner| inner.fs.open_files.get(fd as usize).cloned().flatten())
        .ok_or(EBADF)
}
