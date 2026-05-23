//! Block device file — raw read/write through the buffer cache.
//!
//! Block device files bypass the Minix block mapping and instead use the
//! device number (from `direct_zones[0]`) plus a byte offset to address
//! sectors directly.
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

use user_lib::syscall::fs::{Stat, Whence};

use super::File;
use crate::{
    driver::DevNum,
    error::{Errno, Result},
    fs::{
        BLOCK_SIZE,
        buffer::{self, BufferKey},
        minix::Inode,
    },
    sync::Mutex,
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

/// One contiguous slice inside a block-sized buffer-cache entry.
struct BlockChunk {
    block_number: u32,
    offset: usize,
    len: usize,
}

impl BlockDeviceFile {
    /// Create an opened block-device file backed by `inode`'s device number.
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
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut inner = self.inner.lock();
        block_read(self.dev, &mut inner.offset, buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        let mut inner = self.inner.lock();
        block_write(self.dev, &mut inner.offset, buf)
    }

    fn stat(&self) -> Result<Stat> {
        Ok(self.inode.stat())
    }

    fn seek(&self, offset: i32, whence: Whence) -> Result<usize> {
        let mut inner = self.inner.lock();
        let new_offset = match whence {
            Whence::Set => usize::try_from(offset).map_err(|_| Errno::INVAL)?,
            Whence::Current => inner
                .offset
                .checked_add_signed(offset as isize)
                .ok_or(Errno::INVAL)?,
            Whence::End => return Err(Errno::INVAL),
        };
        inner.offset = new_offset;
        Ok(inner.offset)
    }
}

/// Read raw bytes from a block device through the buffer cache.
fn block_read(dev: DevNum, pos: &mut usize, buf: &mut [u8]) -> Result<usize> {
    let mut buf_offset = 0;

    while buf_offset < buf.len() {
        let chunk = BlockChunk::new(*pos, buf.len() - buf_offset)?;
        let key = BufferKey::new(dev, chunk.block_number);
        let Some(handle) = buffer::read(key) else {
            return partial_io_result(buf_offset);
        };

        let end = buf_offset + chunk.len;
        handle.read_bytes(chunk.offset, &mut buf[buf_offset..end]);
        *pos += chunk.len;
        buf_offset = end;
    }

    Ok(buf_offset)
}

/// Write raw bytes to a block device through the buffer cache.
fn block_write(dev: DevNum, pos: &mut usize, buf: &[u8]) -> Result<usize> {
    let mut buf_offset = 0;

    while buf_offset < buf.len() {
        let chunk = BlockChunk::new(*pos, buf.len() - buf_offset)?;
        let key = BufferKey::new(dev, chunk.block_number);

        // For a full-block overwrite we only need a buffer slot; for a
        // partial write we must read the existing content first.
        let handle = if chunk.is_full_block() {
            buffer::get(key)
        } else {
            let Some(h) = buffer::read(key) else {
                return partial_io_result(buf_offset);
            };
            h
        };

        let end = buf_offset + chunk.len;
        handle.write_bytes(chunk.offset, &buf[buf_offset..end]);
        *pos += chunk.len;
        buf_offset = end;
    }

    Ok(buf_offset)
}

fn partial_io_result(done: usize) -> Result<usize> {
    if done == 0 { Err(Errno::IO) } else { Ok(done) }
}

impl BlockChunk {
    /// Split a byte offset and remaining byte count into one buffer-cache slice.
    fn new(pos: usize, remaining: usize) -> Result<Self> {
        let block_number = u32::try_from(pos / BLOCK_SIZE).map_err(|_| Errno::INVAL)?;
        let offset = pos % BLOCK_SIZE;
        let len = (BLOCK_SIZE - offset).min(remaining);

        Ok(Self {
            block_number,
            offset,
            len,
        })
    }

    /// Return whether this chunk overwrites the whole cache block.
    fn is_full_block(&self) -> bool {
        self.offset == 0 && self.len == BLOCK_SIZE
    }
}
