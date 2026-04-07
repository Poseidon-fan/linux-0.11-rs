//! TTY configuration types with an ioctl-compatible C layout.

use bitflags::bitflags;
use core::mem::size_of;

/// Number of control characters stored in `Termios::control_chars`.
pub const NCCS: usize = 17;

/// Index of the interrupt character in [`Termios::control_chars`].
pub const VINTR: usize = 0;
/// Index of the quit character in [`Termios::control_chars`].
pub const VQUIT: usize = 1;
/// Index of the erase character in [`Termios::control_chars`].
pub const VERASE: usize = 2;
/// Index of the line-kill character in [`Termios::control_chars`].
pub const VKILL: usize = 3;
/// Index of the end-of-file character in [`Termios::control_chars`].
pub const VEOF: usize = 4;
/// Index of the non-canonical timeout field in [`Termios::control_chars`].
pub const VTIME: usize = 5;
/// Index of the non-canonical minimum-read field in [`Termios::control_chars`].
pub const VMIN: usize = 6;
/// Index of the switch character in [`Termios::control_chars`].
pub const VSWTC: usize = 7;
/// Index of the XON character in [`Termios::control_chars`].
pub const VSTART: usize = 8;
/// Index of the XOFF character in [`Termios::control_chars`].
pub const VSTOP: usize = 9;
/// Index of the suspend character in [`Termios::control_chars`].
pub const VSUSP: usize = 10;
/// Index of the first extra end-of-line character in [`Termios::control_chars`].
pub const VEOL: usize = 11;
/// Index of the reprint character in [`Termios::control_chars`].
pub const VREPRINT: usize = 12;
/// Index of the discard character in [`Termios::control_chars`].
pub const VDISCARD: usize = 13;
/// Index of the word-erase character in [`Termios::control_chars`].
pub const VWERASE: usize = 14;
/// Index of the literal-next character in [`Termios::control_chars`].
pub const VLNEXT: usize = 15;
/// Index of the second extra end-of-line character in [`Termios::control_chars`].
pub const VEOL2: usize = 16;

/// Default control-character table.
pub const INIT_CONTROL_CHARS: [u8; NCCS] = [
    0x03, 0x1c, 0x7f, 0x15, 0x04, 0x00, 0x01, 0x00, 0x11, 0x13, 0x1a, 0x00, 0x12, 0x0f, 0x17, 0x16,
    0x00,
];

bitflags! {
    /// Input mode bits stored in the ABI field `c_iflag`.
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct InputMode: u32 {
        const IGNBRK = 0o000001;
        const BRKINT = 0o000002;
        const IGNPAR = 0o000004;
        const PARMRK = 0o000010;
        const INPCK = 0o000020;
        const ISTRIP = 0o000040;
        const INLCR = 0o000100;
        const IGNCR = 0o000200;
        const ICRNL = 0o000400;
        const IUCLC = 0o001000;
        const IXON = 0o002000;
        const IXANY = 0o004000;
        const IXOFF = 0o010000;
        const IMAXBEL = 0o020000;
        const _ = !0;
    }
}

bitflags! {
    /// Output mode bits stored in the ABI field `c_oflag`.
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct OutputMode: u32 {
        const OPOST = 0o000001;
        const OLCUC = 0o000002;
        const ONLCR = 0o000004;
        const OCRNL = 0o000010;
        const ONOCR = 0o000020;
        const ONLRET = 0o000040;
        const OFILL = 0o000100;
        const OFDEL = 0o000200;
        const NLDLY = 0o000400;
        const NL1 = 0o000400;
        const CRDLY = 0o003000;
        const CR1 = 0o001000;
        const CR2 = 0o002000;
        const CR3 = 0o003000;
        const TABDLY = 0o014000;
        const TAB1 = 0o004000;
        const TAB2 = 0o010000;
        const TAB3 = 0o014000;
        const XTABS = 0o014000;
        const BSDLY = 0o020000;
        const BS1 = 0o020000;
        const VTDLY = 0o040000;
        const VT1 = 0o040000;
        const FFDLY = 0o040000;
        const FF1 = 0o040000;
        const _ = !0;
    }
}

