//! Kernel ↔ user-space data transfer via the FS segment register.
//!
//! All reads and writes go through `%fs`, which points to the current task's
//! user data segment during system calls.

use alloc::string::String;
use core::{arch::asm, mem};

/// Maximum length for user-space pathname strings.
pub const MAX_PATH_LEN: usize = 256;

/// Reads a `u8` from `addr` through the FS segment.
#[inline]
pub fn read_u8(addr: *const u8) -> u8 {
    let v: u8;
    unsafe {
        asm!(
            "movb %fs:({}), {}",
            in(reg) addr as u32,
            out(reg_byte) v,
            options(nomem, nostack, att_syntax)
        );
    }
    v
}

/// Reads a `u16` from `addr` through the FS segment.
#[inline]
pub fn read_u16(addr: *const u16) -> u16 {
    let v: u16;
    unsafe {
        asm!(
            "movw %fs:({}), {1:x}",
            in(reg) addr as u32,
            out(reg) v,
            options(nomem, nostack, att_syntax)
        );
    }
    v
}

/// Reads a `u32` from `addr` through the FS segment.
#[inline]
pub fn read_u32(addr: *const u32) -> u32 {
    let v: u32;
    unsafe {
        asm!(
            "movl %fs:({}), {}",
            in(reg) addr as u32,
            out(reg) v,
            options(nomem, nostack, att_syntax)
        );
    }
    v
}

/// Writes a `u8` to `addr` through the FS segment.
#[inline]
pub fn write_u8(val: u8, addr: *mut u8) {
    unsafe {
        asm!(
            "movb {}, %fs:({})",
            in(reg_byte) val,
            in(reg) addr as u32,
            options(nomem, nostack, att_syntax)
        );
    }
}

/// Writes a `u32` to `addr` through the FS segment.
#[inline]
pub fn write_u32(val: u32, addr: *mut u32) {
    unsafe {
        asm!(
            "movl {}, %fs:({})",
            in(reg) val,
            in(reg) addr as u32,
            options(nomem, nostack, att_syntax)
        );
    }
}

/// Reads a NUL-terminated C string from user space into a kernel [`String`].
///
/// Stops at the first zero byte or after `max_len` bytes.
pub fn read_string(addr: *const u8, max_len: usize) -> String {
    let mut s = String::new();
    for i in 0..max_len {
        let b = read_u8(unsafe { addr.add(i) });
        if b == 0 {
            break;
        }
        s.push(b as char);
    }
    s
}

/// Read a null-terminated pathname from user space at `addr`.
pub fn read_pathname(addr: u32) -> String {
    read_string(addr as *const u8, MAX_PATH_LEN)
}

/// Copies `buf.len()` bytes from user space at `addr` into `buf`.
pub fn read_bytes(addr: *const u8, buf: &mut [u8]) {
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = read_u8(unsafe { addr.add(i) });
    }
}

/// Copies `buf.len()` bytes from `buf` to user space at `addr`.
pub fn write_bytes(buf: &[u8], addr: *mut u8) {
    for (i, &b) in buf.iter().enumerate() {
        write_u8(b, unsafe { addr.add(i) });
    }
}

/// Copies one plain ABI value from user space.
pub fn read_struct<T: Copy>(addr: *const T) -> T {
    let mut value = mem::MaybeUninit::<T>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), mem::size_of::<T>())
    };
    read_bytes(addr.cast::<u8>(), bytes);
    unsafe { value.assume_init() }
}

/// Copies one plain ABI value to user space.
pub fn write_struct<T: Copy>(value: &T, addr: *mut T) {
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), mem::size_of::<T>())
    };
    write_bytes(bytes, addr.cast::<u8>());
}
