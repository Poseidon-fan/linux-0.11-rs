//! Filesystem subsystem: Minix v1, buffer cache, and VFS-like inode/file layer.
//!
//! - [`minix`] — on-disk and in-memory inode objects, block mapping.
//! - [`buffer`] — block-level read/write cache with hash lookup.
//! - [`file`] — [`File`](file::File) trait and typed implementations (regular, device, pipe).
//! - [`path`] — pathname resolution and permission checks.
//! - [`mount`] — global mount table.
//! - [`bitmap`] — inode/zone allocation bitmaps.
//! - [`layout`] — on-disk superblock and inode format definitions.

use alloc::sync::Arc;

use log::info;

use crate::{
    driver,
    fs::{
        layout::ROOT_INODE_NUMBER,
        minix::{INODE_TABLE, Inode, InodeId, MinixFileSystem},
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
pub mod path;

/// Flush all dirty inode and buffer-cache state to disk.
pub fn sync() {
    INODE_TABLE.lock().sync_inodes();
    buffer::sync_buffers();
}

/// Filesystem logical block size in bytes.
pub const BLOCK_SIZE: usize = 1024;

/// Look up one inode and follow mount points until a backing inode is reached.
///
/// When the looked-up inode is a mount point, the returned inode becomes the
/// root inode of the filesystem mounted on top of that point.
///
/// # Panics
///
/// Panics if `id.device` is zero.
/// Panics if no mounted filesystem exists for `id.device`.
pub fn get_inode(id: InodeId) -> Arc<Inode> {
    assert_ne!(id.device.0, 0, "iget with dev==0");

    let mut current_id = id;

    loop {
        let fs = MOUNT_TABLE
            .lock()
            .get_fs(current_id.device)
            .unwrap_or_else(|| panic!("get_inode on unmounted device {:04x}", current_id.device.0));

        let inode = INODE_TABLE.lock().get_inode_raw(current_id, &fs);

        let mounted_root = MOUNT_TABLE.lock().get_mounted_root_by_mount_point(inode.id);
        let Some(root_inode) = mounted_root else {
            return inode;
        };

        current_id = root_inode.id;
    }
}

/// Mount the root filesystem from the configured root device and set up the
/// initial process's filesystem context (root directory and working directory).
pub fn mount_root() {
    let dev = driver::root_dev();
    let root_fs = MinixFileSystem::open(dev).expect("Failed to open root filesystem");

    // Bootstrap the root mount entry with one raw inode lookup first because
    // the generic `get_inode()` path relies on the mount table to resolve devices.
    let boot_root_inode = INODE_TABLE.lock().get_inode_raw(
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
