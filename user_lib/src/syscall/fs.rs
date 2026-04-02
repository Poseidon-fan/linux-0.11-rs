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

/// File seek origin, matching POSIX `SEEK_SET` / `SEEK_CUR` / `SEEK_END`.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Whence {
    Set = 0,
    Current = 1,
    End = 2,
}

impl Whence {
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Set),
            1 => Some(Self::Current),
            2 => Some(Self::End),
            _ => None,
        }
    }
}

impl SyscallArg for Whence {
    fn into_syscall_arg(self) -> u32 {
        self as u32
    }
}

/// File metadata structure matching the Linux 0.11 `struct stat` ABI.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Stat {
    pub st_dev: u16,
    pub st_ino: u16,
    pub st_mode: u16,
    pub st_nlink: u8,
    pub st_uid: u16,
    pub st_gid: u8,
    pub st_rdev: u16,
    pub st_size: u32,
    pub st_atime: u32,
    pub st_mtime: u32,
    pub st_ctime: u32,
}

/// Time values for [`utime`], matching the POSIX `struct utimbuf` ABI.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TimeUpdate {
    pub access_time: u32,
    pub modification_time: u32,
}

use_syscall!(crate::syscall::NR_OPEN => open(path: *const u8, flags: OpenFlags, mode: u32) -> u32);
use_syscall!(crate::syscall::NR_READ => read(fd: u32, buf: *mut u8, count: u32) -> u32);
use_syscall!(crate::syscall::NR_WRITE => write(fd: u32, buf: *const u8, count: u32) -> u32);
use_syscall!(crate::syscall::NR_CLOSE => close(fd: u32) -> u32);
use_syscall!(crate::syscall::NR_LSEEK => lseek(fd: u32, offset: i32, whence: Whence) -> u32);
use_syscall!(crate::syscall::NR_UNLINK => unlink(path: *const u8) -> u32);
use_syscall!(crate::syscall::NR_STAT => stat(path: *const u8, buf: *mut Stat) -> u32);
use_syscall!(crate::syscall::NR_FSTAT => fstat(fd: u32, buf: *mut Stat) -> u32);
use_syscall!(crate::syscall::NR_DUP => dup(fd: u32) -> u32);
use_syscall!(crate::syscall::NR_DUP2 => dup2(oldfd: u32, newfd: u32) -> u32);
use_syscall!(crate::syscall::NR_MKDIR => mkdir(path: *const u8, mode: u32) -> u32);
use_syscall!(crate::syscall::NR_RMDIR => rmdir(path: *const u8) -> u32);
