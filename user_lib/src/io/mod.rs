//! Traits, helpers, and types for core I/O functionality.
//!
//! Counterpart to [`std::io`]. The module centers on three traits — [`Read`],
//! [`Write`], and [`Seek`] — that abstract over byte streams, plus the
//! [`Error`] / [`ErrorKind`] / [`Result`] error machinery, and the global
//! [`Stdin`], [`Stdout`], and [`Stderr`] handles produced by [`stdin`],
//! [`stdout`], and [`stderr`].
//!
//! Line-oriented input is provided through the [`BufRead`] trait and the
//! [`BufReader`] adapter, with [`Lines`] and [`Split`] iterators built on
//! top.
//!
//! Higher-level adapters (`Cursor`, `Bytes`) are not yet provided.

mod buffered;
mod error;
mod stdio;

use alloc::{string::String, vec::Vec};
use core::{cmp, fmt};

pub use buffered::{BufReader, BufWriter, LineWriter, Lines, Split};
pub(crate) use error::const_io_error;
pub use error::{Error, ErrorKind, Result};
pub(crate) use stdio::flush_stdout;
#[doc(hidden)]
pub use stdio::{_eprint, _print};
pub use stdio::{Stderr, Stdin, Stdout, stderr, stdin, stdout};

const DEFAULT_BUF_SIZE: usize = 1024;

/// The `Read` trait allows for reading bytes from a source.
///
/// Implementors are called *readers*. Readers are defined by one required
/// method, [`read`], which fills a caller-provided buffer with bytes and
/// reports how many were read. The `read_to_end`, `read_to_string`,
/// `read_exact`, and adapter methods are all built on top of it.
///
/// [`read`]: Read::read
pub trait Read {
    /// Pulls some bytes from this source into `buf`, returning how many bytes
    /// were read.
    ///
    /// A return value of `Ok(0)` means end of input. A short read shorter
    /// than `buf.len()` is not an error.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    /// Reads all bytes until EOF, appending them to `buf`.
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        default_read_to_end(self, buf)
    }

    /// Reads all bytes until EOF, parsing them as UTF-8 and appending to
    /// `buf`. Returns an error of kind [`ErrorKind::InvalidData`] if the
    /// bytes are not valid UTF-8.
    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        let start = buf.len();
        let mut bytes = core::mem::take(buf).into_bytes();
        let read = self.read_to_end(&mut bytes)?;
        match String::from_utf8(bytes) {
            Ok(string) => {
                *buf = string;
                Ok(read)
            }
            Err(error) => {
                let mut recovered = error.into_bytes();
                recovered.truncate(start);
                // SAFETY: bytes before `start` are unchanged from the
                // original String, so the prefix is still valid UTF-8.
                *buf = unsafe { String::from_utf8_unchecked(recovered) };
                Err(Error::from(ErrorKind::InvalidData))
            }
        }
    }

    /// Reads exactly enough bytes to fill `buf`. Returns
    /// [`ErrorKind::UnexpectedEof`] if EOF is reached first, and retries on
    /// [`ErrorKind::Interrupted`].
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = buf;
                    buf = &mut tmp[n..];
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if buf.is_empty() {
            Ok(())
        } else {
            Err(const_io_error(
                ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            ))
        }
    }

    /// Borrows this reader by mutable reference.
    fn by_ref(&mut self) -> &mut Self
    where Self: Sized {
        self
    }

    /// Returns an adapter that reads at most `limit` bytes from this reader.
    fn take(self, limit: u64) -> Take<Self>
    where Self: Sized {
        Take { inner: self, limit }
    }
}

/// A trait for objects which are byte-oriented sinks.
///
/// Implementors are called *writers*. Writers are defined by two required
/// methods, [`write`] and [`flush`].
///
/// [`write`]: Write::write
/// [`flush`]: Write::flush
pub trait Write {
    /// Writes a buffer into this writer, returning how many bytes were
    /// written.
    fn write(&mut self, buf: &[u8]) -> Result<usize>;

    /// Flushes any internally buffered output, ensuring it reaches its
    /// destination.
    fn flush(&mut self) -> Result<()>;

    /// Writes the entire `buf` to the writer, retrying on partial writes and
    /// on [`ErrorKind::Interrupted`]. Fails with [`ErrorKind::WriteZero`] if
    /// the writer accepts zero bytes before all of `buf` has been consumed.
    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => {
                    return Err(const_io_error(
                        ErrorKind::WriteZero,
                        "failed to write whole buffer",
                    ));
                }
                Ok(n) => buf = &buf[n..],
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Writes a formatted string into this writer, returning any error
    /// encountered.
    fn write_fmt(&mut self, fmt: fmt::Arguments<'_>) -> Result<()> {
        struct Adapter<'a, W: Write + ?Sized> {
            inner: &'a mut W,
            error: Result<()>,
        }

