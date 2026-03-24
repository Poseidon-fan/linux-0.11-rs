//! On-disk Minix filesystem layout types.

use core::mem::size_of;

/// Maximum file name length stored in one Minix directory entry.
pub const MINIX_NAME_LENGTH: usize = 14;

/// Root inode number in the filesystem image.
pub const ROOT_INODE_NUMBER: InodeNumber = InodeNumber(1);

/// Count of direct zone pointers stored in each disk inode.
pub const DIRECT_ZONE_COUNT: usize = 7;

/// Index of the single-indirect zone pointer inside the zone pointer array.
pub const INDIRECT_ZONE_INDEX: usize = DIRECT_ZONE_COUNT;

/// Index of the double-indirect zone pointer inside the zone pointer array.
pub const DOUBLE_INDIRECT_ZONE_INDEX: usize = DIRECT_ZONE_COUNT + 1;

/// Logical inode number used by runtime metadata and lookup code.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct InodeNumber(pub u32);

/// Minix on-disk super block.
#[repr(C)]
pub struct DiskSuperBlock {
    pub inode_count: u16,
    pub zone_count: u16,
    pub inode_bitmap_block_count: u16,
    pub zone_bitmap_block_count: u16,
    pub first_data_zone: u16,
    pub log_zone_size: u16,
    pub max_file_size: u32,
    pub magic: u16,
}

/// Minix on-disk inode.
#[repr(C)]
pub struct DiskInode {
    pub mode: u16,
    pub user_id: u16,
    pub size: u32,
    pub modification_time: u32,
    pub group_id: u8,
    pub link_count: u8,
    pub direct_zones: [u16; DIRECT_ZONE_COUNT],
    pub single_indirect_zone: u16,
    pub double_indirect_zone: u16,
}

/// Minix on-disk directory entry.
#[repr(C)]
pub struct DiskDirectoryEntry {
    pub inode_number: u16,
    pub name: [u8; MINIX_NAME_LENGTH],
}

const _: () = assert!(size_of::<DiskSuperBlock>() == 20);
const _: () = assert!(size_of::<DiskInode>() == 32);
const _: () = assert!(size_of::<DiskDirectoryEntry>() == 16);
