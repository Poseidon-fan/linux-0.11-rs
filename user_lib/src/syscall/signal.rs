//! Signal numbers, handler sentinels, and sigaction flags.

use bitflags::bitflags;

use crate::{
    syscall::{Syscall, SyscallArg},
    use_syscall,
};

/// Total number of signals (signals 1–32 are valid).
pub const NSIG: usize = 32;

/// POSIX signal numbers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum Signal {
    /// Hangup.
    Hup = 1,
    /// Keyboard interrupt (`Ctrl-C`).
    Int = 2,
    /// Keyboard quit (`Ctrl-\`).
    Quit = 3,
    /// Kill (cannot be caught or ignored).
    Kill = 9,
    /// Invalid memory reference.
    Segv = 11,
    /// Broken pipe.
    Pipe = 13,
    /// Alarm clock.
    Alrm = 14,
    /// Child stopped or terminated.
    Chld = 17,
    /// Stop process (cannot be caught or ignored).
    Stop = 19,
    /// Stop typed at terminal.
    Tstp = 20,
}

impl SyscallArg for Signal {
    fn into_syscall_arg(self) -> u32 {
        self as u32
    }
}

/// Sentinel values for the signal handler field.
///
/// These occupy the function-pointer slot in `sigaction` but represent
/// special kernel-level dispositions rather than actual handlers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum SigHandler {
    /// Perform the default action for the signal.
    Default = 0,
    /// Ignore the signal.
    Ignore = 1,
}

impl SyscallArg for SigHandler {
    fn into_syscall_arg(self) -> u32 {
        self as u32
    }
}

bitflags! {
    /// Flags for the `sa_flags` field of `sigaction`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct SigFlags: u32 {
        /// Do not block the signal while its handler is running.
        const NO_MASK  = 0x4000_0000;
        /// Restore default disposition after the handler fires once.
        const ONE_SHOT = 0x8000_0000;
    }
}

impl SyscallArg for SigFlags {
    fn into_syscall_arg(self) -> u32 {
        self.bits()
    }
}

use_syscall!(Syscall::Kill => kill(pid: i32, signal: Signal) -> u32);
use_syscall!(Syscall::Signal => signal(signal: Signal, handler: u32, restorer: u32) -> u32);
use_syscall!(Syscall::Sgetmask => sgetmask() -> u32);
use_syscall!(Syscall::Ssetmask => ssetmask(mask: u32) -> u32);
use_syscall!(Syscall::Sigaction => sigaction(
    signal: Signal,
    action: *const u8,
    old_action: *mut u8
) -> u32);
