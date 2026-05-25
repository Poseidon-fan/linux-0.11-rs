//! Buffered writer adapters.
//!
//! [`BufWriter`] coalesces small writes into a heap-backed buffer and flushes
//! it to the underlying writer in larger chunks. [`LineWriter`] specialises
//! [`BufWriter`] for line-oriented streams: a write that contains a newline
//! flushes the buffer up through that newline immediately, so concurrent
//! producers (different processes writing to a shared TTY) interleave at
//! line boundaries rather than at format-fragment boundaries.

use alloc::vec::Vec;

use super::{ErrorKind, Result, Write, const_io_error};

/// Wraps a writer and buffers its output.
///
/// Writes go into an in-memory buffer and only reach the underlying writer
/// when the buffer fills up, when [`flush`] is called, or when the
/// `BufWriter` is dropped.
///
/// [`flush`]: Write::flush
pub struct BufWriter<W: Write> {
    inner: W,
    buffer: Vec<u8>,
    capacity: usize,
}

impl<W: Write> BufWriter<W> {
    /// Default buffer size. Sized so a typical formatted line fits without
    /// growing the buffer, but small enough to keep memory pressure modest.
    pub const DEFAULT_CAPACITY: usize = 1024;

    /// Creates a new `BufWriter` with the default buffer capacity.
    #[inline]
    pub const fn new(inner: W) -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY, inner)
    }

    /// Creates a new `BufWriter` with the given buffer capacity.
    ///
    /// The buffer itself is allocated lazily on the first write, so this
    /// constructor is callable in `const` contexts (e.g. for static stdio
    /// handles).
    #[inline]
    pub const fn with_capacity(capacity: usize, inner: W) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            capacity,
        }
    }

    /// Returns a reference to the underlying writer.
    #[inline]
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Returns a mutable reference to the underlying writer.
    ///
    /// Writing through this reference bypasses the buffer.
    #[inline]
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    /// Returns the bytes currently held in the buffer.
    #[inline]
    pub fn buffer(&self) -> &[u8] {
        self.buffer.as_slice()
    }

    /// Returns the configured buffer capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Drains the buffer into the underlying writer.
    fn flush_buf(&mut self) -> Result<()> {
        let mut written = 0;
        while written < self.buffer.len() {
            match self.inner.write(&self.buffer[written..]) {
                Ok(0) => {
                    return Err(const_io_error(
                        ErrorKind::WriteZero,
                        "failed to write the buffered data",
                    ));
                }
                Ok(n) => written += n,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => {
                    self.buffer.drain(..written);
                    return Err(e);
                }
            }
        }
        self.buffer.clear();
        Ok(())
    }
}

impl<W: Write> Write for BufWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if self.buffer.len() + buf.len() > self.capacity {
            self.flush_buf()?;
        }
        if buf.len() >= self.capacity {
            return self.inner.write(buf);
        }
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        self.flush_buf()?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for BufWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush_buf();
    }
}

/// Wraps a writer and buffers output, flushing every time a newline is
/// written.
///
/// This is the right wrapper for line-oriented streams shared by multiple
/// producers (notably the TTY): every complete line reaches the underlying
/// writer in a single call, so concurrent writers interleave at line
/// boundaries rather than mid-line.
pub struct LineWriter<W: Write> {
    inner: BufWriter<W>,
}

impl<W: Write> LineWriter<W> {
    /// Creates a new `LineWriter` with the default buffer capacity.
    #[inline]
    pub const fn new(inner: W) -> Self {
        Self::with_capacity(BufWriter::<W>::DEFAULT_CAPACITY, inner)
    }

    /// Creates a new `LineWriter` with the given buffer capacity.
    #[inline]
    pub const fn with_capacity(capacity: usize, inner: W) -> Self {
        Self {
            inner: BufWriter::with_capacity(capacity, inner),
        }
    }

    /// Returns a reference to the underlying writer.
    #[inline]
    pub fn get_ref(&self) -> &W {
        self.inner.get_ref()
    }

    /// Returns a mutable reference to the underlying writer.
    #[inline]
    pub fn get_mut(&mut self) -> &mut W {
        self.inner.get_mut()
    }
}

impl<W: Write> Write for LineWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        match buf.iter().rposition(|&byte| byte == b'\n') {
            None => self.inner.write(buf),
            Some(idx) => {
                let through_newline = &buf[..=idx];
                self.inner.write_all(through_newline)?;
                self.inner.flush()?;

                let remainder = &buf[idx + 1..];
                if !remainder.is_empty() {
                    self.inner.write_all(remainder)?;
                }
                Ok(buf.len())
            }
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.inner.flush()
    }
}
