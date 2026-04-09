//! Pipe file implementation.
//!
//! A pipe provides a unidirectional byte channel between processes.  Two
//! `PipeFile` endpoints (one read, one write) share a `PipeShared` that
//! contains a 4 KB ring buffer and a wait queue.
//!
//! ```text
//! Writer ──► PipeFile(is_write=true) ──┐
//!                                      ├── Arc<PipeShared>
//! Reader ◄── PipeFile(is_write=false) ─┘
//!                  │
//!            ┌─────┴──────┐
//!            │ PipeState   │
//!            │  buffer[4K] │
//!            │  head/tail  │
//!            │  readers    │
//!            │  writers    │
//!            └─────────────┘
//! ```

use alloc::sync::Arc;

use user_lib::fs::Stat;

use super::File;
use crate::{
    mm::frame::{self, PAGE_SIZE, PhysFrame},
    signal::SIGPIPE,
    sync::KernelCell,
    syscall::{ENOMEM, EPIPE},
    task::{self, WaitQueue},
};

const PIPE_BUF_SIZE: usize = PAGE_SIZE;
const WRAP_MASK: usize = PIPE_BUF_SIZE - 1;

/// Mutable pipe state protected by `KernelCell`.
///
/// The buffer is a raw physical page obtained from the frame allocator.
/// `PhysFrame`'s `Drop` returns the page when the pipe is destroyed.
struct PipeState {
    frame: PhysFrame,
    head: usize,
    tail: usize,
    readers: u32,
    writers: u32,
}

impl PipeState {
    /// Base pointer of the 4 KB pipe buffer page.
    #[inline]
    fn buffer_ptr(&self) -> *mut u8 {
        self.frame.ppn.addr().as_mut_ptr()
    }

    /// Bytes available for reading.
    #[inline]
    fn size(&self) -> usize {
        self.head.wrapping_sub(self.tail) & WRAP_MASK
    }

    /// Bytes of free space for writing (max `PIPE_BUF_SIZE - 1`).
    #[inline]
    fn space(&self) -> usize {
        (PIPE_BUF_SIZE - 1) - self.size()
    }
}

/// Shared state between the read and write ends of a pipe.
struct PipeShared {
    state: KernelCell<PipeState>,
    wait: WaitQueue,
}

/// An opened pipe endpoint.
///
/// When `is_write` is false this is the read end; when true, the write end.
/// Dropping the last `Arc` to a given endpoint decrements the corresponding
/// reader/writer count in `PipeState` and wakes any blocked peer.
pub struct PipeFile {
    shared: Arc<PipeShared>,
    is_write: bool,
}

impl PipeFile {
    /// Create a connected (reader, writer) pair ready to install into fds.
    ///
    /// The buffer page is allocated from the physical frame allocator.
    pub fn create_pair() -> Result<(Arc<Self>, Arc<Self>), u32> {
        let page = frame::alloc().ok_or(ENOMEM)?;
        let shared = Arc::new(PipeShared {
            state: KernelCell::new(PipeState {
                frame: page,
                head: 0,
                tail: 0,
                readers: 1,
                writers: 1,
            }),
            wait: WaitQueue::new(),
        });
        let reader = Arc::new(PipeFile {
            shared: Arc::clone(&shared),
            is_write: false,
        });
        let writer = Arc::new(PipeFile {
            shared,
            is_write: true,
        });
        Ok((reader, writer))
    }
}

impl File for PipeFile {
    /// Read from the pipe (read-end only).
    ///
    /// Blocks (uninterruptible) while the buffer is empty and at least one
    /// write end is still open.  Returns 0 (EOF) when all writers are gone
    /// and the buffer is drained.
    fn read(&self, output: &mut [u8]) -> Result<usize, u32> {
        let count = output.len();
        let mut total = 0usize;

        while total < count {
            let (chunk, no_writers) = self.shared.state.exclusive(|s| {
                let size = s.size();
                if size == 0 {
                    return (0, s.writers == 0);
                }
                let chars = (PIPE_BUF_SIZE - s.tail).min(count - total).min(size);
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        s.buffer_ptr().add(s.tail),
                        output[total..].as_mut_ptr(),
                        chars,
                    );
                }
                s.tail = (s.tail + chars) & WRAP_MASK;
                (chars, false)
            });

            if chunk > 0 {
                total += chunk;
                continue;
            }

            // Buffer is empty.
            self.shared.wait.wake();
            if no_writers {
                return Ok(total);
            }
            self.shared.wait.sleep();
        }

        self.shared.wait.wake();
        Ok(total)
    }

    /// Write to the pipe (write-end only).
    ///
    /// Blocks (uninterruptible) while the buffer is full and at least one
    /// read end is still open.  Delivers `SIGPIPE` and returns `EPIPE` when
    /// all readers are gone.
    fn write(&self, input: &[u8]) -> Result<usize, u32> {
        let count = input.len();
        let mut total = 0usize;

        while total < count {
            let (chunk, no_readers) = self.shared.state.exclusive(|s| {
                let space = s.space();
                if space == 0 {
                    return (0, s.readers == 0);
                }
                let chars = (PIPE_BUF_SIZE - s.head).min(count - total).min(space);
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        input[total..].as_ptr(),
                        s.buffer_ptr().add(s.head),
                        chars,
                    );
                }
                s.head = (s.head + chars) & WRAP_MASK;
                (chars, false)
            });

            if chunk > 0 {
                total += chunk;
                continue;
            }

            // Buffer is full.
            self.shared.wait.wake();
            if no_readers {
                task::with_current(|inner| inner.signal_info.raise(SIGPIPE));
                return if total > 0 { Ok(total) } else { Err(EPIPE) };
            }
            self.shared.wait.sleep();
        }

        self.shared.wait.wake();
        Ok(total)
    }

    fn stat(&self) -> Result<Stat, u32> {
        let size = self.shared.state.exclusive(|s| s.size());
        Ok(Stat {
            st_dev: 0,
            st_ino: 0,
            st_mode: 0o10600, // S_IFIFO | owner rw
            st_nlink: 1,
            st_uid: 0,
            st_gid: 0,
            st_rdev: 0,
            st_size: size as u32,
            st_atime: 0,
            st_mtime: 0,
            st_ctime: 0,
        })
    }
}

impl Drop for PipeFile {
    fn drop(&mut self) {
        self.shared.state.exclusive(|s| {
            if self.is_write {
                s.writers -= 1;
            } else {
                s.readers -= 1;
            }
        });
        self.shared.wait.wake();
    }
}
