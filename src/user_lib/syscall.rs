//! User-space system call wrappers.
//!
//! This module provides a safe, typed interface for invoking kernel system
//! calls from user mode (ring 3) via `int $0x80`. It is structured in three
//! layers:
//!
//! 1. **`NR_*` constants** — system call numbers (0–73).
//! 2. **`raw_syscall0` .. `raw_syscall3`** — low-level inline-assembly
//!    functions that issue `int $0x80` and convert the raw i32 return into
//!    `Result<usize, usize>` (negative → `Err(errno)`).
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

use crate::syscall::NR_TEST;

// ===========================================================================
// Low-level syscall primitives — thin wrappers around `int $0x80`
// ===========================================================================

/// Issue a system call with **no arguments**.
///
/// Returns `Ok(retval)` on success or `Err(errno)` on failure.
#[inline(always)]
fn raw_syscall0(nr: usize) -> Result<usize, usize> {
    let ret: i32;
    unsafe {
        asm!(
            "int $0x80",
            inlateout("eax") nr as i32 => ret,
            options(att_syntax, nostack),
        );
    }
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err((-ret) as usize)
    }
}

/// Issue a system call with **one argument** (in `EBX`).
#[inline(always)]
fn raw_syscall1(nr: usize, arg1: usize) -> Result<usize, usize> {
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
        Ok(ret as usize)
    } else {
        Err((-ret) as usize)
    }
}

/// Issue a system call with **two arguments** (in `EBX`, `ECX`).
#[inline(always)]
fn raw_syscall2(nr: usize, arg1: usize, arg2: usize) -> Result<usize, usize> {
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
        Ok(ret as usize)
    } else {
        Err((-ret) as usize)
    }
}

/// Issue a system call with **three arguments** (in `EBX`, `ECX`, `EDX`).
#[inline(always)]
fn raw_syscall3(nr: usize, arg1: usize, arg2: usize, arg3: usize) -> Result<usize, usize> {
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
        Ok(ret as usize)
    } else {
        Err((-ret) as usize)
    }
}

// Syntax:  define_syscall!(NR_XXX => fn_name(arg: Type, ...) -> RetType)
//
// A single macro with four match arms (0–3 arguments). Each arm generates
// an `#[inline(always)] pub fn` that forwards to `raw_syscallN`, casting
// every argument to `usize` and the success value to `RetType`.
macro_rules! define_syscall {
    // 0 arguments
    ($nr:expr => $name:ident() -> $ret:ty) => {
        #[inline(always)]
        pub fn $name() -> Result<$ret, usize> {
            raw_syscall0($nr).map(|v| v as $ret)
        }
    };

    // 1 argument
    ($nr:expr => $name:ident($a:ident : $atype:ty) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype) -> Result<$ret, usize> {
            raw_syscall1($nr, $a as usize).map(|v| v as $ret)
        }
    };

    // 2 arguments
    ($nr:expr => $name:ident(
        $a:ident : $atype:ty,
        $b:ident : $btype:ty
    ) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype, $b: $btype) -> Result<$ret, usize> {
            raw_syscall2($nr, $a as usize, $b as usize).map(|v| v as $ret)
        }
    };

    // 3 arguments
    ($nr:expr => $name:ident(
        $a:ident : $atype:ty,
        $b:ident : $btype:ty,
        $c:ident : $ctype:ty
    ) -> $ret:ty) => {
        #[inline(always)]
        pub fn $name($a: $atype, $b: $btype, $c: $ctype) -> Result<$ret, usize> {
            raw_syscall3($nr, $a as usize, $b as usize, $c as usize).map(|v| v as $ret)
        }
    };
}

define_syscall!(NR_TEST => test(param: u32) -> usize);
