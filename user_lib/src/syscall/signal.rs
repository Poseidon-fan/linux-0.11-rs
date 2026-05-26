//! Signal numbers, handler sentinels, and sigaction flags.

use bitflags::bitflags;

use super::Errno;
use crate::{
    syscall::{Syscall, SyscallArg},
    use_syscall,
};

/// Total number of signals (signals 1–32 are valid).
pub const NSIG: usize = 32;

/// POSIX signal numbers (1–31).
///
/// Signal 0 is the "null signal" used to check whether a process
/// exists / whether the caller has permission to send it real signals;
/// it is represented separately by [`SIGNULL`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum Signal {
    /// Hangup.
    Hup = 1,
    /// Keyboard interrupt (`Ctrl-C`).
    Int = 2,
    /// Keyboard quit (`Ctrl-\`).
    Quit = 3,
    /// Illegal instruction.
    Ill = 4,
    /// Trace / breakpoint trap.
    Trap = 5,
    /// Abort (also known as SIGIOT).
    Abrt = 6,
    /// Bus error (bad memory access).
    Bus = 7,
    /// Floating-point exception.
    Fpe = 8,
    /// Kill (cannot be caught or ignored).
    Kill = 9,
    /// User-defined signal 1.
    Usr1 = 10,
    /// Segmentation violation (invalid memory reference).
    Segv = 11,
    /// User-defined signal 2.
    Usr2 = 12,
    /// Broken pipe.
    Pipe = 13,
    /// Alarm clock.
    Alrm = 14,
    /// Software termination signal.
    Term = 15,
    /// Stack fault on coprocessor.
    Stkflt = 16,
    /// Child stopped or terminated.
    Chld = 17,
    /// Continue stopped process.
    Cont = 18,
    /// Stop process (cannot be caught or ignored).
    Stop = 19,
    /// Stop typed at terminal.
    Tstp = 20,
    /// Background read from controlling tty.
    Ttin = 21,
    /// Background write to controlling tty.
    Ttou = 22,
    /// Urgent condition on socket.
    Urg = 23,
    /// CPU time limit exceeded.
    Xcpu = 24,
    /// File size limit exceeded.
    Xfsz = 25,
    /// Virtual alarm clock.
    Vtalrm = 26,
    /// Profiling timer expired.
    Prof = 27,
    /// Window size change.
    Winch = 28,
    /// I/O now possible (also known as SIGPOLL).
    Io = 29,
    /// Power failure.
    Pwr = 30,
    /// Bad system call.
    Sys = 31,
}

/// The null signal (POSIX): check process existence / permission without
/// delivering a signal.  Unlike the other [`Signal`] variants this is `0`,
/// which the `signal(2)` / `sigaction(2)` syscalls reject — it is only
/// valid with `kill(2)`.
pub const SIGNULL: u32 = 0;

