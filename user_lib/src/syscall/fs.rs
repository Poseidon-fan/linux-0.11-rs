//! File-system types and syscall wrappers.

use bitflags::bitflags;

use crate::{
    syscall::{SyscallArg, nr::Syscall},
    use_syscall,
};

/// The access-mode portion of open flags (bits 0–1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// Additional open-mode option bits (bits 2 and above).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct OpenOptions: u32 {
        const CREATE             = 0o0100;
        const EXCLUSIVE          = 0o0200;
        const NO_CONTROLLING_TTY = 0o0400;
        const TRUNCATE           = 0o1000;
        const APPEND             = 0o2000;
        const NONBLOCK           = 0o4000;
        const NDELAY             = Self::NONBLOCK.bits();
    }
}

/// Combined open flags: access mode (bits 0–1) OR-ed with option bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// File seek origin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum Whence {
    /// Seek from the beginning of the file.
    Set = 0,
    /// Seek from the current position.
    Current = 1,
    /// Seek from the end of the file.
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

/// `fcntl` command codes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum FcntlCmd {
    /// Duplicate file descriptor (find first free fd ≥ arg).
    DupFd = 0,
    /// Get close-on-exec flag.
    GetFd = 1,
    /// Set close-on-exec flag.
    SetFd = 2,
    /// Get file status flags.
    GetFlags = 3,
    /// Set file status flags.
    SetFlags = 4,
}

impl SyscallArg for FcntlCmd {
    fn into_syscall_arg(self) -> u32 {
        self as u32
    }
}

/// File metadata, matching the Linux 0.11 `struct stat` ABI.
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

/// Time values for `utime`, matching the POSIX `struct utimbuf` ABI.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TimeUpdate {
    pub access_time: u32,
    pub modification_time: u32,
}

use_syscall!(Syscall::Open   => open(path: *const u8, flags: OpenFlags, mode: u32) -> u32);
use_syscall!(Syscall::Read   => read(fd: u32, buf: *mut u8, count: u32) -> u32);
use_syscall!(Syscall::Write  => write(fd: u32, buf: *const u8, count: u32) -> u32);
use_syscall!(Syscall::Close  => close(fd: u32) -> u32);
use_syscall!(Syscall::Creat  => creat(path: *const u8, mode: u32) -> u32);
use_syscall!(Syscall::Link   => link(old_path: *const u8, new_path: *const u8) -> u32);
use_syscall!(Syscall::Lseek  => lseek(fd: u32, offset: i32, whence: Whence) -> u32);
use_syscall!(Syscall::Chdir  => chdir(path: *const u8) -> u32);
use_syscall!(Syscall::Mknod  => mknod(path: *const u8, mode: u32, dev: u32) -> u32);
use_syscall!(Syscall::Chmod  => chmod(path: *const u8, mode: u32) -> u32);
use_syscall!(Syscall::Chown  => chown(path: *const u8, uid: u32, gid: u32) -> u32);
use_syscall!(Syscall::Unlink => unlink(path: *const u8) -> u32);
use_syscall!(Syscall::Stat   => stat(path: *const u8, buf: *mut Stat) -> u32);
use_syscall!(Syscall::Mount  => mount(
    dev_name: *const u8,
    dir_name: *const u8,
    rw_flag: u32
) -> u32);
use_syscall!(Syscall::Umount => umount(dev_name: *const u8) -> u32);
use_syscall!(Syscall::Fstat  => fstat(fd: u32, buf: *mut Stat) -> u32);
use_syscall!(Syscall::Utime  => utime(path: *const u8, times: *const TimeUpdate) -> u32);
use_syscall!(Syscall::Access => access(path: *const u8, mode: u32) -> u32);
use_syscall!(Syscall::Dup    => dup(fd: u32) -> u32);
use_syscall!(Syscall::Pipe   => pipe(fds: *mut u32) -> u32);
use_syscall!(Syscall::Ioctl  => ioctl(fd: u32, request: u32, arg: u32) -> u32);
use_syscall!(Syscall::Fcntl  => fcntl(fd: u32, command: FcntlCmd, arg: u32) -> u32);
use_syscall!(Syscall::Umask  => umask(mask: u32) -> u32);
use_syscall!(Syscall::Chroot => chroot(path: *const u8) -> u32);
use_syscall!(Syscall::Ustat  => ustat(dev: u32, ubuf: *mut u8) -> u32);
use_syscall!(Syscall::Dup2   => dup2(oldfd: u32, newfd: u32) -> u32);
use_syscall!(Syscall::Rename => rename(old_path: *const u8, new_path: *const u8) -> u32);
use_syscall!(Syscall::Mkdir  => mkdir(path: *const u8, mode: u32) -> u32);
use_syscall!(Syscall::Rmdir  => rmdir(path: *const u8) -> u32);
use_syscall!(Syscall::Sync   => sync() -> u32);
