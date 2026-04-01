//! Runtime Minix filesystem objects.

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{array, mem::size_of};
use log::error;

use lazy_static::lazy_static;

use user_lib::fs::Stat;

use crate::{
    driver::DevNum,
    fs::{
        BLOCK_SIZE,
        bitmap::Bitmap,
        buffer::{self, BufferKey},
        layout::{
            DIRECTORY_ENTRY_SIZE, DataBlock, DiskDirectoryEntry, DiskInode, DiskSuperBlock,
            INODES_PER_BLOCK, InodeBlock, InodeMode, InodeNumber, InodeType, MINIX_SUPER_MAGIC,
        },
    },
    sync::Mutex,
    syscall::{EFBIG, EIO, ENOENT, ENOSPC, ERROR},
    task, time,
};

/// Maximum number of bitmap blocks cached from one Minix super block.
pub const MINIX_BITMAP_BLOCK_SLOTS: usize = 8;

/// Number of runtime inode slots kept in the global inode table.
pub const INODE_TABLE_CAPACITY: usize = 32;

/// Number of 16-bit zone pointers stored in one indirect block.
const INDIRECT_ENTRY_COUNT: usize = BLOCK_SIZE / size_of::<u16>();

/// Maximum logical block count representable by direct and indirect pointers.
const MAX_LOGICAL_BLOCKS: usize =
    7 + INDIRECT_ENTRY_COUNT + INDIRECT_ENTRY_COUNT * INDIRECT_ENTRY_COUNT;

/// One indirect block interpreted as 16-bit Minix zone identifiers.
type IndirectBlock = [u16; INDIRECT_ENTRY_COUNT];

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

impl Inode {
    /// Write this inode back to its inode-table block and clear the dirty flag.
    ///
    /// If the inode is not dirty or its backing filesystem is no longer
    /// available, this function returns without modifying on-disk state.
    pub fn sync(&self) {
        let mut inner = self.inner.lock();
        if !inner.is_dirty {
            return;
        }

        if let Some(fs) = self.file_system.upgrade() {
            fs.lock()
                .write_inode(self.id.inode_number, &inner.disk_inode);
            inner.is_dirty = false;
        }
    }

    /// Build a `Stat` structure from this inode's metadata.
    pub fn stat(&self) -> Stat {
        let inner = self.inner.lock();
        let disk = &inner.disk_inode;
        Stat {
            st_dev: self.id.device.0,
            st_ino: self.id.inode_number.0,
            st_mode: disk.mode.0,
            st_nlink: disk.link_count,
            st_uid: disk.user_id,
            st_gid: disk.group_id,
            st_rdev: 0,
            st_size: disk.size,
            st_atime: inner.access_time,
            st_mtime: disk.modification_time,
            st_ctime: inner.change_time,
        }
    }

    /// Release all data blocks and set size to 0.
    pub fn truncate(&self) {
        let Some(fs) = self.file_system.upgrade() else {
            return;
        };
        let fs = fs.lock();
        let dev = self.id.device;
        let mut inner = self.inner.lock();
        let disk = &mut inner.disk_inode;

        let free = |zone: &mut u16| {
            if *zone == 0 {
                return;
            }
            // Read the indirect block and free every zone it references.
            if let Some(buf) = buffer::read_block(BufferKey {
                dev,
                block_nr: u32::from(*zone),
            }) {
                buf.read(|table: &IndirectBlock| {
                    for &e in table {
                        fs.free_zone(e);
                    }
                });
                buffer::release_block(buf);
            }
            fs.free_zone(*zone);
            *zone = 0;
        };

        for z in &mut disk.direct_zones {
            fs.free_zone(*z);
            *z = 0;
        }
        free(&mut disk.single_indirect_zone);

        if disk.double_indirect_zone != 0 {
            if let Some(buf) = buffer::read_block(BufferKey {
                dev,
                block_nr: u32::from(disk.double_indirect_zone),
            }) {
                let entries = buf.read(|table: &IndirectBlock| *table);
                buffer::release_block(buf);
                for mut e in entries {
                    free(&mut e);
                }
            }
            fs.free_zone(disk.double_indirect_zone);
            disk.double_indirect_zone = 0;
        }

        disk.size = 0;
        let now = time::current_time();
        disk.modification_time = now;
        inner.change_time = now;
        inner.is_dirty = true;
    }

