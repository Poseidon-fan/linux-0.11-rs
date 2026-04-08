//! Opened-file abstraction and per-type implementations.
//!
//! The [`File`] trait provides read/write/seek/stat/ioctl and is implemented by:
//!
//! - [`InodeFile`] — regular files and directories backed by a Minix inode.
//! - [`BlockDeviceFile`] — raw block device access through the buffer cache.
//! - [`CharDeviceFile`] — character devices dispatched by major number.
//! - [`PipeFile`] — unidirectional byte channel between processes.

mod block_device;
mod char_device;
mod inode;
mod pipe;

pub use block_device::BlockDeviceFile;
pub use char_device::CharDeviceFile;
pub use inode::InodeFile;
pub use pipe::PipeFile;
use user_lib::fs::{Stat, Whence};

/// Generic opened file object in kernel.
pub trait File: Send + Sync {
    fn read(&self, buffer: &mut [u8]) -> Result<usize, u32>;
    fn write(&self, buffer: &[u8]) -> Result<usize, u32>;
    fn stat(&self) -> Result<Stat, u32>;

    /// Reposition the file offset. Returns the new absolute offset on success.
    ///
    /// The default implementation returns `ESPIPE`, which is correct for
    /// non-seekable file types (pipes, character devices, etc.).
    fn seek(&self, _offset: i32, _whence: Whence) -> Result<usize, u32> {
        Err(crate::syscall::ESPIPE)
    }

    /// Device-specific control operation.
    ///
    /// The default implementation returns `ENOTTY`, which is correct for
    /// non-device file types (regular files, directories, etc.).
    fn ioctl(&self, _cmd: u32, _arg: u32) -> Result<u32, u32> {
        Err(crate::syscall::ENOTTY)
    }
}
