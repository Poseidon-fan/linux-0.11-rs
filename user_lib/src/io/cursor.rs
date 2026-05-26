//! In-memory reader / writer backed by a byte buffer.

use alloc::vec::Vec;
use core::cmp;

use crate::io::{self, BufRead, Read, Result, Seek, SeekFrom, Write};

/// A `Cursor` wraps an in-memory buffer and provides it with a [`Seek`]
/// implementation.
///
/// `Cursor<T>` implements [`Read`] and [`BufRead`] when `T: AsRef<[u8]>`,
/// and implements [`Write`] when `T: &mut [u8]` or `T: Vec<u8>`.
///
/// Counterpart to [`std::io::Cursor`].
///
/// # Examples
///
/// ```
/// use user_lib::io::{self, Cursor, Read, Write};
///
/// let mut buf = [0u8; 10];
/// let mut cursor = Cursor::new(&mut buf[..]);
/// cursor.write_all(b"hello")?;
/// assert_eq!(cursor.position(), 5);
/// # Ok::<(), io::Error>(())
/// ```
#[derive(Debug, Default, Eq, PartialEq)]
pub struct Cursor<T> {
    inner: T,
    pos: u64,
}

impl<T> Cursor<T> {
    /// Creates a new cursor wrapping the provided in-memory buffer.
    ///
    /// The initial position is `0`, even if the underlying buffer already
    /// contains data.
    pub const fn new(inner: T) -> Cursor<T> {
        Cursor { pos: 0, inner }
    }

    /// Consumes this cursor, returning the underlying value.
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Returns a shared reference to the underlying buffer.
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Returns a mutable reference to the underlying buffer.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Returns the current byte position of this cursor.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// Sets the byte position of this cursor.
    pub fn set_position(&mut self, pos: u64) {
        self.pos = pos;
    }
}

impl<T: Clone> Clone for Cursor<T> {
    fn clone(&self) -> Self {
        Cursor {
            inner: self.inner.clone(),
            pos: self.pos,
        }
    }
}

// ——— Helpers ———

impl<T: AsRef<[u8]>> Cursor<T> {
    /// Splits the underlying slice at the cursor position, returning
    /// `(already_read, remaining)`.
    fn slice_pair(&self) -> (&[u8], &[u8]) {
        let slice = self.inner.as_ref();
        let pos = (self.pos as usize).min(slice.len());
        slice.split_at(pos)
    }
}

// ——— Read ———

impl<T: AsRef<[u8]>> Read for Cursor<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let remaining = self.slice_pair().1;
        let n = cmp::min(buf.len(), remaining.len());
        if n == 0 {
            return Ok(0);
        }
        buf[..n].copy_from_slice(&remaining[..n]);
        self.pos += n as u64;
        Ok(n)
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let remaining = self.slice_pair().1;
        let len = remaining.len();
        buf.extend_from_slice(remaining);
        self.pos += len as u64;
        Ok(len)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        let remaining = self.slice_pair().1;
        if buf.len() > remaining.len() {
            self.pos = self.inner.as_ref().len() as u64;
            return Err(io::const_io_error(
                io::ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            ));
        }
        buf.copy_from_slice(&remaining[..buf.len()]);
        self.pos += buf.len() as u64;
        Ok(())
    }
}

// ——— BufRead ———

impl<T: AsRef<[u8]>> BufRead for Cursor<T> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(self.slice_pair().1)
    }

    fn consume(&mut self, amt: usize) {
        self.pos += amt as u64;
    }
}

// ——— Seek ———

impl<T: AsRef<[u8]>> Seek for Cursor<T> {
    fn seek(&mut self, style: SeekFrom) -> Result<u64> {
        let (base, offset) = match style {
            SeekFrom::Start(n) => {
                self.pos = n;
                return Ok(n);
            }
            SeekFrom::End(n) => (self.inner.as_ref().len() as u64, n),
            SeekFrom::Current(n) => (self.pos, n),
        };
        match base.checked_add_signed(offset) {
            Some(n) => {
                self.pos = n;
                Ok(self.pos)
            }
            None => Err(io::const_io_error(
                io::ErrorKind::InvalidInput,
                "invalid seek to a negative or overflowing position",
            )),
        }
    }
}

// ——— Write for `&mut [u8]` (non-resizing) ———

impl Write for Cursor<&mut [u8]> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let pos = (self.pos as usize).min(self.inner.len());
        let writable = &mut self.inner[pos..];
        let n = cmp::min(buf.len(), writable.len());
        writable[..n].copy_from_slice(&buf[..n]);
        self.pos += n as u64;
        Ok(n)
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let n = self.write(buf)?;
        if n < buf.len() {
            Err(io::const_io_error(
                io::ErrorKind::WriteZero,
                "failed to write whole buffer",
            ))
        } else {
            Ok(())
        }
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

// ——— Write for `Vec<u8>` (resizing) ———

impl Write for Cursor<Vec<u8>> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let pos = self.pos as usize;

        // Ensure the vec is long enough; pad with zeros if pos > len.
        let required = pos.saturating_add(buf.len());
        if required > self.inner.len() {
            self.inner.resize(required, 0);
        }

        let dest = &mut self.inner[pos..pos + buf.len()];
        dest.copy_from_slice(buf);
        self.pos += buf.len() as u64;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.write(buf)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
