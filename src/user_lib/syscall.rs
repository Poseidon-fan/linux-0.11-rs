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

use core::arch::asm;

use crate::syscall::process::{
    NR_EXIT, NR_FORK, NR_GETEGID, NR_GETEUID, NR_GETGID, NR_GETPGRP, NR_GETPID, NR_GETPPID,
    NR_GETUID, NR_PAUSE, NR_SETGID, NR_SETPGID, NR_SETREGID, NR_SETREUID, NR_SETSID, NR_SETUID,
    NR_WAITPID,
};

// ===========================================================================
// Low-level syscall primitives — thin wrappers around `int $0x80`
// ===========================================================================

/// Issue a system call with **no arguments**.
///
/// Returns `Ok(retval)` on success or `Err(errno)` on failure.
#[inline(always)]
fn raw_syscall0(nr: u32) -> Result<u32, u32> {
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
fn raw_syscall1(nr: u32, arg1: u32) -> Result<u32, u32> {
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
fn raw_syscall2(nr: u32, arg1: u32, arg2: u32) -> Result<u32, u32> {
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
fn raw_syscall3(nr: u32, arg1: u32, arg2: u32, arg3: u32) -> Result<u32, u32> {
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
macro_rules! use_syscall {
    // 0 arguments
    ($nr:expr => $name:ident() -> $ret:ty) => {
        #[inline(always)]
        pub fn $name() -> Result<$ret, u32> {
            raw_syscall0($nr).map(|v| v as $ret)
        }
    };

    // 1 argument
    ($nr:expr => $name:ident($a:ident : $atype:ty) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype) -> Result<$ret, u32> {
            raw_syscall1($nr, $a as u32).map(|v| v as $ret)
        }
    };

    // 2 arguments
    ($nr:expr => $name:ident(
        $a:ident : $atype:ty,
        $b:ident : $btype:ty
    ) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype, $b: $btype) -> Result<$ret, u32> {
            raw_syscall2($nr, $a as u32, $b as u32).map(|v| v as $ret)
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
            raw_syscall3($nr, $a as u32, $b as u32, $c as u32).map(|v| v as $ret)
        }
    };
}

use_syscall!(NR_EXIT => exit() -> u32);
use_syscall!(NR_FORK => fork() -> u32);
use_syscall!(NR_WAITPID => waitpid(pid: i32, stat_addr: *mut u32, options: u32) -> u32);
use_syscall!(NR_PAUSE => pause() -> u32);
use_syscall!(NR_GETPID => getpid() -> u32);
use_syscall!(NR_GETPPID => getppid() -> u32);
use_syscall!(NR_GETPGRP => getpgrp() -> u32);
use_syscall!(NR_GETUID => getuid() -> u32);
use_syscall!(NR_GETEUID => geteuid() -> u32);
use_syscall!(NR_GETGID => getgid() -> u32);
use_syscall!(NR_GETEGID => getegid() -> u32);
use_syscall!(NR_SETUID => setuid(uid: u32) -> u32);
use_syscall!(NR_SETGID => setgid(gid: u32) -> u32);
use_syscall!(NR_SETREUID => setreuid(ruid: u32, euid: u32) -> u32);
use_syscall!(NR_SETREGID => setregid(rgid: u32, egid: u32) -> u32);
use_syscall!(NR_SETPGID => setpgid(pid: i32, pgid: i32) -> u32);
use_syscall!(NR_SETSID => setsid() -> u32);
use_syscall!(crate::syscall::NR_TEST => test(value: i32) -> u32);
