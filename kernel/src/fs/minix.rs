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
        buffer::{self, BufferKey},
        layout::{
            DiskInode, DiskSuperBlock, INODES_PER_BLOCK, InodeBlock, InodeNumber, MINIX_SUPER_MAGIC,
        },
    },
    sync::Mutex,
};

/// Maximum number of bitmap blocks cached from one Minix super block.
pub const MINIX_BITMAP_BLOCK_SLOTS: usize = 8;

/// Number of runtime inode slots kept in the global inode table.
pub const INODE_TABLE_CAPACITY: usize = 32;

lazy_static! {
    /// Global runtime inode table protected by a mutex.
    pub static ref INODE_TABLE: Mutex<InodeTable> = Mutex::new(InodeTable::new());
}

/// Fixed-capacity table that caches runtime inode objects.
pub struct InodeTable {
    slots: [Option<Arc<Inode>>; INODE_TABLE_CAPACITY],
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
                buffer::release_blocks(inode_bitmap_bufs);
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
                buffer::release_blocks(inode_bitmap_bufs);
                buffer::release_blocks(zone_bitmap_bufs);
                return None;
            };
            block += 1;
            zone_bitmap_bufs.push(buf);
        }

        // Inode bitmap: bit j → inode j. Valid inodes are 1..inode_count, so
        // bit_count covers bits 0..inode_count inclusive (inode_count + 1 bits).
        // Bit 0 is marked occupied by `Bitmap::new`.
        let inode_bitmap = Bitmap::new(0, inode_bitmap_bufs, super_block.inode_count as usize + 1);

        // Zone bitmap: bit j → zone (first_data_zone - 1 + j). The bitmap
        // covers zones [first_data_zone-1, zone_count-1], i.e.
        // zone_count - first_data_zone + 1 bits total. Bit 0 is marked
        // occupied by `Bitmap::new`.
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

    /// Read one on-disk inode by its number.
    ///
    /// The caller must ensure `nr` is a valid, non-zero inode number within
    /// the filesystem's inode count.
    pub fn read_inode(&self, nr: InodeNumber) -> Option<DiskInode> {
        let index = (nr.0 - 1) as usize;
        let block_nr = 2
            + self.super_block.inode_bitmap_block_count as u32
            + self.super_block.zone_bitmap_block_count as u32
            + (index / INODES_PER_BLOCK) as u32;
        let offset = index % INODES_PER_BLOCK;

        let buf = buffer::read_block(BufferKey {
            dev: self.device,
            block_nr,
        })?;
        let disk_inode = buf.read(|block: &InodeBlock| block[offset]);
        buffer::release_block(buf);
        Some(disk_inode)
    }
}

impl InodeTable {
    fn new() -> Self {
        Self {
            slots: array::from_fn(|_| None),
        }
    }

    /// Insert an inode into the first free slot.
    ///
    /// Returns the slot index on success, or `None` if the table is full.
    pub fn insert(&mut self, inode: Arc<Inode>) -> Option<usize> {
        let slot = self.slots.iter().position(|s| s.is_none())?;
        self.slots[slot] = Some(inode);
        Some(slot)
    }
}
