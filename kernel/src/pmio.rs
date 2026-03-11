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

/// Read one PIO word stream from the specified I/O port.
#[inline]
pub fn port_read_words(port: u16, dst: *mut u16, word_count: usize) {
    unsafe {
        asm!(
            "push %edi",
            "cld",
            "movl {dst:e}, %edi",
            "rep insw",
            "pop %edi",
            dst = in(reg) dst,
            in("dx") port,
            inout("ecx") word_count => _,
            options(att_syntax)
        );
    }
}

/// Write one PIO word stream to the specified I/O port.
#[inline]
pub fn port_write_words(port: u16, src: *const u16, word_count: usize) {
    unsafe {
        asm!(
            "push %esi",
            "cld",
            "movl {src:e}, %esi",
            "rep outsw",
            "pop %esi",
            src = in(reg) src,
            in("dx") port,
            inout("ecx") word_count => _,
            options(att_syntax)
        );
    }
}
