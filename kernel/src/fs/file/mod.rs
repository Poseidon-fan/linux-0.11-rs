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
use user_lib::syscall::fs::{Stat, Whence};

use crate::error::{Errno, Result};

/// Generic opened file object in kernel.
pub trait File: Send + Sync {
    fn read(&self, buffer: &mut [u8]) -> Result<usize>;
    fn write(&self, buffer: &[u8]) -> Result<usize>;
    fn stat(&self) -> Result<Stat>;

    /// Reposition the file offset. Returns the new absolute offset on success.
    ///
    /// The default implementation returns `Errno::SPIPE`, which is correct for
    /// non-seekable file types (pipes, character devices, etc.).
    fn seek(&self, _offset: i32, _whence: Whence) -> Result<usize> {
        Err(Errno::SPIPE)
    }

    /// Device- or file-specific control operation.
    ///
    /// The default implementation returns `Errno::NOTTY`, which is correct for
    /// file types that do not provide ioctl commands.
    fn ioctl(&self, _cmd: u32, _arg: u32) -> Result<u32> {
        Err(Errno::NOTTY)
    }
}
