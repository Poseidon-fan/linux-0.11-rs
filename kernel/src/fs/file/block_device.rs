//! Block device file — raw read/write through the buffer cache.
//!
//! Block device files bypass the Minix block mapping and instead use the
//! device number (from `direct_zones[0]`) plus a byte offset to address
//! sectors directly. This mirrors the original `block_read` / `block_write`
//! in `block_dev.c`.
//!
//! ```text
//!            ┌─────────────────────────────┐
//! user buf   │ ....data to read/write....  │
//!            └─────────────────────────────┘
//!                  │              ▲
//!   block_write    ▼              │  block_read
//!            ┌──────────┐  ┌──────────┐
//!            │ buffer   │  │ buffer   │  ← buffer cache (1 KB blocks)
//!            │ cache    │  │ cache    │
//!            └──────────┘  └──────────┘
//!                  │              ▲
//!                  ▼              │
//!            ┌──────────────────────────┐
//!            │   block device hardware  │
//!            └──────────────────────────┘
//! ```

use alloc::sync::Arc;
use core::ptr;

use user_lib::fs::{Stat, Whence};

use super::File;
use crate::{
    driver::DevNum,
    fs::{
        BLOCK_SIZE,
        buffer::{self, BufferKey},
        minix::Inode,
    },
    sync::Mutex,
    syscall::*,
};

/// Opened block device file.
pub struct BlockDeviceFile {
    dev: DevNum,
    inode: Arc<Inode>,
    inner: Mutex<BlockDeviceInner>,
}

struct BlockDeviceInner {
    offset: usize,
}

impl BlockDeviceFile {
    pub fn new(inode: Arc<Inode>) -> Self {
        let dev = inode.device_number();
        Self {
            dev,
            inode,
            inner: Mutex::new(BlockDeviceInner { offset: 0 }),
        }
    }
}

impl File for BlockDeviceFile {
    fn read(&self, buf: &mut [u8]) -> Result<usize, u32> {
        let mut inner = self.inner.lock();
        let bytes_read = block_read(self.dev, &mut inner.offset, buf)?;
        Ok(bytes_read)
    }

    fn write(&self, buf: &[u8]) -> Result<usize, u32> {
        let mut inner = self.inner.lock();
        let bytes_written = block_write(self.dev, &mut inner.offset, buf)?;
        Ok(bytes_written)
    }

    fn stat(&self) -> Result<Stat, u32> {
        Ok(self.inode.stat())
    }

    fn seek(&self, offset: i32, whence: Whence) -> Result<usize, u32> {
        let mut inner = self.inner.lock();
        let new_offset = match whence {
            Whence::Set => offset as isize,
            Whence::Current => inner.offset as isize + offset as isize,
            Whence::End => return Err(EINVAL),
        };
        if new_offset < 0 {
            return Err(EINVAL);
        }
        inner.offset = new_offset as usize;
        Ok(inner.offset)
    }
}

/// Read raw bytes from a block device through the buffer cache.
fn block_read(dev: DevNum, pos: &mut usize, buf: &mut [u8]) -> Result<usize, u32> {
    let mut count = buf.len();
    let mut buf_offset = 0;
    let mut total_read = 0;

    while count > 0 {
        let block_nr = (*pos / BLOCK_SIZE) as u32;
        let offset_in_block = *pos % BLOCK_SIZE;
        let mut chars = BLOCK_SIZE - offset_in_block;
        if chars > count {
            chars = count;
        }

        let key = BufferKey { dev, block_nr };
        let Some(handle) = buffer::read_block(key) else {
            return if total_read > 0 {
                Ok(total_read)
            } else {
                Err(EIO)
            };
        };

        unsafe {
            let src = handle.data.as_ptr().add(offset_in_block);
            ptr::copy_nonoverlapping(src, buf.as_mut_ptr().add(buf_offset), chars);
        }

        *pos += chars;
        buf_offset += chars;
        total_read += chars;
        count -= chars;
    }

    Ok(total_read)
}

/// Write raw bytes to a block device through the buffer cache.
fn block_write(dev: DevNum, pos: &mut usize, buf: &[u8]) -> Result<usize, u32> {
    let mut count = buf.len();
    let mut buf_offset = 0;
    let mut total_written = 0;

    while count > 0 {
        let block_nr = (*pos / BLOCK_SIZE) as u32;
        let offset_in_block = *pos % BLOCK_SIZE;
        let mut chars = BLOCK_SIZE - offset_in_block;
        if chars > count {
            chars = count;
        }

        let key = BufferKey { dev, block_nr };

        // For a full-block overwrite we only need a buffer slot; for a
        // partial write we must read the existing content first.
        let handle = if chars == BLOCK_SIZE {
            buffer::acquire_block(key)
        } else {
            let Some(h) = buffer::read_block(key) else {
                return if total_written > 0 {
                    Ok(total_written)
                } else {
                    Err(EIO)
                };
            };
            h
        };

        unsafe {
            let dst = handle.data.as_ptr().add(offset_in_block);
            ptr::copy_nonoverlapping(buf.as_ptr().add(buf_offset), dst, chars);
        }
        handle.set_dirty(true);

        *pos += chars;
        buf_offset += chars;
        total_written += chars;
        count -= chars;
    }

    Ok(total_written)
}
