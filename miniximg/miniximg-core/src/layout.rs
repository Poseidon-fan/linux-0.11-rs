//! On-disk Minix layout types and byte encoding helpers.

use std::mem::size_of;

use crate::error::{MinixError, Result};

/// The Minix filesystem logical block size used by the current kernel.
pub const BLOCK_SIZE: usize = 1024;

/// The maximum file-name length stored in one directory entry.
pub const MINIX_NAME_LENGTH: usize = 14;

/// The Minix v1 magic number expected by the current kernel.
pub const MINIX_SUPER_MAGIC: u16 = 0x137F;

/// The root inode number.
pub const ROOT_INODE_NUMBER: u16 = 1;

/// The number of direct data-zone pointers stored in one inode.
pub const DIRECT_ZONE_COUNT: usize = 7;

/// The number of 16-bit entries that fit in one indirect block.
pub const INDIRECT_ENTRY_COUNT: usize = BLOCK_SIZE / size_of::<u16>();

/// The number of bytes occupied by one serialized inode.
pub const DISK_INODE_SIZE: usize = 32;

/// The number of bytes occupied by one serialized super block.
pub const DISK_SUPER_BLOCK_SIZE: usize = 20;

/// The number of bytes occupied by one serialized directory entry.
pub const DIRECTORY_ENTRY_SIZE: usize = 16;

/// The number of inodes that fit in one filesystem block.
pub const INODES_PER_BLOCK: usize = BLOCK_SIZE / DISK_INODE_SIZE;

/// The maximum logical block count reachable through direct and indirect zones.
pub const MAX_LOGICAL_BLOCKS: usize =
    DIRECT_ZONE_COUNT + INDIRECT_ENTRY_COUNT + INDIRECT_ENTRY_COUNT * INDIRECT_ENTRY_COUNT;

/// The maximum representable file size under the current addressing scheme.
pub const MAX_FILE_SIZE: u32 = (MAX_LOGICAL_BLOCKS * BLOCK_SIZE) as u32;

/// The semantically decoded inode type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InodeType {
    /// Ordinary file data stored in data zones.
    Regular,
    /// A directory containing name-to-inode mappings.
    Directory,
    /// A FIFO special file.
    Fifo,
    /// A block-device special file.
    BlockDevice,
    /// A character-device special file.
    CharacterDevice,
}

/// A small wrapper around the low permission bits stored in one inode mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InodeModeFlags(pub u16);

/// A semantically typed inode mode wrapper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct InodeMode(pub u16);

/// The on-disk Minix super block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DiskSuperBlock {
    /// The number of usable inode slots.
    pub inode_count: u16,
    /// The total zone count recorded by the image.
    pub zone_count: u16,
    /// The number of inode bitmap blocks.
    pub inode_bitmap_block_count: u16,
    /// The number of zone bitmap blocks.
    pub zone_bitmap_block_count: u16,
    /// The first data zone number.
    pub first_data_zone: u16,
    /// The log2 multiplier for zone size. The current kernel requires zero.
    pub log_zone_size: u16,
    /// The maximum representable file size.
    pub max_file_size: u32,
    /// The filesystem magic number.
    pub magic: u16,
}

/// The on-disk Minix inode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DiskInode {
    /// The type and permission bits.
    pub mode: InodeMode,
    /// The owning user ID.
    pub user_id: u16,
    /// The file size in bytes.
    pub size: u32,
    /// The modification timestamp.
    pub modification_time: u32,
    /// The owning group ID.
    pub group_id: u8,
    /// The link count.
    pub link_count: u8,
    /// The direct data zones.
    pub direct_zones: [u16; DIRECT_ZONE_COUNT],
    /// The single-indirect zone pointer.
    pub single_indirect_zone: u16,
    /// The double-indirect zone pointer.
    pub double_indirect_zone: u16,
}

/// The on-disk Minix directory entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DiskDirectoryEntry {
    /// The referenced inode number.
    pub inode_number: u16,
    /// The fixed-size file name field.
    pub name: [u8; MINIX_NAME_LENGTH],
}

impl InodeModeFlags {
    /// The mask that selects permission and special bits.
    pub const MASK: u16 = 0o007777;
}

impl InodeMode {
    /// The mask that selects the inode type field.
    pub const TYPE_MASK: u16 = 0o170000;

    /// The mask that selects the permission and special bits.
    pub const FLAGS_MASK: u16 = InodeModeFlags::MASK;

