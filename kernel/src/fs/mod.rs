//! Filesystem subsystem: Minix v1, buffer cache, and VFS-like inode/file layer.
//!
//! - [`minix`] — on-disk and in-memory inode objects, block mapping.
//! - [`buffer`] — block-level read/write cache with hash lookup.
//! - [`file`] — [`File`](file::File) trait and typed implementations (regular, device, pipe).
//! - [`path`] — pathname resolution and permission checks.
//! - [`mount`] — global mount table.
//! - [`bitmap`] — inode/zone allocation bitmaps.
//! - [`layout`] — on-disk superblock and inode format definitions.

mod bitmap;
pub mod buffer;
pub mod file;
mod layout;
pub mod minix;
pub mod mount;
pub mod path;

use alloc::sync::Arc;

pub use layout::{InodeMode, InodeModeFlags, InodeType, ROOT_INODE_NUMBER};
use log::info;

use crate::{
    driver,
    fs::{
        minix::{InodeId, MinixFileSystem},
        mount::{MOUNT_TABLE, Mount},
    },
    task,
};

/// Flush all dirty inode and buffer-cache state to disk.
pub fn sync() {
    minix::INODE_TABLE.lock().sync_inodes();
    buffer::sync_buffers();
}

/// Filesystem logical block size in bytes.
pub const BLOCK_SIZE: usize = 1024;

pub use mount::get_inode;

/// Mount the root filesystem from the configured root device and set up the
/// initial process's filesystem context (root directory and working directory).
pub fn mount_root() {
    let dev = driver::root_dev();
    let root_fs = MinixFileSystem::open(dev).expect("Failed to open root filesystem");

    // Bootstrap the root mount entry with one raw inode lookup first because
    // the generic `get_inode()` path relies on the mount table to resolve devices.
    let boot_root_inode = minix::INODE_TABLE.lock().get_inode_raw(
        InodeId {
            device: dev,
            inode_number: ROOT_INODE_NUMBER,
        },
        &root_fs,
    );

    let mount_entry = Arc::new(Mount {
        device: dev,
        file_system: Arc::clone(&root_fs),
        root_inode: Arc::clone(&boot_root_inode),
        mount_point_inode: None,
    });

    MOUNT_TABLE
        .lock()
        .insert(mount_entry)
        .expect("No free mount table slot");

    let root_inode = get_inode(InodeId {
        device: dev,
        inode_number: ROOT_INODE_NUMBER,
    });

    task::with_current(|inner| {
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
