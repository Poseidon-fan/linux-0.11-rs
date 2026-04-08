//! Regular file backed by a Minix inode.

use alloc::sync::Arc;

use user_lib::fs::{AccessMode, OpenOptions, Stat, Whence};

use super::File;
use crate::{fs::minix::Inode, sync::Mutex, syscall::EINVAL};

/// Open file object backed by one inode data area.
///
/// This wrapper is used for ordinary files and directories whose readable
/// contents come from the inode's mapped data blocks. Device nodes and pipes
/// use different runtime objects because their I/O semantics do not go
/// through the regular Minix block mapping path.
pub struct InodeFile {
    access_mode: AccessMode,
    open_options: OpenOptions,
    inner: Mutex<InodeFileInner>,
}

/// Mutable open-file state that is private to one opened inode file.
///
/// This mirrors the role of Linux 0.11 `struct file` fields that belong to
/// one open instance instead of the inode itself.
struct InodeFileInner {
    inode: Arc<Inode>,
    offset: usize,
}

impl InodeFile {
    pub fn new(inode: Arc<Inode>, access_mode: AccessMode, open_options: OpenOptions) -> Self {
        Self {
            access_mode,
            open_options,
            inner: Mutex::new(InodeFileInner { inode, offset: 0 }),
        }
    }
}

impl File for InodeFile {
    fn read(&self, buffer: &mut [u8]) -> Result<usize, u32> {
        if self.access_mode == AccessMode::WriteOnly {
            return Err(crate::syscall::EBADF);
        }
        let mut inner = self.inner.lock();
        let bytes_read = inner.inode.read_at(inner.offset, buffer)?;
        inner.offset += bytes_read;
        Ok(bytes_read)
    }

    fn stat(&self) -> Result<Stat, u32> {
        Ok(self.inner.lock().inode.stat())
    }

    fn write(&self, buffer: &[u8]) -> Result<usize, u32> {
        if self.access_mode == AccessMode::ReadOnly {
            return Err(crate::syscall::EBADF);
        }
        let mut inner = self.inner.lock();
        let offset = if self.open_options.contains(OpenOptions::APPEND) {
            inner.inode.inner.lock().disk_inode.size as usize
        } else {
            inner.offset
        };
        let bytes_written = inner.inode.write_at(offset, buffer)?;
        inner.offset = offset + bytes_written;
        Ok(bytes_written)
    }

    fn seek(&self, offset: i32, whence: Whence) -> Result<usize, u32> {
        let mut inner = self.inner.lock();
        let new_offset = match whence {
            Whence::Set => offset as isize,
            Whence::Current => inner.offset as isize + offset as isize,
            Whence::End => inner.inode.inner.lock().disk_inode.size as isize + offset as isize,
        };
        if new_offset < 0 {
            return Err(EINVAL);
        }
        inner.offset = new_offset as usize;
        Ok(inner.offset)
    }
}