    /// Build a regular-file mode word from permission bits.
    pub const fn regular(permissions: u16) -> Self {
        Self(0o100000 | (permissions & Self::FLAGS_MASK))
    }

    /// Build a directory mode word from permission bits.
    pub const fn directory(permissions: u16) -> Self {
        Self(0o040000 | (permissions & Self::FLAGS_MASK))
    }

    /// Return the stored inode type if it is recognized.
    pub const fn file_type(self) -> Option<InodeType> {
        match self.0 & Self::TYPE_MASK {
            0o100000 => Some(InodeType::Regular),
            0o040000 => Some(InodeType::Directory),
            0o010000 => Some(InodeType::Fifo),
            0o060000 => Some(InodeType::BlockDevice),
            0o020000 => Some(InodeType::CharacterDevice),
            _ => None,
        }
    }

    /// Return the permission and special bits without the type field.
    pub const fn flags(self) -> InodeModeFlags {
        InodeModeFlags(self.0 & Self::FLAGS_MASK)
    }
}

impl DiskSuperBlock {
    /// Decode one super block from the beginning of a block buffer.
    pub fn decode(block: &[u8; BLOCK_SIZE]) -> Result<Self> {
        let bytes = &block[..DISK_SUPER_BLOCK_SIZE];

        Ok(Self {
            inode_count: read_u16(bytes, 0)?,
            zone_count: read_u16(bytes, 2)?,
            inode_bitmap_block_count: read_u16(bytes, 4)?,
            zone_bitmap_block_count: read_u16(bytes, 6)?,
            first_data_zone: read_u16(bytes, 8)?,
            log_zone_size: read_u16(bytes, 10)?,
            max_file_size: read_u32(bytes, 12)?,
            magic: read_u16(bytes, 16)?,
        })
    }

    /// Encode the super block into the beginning of a block buffer.
    pub fn encode(self, block: &mut [u8; BLOCK_SIZE]) {
        write_u16(block, 0, self.inode_count);
        write_u16(block, 2, self.zone_count);
        write_u16(block, 4, self.inode_bitmap_block_count);
        write_u16(block, 6, self.zone_bitmap_block_count);
        write_u16(block, 8, self.first_data_zone);
        write_u16(block, 10, self.log_zone_size);
        write_u32(block, 12, self.max_file_size);
        write_u16(block, 16, self.magic);
    }
}

impl DiskInode {
    /// Return an all-zero inode suitable for a fresh inode slot.
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

    /// Return whether the inode slot is fully zero-filled.
    pub const fn is_zeroed(&self) -> bool {
        self.mode.0 == 0
            && self.user_id == 0
            && self.size == 0
            && self.modification_time == 0
            && self.group_id == 0
            && self.link_count == 0
            && self.direct_zones[0] == 0
            && self.direct_zones[1] == 0
            && self.direct_zones[2] == 0
            && self.direct_zones[3] == 0
            && self.direct_zones[4] == 0
            && self.direct_zones[5] == 0
            && self.direct_zones[6] == 0
            && self.single_indirect_zone == 0
            && self.double_indirect_zone == 0
    }

    /// Decode one inode from a 32-byte inode slot.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < DISK_INODE_SIZE {
            return Err(MinixError::Corrupt(
                "inode slot is shorter than the Minix inode size".into(),
            ));
        }

        Ok(Self {
            mode: InodeMode(read_u16(bytes, 0)?),
            user_id: read_u16(bytes, 2)?,
            size: read_u32(bytes, 4)?,
            modification_time: read_u32(bytes, 8)?,
            group_id: bytes[12],
            link_count: bytes[13],
            direct_zones: [
                read_u16(bytes, 14)?,
                read_u16(bytes, 16)?,
                read_u16(bytes, 18)?,
                read_u16(bytes, 20)?,
                read_u16(bytes, 22)?,
                read_u16(bytes, 24)?,
                read_u16(bytes, 26)?,
            ],
            single_indirect_zone: read_u16(bytes, 28)?,
            double_indirect_zone: read_u16(bytes, 30)?,
        })
    }

    /// Encode one inode into a 32-byte inode slot.
    pub fn encode(self, bytes: &mut [u8]) {
        write_u16(bytes, 0, self.mode.0);
        write_u16(bytes, 2, self.user_id);
        write_u32(bytes, 4, self.size);
        write_u32(bytes, 8, self.modification_time);
        bytes[12] = self.group_id;
        bytes[13] = self.link_count;
        write_u16(bytes, 14, self.direct_zones[0]);
        write_u16(bytes, 16, self.direct_zones[1]);
        write_u16(bytes, 18, self.direct_zones[2]);
        write_u16(bytes, 20, self.direct_zones[3]);
        write_u16(bytes, 22, self.direct_zones[4]);
        write_u16(bytes, 24, self.direct_zones[5]);
        write_u16(bytes, 26, self.direct_zones[6]);
        write_u16(bytes, 28, self.single_indirect_zone);
        write_u16(bytes, 30, self.double_indirect_zone);
    }
}

