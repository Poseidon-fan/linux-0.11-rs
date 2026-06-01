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

use user_lib::syscall::{fs::Stat, signal::Signal};

use super::File;
use crate::{
    error::{Errno, Result},
    mm::frame::{self, PAGE_SIZE, PhysFrame},
    sync::KernelCell,
    task::{self, WaitQueue},
};

/// An opened pipe endpoint.
///
/// When `is_write` is false this is the read end; when true, the write end.
/// Dropping the last `Arc` to a given endpoint decrements the corresponding
/// reader/writer count in `PipeState` and wakes any blocked peer.
pub struct PipeFile {
    /// Shared ring buffer and endpoint bookkeeping.
    shared: Arc<PipeShared>,
    /// `true` for the write end, `false` for the read end.
    is_write: bool,
}

/// Size of the pipe ring buffer in bytes (one physical page).
const PIPE_BUF_SIZE: usize = PAGE_SIZE;
/// Mask used to wrap ring-buffer indices within [`PIPE_BUF_SIZE`].
const WRAP_MASK: usize = PIPE_BUF_SIZE - 1;

/// Shared state between the read and write ends of a pipe.
struct PipeShared {
    /// Ring-buffer state protected by a kernel critical section.
    state: KernelCell<PipeState>,
    /// Wait queue for blocked readers and writers.
    wait: WaitQueue,
}

/// Mutable pipe state protected by `KernelCell`.
///
/// The buffer is a raw physical page obtained from the frame allocator.
/// `PhysFrame`'s `Drop` returns the page when the pipe is destroyed.
struct PipeState {
    /// Physical frame backing the ring buffer.
    frame: PhysFrame,
    /// Write index into the ring buffer.
    head: usize,
    /// Read index into the ring buffer.
    tail: usize,
    /// Number of open read ends.
    readers: u32,
    /// Number of open write ends.
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

impl PipeFile {
    /// Create a connected (reader, writer) pair ready to install into fds.
    ///
    /// The buffer page is allocated from the physical frame allocator.
    pub fn create_pair() -> Result<(Arc<Self>, Arc<Self>)> {
        let page = frame::alloc().ok_or(Errno::NOMEM)?;
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
    fn read(&self, output: &mut [u8]) -> Result<usize> {
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
    /// read end is still open.  Delivers `SIGPIPE` and returns `Errno::PIPE` when
    /// all readers are gone.
    fn write(&self, input: &[u8]) -> Result<usize> {
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
                task::with_current(|inner| inner.signal_info.raise(Signal::Pipe as u32));
                return if total > 0 {
                    Ok(total)
                } else {
                    Err(Errno::PIPE)
                };
            }
            self.shared.wait.sleep();
        }

        self.shared.wait.wake();
        Ok(total)
    }

    fn stat(&self) -> Result<Stat> {
        let size = self.shared.state.exclusive(|s| s.size());
        Ok(Stat {
            st_dev: 0,
            st_ino: 0,
            st_mode: 0o10600, // S_IFIFO | owner rw
            st_nlink: 1,
            st_uid: 0,
            st_gid: 0,
            st_rdev: 0,
            st_size: size as i32,
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
