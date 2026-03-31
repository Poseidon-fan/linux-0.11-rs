use bitflags::bitflags;

use crate::{syscall::SyscallArg, use_syscall};

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AccessMode {
    ReadOnly = 0,
    WriteOnly = 1,
    ReadWrite = 2,
}

impl AccessMode {
    #[inline(always)]
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::ReadOnly),
            1 => Some(Self::WriteOnly),
            2 => Some(Self::ReadWrite),
            _ => None,
        }
    }
}

bitflags! {
    pub struct OpenOptions: u32 {
        const CREATE = 0o0100;
        const EXCLUSIVE = 0o0200;
        const NO_CONTROLLING_TTY = 0o0400;
        const TRUNCATE = 0o1000;
        const APPEND = 0o2000;
        const NONBLOCK = 0o4000;
        const NDELAY = Self::NONBLOCK.bits();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct OpenFlags(u32);

impl OpenFlags {
    #[inline(always)]
    pub const fn new(access_mode: AccessMode, options: OpenOptions) -> Self {
        Self(access_mode as u32 | options.bits())
    }

    #[inline(always)]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    #[inline(always)]
    pub fn into_parts(self) -> Option<(AccessMode, OpenOptions)> {
        let access_mode = AccessMode::from_raw(self.0 & 0b11)?;
        let options = OpenOptions::from_bits_retain(self.0 & !0b11);
        Some((access_mode, options))
    }
}

impl SyscallArg for OpenFlags {
    fn into_syscall_arg(self) -> u32 {
        self.0
    }
}

use_syscall!(crate::syscall::NR_OPEN => open(path: *const u8, flags: OpenFlags, mode: u32) -> u32);
