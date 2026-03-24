pub mod inode;

use bitflags::bitflags;

use crate::driver::DevNum;

/// Generic file operations exposed through one file descriptor entry.
pub trait File: Send + Sync {
    /// Read data from the current file position.
    fn read(&self, buffer: &mut [u8]) -> Result<usize, u32>;

    /// Write data to the current file position.
    fn write(&self, buffer: &[u8]) -> Result<usize, u32>;

    /// Adjust the current file position.
    fn seek(&self, position: SeekFrom) -> Result<u64, u32>;

    /// Read file metadata visible through `stat(2)`.
    fn stat(&self) -> Result<FileStat, u32>;
}

bitflags! {
    /// Open flags stored in one open-file object.
    pub struct OpenFlags: u32 {
        const READ_ONLY = 0;
        const WRITE_ONLY = 1 << 0;
        const READ_WRITE = 1 << 1;
        const CREATE = 1 << 6;
        const EXCLUSIVE = 1 << 7;
        const TRUNCATE = 1 << 9;
        const APPEND = 1 << 10;
    }
}

/// Seek origin and displacement for one file-position update.
pub enum SeekFrom {
    /// Seek from file start.
    Start(u64),
    /// Seek relative to the current file position.
    Current(i64),
    /// Seek relative to file end.
    End(i64),
}

/// Metadata returned by `stat(2)` style queries.
pub struct FileStat {
    pub device: DevNum,
    pub inode_number: u32,
    pub mode: u16,
    pub hard_link_count: u16,
    pub user_id: u16,
    pub group_id: u16,
    pub special_device: DevNum,
    pub size: u32,
    pub access_time: u32,
    pub modification_time: u32,
    pub change_time: u32,
}
