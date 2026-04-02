//! User-space data access via the FS segment.
//!
//! Reads and writes through the FS segment register, used for kernel/user
//! data transfer when FS points to a user data segment.

use core::arch::asm;

use alloc::string::String;

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
