//! Runtime Minix filesystem objects.

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::array;

use lazy_static::lazy_static;

use crate::{
    driver::DevNum,
    fs::{
        bitmap::Bitmap,
        buffer::{self, BufferHandle, BufferKey},
        layout::{BitmapBlock, DiskInode, DiskSuperBlock, InodeNumber, MINIX_SUPER_MAGIC},
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

/// Release every buffer in the list via [`buffer::release_block`].
fn release_buffers(bufs: Vec<Arc<BufferHandle>>) {
    for buf in bufs {
        buffer::release_block(buf);
    }
}

impl MinixFileSystem {
    /// Read and validate the filesystem on `dev`, loading all bitmap blocks into
    /// memory. Returns `None` if the device is unreadable or carries no valid
    /// Minix filesystem.
    pub fn open(dev: DevNum) -> Option<Arc<Mutex<Self>>> {
        // Super block occupies logical block 1.
        let sb_buf = buffer::read_block(BufferKey { dev, block_nr: 1 })?;
        let super_block: DiskSuperBlock = sb_buf.read(|sb: &DiskSuperBlock| *sb);
        buffer::release_block(sb_buf);

        if super_block.magic != MINIX_SUPER_MAGIC {
            return None;
        }

        // Bitmap blocks start immediately after the super block at block 2.
        let mut block = 2u32;

        let mut inode_bitmap_bufs = Vec::new();
        for _ in 0..super_block.inode_bitmap_block_count {
            let Some(buf) = buffer::read_block(BufferKey {
                dev,
                block_nr: block,
            }) else {
                release_buffers(inode_bitmap_bufs);
                return None;
            };
            block += 1;
            inode_bitmap_bufs.push(buf);
        }

        let mut zone_bitmap_bufs = Vec::new();
        for _ in 0..super_block.zone_bitmap_block_count {
            let Some(buf) = buffer::read_block(BufferKey {
                dev,
                block_nr: block,
            }) else {
                release_buffers(inode_bitmap_bufs);
                release_buffers(zone_bitmap_bufs);
                return None;
            };
            block += 1;
            zone_bitmap_bufs.push(buf);
        }

        // Bit 0 of each bitmap is permanently reserved. Marking it ensures
        // inode 0 (invalid) and the last pre-data zone are never allocated.
        // The write goes through Arc<BufferHandle>, so the underlying buffer
        // data is shared — the reservation survives the Vec-to-Bitmap move
        // below (only the Arc is moved, not the buffer contents).
        inode_bitmap_bufs[0].write(|b: &mut BitmapBlock| b[0] |= 1);
        zone_bitmap_bufs[0].write(|b: &mut BitmapBlock| b[0] |= 1);

        // Inode bitmap: bit j → inode j. Valid inodes are 1..inode_count, so
        // bit_count covers bits 0..inode_count inclusive (inode_count + 1 bits).
        let inode_bitmap = Bitmap::new(0, inode_bitmap_bufs, super_block.inode_count as usize + 1);

        // Zone bitmap: bit j → zone (first_data_zone - 1 + j). The bitmap
        // covers zones [first_data_zone-1, zone_count-1], i.e.
        // zone_count - first_data_zone + 1 bits total.
        let zone_bitmap = Bitmap::new(
            super_block.first_data_zone as u32 - 1,
            zone_bitmap_bufs,
            super_block.zone_count as usize - super_block.first_data_zone as usize + 1,
        );

        Some(Arc::new(Mutex::new(Self {
            device: dev,
            super_block,
            inode_bitmap,
            zone_bitmap,
        })))
    }
}