    /// Map one logical file block to its backing disk block.
    ///
    /// The result mirrors the original Minix `_bmap` contract:
    /// `Ok(0)` means the logical block is currently unmapped or allocation
    /// failed when `create` was requested.
    pub fn map_block_id(&self, logic_id: usize, create: bool) -> Result<u32, u32> {
        if logic_id >= MAX_LOGICAL_BLOCKS {
            return Err(EFBIG);
        }

        let Some(fs) = self.file_system.upgrade() else {
            return Err(EIO);
        };

        let mut inner = self.inner.lock();
        let resolve_indirect_entry = |block_nr: u16, index: usize| -> Result<u16, u32> {
            if block_nr == 0 {
                return Ok(0);
            }

            let buf = buffer::read_block(BufferKey {
                dev: self.id.device,
                block_nr: u32::from(block_nr),
            })
            .ok_or(EIO)?;

            let zone = match (buf.read(|table: &IndirectBlock| table[index]), create) {
                (0, true) => fs
                    .lock()
                    .alloc_zone()
                    .inspect(|&new_zone| {
                        buf.write(|table: &mut IndirectBlock| table[index] = new_zone)
                    })
                    .unwrap_or(0),
                (zone, _) => zone,
            };
            buffer::release_block(buf);
            Ok(zone)
        };

        let block_id = match logic_id {
            0..7 => match (inner.disk_inode.direct_zones[logic_id], create) {
                (0, true) => match fs.lock().alloc_zone() {
                    Some(new_zone) => {
                        inner.disk_inode.direct_zones[logic_id] = new_zone;
                        inner.is_dirty = true;
                        inner.change_time = time::current_time();
                        new_zone
                    }
                    None => 0,
                },
                (zone, _) => zone,
            },
            _ if logic_id < 7 + INDIRECT_ENTRY_COUNT => {
                let root_zone = match (inner.disk_inode.single_indirect_zone, create) {
                    (0, true) => match fs.lock().alloc_zone() {
                        Some(new_zone) => {
                            inner.disk_inode.single_indirect_zone = new_zone;
                            inner.is_dirty = true;
                            inner.change_time = time::current_time();
                            new_zone
                        }
                        None => 0,
                    },
                    (zone, _) => zone,
                };

                resolve_indirect_entry(root_zone, logic_id - 7)?
            }
            _ => {
                let root_zone = match (inner.disk_inode.double_indirect_zone, create) {
                    (0, true) => match fs.lock().alloc_zone() {
                        Some(new_zone) => {
                            inner.disk_inode.double_indirect_zone = new_zone;
                            inner.is_dirty = true;
                            inner.change_time = time::current_time();
                            new_zone
                        }
                        None => 0,
                    },
                    (zone, _) => zone,
                };

                let block = logic_id - 7 - INDIRECT_ENTRY_COUNT;
                let outer_index = block / INDIRECT_ENTRY_COUNT;
                let inner_index = block % INDIRECT_ENTRY_COUNT;
                let second_level_zone = resolve_indirect_entry(root_zone, outer_index)?;
                resolve_indirect_entry(second_level_zone, inner_index)?
            }
        };

        Ok(u32::from(block_id))
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize, u32> {
        if buf.is_empty() {
            return Ok(0);
        }

        let size = self.inner.lock().disk_inode.size as usize;

        if offset >= size {
            return Ok(0);
        }

        let mut pos = offset;
        let mut read = 0usize;
        let mut left = buf.len().min(size - offset);

        while left > 0 {
            let logical_block = pos / BLOCK_SIZE;
            let block_offset = pos % BLOCK_SIZE;
            let chunk_len = left.min(BLOCK_SIZE - block_offset);
            let target = &mut buf[read..read + chunk_len];
            let block_id = self.map_block_id(logical_block, false)?;

            if block_id == 0 {
                target.fill(0);
            } else {
                let Some(block_buf) = buffer::read_block(BufferKey {
                    dev: self.id.device,
                    block_nr: block_id,
                }) else {
                    return if read > 0 { Ok(read) } else { Err(EIO) };
                };
                block_buf.read(|block: &DataBlock| {
                    target.copy_from_slice(&block[block_offset..block_offset + chunk_len]);
                });
                buffer::release_block(block_buf);
            }

            pos += chunk_len;
            read += chunk_len;
            left -= chunk_len;
        }

        self.inner.lock().access_time = time::current_time();
        Ok(read)
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize, u32> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut pos = offset;
        let mut written = 0usize;
        let mut failure = None;

        while written < buf.len() {
            let logical_block = pos / BLOCK_SIZE;
            let block_offset = pos % BLOCK_SIZE;
            let chunk_len = (buf.len() - written).min(BLOCK_SIZE - block_offset);
            let block_id = match self.map_block_id(logical_block, true) {
                Ok(0) => {
                    failure = Some(ERROR);
                    break;
                }
                Ok(block_id) => block_id,
                Err(errno) => {
                    failure = Some(errno);
                    break;
                }
            };

            let Some(block_buf) = buffer::read_block(BufferKey {
                dev: self.id.device,
                block_nr: block_id,
            }) else {
                failure = Some(ERROR);
                break;
            };

            let source = &buf[written..written + chunk_len];
            block_buf.write(|block: &mut DataBlock| {
                block[block_offset..block_offset + chunk_len].copy_from_slice(source);
            });
            buffer::release_block(block_buf);

            pos += chunk_len;
            written += chunk_len;

            let mut inner = self.inner.lock();
            if pos > inner.disk_inode.size as usize {
                inner.disk_inode.size = pos as u32;
                inner.is_dirty = true;
            }
        }

        let now = time::current_time();
        let mut inner = self.inner.lock();
        inner.disk_inode.modification_time = now;
        inner.change_time = now;
        inner.is_dirty = true;

        if written > 0 {
            Ok(written)
        } else {
            Err(failure.unwrap_or(ERROR))
        }
    }
}

// Directory operations
impl Inode {
    pub fn lookup(&self, name: &str) -> Result<Option<InodeNumber>, u32> {
        assert!(self.inner.lock().disk_inode.mode.file_type() == InodeType::Directory);
        let file_count = self.inner.lock().disk_inode.size as usize / DIRECTORY_ENTRY_SIZE;
        let mut dirent = DiskDirectoryEntry::empty();
        for i in 0..file_count {
            let len = self.read_at(DIRECTORY_ENTRY_SIZE * i, dirent.as_bytes_mut())?;
            assert_eq!(len, DIRECTORY_ENTRY_SIZE);
            if dirent.inode_number.0 == 0 {
                continue;
            }
            if dirent.name() == name {
                return Ok(Some(dirent.inode_number));
            }
        }
        Ok(None)
    }

