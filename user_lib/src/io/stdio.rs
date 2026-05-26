//! The standard input, output, and error streams.
//!
//! Counterpart to the stdio handle types in [`std::io`]. Each handle is a
//! zero-sized token referring to a fixed file descriptor — `0` for
//! [`Stdin`], `1` for [`Stdout`], `2` for [`Stderr`] — and is obtained from
//! the free functions [`stdin`], [`stdout`], and [`stderr`].
//!
//! [`Stdout`] is line-buffered: writes are coalesced in a process-wide
//! [`LineWriter`](super::LineWriter) and flushed when a newline appears or
//! when [`flush`](super::Write::flush) is called explicitly. This compresses
//! a `println!` (which `core::fmt` splits into several `write_str` calls)
//! into a single `write` syscall, so concurrent producers writing to a
//! shared TTY interleave at line boundaries instead of mid-line.
//!
//! [`Stderr`] is unbuffered, matching `std`'s convention so panic and
//! diagnostic output reaches the kernel even without a trailing newline.
//!
//! `std`'s `StdinLock` / `StdoutLock` / `StderrLock` types are absent: this
//! kernel runs single-threaded user processes, so there is nothing to lock
//! against. The bare handles are themselves the locked view.

use core::{cell::UnsafeCell, fmt};

use super::{Error, ErrorKind, LineWriter, Read, Result, Write};
use crate::syscall;

const MAX_RW_COUNT: usize = i32::MAX as usize;

/// A handle to the standard input of the current process.
///
/// Construct with [`stdin`].
#[derive(Debug)]
pub struct Stdin {
    _priv: (),
}

/// Constructs a new handle to the standard input of the current process.
#[must_use]
pub fn stdin() -> Stdin {
    Stdin { _priv: () }
}

impl Stdin {
    /// Reads a line of input from this handle into `buf`, including any
    /// trailing newline. Returns the number of bytes read.
    pub fn read_line(&mut self, buf: &mut alloc::string::String) -> Result<usize> {
        let mut byte = [0u8; 1];
        let start = buf.len();
        loop {
            match self.read(&mut byte)? {
                0 => break,
                _ => {
                    let ch = byte[0];
                    if !ch.is_ascii() {
                        return Err(Error::from(ErrorKind::InvalidData));
                    }
                    buf.push(ch as char);
                    if ch == b'\n' {
                        break;
                    }
                }
            }
        }
        Ok(buf.len() - start)
    }
}

impl Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        read_fd(0, buf)
    }
}

impl Read for &Stdin {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        read_fd(0, buf)
    }
}

/// A handle to the standard output of the current process.
///
/// Construct with [`stdout`]. All writes go through a process-wide
/// line-buffered writer; see the module documentation for details.
#[derive(Debug)]
pub struct Stdout {
    _priv: (),
}

/// Constructs a new handle to the standard output of the current process.
#[must_use]
pub fn stdout() -> Stdout {
    Stdout { _priv: () }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        STDOUT.with(|writer| writer.write(buf))
    }

    fn flush(&mut self) -> Result<()> {
        STDOUT.with(Write::flush)
    }
}

impl Write for &Stdout {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        STDOUT.with(|writer| writer.write(buf))
    }

    fn flush(&mut self) -> Result<()> {
        STDOUT.with(Write::flush)
    }
}

/// A handle to the standard error of the current process.
///
/// Construct with [`stderr`]. Writes are unbuffered.
#[derive(Debug)]
pub struct Stderr {
    _priv: (),
}

/// Constructs a new handle to the standard error of the current process.
#[must_use]
pub fn stderr() -> Stderr {
    Stderr { _priv: () }
}

impl Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        write_fd(2, buf)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Write for &Stderr {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        write_fd(2, buf)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Unbuffered fd-1 sink. The line-buffering layer lives one level up in
/// [`STDOUT`].
struct StdoutRaw;

impl Write for StdoutRaw {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        write_fd(1, buf)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Process-wide line-buffered stdout state.
///
/// Single-threaded user space lets us serve mutable access to the
/// `LineWriter` through an [`UnsafeCell`] without any locking primitive.
struct ProcessStdout {
    writer: UnsafeCell<LineWriter<StdoutRaw>>,
}

unsafe impl Sync for ProcessStdout {}

impl ProcessStdout {
    const fn new() -> Self {
        Self {
            writer: UnsafeCell::new(LineWriter::new(StdoutRaw)),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut LineWriter<StdoutRaw>) -> R) -> R {
        // SAFETY: user programs are single-threaded. Callers do not re-enter
        // this routine from within `f`.
        f(unsafe { &mut *self.writer.get() })
    }
}

static STDOUT: ProcessStdout = ProcessStdout::new();

/// Flushes any data buffered for [`Stdout`].
///
/// Called from [`crate::process::exit`] so a final partial line — written
/// without a trailing newline — still reaches the kernel before the process
/// terminates.
pub(crate) fn flush_stdout() {
    let _ = STDOUT.with(Write::flush);
}

fn read_fd(fd: u32, buf: &mut [u8]) -> Result<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    let count = core::cmp::min(buf.len(), MAX_RW_COUNT) as i32;
    match syscall::fs::read(fd, buf.as_mut_ptr(), count) {
        Ok(count) => Ok(count as usize),
        Err(errno) => Err(Error::from(errno)),
    }
}

fn write_fd(fd: u32, buf: &[u8]) -> Result<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    let count = core::cmp::min(buf.len(), MAX_RW_COUNT) as i32;
    match syscall::fs::write(fd, buf.as_ptr(), count) {
        Ok(count) => Ok(count as usize),
        Err(errno) => Err(Error::from(errno)),
    }
}

/// Internal helper used by `print!` / `println!`.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    if let Err(error) = stdout().write_fmt(args) {
        panic!("failed printing to stdout: {error}");
    }
}

/// Internal helper used by `eprint!` / `eprintln!`.
#[doc(hidden)]
pub fn _eprint(args: fmt::Arguments<'_>) {
    if let Err(error) = stderr().write_fmt(args) {
        panic!("failed printing to stderr: {error}");
    }
}
