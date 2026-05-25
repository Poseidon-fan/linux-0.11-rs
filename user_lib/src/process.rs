//! A module for working with processes.
//!
//! By analogy with [`std::process`], this module collects everything related
//! to the running process: terminating the current process, querying its
//! identity, and spawning and supervising child processes on top of the
//! `fork` / `execve` / `waitpid` system calls.
//!
//! The [`Termination`] trait and [`ExitCode`] type live here rather than in
//! [`crate::rt`] because they describe the *result* of running a program,
//! which is a process-level concept. The runtime in [`crate::rt`] only
//! consumes them: when `main` returns, its value is fed through
//! `Termination::report` and then handed to [`ExitCode::exit_process`].

use core::convert::Infallible;

use crate::{io, syscall};

/// Terminates the current process with the given status code.
///
/// Any data still buffered in [`io::Stdout`] is flushed before the kernel
/// is asked to terminate the process.
///
/// The lower 8 bits of `code` are passed to the kernel as the exit status,
/// matching the conventional Unix exit code range.
pub fn exit(code: i32) -> ! {
    io::flush_stdout();
    let _ = syscall::process::exit(code as u32);
    loop {
        core::hint::spin_loop();
    }
}

/// Aborts the process with a non-zero status conventionally associated with
/// abnormal termination.
///
/// Without a working signal-delivery path back to the current task, this is
/// implemented as a plain `exit(134)` — the value `128 + SIGABRT` that Unix
/// shells report for processes killed by `SIGABRT`.
pub fn abort() -> ! {
    exit(134)
}

/// 8-bit status returned to the parent after the process exits.
///
/// Constructed via [`ExitCode::SUCCESS`], [`ExitCode::FAILURE`], or
/// [`From<u8>`]. The runtime calls [`ExitCode::exit_process`] on the value
/// produced by [`Termination::report`] when `main` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitCode(u8);

impl ExitCode {
    /// Conventional exit code for successful termination (`0`).
    pub const SUCCESS: ExitCode = ExitCode(0);
    /// Conventional exit code for unsuccessful termination (`1`).
    pub const FAILURE: ExitCode = ExitCode(1);

    /// Terminates the current process with this exit code.
    pub fn exit_process(self) -> ! {
        exit(self.0 as i32)
    }
}

impl From<u8> for ExitCode {
    #[inline]
    fn from(code: u8) -> Self {
        ExitCode(code)
    }
}

/// Trait describing how a value returned from `main` is converted into an
/// [`ExitCode`].
///
/// Implementations are provided for the same shapes accepted by `std`: `()`,
/// [`Infallible`], [`ExitCode`], and `Result<T: Termination, E: Debug>`.
pub trait Termination {
    /// Returns the exit code produced by this value.
    fn report(self) -> ExitCode;
}

impl Termination for () {
    #[inline]
    fn report(self) -> ExitCode {
        ExitCode::SUCCESS
    }
}

impl Termination for Infallible {
    #[inline]
    fn report(self) -> ExitCode {
        match self {}
    }
}

impl Termination for ExitCode {
    #[inline]
    fn report(self) -> ExitCode {
        self
    }
}

impl<T: Termination, E: core::fmt::Debug> Termination for Result<T, E> {
    fn report(self) -> ExitCode {
        match self {
            Ok(value) => value.report(),
            Err(error) => {
                crate::println!("Error: {:?}", error);
                ExitCode::FAILURE
            }
        }
    }
}
