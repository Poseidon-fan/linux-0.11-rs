//! Filesystem subsystem.

use alloc::sync::Arc;

use crate::{
    driver,
    fs::{
        layout::ROOT_INODE_NUMBER,
        minix::{INODE_TABLE, Inode, InodeId, InodeInner, MinixFileSystem},
        mount::{MOUNT_TABLE, Mount},
    },
    sync::Mutex,
    task::current_task,
};

mod bitmap;
pub mod buffer;
pub mod file;
pub mod layout;
pub mod minix;
pub mod mount;

/// Filesystem logical block size in bytes.
pub const BLOCK_SIZE: usize = 1024;

/// Mount the root filesystem from the configured root device and set up the
/// initial process's filesystem context (root directory and working directory).
pub fn mount_root() {
    let dev = driver::root_dev();
    let root_fs = MinixFileSystem::open(dev).expect("Failed to open root filesystem");

    let disk_inode = root_fs
        .lock()
        .read_inode(ROOT_INODE_NUMBER)
        .expect("Failed to read root inode");

    let root_inode = Arc::new(Inode {
        id: InodeId {
            device: dev,
            inode_number: ROOT_INODE_NUMBER,
        },
        file_system: Arc::downgrade(&root_fs),
        inner: Mutex::new(InodeInner {
            disk_inode,
            is_dirty: false,
            access_time: 0,
            change_time: 0,
        }),
    });

    INODE_TABLE
        .lock()
        .insert(Arc::clone(&root_inode))
        .expect("No free inode table slot");

    let mount_entry = Arc::new(Mount {
        device: dev,
        file_system: Arc::clone(&root_fs),
        root_inode: Arc::clone(&root_inode),
        mount_point_inode: None,
    });

    MOUNT_TABLE
        .lock()
        .insert(mount_entry)
        .expect("No free mount table slot");

    current_task().pcb.inner.exclusive(|inner| {
        inner.fs.root_directory = Some(Arc::clone(&root_inode));
        inner.fs.current_directory = Some(root_inode);
    });

    let fs = root_fs.lock();
    crate::println!(
        "{}/{} free blocks",
        fs.zone_bitmap.count_free(),
        fs.super_block.zone_count
    );
    crate::println!(
        "{}/{} free inodes",
        fs.inode_bitmap.count_free(),
        fs.super_block.inode_count
    );
}
