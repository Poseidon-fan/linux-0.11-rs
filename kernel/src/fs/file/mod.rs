pub mod inode;
#[allow(unused_imports)]
pub use inode::InodeFile;

use user_lib::fs::Whence;

/// Generic opened file object in kernel.
pub trait File: Send + Sync {
    fn read(&self, buffer: &mut [u8]) -> Result<usize, u32>;
    fn write(&self, buffer: &[u8]) -> Result<usize, u32>;

    /// Reposition the file offset. Returns the new absolute offset on success.
    ///
    /// The default implementation returns `ESPIPE`, which is correct for
    /// non-seekable file types (pipes, character devices, etc.).
    fn seek(&self, _offset: i32, _whence: Whence) -> Result<usize, u32> {
        Err(crate::syscall::ESPIPE)
    }
}
