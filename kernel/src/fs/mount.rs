//! Global mount table objects used by pathname traversal.

use alloc::sync::Arc;
use core::array;

use lazy_static::lazy_static;

use crate::{
    fs::minix::{Inode, MinixFileSystem},
    sync::Mutex,
};

/// Number of mounted filesystem slots kept in the global mount table.
pub const MOUNT_TABLE_CAPACITY: usize = 8;

lazy_static! {
    /// Global mount table modeled after the fixed-size mount array.
    pub static ref MOUNT_TABLE: Mutex<[Option<Arc<Mount>>; MOUNT_TABLE_CAPACITY]> =
        Mutex::new(array::from_fn(|_| None));
}

/// One mounted filesystem entry stored in the global mount table.
pub struct Mount {
    pub file_system: Arc<Mutex<MinixFileSystem>>,
    pub root_inode: Arc<Inode>,
    /// Inode covered by this mount entry. The root filesystem has no mount point.
    pub mount_point_inode: Option<Arc<Inode>>,
}