    /// Add a new directory entry mapping `name` to `inode_number`.
    ///
    /// Reuses the first empty slot (inode_number == 0) if one exists; otherwise
    /// appends at the end, allocating a new data block when needed.
    pub fn add_entry(&self, name: &str, inode_number: InodeNumber) -> Result<(), u32> {
        assert!(self.inner.lock().disk_inode.mode.file_type() == InodeType::Directory);

        let entry_count = self.inner.lock().disk_inode.size as usize / DIRECTORY_ENTRY_SIZE;

        // Scan for an empty slot.
        let mut slot = entry_count;
        let mut dirent = DiskDirectoryEntry::empty();
        for i in 0..entry_count {
            self.read_at(DIRECTORY_ENTRY_SIZE * i, dirent.as_bytes_mut())?;
            if dirent.inode_number.0 == 0 {
                slot = i;
                break;
            }
        }

        // Write the new entry at the chosen slot.
        let new_entry = DiskDirectoryEntry::new(name, inode_number);
        let offset = slot * DIRECTORY_ENTRY_SIZE;
        let written = self.write_at(offset, new_entry.as_bytes())?;
        assert_eq!(written, DIRECTORY_ENTRY_SIZE);

        Ok(())
    }

    /// Create a new regular file inside this directory.
    /// The caller is responsible for permission checks.
    pub fn create_file(self: &Arc<Self>, name: &str, mode: u16) -> Result<Arc<Inode>, u32> {
        self.alloc_and_link(name, 0o100000, mode, 1)
    }

