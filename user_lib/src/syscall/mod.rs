//! User-space system call wrappers.
//!
//! This module provides a safe, typed interface for invoking kernel system
//! calls from user mode (ring 3) via `int $0x80`. It is structured in three
//! layers:
//!
//! 1. **`NR_*` constants** — system call numbers (`u32`).
//! 2. **`raw_syscall0` .. `raw_syscall3`** — low-level inline-assembly
//!    functions that issue `int $0x80` and convert the raw i32 return into
//!    `Result<u32, u32>` (negative → `Err(errno)`).
//! 3. **`define_syscall!` macro** — generates typed `pub fn` wrappers using
//!    a function-signature-like syntax:
//!    `define_syscall!(NR_XXX => fn_name(arg: Type, ...) -> RetType)`.
//!    The macro matches on argument count (0–3) and calls the corresponding
//!    `raw_syscallN`, casting arguments to `u32` and the return value to
//!    `RetType`.
//!
//! This layer intentionally keeps 32-bit syscall words (`u32`) for ABI
//! compatibility with the i386 register convention. It does not use `usize`
//! as the canonical syscall argument type.
//!
//! Special cases like `sys_exit` (diverging `!`) are hand-written because
//! they cannot be expressed through the macro.
//!
//! # `int $0x80` register convention
//!
//! ```text
//!   EAX  ─  syscall number (in) / return value (out)
//!   EBX  ─  1st argument
//!   ECX  ─  2nd argument
//!   EDX  ─  3rd argument
//! ```
//!
//! A negative return value indicates an error; its absolute value is the
//! `errno` code.

pub mod fs;

use core::arch::asm;

/// Convert one typed syscall wrapper argument into the raw 32-bit ABI word.
///
/// This keeps the public wrapper signatures expressive while centralizing the
/// ABI conversion rules required by `int $0x80`.
pub trait SyscallArg {
    fn into_syscall_arg(self) -> u32;
}

impl SyscallArg for u32 {
    fn into_syscall_arg(self) -> u32 {
        self
    }
}

impl SyscallArg for i32 {
    fn into_syscall_arg(self) -> u32 {
        self as u32
    }
}

impl<T> SyscallArg for *const T {
    fn into_syscall_arg(self) -> u32 {
        self as u32
    }
}

impl<T> SyscallArg for *mut T {
    fn into_syscall_arg(self) -> u32 {
        self as u32
    }
}

// System call numbers, ordered as in sys_call_table (index = syscall number).
pub const NR_SETUP: u32 = 0;
pub const NR_EXIT: u32 = 1;
pub const NR_FORK: u32 = 2;
pub const NR_READ: u32 = 3;
pub const NR_WRITE: u32 = 4;
pub const NR_OPEN: u32 = 5;
pub const NR_CLOSE: u32 = 6;
pub const NR_WAITPID: u32 = 7;
pub const NR_CREAT: u32 = 8;
pub const NR_LINK: u32 = 9;
pub const NR_UNLINK: u32 = 10;
pub const NR_EXECVE: u32 = 11;
pub const NR_CHDIR: u32 = 12;
pub const NR_TIME: u32 = 13;
pub const NR_MKNOD: u32 = 14;
pub const NR_CHMOD: u32 = 15;
pub const NR_CHOWN: u32 = 16;
pub const NR_BREAK: u32 = 17;
pub const NR_STAT: u32 = 18;
pub const NR_LSEEK: u32 = 19;
pub const NR_GETPID: u32 = 20;
pub const NR_MOUNT: u32 = 21;
pub const NR_UMOUNT: u32 = 22;
pub const NR_SETUID: u32 = 23;
pub const NR_GETUID: u32 = 24;
pub const NR_STIME: u32 = 25;
pub const NR_PTRACE: u32 = 26;
pub const NR_ALARM: u32 = 27;
pub const NR_FSTAT: u32 = 28;
pub const NR_PAUSE: u32 = 29;
pub const NR_UTIME: u32 = 30;
pub const NR_STTY: u32 = 31;
pub const NR_GTTY: u32 = 32;
pub const NR_ACCESS: u32 = 33;
pub const NR_NICE: u32 = 34;
pub const NR_FTIME: u32 = 35;
pub const NR_SYNC: u32 = 36;
pub const NR_KILL: u32 = 37;
pub const NR_RENAME: u32 = 38;
pub const NR_MKDIR: u32 = 39;
pub const NR_RMDIR: u32 = 40;
pub const NR_DUP: u32 = 41;
pub const NR_PIPE: u32 = 42;
pub const NR_TIMES: u32 = 43;
pub const NR_PROF: u32 = 44;
pub const NR_BRK: u32 = 45;
pub const NR_SETGID: u32 = 46;
pub const NR_GETGID: u32 = 47;
pub const NR_SIGNAL: u32 = 48;
pub const NR_GETEUID: u32 = 49;
pub const NR_GETEGID: u32 = 50;
pub const NR_ACCT: u32 = 51;
pub const NR_PHYS: u32 = 52;
pub const NR_LOCK: u32 = 53;
pub const NR_IOCTL: u32 = 54;
pub const NR_FCNTL: u32 = 55;
pub const NR_MPX: u32 = 56;
pub const NR_SETPGID: u32 = 57;
pub const NR_ULIMIT: u32 = 58;
pub const NR_UNAME: u32 = 59;
pub const NR_UMASK: u32 = 60;
pub const NR_CHROOT: u32 = 61;
pub const NR_USTAT: u32 = 62;
pub const NR_DUP2: u32 = 63;
pub const NR_GETPPID: u32 = 64;
pub const NR_GETPGRP: u32 = 65;
pub const NR_SETSID: u32 = 66;
pub const NR_SIGACTION: u32 = 67;
pub const NR_SGETMASK: u32 = 68;
pub const NR_SSETMASK: u32 = 69;
pub const NR_SETREUID: u32 = 70;
pub const NR_SETREGID: u32 = 71;
pub const NR_IAM: u32 = 72;
pub const NR_WHOAMI: u32 = 73;
pub const NR_TEST: u32 = 74;

