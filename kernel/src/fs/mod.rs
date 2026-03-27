//! Filesystem subsystem.

use alloc::sync::Arc;
use log::info;

use crate::{
    driver,
    fs::{
        layout::ROOT_INODE_NUMBER,
        minix::{INODE_TABLE, InodeId, MinixFileSystem},
        mount::{MOUNT_TABLE, Mount},
    },
    task::current_task,
};

pub mod bitmap;
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

    let root_inode = INODE_TABLE
        .lock()
        .get_inode(
            InodeId {
                device: dev,
                inode_number: ROOT_INODE_NUMBER,
            },
            &root_fs,
        )
        .expect("Failed to read root inode");

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
    info!(
        "{}/{} free blocks",
        fs.zone_bitmap.count_free(),
        fs.super_block.zone_count
    );
    info!(
        "{}/{} free inodes",
        fs.inode_bitmap.count_free(),
        fs.super_block.inode_count
    );
}