bitflags! {
    /// Control mode bits stored in the ABI field `c_cflag`.
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ControlMode: u32 {
        const CBAUD = 0o000017;
        const B50 = 0o000001;
        const B75 = 0o000002;
        const B110 = 0o000003;
        const B134 = 0o000004;
        const B150 = 0o000005;
        const B200 = 0o000006;
        const B300 = 0o000007;
        const B600 = 0o000010;
        const B1200 = 0o000011;
        const B1800 = 0o000012;
        const B2400 = 0o000013;
        const B4800 = 0o000014;
        const B9600 = 0o000015;
        const B19200 = 0o000016;
        const B38400 = 0o000017;
        const CSIZE = 0o000060;
        const CS6 = 0o000020;
        const CS7 = 0o000040;
        const CS8 = 0o000060;
        const CSTOPB = 0o000100;
        const CREAD = 0o000200;
        const CPARENB = 0o000400;
        const CPARODD = 0o001000;
        const HUPCL = 0o002000;
        const CLOCAL = 0o004000;
        const CIBAUD = 0o3600000;
        const CRTSCTS = 0o20000000000u32;
        const _ = !0;
    }
}

bitflags! {
    /// Local line-discipline bits stored in the ABI field `c_lflag`.
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct LocalMode: u32 {
        const ISIG = 0o000001;
        const ICANON = 0o000002;
        const XCASE = 0o000004;
        const ECHO = 0o000010;
        const ECHOE = 0o000020;
        const ECHOK = 0o000040;
        const ECHONL = 0o000100;
        const NOFLSH = 0o000200;
        const TOSTOP = 0o000400;
        const ECHOCTL = 0o001000;
        const ECHOPRT = 0o002000;
        const ECHOKE = 0o004000;
        const FLUSHO = 0o010000;
        const PENDIN = 0o040000;
        const IEXTEN = 0o100000;
        const _ = !0;
    }
}

/// TTY settings with an ABI-compatible i386 C layout.
///
/// The layout is fixed because user space passes pointers to this structure
/// directly through `ioctl`.
///
/// ```text
/// +----------------------+ offset 0x00
/// | input_mode      : u32 |
/// | output_mode     : u32 |
/// | control_mode    : u32 |
/// | local_mode      : u32 |
/// | line_discipline : u8  | offset 0x10
/// | control_chars   : [u8; 17]
/// |                    ...| offset 0x11
/// +----------------------+
/// | tail padding         |
/// +----------------------+ size = 36 bytes on i386
/// ```
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Termios {
    /// Input mode flags.
    pub input_mode: InputMode,
    /// Output mode flags.
    pub output_mode: OutputMode,
    /// Control mode flags.
    pub control_mode: ControlMode,
    /// Local mode flags.
    pub local_mode: LocalMode,
    /// Line-discipline selector.
    pub line_discipline: u8,
    /// Control-character table.
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

    /// Returns the default serial configuration.
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

    /// Returns one control character by its ABI slot index.
    pub fn control_char(&self, index: usize) -> u8 {
        self.control_chars[index]
    }

    /// Updates one control character by its ABI slot index.
    pub fn set_control_char(&mut self, index: usize, value: u8) {
        self.control_chars[index] = value;
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
const _: () = assert!(size_of::<Termios>() == 36);
const _: () = assert!(core::mem::offset_of!(Termios, input_mode) == 0);
const _: () = assert!(core::mem::offset_of!(Termios, output_mode) == 4);
const _: () = assert!(core::mem::offset_of!(Termios, control_mode) == 8);
const _: () = assert!(core::mem::offset_of!(Termios, local_mode) == 12);
const _: () = assert!(core::mem::offset_of!(Termios, line_discipline) == 16);
const _: () = assert!(core::mem::offset_of!(Termios, control_chars) == 17);