    /// Create a new directory inside this directory.
    /// The caller is responsible for permission checks.
    pub fn create_directory(self: &Arc<Self>, name: &str, mode: u16) -> Result<Arc<Inode>, u32> {
        let inode = self.alloc_and_link(name, 0o040000, mode, 2)?;

        let inode_number = inode.id.inode_number;
        inode.add_entry(".", inode_number)?;
        inode.add_entry("..", self.id.inode_number)?;

        let mut parent_inner = self.inner.lock();
        parent_inner.disk_inode.link_count += 1;
        parent_inner.change_time = time::current_time();
        parent_inner.is_dirty = true;

        Ok(inode)
    }

    /// Allocate an inode, initialise its metadata, and link it into this directory.
    fn alloc_and_link(
        self: &Arc<Self>,
        name: &str,
        type_bits: u16,
        mode: u16,
        link_count: u8,
    ) -> Result<Arc<Inode>, u32> {
        let fs = self.file_system.upgrade().ok_or(EIO)?;
        let inode_number = fs.lock().alloc_inode().ok_or(ENOSPC)?;

        let inode = INODE_TABLE.lock().get_inode_raw(
            InodeId {
                device: self.id.device,
                inode_number,
            },
            &fs,
        );

        let (euid, egid, umask) = task::current_task()
            .pcb
            .inner
            .exclusive(|inner| (inner.identity.euid, inner.identity.egid, inner.fs.umask));

        let now = time::current_time();
        {
            let mut inner = inode.inner.lock();
            inner.disk_inode.mode = InodeMode(type_bits | (mode & 0o777 & !umask));
            inner.disk_inode.user_id = euid;
            inner.disk_inode.group_id = egid as u8;
            inner.disk_inode.link_count = link_count;
            inner.disk_inode.modification_time = now;
            inner.access_time = now;
            inner.change_time = now;
            inner.is_dirty = true;
        }

        if let Err(e) = self.add_entry(name, inode_number) {
            let mut inner = inode.inner.lock();
            inner.disk_inode.link_count = 0;
            inner.is_dirty = true;
            return Err(e);
        }

        Ok(inode)
    }

