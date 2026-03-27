//! Runtime Minix filesystem objects.

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::array;

use lazy_static::lazy_static;
use log::warn;

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
///
/// Each slot holds one `Arc<Inode>`. A slot is considered *unused* when the
/// table's `Arc` is the sole remaining strong reference
/// (`Arc::strong_count == 1`); such slots may be evicted to make room for
/// new inodes. A clock pointer provides round-robin fairness during
/// eviction scans.
pub struct InodeTable {
    slots: [Option<Arc<Inode>>; INODE_TABLE_CAPACITY],
    clock: usize,
}

/// Runtime inode identifier used in the global inode table and mount lookups.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
        let (block_nr, offset) = self.inode_block_position(nr);
        let buf = buffer::read_block(BufferKey {
            dev: self.device,
            block_nr,
        })?;
        let disk_inode = buf.read(|block: &InodeBlock| block[offset]);
        buffer::release_block(buf);
        Some(disk_inode)
    }

    /// Write one on-disk inode back to its block.
    pub fn write_inode(&self, nr: InodeNumber, inode: &DiskInode) {
        let (block_nr, offset) = self.inode_block_position(nr);
        let Some(buf) = buffer::read_block(BufferKey {
            dev: self.device,
            block_nr,
        }) else {
            warn!("write_inode: unable to read inode block for {:?}", nr.0);
            return;
        };
        buf.write(|block: &mut InodeBlock| block[offset] = *inode);
        buffer::release_block(buf);
    }

    fn inode_block_position(&self, nr: InodeNumber) -> (u32, usize) {
        let index = (nr.0 - 1) as usize;
        let block_nr = 2
            + self.super_block.inode_bitmap_block_count as u32
            + self.super_block.zone_bitmap_block_count as u32
            + (index / INODES_PER_BLOCK) as u32;
        let offset = index % INODES_PER_BLOCK;
        (block_nr, offset)
    }
}

impl InodeTable {
    fn new() -> Self {
        Self {
            slots: array::from_fn(|_| None),
            clock: 0,
        }
    }

    /// Look up or load an inode identified by `id`.
    ///
    /// If a cached entry exists, returns a new `Arc` handle to it. Otherwise
    /// reads the inode from disk via `fs`, places it in a free or evicted
    /// slot, and returns the handle. Returns `None` when the disk read fails.
    ///
    /// # Panics
    ///
    /// Panics if every slot is actively referenced by external code and no
    /// eviction candidate exists.
    pub fn get_inode(
        &mut self,
        id: InodeId,
        fs: &Arc<Mutex<MinixFileSystem>>,
    ) -> Option<Arc<Inode>> {
        if let Some(inode) = self.lookup(id) {
            return Some(inode);
        }

        let disk_inode = fs.lock().read_inode(id.inode_number)?;
        let inode = Arc::new(Inode {
            id,
            file_system: Arc::downgrade(fs),
            inner: Mutex::new(InodeInner {
                disk_inode,
                is_dirty: false,
                access_time: 0,
                change_time: 0,
            }),
        });

        self.install(Arc::clone(&inode));
        Some(inode)
    }

    /// Iterate all cached inodes on `dev` and flush dirty ones to disk.
    pub fn sync_inodes(&self) {
        for slot in &self.slots {
            let Some(arc) = slot else { continue };
            let inner = arc.inner.lock();
            if inner.is_dirty {
                if let Some(fs) = arc.file_system.upgrade() {
                    fs.lock()
                        .write_inode(arc.id.inode_number, &inner.disk_inode);
                }
            }
        }
    }

    /// Search cached slots for a matching inode and return a cloned handle.
    fn lookup(&self, id: InodeId) -> Option<Arc<Inode>> {
        self.slots.iter().find_map(|slot| {
            let arc = slot.as_ref()?;
            (arc.id == id).then(|| Arc::clone(arc))
        })
    }

    /// Place `inode` into a free or evicted slot.
    fn install(&mut self, inode: Arc<Inode>) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.is_none()) {
            *slot = Some(inode);
            return;
        }

        let idx = self
            .find_victim()
            .expect("inode table full: all slots actively referenced");

        self.flush_slot(idx);
        self.slots[idx] = Some(inode);
        self.clock = (idx + 1) % INODE_TABLE_CAPACITY;
    }

    /// Clock-scan for an eviction candidate whose `strong_count` is 1
    /// (only the table holds it). Prefers a clean slot to avoid synchronous
    /// disk writes.
    fn find_victim(&self) -> Option<usize> {
        let mut dirty_fallback = None;

        for i in 0..INODE_TABLE_CAPACITY {
            let idx = (self.clock + i) % INODE_TABLE_CAPACITY;
            let Some(arc) = &self.slots[idx] else {
                continue;
            };
            if Arc::strong_count(arc) != 1 {
                continue;
            }
            if !arc.inner.lock().is_dirty {
                return Some(idx);
            }
            if dirty_fallback.is_none() {
                dirty_fallback = Some(idx);
            }
        }

        dirty_fallback
    }

    /// Write back and clean up the inode at `idx` before eviction.
    ///
    /// Handles two cases:
    /// - **dirty**: writes the in-memory copy back to disk.
    /// - **zero link count**: the file has been fully unlinked while cached;
    ///   its data blocks and bitmap entry should be freed (TODO).
    fn flush_slot(&mut self, idx: usize) {
        let victim = self.slots[idx].take().unwrap();
        let inner = victim.inner.lock();

        if inner.disk_inode.link_count == 0 {
            // TODO: truncate data blocks + free inode bitmap entry
            return;
        }

        if inner.is_dirty {
            if let Some(fs) = victim.file_system.upgrade() {
                fs.lock()
                    .write_inode(victim.id.inode_number, &inner.disk_inode);
            }
        }
    }
}
