//! Filesystem-level syscall handlers (setup, mount, umount, sync).

use alloc::sync::Arc;

use user_lib::syscall::Syscall;

use crate::{
    define_syscall_handler,
    driver::{self, block::hd},
    error::{Errno, Result},
    fs::{
        self, InodeType, ROOT_INODE_NUMBER, buffer,
        minix::{INODE_TABLE, InodeId, MinixFileSystem},
        mount::{MOUNT_TABLE, Mount},
        path,
    },
    syscall::context::SyscallContext,
};

define_syscall_handler!(
    Syscall::Setup = 0,
    fn sys_setup(ctx: &mut SyscallContext) -> Result<u32> {
        let (drive_info_addr, _, _) = ctx.args();
        hd::setup_from_bios(drive_info_addr as *const u8)?;
        fs::mount_root();
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Sync = 36,
    fn sys_sync(_ctx: &mut SyscallContext) -> Result<u32> {
        fs::sync();
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Mount = 21,
    fn sys_mount(ctx: &mut SyscallContext) -> Result<u32> {
        use crate::segment::uaccess;

        let (dev_name_ptr, dir_name_ptr, _rw_flag) = ctx.args();
        let dev_name = uaccess::read_pathname(dev_name_ptr);
        let dir_name = uaccess::read_pathname(dir_name_ptr);

        let dev_inode = path::resolve_path(&dev_name).ok_or(Errno::NOENT)?;
        if dev_inode.file_type() != InodeType::BlockDevice {
            return Err(Errno::PERM);
        }
        let dev = dev_inode.device_number();
        drop(dev_inode);

        let dir_inode = path::resolve_path(&dir_name).ok_or(Errno::NOENT)?;
        if dir_inode.file_type() != InodeType::Directory {
            return Err(Errno::PERM);
        }
        if dir_inode.id.inode_number == ROOT_INODE_NUMBER {
            return Err(Errno::BUSY);
        }

        let mut mt = MOUNT_TABLE.lock();
        if mt.find_fs(dev).is_some() {
            return Err(Errno::BUSY);
        }
        if mt.is_mount_point(dir_inode.id) {
            return Err(Errno::PERM);
        }

        let new_fs = MinixFileSystem::open(dev).ok_or(Errno::BUSY)?;
        let root_inode = INODE_TABLE
            .lock()
            .acquire_raw(InodeId::new(dev, ROOT_INODE_NUMBER), &new_fs);

        mt.insert(Arc::new(Mount {
            device: dev,
            file_system: new_fs,
            root_inode,
            mount_point_inode: Some(dir_inode),
        }))
        .ok_or(Errno::BUSY)?;

        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Umount = 22,
    fn sys_umount(ctx: &mut SyscallContext) -> Result<u32> {
        use crate::segment::uaccess;

        let (dev_name_ptr, _, _) = ctx.args();
        let dev_name = uaccess::read_pathname(dev_name_ptr);

        let dev_inode = path::resolve_path(&dev_name).ok_or(Errno::NOENT)?;
        if dev_inode.file_type() != InodeType::BlockDevice {
            return Err(Errno::NOTBLK);
        }
        let dev = dev_inode.device_number();
        drop(dev_inode);

        if dev == driver::root_dev() {
            return Err(Errno::BUSY);
        }
        if MOUNT_TABLE.lock().find_fs(dev).is_none() {
            return Err(Errno::NOENT);
        }

        let mut inode_table = INODE_TABLE.lock();
        if inode_table.has_active_inodes(dev) {
            return Err(Errno::BUSY);
        }
        inode_table.evict_device(dev);
        drop(inode_table);

        buffer::sync_dirty(|key| key.dev() == dev);
        MOUNT_TABLE.lock().remove_by_device(dev);
        Ok(0)
    }
);
