//! User-space system call interface.
//!
//! Organized in three layers:
//!
//! 1. **`Syscall` enum** — typed system call numbers; the discriminant equals
//!    the `EAX` value for `int $0x80`.
//! 2. **`raw_syscall0` .. `raw_syscall3`** — inline-assembly functions that
//!    issue `int $0x80` and convert the raw `i32` return into
//!    `Result<u32, Errno>` (negative → `Err(errno)`).
//! 3. **`use_syscall!` macro** — generates typed `pub fn` wrappers:
//!    `use_syscall!(Syscall::Read => read(fd: u32, buf: *mut u8, n: u32) -> u32)`
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
//! A negative return value indicates an error; its absolute value is the errno
//! code.

pub mod errno;
pub mod fs;
pub mod misc;
mod number;
pub mod process;
pub mod signal;
pub mod tty;

use core::arch::asm;

pub use errno::Errno;
pub use number::Syscall;

/// Converts a typed syscall argument into the raw 32-bit ABI word.
///
/// Centralizes the ABI conversion rules required by `int $0x80`, keeping
/// public wrapper signatures expressive.
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

impl SyscallArg for Syscall {
    fn into_syscall_arg(self) -> u32 {
        self as u32
    }
}

// ===========================================================================
// Low-level syscall primitives — thin wrappers around `int $0x80`
// ===========================================================================

/// Issue a system call with **no arguments**.
#[inline(always)]
pub fn raw_syscall0(number: Syscall) -> Result<u32, Errno> {
    let ret: i32;
    unsafe {
        asm!(
            "int $0x80",
            inlateout("eax") number as i32 => ret,
            options(att_syntax, nostack),
        );
    }
    if ret >= 0 {
        Ok(ret as u32)
    } else {
        Err(Errno((-ret) as u32))
    }
}

/// Issue a system call with **one argument** (in `EBX`).
#[inline(always)]
pub fn raw_syscall1(number: Syscall, arg1: u32) -> Result<u32, Errno> {
    let ret: i32;
    unsafe {
        asm!(
            "int $0x80",
            inlateout("eax") number as i32 => ret,
            in("ebx") arg1,
            options(att_syntax, nostack),
        );
    }
    if ret >= 0 {
        Ok(ret as u32)
    } else {
        Err(Errno((-ret) as u32))
    }
}

/// Issue a system call with **two arguments** (in `EBX`, `ECX`).
#[inline(always)]
pub fn raw_syscall2(number: Syscall, arg1: u32, arg2: u32) -> Result<u32, Errno> {
    let ret: i32;
    unsafe {
        asm!(
            "int $0x80",
            inlateout("eax") number as i32 => ret,
            in("ebx") arg1,
            in("ecx") arg2,
            options(att_syntax, nostack),
        );
    }
    if ret >= 0 {
        Ok(ret as u32)
    } else {
        Err(Errno((-ret) as u32))
    }
}

/// Issue a system call with **three arguments** (in `EBX`, `ECX`, `EDX`).
#[inline(always)]
pub fn raw_syscall3(number: Syscall, arg1: u32, arg2: u32, arg3: u32) -> Result<u32, Errno> {
    let ret: i32;
    unsafe {
        asm!(
            "int $0x80",
            inlateout("eax") number as i32 => ret,
            in("ebx") arg1,
            in("ecx") arg2,
            in("edx") arg3,
            options(att_syntax, nostack),
        );
    }
    if ret >= 0 {
        Ok(ret as u32)
    } else {
        Err(Errno((-ret) as u32))
    }
}

/// Generates a typed `pub fn` syscall wrapper.
///
/// Syntax: `use_syscall!(Syscall::Variant => fn_name(arg: Type, ...) -> RetType)`
///
/// Supports 0 to 3 arguments. Each argument type must implement [`SyscallArg`].
/// The return type must be castable from `u32`.
#[macro_export]
macro_rules! use_syscall {
    // 0 arguments
    ($number:expr => $name:ident() -> $ret:ty) => {
        #[inline(always)]
        pub fn $name() -> Result<$ret, $crate::syscall::errno::Errno> {
            $crate::syscall::raw_syscall0($number).map(|v| v as $ret)
        }
    };

    // 1 argument
    ($number:expr => $name:ident($a:ident : $atype:ty) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype) -> Result<$ret, $crate::syscall::errno::Errno> {
            $crate::syscall::raw_syscall1(
                $number,
                $crate::syscall::SyscallArg::into_syscall_arg($a),
            )
            .map(|v| v as $ret)
        }
    };

    // 2 arguments
    ($number:expr => $name:ident(
        $a:ident : $atype:ty,
        $b:ident : $btype:ty
    ) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype, $b: $btype) -> Result<$ret, $crate::syscall::errno::Errno> {
            $crate::syscall::raw_syscall2(
                $number,
                $crate::syscall::SyscallArg::into_syscall_arg($a),
                $crate::syscall::SyscallArg::into_syscall_arg($b),
            )
            .map(|v| v as $ret)
        }
    };

    // 3 arguments
    ($number:expr => $name:ident(
        $a:ident : $atype:ty,
        $b:ident : $btype:ty,
        $c:ident : $ctype:ty
    ) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name(
            $a: $atype,
            $b: $btype,
            $c: $ctype,
        ) -> Result<$ret, $crate::syscall::errno::Errno> {
            $crate::syscall::raw_syscall3(
                $number,
                $crate::syscall::SyscallArg::into_syscall_arg($a),
                $crate::syscall::SyscallArg::into_syscall_arg($b),
                $crate::syscall::SyscallArg::into_syscall_arg($c),
            )
            .map(|v| v as $ret)
        }
    };
}