impl Signal {
    /// Convert a raw signal number into a `Signal`.
    ///
    /// Returns `None` for 0 (the null signal) or numbers outside 1..=31.
    #[inline]
    pub fn from_u32(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::Hup),
            2 => Some(Self::Int),
            3 => Some(Self::Quit),
            4 => Some(Self::Ill),
            5 => Some(Self::Trap),
            6 => Some(Self::Abrt),
            7 => Some(Self::Bus),
            8 => Some(Self::Fpe),
            9 => Some(Self::Kill),
            10 => Some(Self::Usr1),
            11 => Some(Self::Segv),
            12 => Some(Self::Usr2),
            13 => Some(Self::Pipe),
            14 => Some(Self::Alrm),
            15 => Some(Self::Term),
            16 => Some(Self::Stkflt),
            17 => Some(Self::Chld),
            18 => Some(Self::Cont),
            19 => Some(Self::Stop),
            20 => Some(Self::Tstp),
            21 => Some(Self::Ttin),
            22 => Some(Self::Ttou),
            23 => Some(Self::Urg),
            24 => Some(Self::Xcpu),
            25 => Some(Self::Xfsz),
            26 => Some(Self::Vtalrm),
            27 => Some(Self::Prof),
            28 => Some(Self::Winch),
            29 => Some(Self::Io),
            30 => Some(Self::Pwr),
            31 => Some(Self::Sys),
            _ => None,
        }
    }

    /// Returns the signal number.
    #[inline]
    pub fn number(self) -> u32 {
        self as u32
    }

    /// Returns the signal name in uppercase without the `SIG` prefix
    /// (e.g. `"TERM"`, `"KILL"`, `"INT"`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Hup => "HUP",
            Self::Int => "INT",
            Self::Quit => "QUIT",
            Self::Ill => "ILL",
            Self::Trap => "TRAP",
            Self::Abrt => "ABRT",
            Self::Bus => "BUS",
            Self::Fpe => "FPE",
            Self::Kill => "KILL",
            Self::Usr1 => "USR1",
            Self::Segv => "SEGV",
            Self::Usr2 => "USR2",
            Self::Pipe => "PIPE",
            Self::Alrm => "ALRM",
            Self::Term => "TERM",
            Self::Stkflt => "STKFLT",
            Self::Chld => "CHLD",
            Self::Cont => "CONT",
            Self::Stop => "STOP",
            Self::Tstp => "TSTP",
            Self::Ttin => "TTIN",
            Self::Ttou => "TTOU",
            Self::Urg => "URG",
            Self::Xcpu => "XCPU",
            Self::Xfsz => "XFSZ",
            Self::Vtalrm => "VTALRM",
            Self::Prof => "PROF",
            Self::Winch => "WINCH",
            Self::Io => "IO",
            Self::Pwr => "PWR",
            Self::Sys => "SYS",
        }
    }

    /// Parse a signal name or number string.
    ///
    /// Accepts: `"9"`, `"KILL"`, `"SIGKILL"`, `"kill"`, `"sigkill"`.
    /// Returns `None` if the string does not name a valid signal.
    pub fn parse(raw: &str) -> Option<Self> {
        // Strip optional SIG / sig prefix.
        let name = if raw.len() > 3 && (raw.starts_with("SIG") || raw.starts_with("sig")) {
            &raw[3..]
        } else {
            raw
        };

        // Try numeric first.
        if let Ok(num) = name.parse::<u32>() {
            return Self::from_u32(num);
        }

        // Case-insensitive name match.
        match name.to_ascii_uppercase().as_str() {
            "HUP" => Some(Self::Hup),
            "INT" => Some(Self::Int),
            "QUIT" => Some(Self::Quit),
            "ILL" => Some(Self::Ill),
            "TRAP" => Some(Self::Trap),
            "ABRT" | "IOT" => Some(Self::Abrt),
            "BUS" => Some(Self::Bus),
            "FPE" => Some(Self::Fpe),
            "KILL" => Some(Self::Kill),
            "USR1" => Some(Self::Usr1),
            "SEGV" => Some(Self::Segv),
            "USR2" => Some(Self::Usr2),
            "PIPE" => Some(Self::Pipe),
            "ALRM" => Some(Self::Alrm),
            "TERM" => Some(Self::Term),
            "STKFLT" => Some(Self::Stkflt),
            "CHLD" | "CLD" => Some(Self::Chld),
            "CONT" => Some(Self::Cont),
            "STOP" => Some(Self::Stop),
            "TSTP" => Some(Self::Tstp),
            "TTIN" => Some(Self::Ttin),
            "TTOU" => Some(Self::Ttou),
            "URG" => Some(Self::Urg),
            "XCPU" => Some(Self::Xcpu),
            "XFSZ" => Some(Self::Xfsz),
            "VTALRM" => Some(Self::Vtalrm),
            "PROF" => Some(Self::Prof),
            "WINCH" => Some(Self::Winch),
            "IO" | "POLL" => Some(Self::Io),
            "PWR" => Some(Self::Pwr),
            "SYS" | "UNUSED" => Some(Self::Sys),
            _ => None,
        }
    }
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

/// Send any signal number (including `SIGNULL`) to a process.
///
/// Unlike [`kill`], this accepts raw `u32` values so that callers can send
/// signal 0 to test process existence, or send signals not yet represented
/// in the [`Signal`] enum.
#[inline]
pub fn kill_raw(pid: i32, sig: u32) -> Result<u32, Errno> {
    super::raw_syscall2(Syscall::Kill, pid as u32, sig)
}
