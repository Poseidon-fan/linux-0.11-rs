//! Utility readers and writers: [`empty`], [`sink`], [`repeat`].

use alloc::vec::Vec;
use core::fmt;

use crate::io::{BufRead, Read, Result, Seek, SeekFrom, Write};

// ---------------------------------------------------------------------------
// Empty — always-EOF reader that ignores writes
// ---------------------------------------------------------------------------

/// A reader that is always at EOF, and a writer that discards all data.
///
/// Returned by [`empty()`].
#[derive(Copy, Clone, Debug, Default)]
pub struct Empty;

/// Creates a value that is always at EOF for reads, and ignores all data
/// written to it.
///
/// Counterpart to [`std::io::empty`].
///
/// # Examples
///
/// ```
/// use user_lib::io::{self, Read};
///
/// let mut buf = [0u8; 4];
/// assert_eq!(io::empty().read(&mut buf)?, 0);
/// # Ok::<(), io::Error>(())
/// ```
pub const fn empty() -> Empty {
    Empty
}

impl Read for Empty {
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        if buf.is_empty() {
            Ok(())
        } else {
            Err(crate::io::const_io_error(
                crate::io::ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            ))
        }
    }

    fn read_to_end(&mut self, _buf: &mut Vec<u8>) -> Result<usize> {
        Ok(0)
    }
}

impl BufRead for Empty {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(&[])
    }

    fn consume(&mut self, _amt: usize) {}
}

impl Seek for Empty {
    fn seek(&mut self, _pos: SeekFrom) -> Result<u64> {
        Ok(0)
    }
}

impl Write for Empty {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    fn write_all(&mut self, _buf: &[u8]) -> Result<()> {
        Ok(())
    }

    fn write_fmt(&mut self, _args: fmt::Arguments<'_>) -> Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Sink — writer that swallows all data
// ---------------------------------------------------------------------------

/// A writer which moves data into the void.
///
/// Returned by [`sink()`].
#[derive(Copy, Clone, Debug, Default)]
pub struct Sink;

/// Creates an instance of a writer which will successfully consume all data.
///
/// Counterpart to [`std::io::sink`].
///
/// # Examples
///
/// ```
/// use user_lib::io::{self, Write};
///
/// let n = io::sink().write(b"hello world")?;
/// assert_eq!(n, 11);
/// # Ok::<(), io::Error>(())
/// ```
pub const fn sink() -> Sink {
    Sink
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    fn write_all(&mut self, _buf: &[u8]) -> Result<()> {
        Ok(())
    }

    fn write_fmt(&mut self, _args: fmt::Arguments<'_>) -> Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Repeat — reader that endlessly repeats one byte
// ---------------------------------------------------------------------------

/// A reader that infinitely repeats one byte.
///
/// Returned by [`repeat()`].
#[derive(Copy, Clone, Debug)]
pub struct Repeat {
    byte: u8,
}

/// Creates an instance of a reader that infinitely repeats one byte.
///
/// Counterpart to [`std::io::repeat`].
///
/// # Examples
///
/// ```
/// use user_lib::io::{self, Read};
///
/// let mut buf = [0u8; 3];
/// io::repeat(0b101).read_exact(&mut buf)?;
/// assert_eq!(buf, [0b101, 0b101, 0b101]);
/// # Ok::<(), io::Error>(())
/// ```
pub const fn repeat(byte: u8) -> Repeat {
    Repeat { byte }
}

impl Read for Repeat {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        buf.fill(self.byte);
        Ok(buf.len())
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        buf.fill(self.byte);
        Ok(())
    }

    fn read_to_end(&mut self, _buf: &mut Vec<u8>) -> Result<usize> {
        Err(crate::io::const_io_error(
            crate::io::ErrorKind::OutOfMemory,
            "infinite repeat cannot be read to end",
        ))
    }
}
