//! Console device — VGA text output + PS/2 keyboard input.
//!
//! The console serves as the TTY backend for channel 0. It provides:
//!
//! - [`init`]: Detects VGA hardware, registers the keyboard ISR on IRQ1.
//! - [`flush_output`]: Drains the TTY `tx` queue through the VGA text driver.
//!
//! VGA state and keyboard state are each protected by their own `KernelCell`.
//! The `flush_output` path acquires them sequentially (never nested):
//!
//! ```text
//! 1. DEVICES[0].state.exclusive() → batch-pop tx into stack buffer
//! 2. CONSOLE.exclusive()          → write buffer through VT102 parser
//! ```

mod keyboard;
mod vga;

use vga::CONSOLE;

use super::tty::Tty;
use crate::{
    pmio::{inb_p, outb, outb_p},
    trap,
};

/// Initialize the VGA console and register the keyboard interrupt handler.
///
/// Must be called after `task::init()` (which sets up the IDT base) but
/// before any TTY I/O is attempted.
pub fn init() {
    CONSOLE.exclusive(|vga| vga.detect_and_init());

    trap::set_intr_gate(0x21, keyboard::keyboard_interrupt);

    // Unmask IRQ1 (keyboard) on the master 8259A PIC.
    outb_p(inb_p(0x21) & 0xfd, 0x21);

    // Reset the keyboard controller by toggling the acknowledge line.
    let a = inb_p(0x61);
    outb_p(a | 0x80, 0x61);
    outb(a, 0x61);

    // From this point on, kernel print!/println! output flows through
    // the TTY layer and reaches the display via the console backend.
    crate::logging::set_tty_ready();
}

/// TTY backend flush callback for console channel 0.
///
/// Drains the `tx` ring buffer in batches and writes each batch through
/// the VGA VT102 parser, then synchronizes the hardware cursor.
pub fn flush_output(channel: usize) {
    let mut buf = [0u8; 256];
    loop {
        let count = Tty::device(channel).state.exclusive(|state| {
            let mut n = 0;
            while n < buf.len() {
                match state.tx.pop() {
                    Some(b) => {
                        buf[n] = b;
                        n += 1;
                    }
                    None => break,
                }
            }
            n
        });

        if count == 0 {
            break;
        }

        CONSOLE.exclusive(|vga| {
            for &byte in &buf[..count] {
                vga.write_byte(byte);
            }
            vga.sync_hardware_cursor();
        });
    }

    // Wake writers that may be blocked on a full tx queue.
    crate::task::WaitQueue::wake_up(&Tty::device(channel).output_wait);
}