impl DiskDirectoryEntry {
    /// Return an empty directory entry.
    pub const fn empty() -> Self {
        Self {
            inode_number: 0,
            name: [0; MINIX_NAME_LENGTH],
        }
    }

    /// Build one directory entry from a validated file name.
    pub fn new(name: &str, inode_number: u16) -> Result<Self> {
        if name.len() > MINIX_NAME_LENGTH {
            return Err(MinixError::NameTooLong {
                name: name.into(),
                max_bytes: MINIX_NAME_LENGTH,
            });
        }

        let mut bytes = [0_u8; MINIX_NAME_LENGTH];
        bytes[..name.len()].copy_from_slice(name.as_bytes());

        Ok(Self {
            inode_number,
            name: bytes,
        })
    }

    /// Decode one directory entry from a 16-byte slot.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < DIRECTORY_ENTRY_SIZE {
            return Err(MinixError::Corrupt(
                "directory entry slot is shorter than the Minix directory-entry size".into(),
            ));
        }

        let mut name = [0_u8; MINIX_NAME_LENGTH];
        name.copy_from_slice(&bytes[2..DIRECTORY_ENTRY_SIZE]);

        Ok(Self {
            inode_number: read_u16(bytes, 0)?,
            name,
        })
    }

    /// Encode one directory entry into a 16-byte slot.
    pub fn encode(self, bytes: &mut [u8]) {
        write_u16(bytes, 0, self.inode_number);
        bytes[2..DIRECTORY_ENTRY_SIZE].copy_from_slice(&self.name);
    }

    /// Return the file name without trailing NUL bytes.
    pub fn name(&self) -> Result<String> {
        let len = self
            .name
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(MINIX_NAME_LENGTH);

        let raw = &self.name[..len];
        let value = std::str::from_utf8(raw).map_err(|_| {
            MinixError::Corrupt("directory entry contains non-UTF-8 file-name bytes".into())
        })?;

        Ok(value.into())
    }
}

/// Read one little-endian `u16` from a byte slice.
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let slice = bytes.get(offset..offset + 2).ok_or_else(|| {
        MinixError::Corrupt("truncated little-endian u16 field in on-disk structure".into())
    })?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

/// Read one little-endian `u32` from a byte slice.
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let slice = bytes.get(offset..offset + 4).ok_or_else(|| {
        MinixError::Corrupt("truncated little-endian u32 field in on-disk structure".into())
    })?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Write one little-endian `u16` into a byte slice.
fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// Write one little-endian `u32` into a byte slice.
fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    //! Layout-focused unit tests.

    use std::mem::size_of;

    use super::*;

    /// Confirm the `repr(C)` layout matches the serialized Minix inode width.
    #[test]
    fn disk_inode_size_matches_minix_layout() {
        assert_eq!(size_of::<DiskInode>(), DISK_INODE_SIZE);
    }

    /// Confirm the `repr(C)` layout matches the serialized Minix super block width.
    #[test]
    fn disk_super_block_size_matches_minix_layout() {
        assert_eq!(size_of::<DiskSuperBlock>(), DISK_SUPER_BLOCK_SIZE);
    }

    /// Confirm the `repr(C)` layout matches the serialized Minix directory-entry width.
    #[test]
    fn directory_entry_size_matches_minix_layout() {
        assert_eq!(size_of::<DiskDirectoryEntry>(), DIRECTORY_ENTRY_SIZE);
    }

    /// Confirm directory entries reject names longer than fourteen bytes.
    #[test]
    fn directory_entry_rejects_long_names() {
        let error = DiskDirectoryEntry::new("123456789012345", 1).unwrap_err();
        assert!(matches!(error, MinixError::NameTooLong { .. }));
    }
}
