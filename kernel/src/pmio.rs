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

/// Read a byte from the specified CMOS register.
///
/// The register index is written with the NMI-disable bit set, matching the
/// early-kernel access pattern used while probing legacy firmware state.
#[inline]
pub fn read_cmos(register: u8) -> u8 {
    outb_p(CMOS_NMI_DISABLE_FLAG | register, CMOS_ADDRESS_PORT);
    inb_p(CMOS_DATA_PORT)
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

// ---------------------------------------------------------------------------
// 8259A Programmable Interrupt Controller (PIC)
// ---------------------------------------------------------------------------

/// Master PIC command/status register.
pub const PIC_MASTER_COMMAND: u16 = 0x20;
/// Master PIC interrupt mask register (IMR).
pub const PIC_MASTER_MASK: u16 = 0x21;
/// Slave PIC command/status register.
pub const PIC_SLAVE_COMMAND: u16 = 0xA0;
/// Slave PIC interrupt mask register (IMR).
pub const PIC_SLAVE_MASK: u16 = 0xA1;
/// End-of-interrupt command byte sent to a PIC command register.
pub const PIC_EOI: u8 = 0x20;

/// IRQ0 (PIT timer) mask bit in the master PIC IMR.
pub const PIC_IRQ_TIMER: u8 = 1 << 0;
/// IRQ1 (keyboard) mask bit in the master PIC IMR.
pub const PIC_IRQ_KEYBOARD: u8 = 1 << 1;
/// IRQ2 (cascade to slave PIC) mask bit in the master PIC IMR.
pub const PIC_IRQ_CASCADE: u8 = 1 << 2;
/// IRQ3 (COM2) mask bit in the master PIC IMR.
pub const PIC_IRQ_COM2: u8 = 1 << 3;
/// IRQ4 (COM1) mask bit in the master PIC IMR.
pub const PIC_IRQ_COM1: u8 = 1 << 4;
/// IRQ14 (ATA hard disk) mask bit in the slave PIC IMR.
pub const PIC_IRQ_HD: u8 = 1 << 6;

// ---------------------------------------------------------------------------
// 8253/8254 Programmable Interval Timer (PIT)
// ---------------------------------------------------------------------------

/// PIT channel 0 data port (wired to IRQ0 for scheduler ticks).
pub const PIT_CH0_DATA: u16 = 0x40;
/// PIT channel 2 data port (wired to the PC speaker).
pub const PIT_CH2_DATA: u16 = 0x42;
/// PIT mode/command register.
pub const PIT_COMMAND: u16 = 0x43;

/// PIT command: channel 0, lobyte/hibyte, mode 3 (square wave), 16-bit binary.
pub const PIT_CH0_SQUARE_WAVE: u8 = 0x36;
/// PIT command: channel 2, lobyte/hibyte, mode 3 (square wave), 16-bit binary.
pub const PIT_CH2_SQUARE_WAVE: u8 = 0xB6;

// ---------------------------------------------------------------------------
// 8042 Keyboard Controller
// ---------------------------------------------------------------------------

/// Keyboard controller data port (read scan codes / write commands).
pub const KBD_DATA_PORT: u16 = 0x60;
/// Keyboard controller port B (also gates the PC speaker).
pub const KBD_CONTROL_PORT_B: u16 = 0x61;
/// Keyboard controller status/command port.
pub const KBD_STATUS_PORT: u16 = 0x64;
/// Acknowledge bit pulsed on port B after reading a scan code.
pub const KBD_ACK_BIT: u8 = 1 << 7;

// ---------------------------------------------------------------------------
// x87 Coprocessor
// ---------------------------------------------------------------------------

/// Coprocessor busy-clear port (write any value to reset the busy latch).
pub const COPROC_BUSY_PORT: u16 = 0xF0;

// ---------------------------------------------------------------------------
// 8250 / 16450 UART Serial Ports (COM1 / COM2)
// ---------------------------------------------------------------------------

/// COM1 base I/O port address.
pub const SERIAL_COM1_BASE: u16 = 0x3F8;
/// COM2 base I/O port address.
pub const SERIAL_COM2_BASE: u16 = 0x2F8;

// ---------------------------------------------------------------------------
// VGA / EGA CRT Controller Registers
// ---------------------------------------------------------------------------

/// CRT controller index register for color displays.
pub const VGA_CRTC_INDEX_COLOR: u16 = 0x3D4;
/// CRT controller data register for color displays.
pub const VGA_CRTC_DATA_COLOR: u16 = 0x3D5;
/// CRT controller index register for monochrome displays.
pub const VGA_CRTC_INDEX_MONO: u16 = 0x3B4;
/// CRT controller data register for monochrome displays.
pub const VGA_CRTC_DATA_MONO: u16 = 0x3B5;

// ---------------------------------------------------------------------------
// ATA / IDE hard disk controller (primary channel)
// ---------------------------------------------------------------------------

/// ATA task-file data register.
pub const ATA_DATA_PORT: u16 = 0x1F0;
/// ATA error register (read) / write-precompensation register (write).
pub const ATA_ERROR_PORT: u16 = 0x1F1;
/// ATA sector-count register.
pub const ATA_SECTOR_COUNT_PORT: u16 = 0x1F2;
/// ATA sector-number register.
pub const ATA_SECTOR_NUMBER_PORT: u16 = 0x1F3;
/// ATA cylinder-low register.
pub const ATA_CYLINDER_LOW_PORT: u16 = 0x1F4;
/// ATA cylinder-high register.
pub const ATA_CYLINDER_HIGH_PORT: u16 = 0x1F5;
/// ATA drive/head selector register.
pub const ATA_DRIVE_HEAD_PORT: u16 = 0x1F6;
/// ATA status register (read) / command register (write).
pub const ATA_STATUS_PORT: u16 = 0x1F7;
/// ATA device-control register (alternate status).
pub const ATA_CONTROL_PORT: u16 = 0x3F6;

// ---------------------------------------------------------------------------
// CMOS / RTC
// ---------------------------------------------------------------------------

const CMOS_ADDRESS_PORT: u16 = 0x70;
const CMOS_DATA_PORT: u16 = 0x71;
const CMOS_NMI_DISABLE_FLAG: u8 = 0x80;