        impl<W: Write + ?Sized> fmt::Write for Adapter<'_, W> {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                match self.inner.write_all(s.as_bytes()) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        self.error = Err(error);
                        Err(fmt::Error)
                    }
                }
            }
        }

        let mut adapter = Adapter {
            inner: self,
            error: Ok(()),
        };
        match fmt::Write::write_fmt(&mut adapter, fmt) {
            Ok(()) => Ok(()),
            Err(_) => match adapter.error {
                Err(error) => Err(error),
                Ok(()) => Err(const_io_error(ErrorKind::Other, "formatter error")),
            },
        }
    }

    /// Borrows this writer by mutable reference.
    fn by_ref(&mut self) -> &mut Self
    where Self: Sized {
        self
    }
}

/// Enumeration of possible methods to seek within an I/O object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SeekFrom {
    /// Sets the offset to the provided number of bytes.
    Start(u64),
    /// Sets the offset to the size of this object plus the specified number
    /// of bytes (which may be negative).
    End(i64),
    /// Sets the offset to the current position plus the specified number of
    /// bytes (which may be negative).
    Current(i64),
}

/// The `Seek` trait provides a cursor which can be moved within a stream of
/// bytes.
pub trait Seek {
    /// Seeks to an offset in the stream and returns the new position from
    /// the start.
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;

    /// Rewinds to the beginning of the stream.
    fn rewind(&mut self) -> Result<()> {
        self.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    /// Returns the current position from the start of the stream.
    fn stream_position(&mut self) -> Result<u64> {
        self.seek(SeekFrom::Current(0))
    }
}

/// A `BufRead` is a type of `Reader` which has an internal buffer, allowing
/// it to perform extra ways of reading.
///
/// Counterpart to [`std::io::BufRead`]. Implementors provide [`fill_buf`]
/// (return the bytes currently available without doing more I/O than
/// necessary) and [`consume`] (advance past `amt` bytes of the previously
/// returned slice). All line / delimiter helpers are built on top.
///
/// [`fill_buf`]: BufRead::fill_buf
/// [`consume`]: BufRead::consume
pub trait BufRead: Read {
    /// Returns the contents of the internal buffer, filling it with more
    /// data from the inner reader if it is empty.
    fn fill_buf(&mut self) -> Result<&[u8]>;

    /// Marks the first `amt` bytes of the internal buffer as consumed so
    /// they are no longer returned by [`fill_buf`] / [`read`].
    ///
    /// [`fill_buf`]: BufRead::fill_buf
    /// [`read`]: Read::read
    fn consume(&mut self, amt: usize);

    /// Returns `true` if the internal buffer has any pending data, fetching
    /// more from the inner reader if needed.
    fn has_data_left(&mut self) -> Result<bool> {
        self.fill_buf().map(|b| !b.is_empty())
    }

    /// Reads bytes until `delim` is found, appending them (including the
    /// delimiter) to `buf`. Returns the total number of bytes appended.
    /// On EOF returns `Ok(0)` without modifying `buf` beyond what had
    /// already been appended.
    fn read_until(&mut self, delim: u8, buf: &mut Vec<u8>) -> Result<usize> {
        default_read_until(self, delim, buf)
    }

