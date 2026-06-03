//! Console device — VGA text output plus PS/2 keyboard input.
//!
//! The console is the TTY backend for channel 0. [`init`] detects the VGA
//! hardware and registers the keyboard ISR on IRQ1; [`ConsoleBackend`] drains
//! the channel's output queue through the VT102 parser.
//!
//! VGA state and keyboard state each live behind their own `KernelCell`. The
//! output path acquires only the VGA cell, never nested with the keyboard one.

mod keyboard;
mod vga;

use vga::CONSOLE;
pub use vga::{ORIG_X, ORIG_Y};

use super::tty::{self, TtyBackend};
use crate::{
    pmio::{
        KBD_ACK_BIT, KBD_CONTROL_PORT_B, PIC_IRQ_KEYBOARD, PIC_MASTER_MASK, inb_p, outb, outb_p,
    },
    trap,
};

/// Backend instance for console channel 0.
pub static CONSOLE_BACKEND: ConsoleBackend = ConsoleBackend;

/// Initialize the VGA console and register the keyboard interrupt handler.
///
/// Must run after `task::init()` (which sets up the IDT base) and before any
/// TTY I/O is attempted.
pub fn init() {
    CONSOLE.exclusive(vga::VgaConsole::detect_and_init);

    trap::set_intr_gate(0x21, keyboard::keyboard_interrupt);

    // Unmask IRQ1 (keyboard) on the master PIC.
    outb_p(inb_p(PIC_MASTER_MASK) & !PIC_IRQ_KEYBOARD, PIC_MASTER_MASK);

    // Reset the keyboard controller by pulsing the acknowledge line.
    let port_b = inb_p(KBD_CONTROL_PORT_B);
    outb_p(port_b | KBD_ACK_BIT, KBD_CONTROL_PORT_B);
    outb(port_b, KBD_CONTROL_PORT_B);

    // From here on, kernel print output flows through the TTY layer and reaches
    // the display via this backend.
    crate::logging::set_tty_ready();
}

/// TTY backend for the VGA console.
pub struct ConsoleBackend;

/// Maximum bytes drained from the output queue per parser batch.
const OUTPUT_BATCH: usize = 256;

impl TtyBackend for ConsoleBackend {
    fn start_output(&self, channel: usize) {
        let mut buf = [0u8; OUTPUT_BATCH];
        loop {
            let count = tty::take_output(channel, &mut buf);
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

        tty::notify_output_drained(channel);
    }
}
