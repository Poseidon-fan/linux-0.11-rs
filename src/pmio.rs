//! Port-Mapped I/O (PMIO) operations for i386.

use core::arch::asm;

/// Write a byte to the specified I/O port.
#[inline]
pub fn outb(value: u8, port: u16) {
    unsafe {
        asm!(
            "outb %al, %dx",
            in("dx") port,
            in("al") value,
            options(nostack, preserves_flags, att_syntax)
        );
    }
}

/// Read a byte from the specified I/O port.
#[inline]
pub fn inb(port: u16) -> u8 {
    unsafe {
        let value: u8;
        asm!(
            "inb %dx, %al",
            out("al") value,
            in("dx") port,
            options(nostack, preserves_flags, att_syntax)
        );
        value
    }
}

/// Write a byte to the specified I/O port with a small delay for slow devices.
#[inline]
pub fn outb_p(value: u8, port: u16) {
    unsafe {
        asm!(
            "outb %al, %dx",
            "jmp 2f",
            "2: jmp 3f",
            "3:",
            in("dx") port,
            in("al") value,
            options(nostack, preserves_flags, att_syntax)
        );
    }
}

/// Read a byte from the specified I/O port with a small delay for slow devices.
#[inline]
pub fn inb_p(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!(
            "inb %dx, %al",
            "jmp 2f",
            "2: jmp 3f",
            "3:",
            out("al") value,
            in("dx") port,
            options(nostack, preserves_flags, att_syntax)
        );
    }
    value
}
