//! Buffered I/O adapters.
//!
//! - [`BufReader`] wraps a [`Read`] source and exposes line / delimiter
//!   reading through [`BufRead`].
//! - [`BufWriter`] coalesces small writes into a heap-backed buffer and
//!   flushes it to the underlying writer in larger chunks.
//! - [`LineWriter`] specialises [`BufWriter`] for line-oriented streams: a
//!   write that contains a newline flushes the buffer up through that
//!   newline immediately, so concurrent producers (different processes
//!   writing to a shared TTY) interleave at line boundaries rather than at
//!   format-fragment boundaries.

use alloc::{string::String, vec, vec::Vec};

use super::{BufRead, ErrorKind, Read, Result, Seek, SeekFrom, Write, const_io_error};

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

// ---------------------------------------------------------------------------
// BufReader
// ---------------------------------------------------------------------------

/// The `BufReader<R>` struct adds buffering to any reader.
///
/// Counterpart to [`std::io::BufReader`]. The buffer is allocated up front
/// at construction; capacity defaults to [`BufReader::DEFAULT_CAPACITY`]
/// (8 KiB).
///
/// `R` is the last field so the type stays valid when `R: ?Sized`, which is
/// how `std` makes `BufReader<dyn Read>` constructible.
pub struct BufReader<R: ?Sized> {
    buf: alloc::boxed::Box<[u8]>,
    pos: usize,
    cap: usize,
    inner: R,
}

impl<R: Read> BufReader<R> {
    /// Default per-instance buffer size, matching [`std::io::BufReader`].
    pub const DEFAULT_CAPACITY: usize = 8 * 1024;

    /// Creates a new `BufReader` with the default capacity.
    pub fn new(inner: R) -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY, inner)
    }

    /// Creates a new `BufReader` with the given capacity.
    pub fn with_capacity(capacity: usize, inner: R) -> Self {
        Self {
            buf: vec![0u8; capacity].into_boxed_slice(),
            pos: 0,
            cap: 0,
            inner,
        }
    }
}

impl<R: ?Sized> BufReader<R> {
    /// Returns a reference to the underlying reader.
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Returns a mutable reference to the underlying reader.
    ///
    /// Reads through this reference bypass the buffer and can desynchronise
    /// it; use [`BufReader::seek`] to recover if needed.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Returns the bytes currently buffered but not yet consumed.
    pub fn buffer(&self) -> &[u8] {
        &self.buf[self.pos..self.cap]
    }

    /// Returns the configured buffer capacity.
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Drops any buffered data without reading from the inner stream.
    fn discard_buffer(&mut self) {
        self.pos = 0;
        self.cap = 0;
    }
}

impl<R: Read> BufReader<R> {
    /// Unwraps the buffered reader, returning the inner reader.
    ///
    /// Any data left in the buffer is discarded.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read + ?Sized> Read for BufReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Bypass the buffer entirely for reads at least as large as the
        // capacity, when there's nothing already buffered. Matches std.
        if self.pos == self.cap && buf.len() >= self.buf.len() {
            self.discard_buffer();
            return self.inner.read(buf);
        }
        let available = self.fill_buf()?;
        let n = core::cmp::min(available.len(), buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let buffered = self.cap - self.pos;
        buf.extend_from_slice(&self.buf[self.pos..self.cap]);
        self.discard_buffer();
        let from_inner = self.inner.read_to_end(buf)?;
        Ok(buffered + from_inner)
    }
}

impl<R: Read + ?Sized> BufRead for BufReader<R> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        if self.pos >= self.cap {
            debug_assert_eq!(self.pos, self.cap);
            // Refill from the inner reader, retrying through Interrupted so
            // callers don't have to.
            loop {
                match self.inner.read(&mut self.buf) {
                    Ok(n) => {
                        self.cap = n;
                        self.pos = 0;
                        break;
                    }
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(&self.buf[self.pos..self.cap])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = core::cmp::min(self.pos + amt, self.cap);
    }
}

impl<R: Seek + Read + ?Sized> Seek for BufReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u32> {
        // Optimisation: a small `Current(n)` seek that lands inside the
        // already-buffered region only needs to nudge `self.pos`.
        if let SeekFrom::Current(offset) = pos {
            let buffered = (self.cap - self.pos) as i32;
            let absorbed = -(self.pos as i32);
            if absorbed <= offset && offset <= buffered {
                let new_pos = (self.pos as i32 + offset) as usize;
                self.pos = new_pos;
                // Best-effort: ask the inner reader for its current logical
                // position so callers see a sensible value.
                let inner_pos = self.inner.stream_position()?;
                let buffered_remaining = (self.cap - self.pos) as u32;
                return Ok(inner_pos.saturating_sub(buffered_remaining));
            }
        }
        self.discard_buffer();
        self.inner.seek(pos)
    }
}

impl<R: ?Sized> core::fmt::Debug for BufReader<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BufReader")
            .field("buffer", &self.buffer())
            .field("capacity", &self.capacity())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Lines / Split iterators
// ---------------------------------------------------------------------------

/// An iterator over the lines of an instance of [`BufRead`].
///
/// Returned by [`BufRead::lines`]. Each line is stripped of its trailing
/// `\n` (and the preceding `\r` if present), matching
/// [`std::io::Lines`].
pub struct Lines<B> {
    buf: B,
}

impl<B> Lines<B> {
    pub(crate) fn new(buf: B) -> Self {
        Self { buf }
    }
}

impl<B: BufRead> Iterator for Lines<B> {
    type Item = Result<String>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        match self.buf.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Some(Ok(line))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

impl<B: BufRead> core::iter::FusedIterator for Lines<B> {}

impl<B> core::fmt::Debug for Lines<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lines").finish()
    }
}

/// An iterator over the contents of a [`BufRead`] split on a single byte.
///
/// Returned by [`BufRead::split`]. The yielded `Vec<u8>` does **not**
/// include the trailing delimiter.
pub struct Split<B> {
    buf: B,
    delim: u8,
}

impl<B> Split<B> {
    pub(crate) fn new(buf: B, delim: u8) -> Self {
        Self { buf, delim }
    }
}

impl<B: BufRead> Iterator for Split<B> {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut chunk = Vec::new();
        match self.buf.read_until(self.delim, &mut chunk) {
            Ok(0) => None,
            Ok(_) => {
                if chunk.last().copied() == Some(self.delim) {
                    chunk.pop();
                }
                Some(Ok(chunk))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

impl<B: BufRead> core::iter::FusedIterator for Split<B> {}

impl<B> core::fmt::Debug for Split<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Split").field("delim", &self.delim).finish()
    }
}
