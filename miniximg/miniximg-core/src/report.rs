//! Shared report models returned by inspection and file operations.

use crate::layout::InodeType;

/// Metadata returned for one inode or path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeMetadata {
    /// The absolute path used to reach the inode.
    pub path: String,
    /// The inode number.
    pub inode_number: u16,
    /// The decoded inode type.
    pub kind: InodeType,
    /// The raw on-disk mode word.
    pub mode: u16,
    /// The owning user ID.
    pub uid: u16,
    /// The owning group ID.
    pub gid: u8,
    /// The file size in bytes.
    pub size: u32,
    /// The hard-link count.
    pub link_count: u8,
    /// The modification time.
    pub modification_time: u32,
    /// The packed device number for block or character special files.
    pub device_number: Option<u16>,
}

/// One directory entry plus the child inode metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntryInfo {
    /// The directory-entry file name.
    pub name: String,
    /// The child inode metadata.
    pub metadata: NodeMetadata,
}

/// One tree node produced by recursive traversal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeEntry {
    /// The nesting depth below the requested root.
    pub depth: usize,
    /// The inode metadata at that path.
    pub metadata: NodeMetadata,
}

/// A summary of one opened Minix image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectReport {
    /// The logical block size in bytes.
    pub block_size: usize,
    /// The filesystem magic number.
    pub magic: u16,
    /// The total inode count.
    pub inode_count: u16,
    /// The total zone count.
    pub zone_count: u16,
    /// The number of inode bitmap blocks.
    pub inode_bitmap_blocks: u16,
    /// The number of zone bitmap blocks.
    pub zone_bitmap_blocks: u16,
    /// The first data-zone number.
    pub first_data_zone: u16,
    /// The recorded maximum file size.
    pub max_file_size: u32,
    /// The number of free inode slots.
    pub free_inodes: usize,
    /// The number of free data zones.
    pub free_zones: usize,
    /// The non-dot entries found in the root directory.
    pub root_entries: Vec<DirectoryEntryInfo>,
}

/// One validation issue emitted by `check`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckIssue {
    /// The best-effort path or object label associated with the issue.
    pub path: Option<String>,
    /// The human-readable issue description.
    pub message: String,
}

/// A collection of validation issues.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CheckReport {
    /// The gathered validation issues.
    pub issues: Vec<CheckIssue>,
}

impl CheckReport {
    /// Return whether the report contains no issues.
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}
