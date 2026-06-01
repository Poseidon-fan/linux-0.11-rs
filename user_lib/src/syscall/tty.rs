//! TTY configuration types with an ioctl-compatible C layout.

use core::mem::size_of;

use bitflags::bitflags;

use crate::syscall::SyscallArg;

/// TTY ioctl request codes.
///
/// The discriminant is the raw `u32` value placed in the third argument of
/// `ioctl(fd, request, arg)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum TtyRequest {
    /// Get terminal attributes.
    GetTermios = 0x5401,
    /// Set terminal attributes immediately.
    SetTermios = 0x5402,
    /// Set terminal attributes after output drains.
    SetTermiosWait = 0x5403,
    /// Flush input, then set terminal attributes after output drains.
    SetTermiosFlush = 0x5404,
    /// Get legacy terminal attributes.
    GetTermio = 0x5405,
    /// Set legacy terminal attributes immediately.
    SetTermio = 0x5406,
    /// Set legacy terminal attributes after output drains.
    SetTermioWait = 0x5407,
    /// Flush input, then set legacy terminal attributes after output drains.
    SetTermioFlush = 0x5408,
    /// Flush input and/or output queues (`TCFLSH`).
    Flush = 0x540B,
    /// Get foreground process group ID.
    GetPgrp = 0x540F,
    /// Set foreground process group ID.
    SetPgrp = 0x5410,
    /// Return the number of bytes still queued for output (`TIOCOUTQ`).
    OutputQueueBytes = 0x5411,
    /// Return the number of bytes available to read (`TIOCINQ`).
    InputQueueBytes = 0x541B,
}

impl SyscallArg for TtyRequest {
    fn into_syscall_arg(self) -> u32 {
        self as u32
    }
}

impl TryFrom<u32> for TtyRequest {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(match value {
            0x5401 => Self::GetTermios,
            0x5402 => Self::SetTermios,
            0x5403 => Self::SetTermiosWait,
            0x5404 => Self::SetTermiosFlush,
            0x5405 => Self::GetTermio,
            0x5406 => Self::SetTermio,
            0x5407 => Self::SetTermioWait,
            0x5408 => Self::SetTermioFlush,
            0x540B => Self::Flush,
            0x540F => Self::GetPgrp,
            0x5410 => Self::SetPgrp,
            0x5411 => Self::OutputQueueBytes,
            0x541B => Self::InputQueueBytes,
            _ => return Err(()),
        })
    }
}

/// Argument values for the [`TtyRequest::Flush`] ioctl.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum FlushSelector {
    /// Discard unread input.
    Input = 0,
    /// Discard unsent output.
    Output = 1,
    /// Discard both queues.
    Both = 2,
}

impl TryFrom<u32> for FlushSelector {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Input,
            1 => Self::Output,
            2 => Self::Both,
            _ => return Err(()),
        })
    }
}

/// Number of control characters stored in [`Termio::control_chars`].
pub const NCC: usize = 8;

/// Number of control characters stored in [`Termios::control_chars`].
pub const NCCS: usize = 17;

/// Index into the control-character table of a [`Termios`] structure.
///
/// Use with [`Termios::control_char`] and [`Termios::set_control_char`] instead
/// of bare `usize` indices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum ControlChar {
    /// Interrupt character (sends `SIGINT`).
    Intr = 0,
    /// Quit character (sends `SIGQUIT`).
    Quit = 1,
    /// Erase character (deletes last typed character).
    Erase = 2,
    /// Line-kill character (erases current input line).
    Kill = 3,
    /// End-of-file character.
    Eof = 4,
    /// Non-canonical read timeout (tenths of a second).
    Time = 5,
    /// Non-canonical minimum bytes before read returns.
    Min = 6,
    /// Switch character.
    Swtc = 7,
    /// XON character (resume output).
    Start = 8,
    /// XOFF character (suspend output).
    Stop = 9,
    /// Suspend character (sends `SIGTSTP`).
    Susp = 10,
    /// First extra end-of-line character.
    Eol = 11,
    /// Reprint character.
    Reprint = 12,
    /// Discard character.
    Discard = 13,
    /// Word-erase character.
    WordErase = 14,
    /// Literal-next character.
    LiteralNext = 15,
    /// Second extra end-of-line character.
    Eol2 = 16,
}

/// Default control-character table.
pub const INIT_CONTROL_CHARS: [u8; NCCS] = [
    0x03, 0x1c, 0x7f, 0x15, 0x04, 0x00, 0x01, 0x00, 0x11, 0x13, 0x1a, 0x00, 0x12, 0x0f, 0x17, 0x16,
    0x00,
];