    /// Check whether this directory contains only `.` and `..` entries.
    pub fn is_empty_directory(&self) -> Result<bool, u32> {
        assert!(self.inner.lock().disk_inode.mode.file_type() == InodeType::Directory);
        let entry_count = self.inner.lock().disk_inode.size as usize / DIRECTORY_ENTRY_SIZE;
        let mut dirent = DiskDirectoryEntry::empty();
        for i in 0..entry_count {
            self.read_at(DIRECTORY_ENTRY_SIZE * i, dirent.as_bytes_mut())?;
            if dirent.inode_number.0 == 0 {
                continue;
            }
            let name = dirent.name();
            if name != "." && name != ".." {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Remove the directory entry matching `name` and return its inode number.
    pub fn remove_entry(&self, name: &str) -> Result<InodeNumber, u32> {
        assert!(self.inner.lock().disk_inode.mode.file_type() == InodeType::Directory);
        let entry_count = self.inner.lock().disk_inode.size as usize / DIRECTORY_ENTRY_SIZE;
        let mut dirent = DiskDirectoryEntry::empty();
        for i in 0..entry_count {
            self.read_at(DIRECTORY_ENTRY_SIZE * i, dirent.as_bytes_mut())?;
            if dirent.inode_number.0 != 0 && dirent.name() == name {
                let inum = dirent.inode_number;
                let empty = DiskDirectoryEntry::empty();
                self.write_at(DIRECTORY_ENTRY_SIZE * i, empty.as_bytes())?;
                return Ok(inum);
            }
        }
        Err(ENOENT)
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
            error!("invalid super block magic number");
            return None;
        }
        if super_block.log_zone_size != 0 {
            error!("invalid log zone size");
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
    ///
    /// # Panics
    ///
    /// Panics when the backing inode-table block cannot be read.
    pub fn read_inode(&self, nr: InodeNumber) -> DiskInode {
        let (block_nr, offset) = self.inode_block_position(nr);
        let buf = buffer::read_block(BufferKey {
            dev: self.device,
            block_nr,
        })
        .unwrap_or_else(|| panic!("unable to read i-node block {}", block_nr));
        let disk_inode = buf.read(|block: &InodeBlock| block[offset]);
        buffer::release_block(buf);
        disk_inode
    }

    /// Write one on-disk inode back to its block.
    ///
    /// # Panics
    ///
    /// Panics when the backing inode-table block cannot be read.
    pub fn write_inode(&self, nr: InodeNumber, inode: &DiskInode) {
        let (block_nr, offset) = self.inode_block_position(nr);
        let buf = buffer::read_block(BufferKey {
            dev: self.device,
            block_nr,
        })
        .unwrap_or_else(|| panic!("unable to read i-node block {}", block_nr));
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

    /// Allocate a fresh inode number from the inode bitmap and write a zeroed
    /// on-disk inode to the inode table block.
    ///
    /// Returns `None` when the bitmap is full.
    pub fn alloc_inode(&self) -> Option<InodeNumber> {
        let nr = InodeNumber(self.inode_bitmap.alloc()? as u16);
        self.write_inode(nr, &DiskInode::zeroed());
        Some(nr)
    }

    /// Allocate one fresh data zone and clear its backing cache block.
    fn alloc_zone(&self) -> Option<u16> {
        let zone = self.zone_bitmap.alloc()? as u16;
        let buf = buffer::acquire_block(BufferKey {
            dev: self.device,
            block_nr: u32::from(zone),
        });
        buf.write(|block: &mut DataBlock| block.fill(0));
        buf.set_uptodate(true);
        buffer::release_block(buf);
        Some(zone)
    }

    /// Release a data zone back to the zone bitmap.
    fn free_zone(&self, zone: u16) {
        if zone != 0 {
            self.zone_bitmap.dealloc(u32::from(zone));
        }
    }

    /// Release an inode number back to the inode bitmap and zero the on-disk
    /// inode table entry.
    pub fn free_inode(&self, inode_number: InodeNumber) {
        self.write_inode(inode_number, &DiskInode::zeroed());
        self.inode_bitmap.dealloc(u32::from(inode_number.0));
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
    /// slot, and returns the handle.
    ///
    /// # Panics
    ///
    /// Panics if `id.device` is zero.
    /// Panics if every slot is actively referenced by external code and no
    /// eviction candidate exists.
    pub(crate) fn get_inode_raw(
        &mut self,
        id: InodeId,
        fs: &Arc<Mutex<MinixFileSystem>>,
    ) -> Arc<Inode> {
        assert_ne!(id.device.0, 0, "iget with dev==0");

        if let Some(inode) = self.lookup(id) {
            return inode;
        }

        let disk_inode = fs.lock().read_inode(id.inode_number);
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
        inode
    }

    /// Iterate all cached inodes on `dev` and flush dirty ones to disk.
    pub fn sync_inodes(&self) {
        for slot in &self.slots {
            let Some(arc) = slot else { continue };
            arc.sync();
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
    ///   its data blocks and bitmap entry are freed.
    fn flush_slot(&mut self, idx: usize) {
        let victim = self.slots[idx].take().unwrap();
        let inner = victim.inner.lock();

        if inner.disk_inode.link_count == 0 {
            drop(inner);
            victim.truncate();
            if let Some(fs) = victim.file_system.upgrade() {
                fs.lock().free_inode(victim.id.inode_number);
            }
            return;
        }

        if inner.is_dirty {
            drop(inner);
            victim.sync();
        }
    }
}
