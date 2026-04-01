//! On-disk Minix filesystem layout types.

use core::mem::size_of;

use bitflags::bitflags;

use crate::fs::BLOCK_SIZE;

/// Maximum file name length stored in one Minix directory entry.
pub const MINIX_NAME_LENGTH: usize = 14;

/// Magic number that identifies a valid Minix filesystem super block.
pub const MINIX_SUPER_MAGIC: u16 = 0x137F;

/// Root inode number in the filesystem image.
pub const ROOT_INODE_NUMBER: InodeNumber = InodeNumber(1);

/// Count of direct zone pointers stored in each disk inode.
pub const DIRECT_ZONE_COUNT: usize = 7;

/// Index of the single-indirect zone pointer inside the zone pointer array.
pub const INDIRECT_ZONE_INDEX: usize = DIRECT_ZONE_COUNT;

/// Index of the double-indirect zone pointer inside the zone pointer array.
pub const DOUBLE_INDIRECT_ZONE_INDEX: usize = DIRECT_ZONE_COUNT + 1;

/// Number of on-disk inodes that fit in one filesystem block.
pub const INODES_PER_BLOCK: usize = BLOCK_SIZE / size_of::<DiskInode>();

/// Number of on-disk directory entries that fit in one filesystem block.
pub const DIRECTORY_ENTRIES_PER_BLOCK: usize = BLOCK_SIZE / size_of::<DiskDirectoryEntry>();

/// Logical inode number used by runtime metadata and lookup code.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct InodeNumber(pub u16);

/// Classified inode type stored in the high bits of one mode word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InodeType {
    /// Ordinary file data stored in data zones.
    Regular,
    /// Directory entries mapping file names to inode numbers.
    Directory,
    /// Named-pipe special file.
    Fifo,
    /// Block-device special file.
    BlockDevice,
    /// Character-device special file.
    CharacterDevice,
}

bitflags! {
    /// Non-type inode mode bits stored below the type field.
    ///
    /// The on-disk mode word is laid out as:
    ///
    /// ```text
    ///  15            12 11   9 8   6 5   3 2   0
    /// +---------------+------+-----+-----+-----+
    /// |   file type   | spec | usr | grp | oth |
    /// +---------------+------+-----+-----+-----+
    /// ```
    ///
    /// `spec` contains set-user-ID, set-group-ID, and sticky bits.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct InodeModeFlags: u16 {
        /// Execute with the inode owner's effective user ID.
        const SET_USER_ID = 0o004000;
        /// Execute with the inode owner's effective group ID.
        const SET_GROUP_ID = 0o002000;
        /// Retain restricted deletion or special directory semantics.
        const STICKY = 0o001000;
        /// Read permission for the inode owner.
        const OWNER_READ = 0o000400;
        /// Write permission for the inode owner.
        const OWNER_WRITE = 0o000200;
        /// Execute/search permission for the inode owner.
        const OWNER_EXECUTE = 0o000100;
        /// Read permission for the owning group.
        const GROUP_READ = 0o000040;
        /// Write permission for the owning group.
        const GROUP_WRITE = 0o000020;
        /// Execute/search permission for the owning group.
        const GROUP_EXECUTE = 0o000010;
        /// Read permission for all other users.
        const OTHER_READ = 0o000004;
        /// Write permission for all other users.
        const OTHER_WRITE = 0o000002;
        /// Execute/search permission for all other users.
        const OTHER_EXECUTE = 0o000001;
    }
}

/// Semantically typed inode mode wrapper that preserves the on-disk `u16` layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct InodeMode(pub u16);

/// Minix on-disk super block.
#[derive(Clone, Copy)]
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
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DiskInode {
    pub mode: InodeMode,
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
    pub inode_number: InodeNumber,
    pub name: [u8; MINIX_NAME_LENGTH],
}

pub type BitmapBlock = [u64; BLOCK_SIZE / size_of::<u64>()];
/// One full block of on-disk inodes, used when reading inode table blocks.
pub type InodeBlock = [DiskInode; INODES_PER_BLOCK];
pub type DataBlock = [u8; BLOCK_SIZE];
pub type DirectoryBlock = [DiskDirectoryEntry; DIRECTORY_ENTRIES_PER_BLOCK];

impl DiskInode {
    /// Return an all-zero inode suitable for a freshly allocated inode slot.
    pub const fn zeroed() -> Self {
        Self {
            mode: InodeMode(0),
            user_id: 0,
            size: 0,
            modification_time: 0,
            group_id: 0,
            link_count: 0,
            direct_zones: [0; DIRECT_ZONE_COUNT],
            single_indirect_zone: 0,
            double_indirect_zone: 0,
        }
    }
}

impl InodeMode {
    /// Mask that selects the inode type field.
    pub const TYPE_MASK: u16 = 0o170000;

    /// Mask that selects the special and permission bits below the type field.
    pub const FLAGS_MASK: u16 = 0o007777;

    /// Decode the inode type field if the stored value is recognized.
    pub const fn file_type(self) -> InodeType {
        match self.0 & Self::TYPE_MASK {
            0o100000 => InodeType::Regular,
            0o040000 => InodeType::Directory,
            0o060000 => InodeType::BlockDevice,
            0o020000 => InodeType::CharacterDevice,
            0o010000 => InodeType::Fifo,
            _ => panic!("invalid inode type"),
        }
    }

    /// Return the special and permission flags stored below the type field.
    pub fn flags(self) -> InodeModeFlags {
        InodeModeFlags::from_bits_retain(self.0 & Self::FLAGS_MASK)
    }
}

impl DiskDirectoryEntry {
    pub const fn empty() -> Self {
        Self {
            inode_number: InodeNumber(0),
            name: [0; MINIX_NAME_LENGTH],
        }
    }

    pub fn new(name: &str, inode_number: InodeNumber) -> Self {
        let mut bytes = [0; MINIX_NAME_LENGTH];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Self {
            inode_number,
            name: bytes,
        }
    }

    pub fn name(&self) -> &str {
        let len = self
            .name
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(MINIX_NAME_LENGTH);
        core::str::from_utf8(&self.name[..len]).unwrap()
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, size_of::<Self>()) }
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self as *mut _ as *mut u8, size_of::<Self>()) }
    }
}

const _: () = assert!(size_of::<DiskSuperBlock>() == 20);
const _: () = assert!(size_of::<DiskInode>() == 32);
const _: () = assert!(size_of::<DiskDirectoryEntry>() == 16);