bitflags! {
    /// Input mode bits (`c_iflag`).
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct InputMode: u32 {
        const IGNBRK  = 0o000001;
        const BRKINT  = 0o000002;
        const IGNPAR  = 0o000004;
        const PARMRK  = 0o000010;
        const INPCK   = 0o000020;
        const ISTRIP  = 0o000040;
        const INLCR   = 0o000100;
        const IGNCR   = 0o000200;
        const ICRNL   = 0o000400;
        const IUCLC   = 0o001000;
        const IXON    = 0o002000;
        const IXANY   = 0o004000;
        const IXOFF   = 0o010000;
        const IMAXBEL = 0o020000;
        const _       = !0;
    }
}

bitflags! {
    /// Output mode bits (`c_oflag`).
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct OutputMode: u32 {
        const OPOST  = 0o000001;
        const OLCUC  = 0o000002;
        const ONLCR  = 0o000004;
        const OCRNL  = 0o000010;
        const ONOCR  = 0o000020;
        const ONLRET = 0o000040;
        const OFILL  = 0o000100;
        const OFDEL  = 0o000200;
        const NLDLY  = 0o000400;
        const NL1    = 0o000400;
        const CRDLY  = 0o003000;
        const CR1    = 0o001000;
        const CR2    = 0o002000;
        const CR3    = 0o003000;
        const TABDLY = 0o014000;
        const TAB1   = 0o004000;
        const TAB2   = 0o010000;
        const TAB3   = 0o014000;
        const XTABS  = 0o014000;
        const BSDLY  = 0o020000;
        const BS1    = 0o020000;
        const VTDLY  = 0o040000;
        const VT1    = 0o040000;
        const FFDLY  = 0o040000;
        const FF1    = 0o040000;
        const _      = !0;
    }
}

bitflags! {
    /// Control mode bits (`c_cflag`).
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ControlMode: u32 {
        const CBAUD   = 0o000017;
        const B50     = 0o000001;
        const B75     = 0o000002;
        const B110    = 0o000003;
        const B134    = 0o000004;
        const B150    = 0o000005;
        const B200    = 0o000006;
        const B300    = 0o000007;
        const B600    = 0o000010;
        const B1200   = 0o000011;
        const B1800   = 0o000012;
        const B2400   = 0o000013;
        const B4800   = 0o000014;
        const B9600   = 0o000015;
        const B19200  = 0o000016;
        const B38400  = 0o000017;
        const CSIZE   = 0o000060;
        const CS6     = 0o000020;
        const CS7     = 0o000040;
        const CS8     = 0o000060;
        const CSTOPB  = 0o000100;
        const CREAD   = 0o000200;
        const CPARENB = 0o000400;
        const CPARODD = 0o001000;
        const HUPCL   = 0o002000;
        const CLOCAL  = 0o004000;
        const CIBAUD  = 0o3600000;
        const CRTSCTS = 0o20000000000u32;
        const _       = !0;
    }
}

bitflags! {
    /// Local line-discipline bits (`c_lflag`).
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct LocalMode: u32 {
        const ISIG    = 0o000001;
        const ICANON  = 0o000002;
        const XCASE   = 0o000004;
        const ECHO    = 0o000010;
        const ECHOE   = 0o000020;
        const ECHOK   = 0o000040;
        const ECHONL  = 0o000100;
        const NOFLSH  = 0o000200;
        const TOSTOP  = 0o000400;
        const ECHOCTL = 0o001000;
        const ECHOPRT = 0o002000;
        const ECHOKE  = 0o004000;
        const FLUSHO  = 0o010000;
        const PENDIN  = 0o040000;
        const IEXTEN  = 0o100000;
        const _       = !0;
    }
}

/// Legacy TTY settings with an ABI-compatible i386 C layout.
///
/// This is the old Linux 0.11 `struct termio` layout used by early user-space
/// programs. The kernel converts it to and from [`Termios`] at ioctl boundaries.
///
/// ```text
/// offset  0: input_mode      (u16)
/// offset  2: output_mode     (u16)
/// offset  4: control_mode    (u16)
/// offset  6: local_mode      (u16)
/// offset  8: line_discipline (u8)
/// offset  9: control_chars   ([u8; 8])
/// total size: 18 bytes
/// ```
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Termio {
    pub input_mode: u16,
    pub output_mode: u16,
    pub control_mode: u16,
    pub local_mode: u16,
    pub line_discipline: u8,
    pub control_chars: [u8; NCC],
}