// ===========================================================================
// Low-level syscall primitives — thin wrappers around `int $0x80`
// ===========================================================================

/// Issue a system call with **no arguments**.
///
/// Returns `Ok(retval)` on success or `Err(errno)` on failure.
#[inline(always)]
pub(crate) fn raw_syscall0(nr: u32) -> Result<u32, u32> {
    let ret: i32;
    unsafe {
        asm!(
            "int $0x80",
            inlateout("eax") nr as i32 => ret,
            options(att_syntax, nostack),
        );
    }
    if ret >= 0 {
        Ok(ret as u32)
    } else {
        Err((-ret) as u32)
    }
}

/// Issue a system call with **one argument** (in `EBX`).
#[inline(always)]
pub(crate) fn raw_syscall1(nr: u32, arg1: u32) -> Result<u32, u32> {
    let ret: i32;
    unsafe {
        asm!(
            "int $0x80",
            inlateout("eax") nr as i32 => ret,
            in("ebx") arg1,
            options(att_syntax, nostack),
        );
    }
    if ret >= 0 {
        Ok(ret as u32)
    } else {
        Err((-ret) as u32)
    }
}

/// Issue a system call with **two arguments** (in `EBX`, `ECX`).
#[inline(always)]
pub(crate) fn raw_syscall2(nr: u32, arg1: u32, arg2: u32) -> Result<u32, u32> {
    let ret: i32;
    unsafe {
        asm!(
            "int $0x80",
            inlateout("eax") nr as i32 => ret,
            in("ebx") arg1,
            in("ecx") arg2,
            options(att_syntax, nostack),
        );
    }
    if ret >= 0 {
        Ok(ret as u32)
    } else {
        Err((-ret) as u32)
    }
}

/// Issue a system call with **three arguments** (in `EBX`, `ECX`, `EDX`).
#[inline(always)]
pub(crate) fn raw_syscall3(nr: u32, arg1: u32, arg2: u32, arg3: u32) -> Result<u32, u32> {
    let ret: i32;
    unsafe {
        asm!(
            "int $0x80",
            inlateout("eax") nr as i32 => ret,
            in("ebx") arg1,
            in("ecx") arg2,
            in("edx") arg3,
            options(att_syntax, nostack),
        );
    }
    if ret >= 0 {
        Ok(ret as u32)
    } else {
        Err((-ret) as u32)
    }
}

// Syntax:  use_syscall!(NR_XXX => fn_name(arg: Type, ...) -> RetType)
//
// A single macro with four match arms (0–3 arguments). Each arm generates
// an `#[inline(always)] pub fn` that forwards to `raw_syscallN`, casting
// every argument to `u32` and the success value to `RetType`.
#[macro_export]
macro_rules! use_syscall {
    // 0 arguments
    ($nr:expr => $name:ident() -> $ret:ty) => {
        #[inline(always)]
        pub fn $name() -> Result<$ret, u32> {
            $crate::raw_syscall0($nr).map(|v| v as $ret)
        }
    };

    // 1 argument
    ($nr:expr => $name:ident($a:ident : $atype:ty) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype) -> Result<$ret, u32> {
            $crate::raw_syscall1($nr, $crate::syscall::SyscallArg::into_syscall_arg($a))
                .map(|v| v as $ret)
        }
    };

    // 2 arguments
    ($nr:expr => $name:ident(
        $a:ident : $atype:ty,
        $b:ident : $btype:ty
    ) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype, $b: $btype) -> Result<$ret, u32> {
            $crate::raw_syscall2(
                $nr,
                $crate::syscall::SyscallArg::into_syscall_arg($a),
                $crate::syscall::SyscallArg::into_syscall_arg($b),
            )
            .map(|v| v as $ret)
        }
    };

    // 3 arguments
    ($nr:expr => $name:ident(
        $a:ident : $atype:ty,
        $b:ident : $btype:ty,
        $c:ident : $ctype:ty
    ) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype, $b: $btype, $c: $ctype) -> Result<$ret, u32> {
            $crate::raw_syscall3(
                $nr,
                $crate::syscall::SyscallArg::into_syscall_arg($a),
                $crate::syscall::SyscallArg::into_syscall_arg($b),
                $crate::syscall::SyscallArg::into_syscall_arg($c),
            )
            .map(|v| v as $ret)
        }
    };
}

use_syscall!(NR_SETUP => setup(drive_info_addr: *const u8) -> u32);
use_syscall!(NR_EXIT => exit() -> u32);
use_syscall!(NR_FORK => fork() -> u32);
use_syscall!(NR_PAUSE => pause() -> u32);

use_syscall!(crate::syscall::NR_TEST => test(value: i32) -> u32);
