//! Shared logical TTY configuration types.
//!
//! These definitions intentionally model the currently used Linux 0.11 TTY
//! settings as ergonomic Rust types instead of preserving the historical ioctl
//! ABI layout. A future compatibility layer can translate between these logical
//! settings and any legacy ABI structs if needed.

use bitflags::bitflags;

bitflags! {
    /// Input processing options applied by the line discipline.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct InputMode: u16 {
        /// Map carriage return (`'\r'`) to newline (`'\n'`).
        const MAP_CR_TO_NL = 1 << 0;
        /// Ignore carriage return characters.
        const IGNORE_CR = 1 << 1;
        /// Map newline (`'\n'`) to carriage return (`'\r'`).
        const MAP_NL_TO_CR = 1 << 2;
        /// Convert incoming uppercase ASCII letters to lowercase.
        const LOWERCASE = 1 << 3;
    }
}

bitflags! {
    /// Output processing options applied before bytes enter the output queue.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct OutputMode: u16 {
        /// Enable output post-processing.
        const POST_PROCESS = 1 << 0;
        /// Map newline (`'\n'`) to CRLF.
        const MAP_NL_TO_CRLF = 1 << 1;
        /// Map carriage return (`'\r'`) to newline (`'\n'`).
        const MAP_CR_TO_NL = 1 << 2;
        /// Treat newline as carriage return for cursor state.
        const NL_RETURNS_CR = 1 << 3;
        /// Convert outgoing lowercase ASCII letters to uppercase.
        const UPPERCASE = 1 << 4;
    }
}

bitflags! {
    /// Local line discipline options.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct LocalMode: u16 {
        /// Recognize signal-generating control characters.
        const SIGNAL_CHARS = 1 << 0;
        /// Enable canonical line editing and line-based reads.
        const CANONICAL = 1 << 1;
        /// Echo received input back to the output queue.
        const ECHO = 1 << 2;
        /// Echo control characters as caret notation (`^C`, `^D`, ...).
        const ECHO_CONTROL = 1 << 3;
    }
}

/// Control characters used by the current TTY configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlChars {
    /// Interrupt character, typically `Ctrl-C`.
    pub interrupt: u8,
    /// Quit character, typically `Ctrl-\`.
    pub quit: u8,
    /// Erase-one-character command, typically `DEL`.
    pub erase: u8,
    /// Kill-current-line command, typically `Ctrl-U`.
    pub kill_line: u8,
    /// End-of-file marker, typically `Ctrl-D`.
    pub eof: u8,
    /// Non-canonical read timeout in deciseconds.
    pub timeout_ds: u8,
    /// Non-canonical minimum number of bytes to read.
    pub min_chars: u8,
    /// Resume-output character, typically `Ctrl-Q`.
    pub start: u8,
    /// Stop-output character, typically `Ctrl-S`.
    pub stop: u8,
}

impl Default for ControlChars {
    fn default() -> Self {
        Self {
            interrupt: 0x03,
            quit: 0x1c,
            erase: 0x7f,
            kill_line: 0x15,
            eof: 0x04,
            timeout_ds: 0,
            min_chars: 1,
            start: 0x11,
            stop: 0x13,
        }
    }
}

/// Logical TTY settings shared between user-space wrappers and the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Termios {
    /// Input processing flags.
    pub input_mode: InputMode,
    /// Output processing flags.
    pub output_mode: OutputMode,
    /// Local line discipline flags.
    pub local_mode: LocalMode,
    /// Active control characters.
    pub control_chars: ControlChars,
}

impl Termios {
    /// Default settings used by the console-style TTY configuration.
    pub fn console_default() -> Self {
        Self {
            input_mode: InputMode::MAP_CR_TO_NL,
            output_mode: OutputMode::POST_PROCESS | OutputMode::MAP_NL_TO_CRLF,
            local_mode: LocalMode::SIGNAL_CHARS
                | LocalMode::CANONICAL
                | LocalMode::ECHO
                | LocalMode::ECHO_CONTROL,
            control_chars: ControlChars {
                interrupt: 0x03,
                quit: 0x1c,
                erase: 0x7f,
                kill_line: 0x15,
                eof: 0x04,
                timeout_ds: 0,
                min_chars: 1,
                start: 0x11,
                stop: 0x13,
            },
        }
    }

    /// Default settings used by the simple raw serial-style configuration.
    pub fn serial_default() -> Self {
        Self {
            input_mode: InputMode::empty(),
            output_mode: OutputMode::empty(),
            local_mode: LocalMode::empty(),
            control_chars: ControlChars {
                interrupt: 0x03,
                quit: 0x1c,
                erase: 0x7f,
                kill_line: 0x15,
                eof: 0x04,
                timeout_ds: 0,
                min_chars: 1,
                start: 0x11,
                stop: 0x13,
            },
        }
    }
}

impl Default for Termios {
    fn default() -> Self {
        Self::console_default()
    }
}