/// TTY settings with an ABI-compatible i386 C layout.
///
/// The layout is fixed because user space passes pointers to this structure
/// directly through `ioctl`.
///
/// ```text
/// offset  0: input_mode      (u32)
/// offset  4: output_mode     (u32)
/// offset  8: control_mode    (u32)
/// offset 12: local_mode      (u32)
/// offset 16: line_discipline (u8)
/// offset 17: control_chars   ([u8; 17])
/// total size: 36 bytes
/// ```
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Termios {
    pub input_mode: InputMode,
    pub output_mode: OutputMode,
    pub control_mode: ControlMode,
    pub local_mode: LocalMode,
    pub line_discipline: u8,
    pub control_chars: [u8; NCCS],
}

impl Termios {
    /// Returns the default console configuration.
    pub const fn console_default() -> Self {
        Self {
            input_mode: InputMode::ICRNL,
            output_mode: OutputMode::OPOST.union(OutputMode::ONLCR),
            control_mode: ControlMode::empty(),
            local_mode: LocalMode::ISIG
                .union(LocalMode::ICANON)
                .union(LocalMode::ECHO)
                .union(LocalMode::ECHOCTL)
                .union(LocalMode::ECHOKE),
            line_discipline: 0,
            control_chars: INIT_CONTROL_CHARS,
        }
    }

    /// Returns the default serial-port configuration.
    pub const fn serial_default() -> Self {
        Self {
            input_mode: InputMode::empty(),
            output_mode: OutputMode::empty(),
            control_mode: ControlMode::B2400.union(ControlMode::CS8),
            local_mode: LocalMode::empty(),
            line_discipline: 0,
            control_chars: INIT_CONTROL_CHARS,
        }
    }

    /// Returns the value of the given control character slot.
    #[inline]
    pub fn control_char(&self, cc: ControlChar) -> u8 {
        self.control_chars[cc as usize]
    }

    /// Sets the value of the given control character slot.
    #[inline]
    pub fn set_control_char(&mut self, cc: ControlChar, value: u8) {
        self.control_chars[cc as usize] = value;
    }

    /// Convert this modern termios value into the legacy termio ABI.
    #[inline]
    pub fn to_termio(&self) -> Termio {
        let mut control_chars = [0u8; NCC];
        control_chars.copy_from_slice(&self.control_chars[..NCC]);

        Termio {
            input_mode: self.input_mode.bits() as u16,
            output_mode: self.output_mode.bits() as u16,
            control_mode: self.control_mode.bits() as u16,
            local_mode: self.local_mode.bits() as u16,
            line_discipline: self.line_discipline,
            control_chars,
        }
    }

    /// Apply a legacy termio value, preserving fields that termio cannot carry.
    #[inline]
    pub fn apply_termio(&mut self, termio: Termio) {
        self.input_mode = InputMode::from_bits_retain(
            (self.input_mode.bits() & !0xffff) | u32::from(termio.input_mode),
        );
        self.output_mode = OutputMode::from_bits_retain(
            (self.output_mode.bits() & !0xffff) | u32::from(termio.output_mode),
        );
        self.control_mode = ControlMode::from_bits_retain(
            (self.control_mode.bits() & !0xffff) | u32::from(termio.control_mode),
        );
        self.local_mode = LocalMode::from_bits_retain(
            (self.local_mode.bits() & !0xffff) | u32::from(termio.local_mode),
        );
        self.line_discipline = termio.line_discipline;
        self.control_chars[..NCC].copy_from_slice(&termio.control_chars);
    }
}

impl Default for Termios {
    fn default() -> Self {
        Self::console_default()
    }
}

const _: () = assert!(size_of::<InputMode>() == size_of::<u32>());
const _: () = assert!(size_of::<OutputMode>() == size_of::<u32>());
const _: () = assert!(size_of::<ControlMode>() == size_of::<u32>());
const _: () = assert!(size_of::<LocalMode>() == size_of::<u32>());
const _: () = assert!(size_of::<Termio>() == 18);
const _: () = assert!(size_of::<Termios>() == 36);
const _: () = assert!(core::mem::offset_of!(Termio, input_mode) == 0);
const _: () = assert!(core::mem::offset_of!(Termio, output_mode) == 2);
const _: () = assert!(core::mem::offset_of!(Termio, control_mode) == 4);
const _: () = assert!(core::mem::offset_of!(Termio, local_mode) == 6);
const _: () = assert!(core::mem::offset_of!(Termio, line_discipline) == 8);
const _: () = assert!(core::mem::offset_of!(Termio, control_chars) == 9);
const _: () = assert!(core::mem::offset_of!(Termios, input_mode) == 0);
const _: () = assert!(core::mem::offset_of!(Termios, output_mode) == 4);
const _: () = assert!(core::mem::offset_of!(Termios, control_mode) == 8);
const _: () = assert!(core::mem::offset_of!(Termios, local_mode) == 12);
const _: () = assert!(core::mem::offset_of!(Termios, line_discipline) == 16);
const _: () = assert!(core::mem::offset_of!(Termios, control_chars) == 17);