    /// Reads bytes until `delim` is found, discarding them. Returns the
    /// total number of bytes discarded (including the delimiter).
    fn skip_until(&mut self, delim: u8) -> Result<usize> {
        let mut total = 0;
        loop {
            let (done, used) = {
                let available = match self.fill_buf() {
                    Ok(slice) => slice,
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                match available.iter().position(|&b| b == delim) {
                    Some(i) => (true, i + 1),
                    None => (false, available.len()),
                }
            };
            self.consume(used);
            total += used;
            if done || used == 0 {
                return Ok(total);
            }
        }
    }

    /// Reads bytes from the underlying reader until a newline (the `0x0A`
    /// byte) is found, appending them to `buf` including the newline. The
    /// trailing newline is *not* stripped — callers usually strip it
    /// themselves.
    ///
    /// Returns `Ok(0)` at EOF. Returns [`ErrorKind::InvalidData`] if the
    /// appended bytes are not valid UTF-8; on that error `buf` is rolled
    /// back to its original length.
    fn read_line(&mut self, buf: &mut String) -> Result<usize> {
        // SAFETY: we hold the borrow on `buf.as_mut_vec()` only while
        // appending raw bytes, and we either commit (with a UTF-8 check)
        // or roll back before returning so the `String` invariant holds
        // again before any user code observes it.
        let start = buf.len();
        let mut bytes = core::mem::take(buf).into_bytes();
        let appended = self.read_until(b'\n', &mut bytes)?;
        match core::str::from_utf8(&bytes[start..]) {
            Ok(_) => {
                // SAFETY: prefix was already valid UTF-8 by `String`
                // invariant, and the newly appended slice was just
                // validated above.
                *buf = unsafe { String::from_utf8_unchecked(bytes) };
                Ok(appended)
            }
            Err(_) => {
                bytes.truncate(start);
                // SAFETY: bytes before `start` are unchanged from the
                // original `String`, so the prefix is still valid UTF-8.
                *buf = unsafe { String::from_utf8_unchecked(bytes) };
                Err(const_io_error(
                    ErrorKind::InvalidData,
                    "stream did not contain valid UTF-8",
                ))
            }
        }
    }

    /// Returns an iterator over the contents of this reader split on the
    /// byte `delim`.
    fn split(self, delim: u8) -> Split<Self>
    where Self: Sized {
        Split::new(self, delim)
    }

    /// Returns an iterator over the lines of this reader.
    ///
    /// Each yielded `String` has its trailing `\n` (and preceding `\r` if
    /// present) stripped, matching [`std::io::Lines`].
    fn lines(self) -> Lines<Self>
    where Self: Sized {
        Lines::new(self)
    }
}

/// Reader adaptor that limits the number of bytes read from the underlying
/// reader.
///
/// Returned by [`Read::take`].
pub struct Take<T> {
    inner: T,
    limit: u64,
}

impl<T> Take<T> {
    /// Returns the remaining number of bytes that can be read.
    #[inline]
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Sets a new byte limit on this adapter.
    #[inline]
    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
    }

    /// Consumes the adapter and returns the underlying reader.
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Returns a shared reference to the underlying reader.
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Returns a mutable reference to the underlying reader.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: Read> Read for Take<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.limit == 0 {
            return Ok(0);
        }
        let max = cmp::min(buf.len() as u64, self.limit) as usize;
        let n = self.inner.read(&mut buf[..max])?;
        self.limit -= n as u64;
        Ok(n)
    }
}

/// Default implementation of `read_to_end` shared by all readers.
fn default_read_to_end<R: Read + ?Sized>(reader: &mut R, buf: &mut Vec<u8>) -> Result<usize> {
    let start_len = buf.len();
    let mut probe = [0u8; DEFAULT_BUF_SIZE];
    loop {
        match reader.read(&mut probe) {
            Ok(0) => return Ok(buf.len() - start_len),
            Ok(n) => buf.extend_from_slice(&probe[..n]),
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Default implementation of `read_until` shared by `BufRead` and
/// `BufReader`.
fn default_read_until<R: BufRead + ?Sized>(
    reader: &mut R,
    delim: u8,
    buf: &mut Vec<u8>,
) -> Result<usize> {
    let mut total = 0;
    loop {
        let (done, used) = {
            let available = match reader.fill_buf() {
                Ok(slice) => slice,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            match available.iter().position(|&b| b == delim) {
                Some(i) => {
                    buf.extend_from_slice(&available[..=i]);
                    (true, i + 1)
                }
                None => {
                    buf.extend_from_slice(available);
                    (false, available.len())
                }
            }
        };
        reader.consume(used);
        total += used;
        if done || used == 0 {
            return Ok(total);
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory adapter impls
// ---------------------------------------------------------------------------

impl Read for &[u8] {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let amt = cmp::min(buf.len(), self.len());
        let (head, tail) = self.split_at(amt);
        if amt == 1 {
            buf[0] = head[0];
        } else {
            buf[..amt].copy_from_slice(head);
        }
        *self = tail;
        Ok(amt)
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        buf.extend_from_slice(self);
        let amt = self.len();
        *self = &[];
        Ok(amt)
    }
}

impl BufRead for &[u8] {
    #[inline]
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(*self)
    }

    #[inline]
    fn consume(&mut self, amt: usize) {
        *self = &self[cmp::min(amt, self.len())..];
    }
}

impl Write for &mut [u8] {
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        let amt = cmp::min(data.len(), self.len());
        let (head, tail) = core::mem::take(self).split_at_mut(amt);
        head.copy_from_slice(&data[..amt]);
        *self = tail;
        Ok(amt)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
