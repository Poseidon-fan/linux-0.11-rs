//! Global mount table objects used by pathname traversal.

use alloc::sync::Arc;
use core::array;

use lazy_static::lazy_static;

use crate::{
    driver::DevNum,
    fs::minix::{INODE_TABLE, Inode, InodeId, MinixFileSystem},
    sync::Mutex,
};

/// Number of mounted filesystem slots kept in the global mount table.
const MOUNT_TABLE_CAPACITY: usize = 8;

lazy_static! {
    /// Global mount table protected by a mutex; accessed through [`MountTable`] methods.
    pub static ref MOUNT_TABLE: Mutex<MountTable> = Mutex::new(MountTable::new());
}

/// One mounted filesystem entry stored in the global mount table.
pub struct Mount {
    pub device: DevNum,
    pub file_system: Arc<Mutex<MinixFileSystem>>,
    pub root_inode: Arc<Inode>,
    /// Inode covered by this mount entry. The root filesystem has no mount point.
    pub mount_point_inode: Option<Arc<Inode>>,
}

/// Fixed-capacity table that tracks all currently mounted filesystems.
pub struct MountTable {
    slots: [Option<Arc<Mount>>; MOUNT_TABLE_CAPACITY],
}

impl MountTable {
    fn new() -> Self {
        Self {
            slots: array::from_fn(|_| None),
        }
    }

    /// Insert a mount entry into the first free slot.
    ///
    /// Returns the slot index on success, or `None` if the table is full.
    pub fn insert(&mut self, mount: Arc<Mount>) -> Option<usize> {
        let slot = self.slots.iter().position(|s| s.is_none())?;
        self.slots[slot] = Some(mount);
        Some(slot)
    }

    /// Return the filesystem mounted on `dev`, or `None` if no such mount exists.
    pub fn get_fs(&self, dev: DevNum) -> Option<Arc<Mutex<MinixFileSystem>>> {
        self.slots.iter().find_map(|slot| {
            let mount = slot.as_ref()?;
            (mount.device == dev).then(|| Arc::clone(&mount.file_system))
        })
    }

    /// Return the root inode of the filesystem mounted on top of `id`.
    pub fn get_mounted_root_by_mount_point(&self, id: InodeId) -> Option<Arc<Inode>> {
        self.slots.iter().find_map(|slot| {
            let mount = slot.as_ref()?;
            (mount.mount_point_inode.as_ref().map(|inode| inode.id) == Some(id))
                .then(|| Arc::clone(&mount.root_inode))
        })
    }

    /// Return the mount-point inode hidden beneath the mounted root `id`.
    pub fn get_mount_point_by_root(&self, id: InodeId) -> Option<Arc<Inode>> {
        self.slots.iter().find_map(|slot| {
            let mount = slot.as_ref()?;
            (mount.root_inode.id == id)
                .then(|| mount.mount_point_inode.as_ref().map(Arc::clone))
                .flatten()
        })
    }

    /// Return `true` if some filesystem is already mounted on top of `id`.
    pub fn is_mount_point(&self, id: InodeId) -> bool {
        self.get_mounted_root_by_mount_point(id).is_some()
    }

    /// Remove the mount entry for `dev` and return it, or `None` if not mounted.
    pub fn remove_by_device(&mut self, dev: DevNum) -> Option<Arc<Mount>> {
        let slot = self
            .slots
            .iter_mut()
            .find(|s| s.as_ref().is_some_and(|m| m.device == dev))?;
        slot.take()
    }
}

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
