//! Runtime Minix filesystem objects.

use alloc::sync::{Arc, Weak};
use core::array;

use lazy_static::lazy_static;

use crate::{
    driver::DevNum,
    fs::{
        bitmap::Bitmap,
        layout::{DiskInode, DiskSuperBlock, InodeNumber},
    },
    sync::Mutex,
};

/// Maximum number of bitmap blocks cached from one Minix super block.
pub const MINIX_BITMAP_BLOCK_SLOTS: usize = 8;

/// Number of runtime inode slots kept in the global inode table.
pub const INODE_TABLE_CAPACITY: usize = 32;

lazy_static! {
    /// Global runtime inode table modeled after the fixed-size Linux 0.11 array.
    pub static ref INODE_TABLE: Mutex<[Option<Arc<Inode>>; INODE_TABLE_CAPACITY]> =
        Mutex::new(array::from_fn(|_| None));
}

/// Runtime inode identifier used in the global inode table and mount lookups.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct InodeId {
    pub device: DevNum,
    pub inode_number: InodeNumber,
}

/// Shared filesystem instance mounted from one block device.
pub struct MinixFileSystem {
    pub device: DevNum,
    pub super_block: DiskSuperBlock,
    pub inode_bitmap: Bitmap<MINIX_BITMAP_BLOCK_SLOTS>,
    pub zone_bitmap: Bitmap<MINIX_BITMAP_BLOCK_SLOTS>,
}

/// Runtime inode object.
pub struct Inode {
    pub id: InodeId,
    pub file_system: Weak<Mutex<MinixFileSystem>>,
    pub inner: Mutex<InodeInner>,
}

pub struct InodeInner {
    pub disk_inode: DiskInode,
    pub is_dirty: bool,
    pub access_time: u32,
    pub change_time: u32,
}
