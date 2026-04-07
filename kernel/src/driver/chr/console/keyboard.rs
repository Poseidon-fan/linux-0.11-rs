//! PS/2 keyboard interrupt handler and scan code translation.
//!
//! Reads scan codes from port 0x60, translates them via static US keymap
//! tables, and pushes ASCII bytes into the console TTY's `raw_rx` queue.
//!
//! Modifier tracking (Shift, Ctrl, Alt, CapsLock, NumLock) is maintained in a
//! [`KeyboardState`] struct protected by `KernelCell`.
//!
//! Scan code → ASCII pipeline:
//!
//! ```text
//!  IRQ1 ──► read 0x60 ──► translate(scancode) ──► raw_rx.push()
//!                                │
//!                                ├─ modifier keys: update state only
//!                                ├─ normal keys: select map, apply Ctrl/Caps
//!                                └─ cursor keys: push ESC [ <letter> sequence
//! ```

use core::arch::naked_asm;

use bitflags::bitflags;

use super::super::tty::Tty;
use crate::{
    pmio::{inb, outb},
    sync::KernelCell,
};

bitflags! {
    /// Active modifier key state, tracked across make/break scan codes.
    #[derive(Clone, Copy)]
    struct Modifiers: u8 {
        const LEFT_SHIFT  = 0x01;
        const RIGHT_SHIFT = 0x02;
        const LEFT_CTRL   = 0x04;
        const RIGHT_CTRL  = 0x08;
        const LEFT_ALT    = 0x10;
        const RIGHT_ALT   = 0x20;
        const CAPS_LOCK   = 0x40;
        const CAPS_PRESSED = 0x80;
    }
}

struct KeyboardState {
    modifiers: Modifiers,
    leds: u8,
    extended_prefix: bool,
}

static KEYBOARD: KernelCell<KeyboardState> = KernelCell::new(KeyboardState {
    modifiers: Modifiers::empty(),
    leds: 2, // Num Lock on
    extended_prefix: false,
});

/// US keyboard layout — unshifted key map.
/// Index = scan code (0x00..0x3A), value = ASCII (0 = no mapping).
#[rustfmt::skip]
static NORMAL_MAP: [u8; 89] = [
    0,   27,  b'1', b'2', b'3', b'4', b'5', b'6',  // 0x00-0x07
    b'7', b'8', b'9', b'0', b'-', b'=', 127, 9,     // 0x08-0x0F
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', // 0x10-0x17
    b'o', b'p', b'[', b']', 13,  0,    b'a', b's',   // 0x18-0x1F
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';', // 0x20-0x27
    b'\'',b'`', 0,   b'\\',b'z', b'x', b'c', b'v',  // 0x28-0x2F
    b'b', b'n', b'm', b',', b'.', b'/', 0,   b'*',   // 0x30-0x37
    0,    b' ', 0,    0,    0,    0,    0,    0,       // 0x38-0x3F
    0,    0,    0,    0,    0,    0,    0,    b'7',     // 0x40-0x47
    b'8', b'9', b'-', b'4', b'5', b'6', b'+', b'1',   // 0x48-0x4F
    b'2', b'3', b'0', b',',                            // 0x50-0x53
    0,    0,    b'<', 0,    0,                          // 0x54-0x58
];

/// US keyboard layout — shifted key map.
#[rustfmt::skip]
static SHIFT_MAP: [u8; 89] = [
    0,   27,  b'!', b'@', b'#', b'$', b'%', b'^',  // 0x00-0x07
    b'&', b'*', b'(', b')', b'_', b'+', 127, 9,     // 0x08-0x0F
    b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I', // 0x10-0x17
    b'O', b'P', b'{', b'}', 13,  0,    b'A', b'S',   // 0x18-0x1F
    b'D', b'F', b'G', b'H', b'J', b'K', b'L', b':', // 0x20-0x27
    b'"', b'~', 0,   b'|', b'Z', b'X', b'C', b'V',  // 0x28-0x2F
    b'B', b'N', b'M', b'<', b'>', b'?', 0,   b'*',   // 0x30-0x37
    0,    b' ', 0,    0,    0,    0,    0,    0,       // 0x38-0x3F
    0,    0,    0,    0,    0,    0,    0,    b'7',     // 0x40-0x47
    b'8', b'9', b'-', b'4', b'5', b'6', b'+', b'1',   // 0x48-0x4F
    b'2', b'3', b'0', b',',                            // 0x50-0x53
    0,    0,    b'>', 0,    0,                          // 0x54-0x58
];

/// Cursor/numpad scan codes (0x47..0x53) to escape sequence final character.
/// For keys producing ESC [ <char>, value > '9' means no tilde suffix.
/// Values <= '9' produce ESC [ <char> ~.
static CURSOR_TABLE: [u8; 13] = [
    b'H', b'A', b'5', // Home, Up, PgUp
    0,    // (gap: 0x4A = numpad -)
    b'D', b'G', b'C', // Left, center, Right
    0,    // (gap: 0x4E = numpad +)
    b'Y', b'B', b'6', // End, Down, PgDn
    b'2', b'3', // Ins, Del
];

impl KeyboardState {
    /// Translate one scan code into zero or more ASCII bytes pushed to the
    /// given buffer. Returns the number of bytes produced.
    fn translate(&mut self, scancode: u8, out: &mut [u8; 8]) -> usize {
        // Handle extended prefix codes.
        if scancode == 0xe0 {
            self.extended_prefix = true;
            return 0;
        }
        if scancode == 0xe1 {
            self.extended_prefix = true;
            return 0;
        }

        let is_break = scancode & 0x80 != 0;
        let code = scancode & 0x7f;

        let was_extended = self.extended_prefix;
        self.extended_prefix = false;

        // --- Modifier keys ---
        match code {
            0x2a => {
                if is_break {
                    self.modifiers.remove(Modifiers::LEFT_SHIFT);
                } else {
                    self.modifiers.insert(Modifiers::LEFT_SHIFT);
                }
                return 0;
            }
            0x36 => {
                if is_break {
                    self.modifiers.remove(Modifiers::RIGHT_SHIFT);
                } else {
                    self.modifiers.insert(Modifiers::RIGHT_SHIFT);
                }
                return 0;
            }
            0x1d => {
                if is_break {
                    if was_extended {
                        self.modifiers.remove(Modifiers::RIGHT_CTRL);
                    } else {
                        self.modifiers.remove(Modifiers::LEFT_CTRL);
                    }
                } else if was_extended {
                    self.modifiers.insert(Modifiers::RIGHT_CTRL);
                } else {
                    self.modifiers.insert(Modifiers::LEFT_CTRL);
                }
                return 0;
            }
            0x38 => {
                if is_break {
                    if was_extended {
                        self.modifiers.remove(Modifiers::RIGHT_ALT);
                    } else {
                        self.modifiers.remove(Modifiers::LEFT_ALT);
                    }
                } else if was_extended {
                    self.modifiers.insert(Modifiers::RIGHT_ALT);
                } else {
                    self.modifiers.insert(Modifiers::LEFT_ALT);
                }
                return 0;
            }
            0x3a => {
                if is_break {
                    self.modifiers.remove(Modifiers::CAPS_PRESSED);
                } else if !self.modifiers.contains(Modifiers::CAPS_PRESSED) {
                    self.modifiers.toggle(Modifiers::CAPS_LOCK);
                    self.modifiers.insert(Modifiers::CAPS_PRESSED);
                }
                return 0;
            }
            0x45 => {
                if !is_break {
                    self.leds ^= 0x02;
                }
                return 0;
            }
            _ => {}
        }

        // Ignore break codes for non-modifier keys.
        if is_break {
            return 0;
        }

        // --- Cursor / numpad keys (0x47..0x53) ---
        if (0x47..=0x53).contains(&code) {
            let idx = (code - 0x47) as usize;
            let shifted = self
                .modifiers
                .intersects(Modifiers::LEFT_SHIFT | Modifiers::RIGHT_SHIFT);
            let num_lock_enabled = (self.leds & 0x02) != 0;
            let use_cursor_sequence = was_extended || !num_lock_enabled || shifted;

            if use_cursor_sequence {
                let ch = CURSOR_TABLE[idx];
                if ch != 0 {
                    out[0] = 0x1b;
                    out[1] = b'[';
                    if ch > b'9' {
                        out[2] = ch;
                        return 3;
                    } else {
                        out[2] = ch;
                        out[3] = b'~';
                        return 4;
                    }
                }
            }
        }

        // --- Normal keys ---
        if (code as usize) >= NORMAL_MAP.len() {
            return 0;
        }

        let shifted = self
            .modifiers
            .intersects(Modifiers::LEFT_SHIFT | Modifiers::RIGHT_SHIFT);
        let mut c = if shifted {
            SHIFT_MAP[code as usize]
        } else {
            NORMAL_MAP[code as usize]
        };

        if c == 0 {
            return 0;
        }

        // CapsLock: swap case for letters.
        if self.modifiers.contains(Modifiers::CAPS_LOCK) {
            if c.is_ascii_lowercase() {
                c = c.to_ascii_uppercase();
            } else if c.is_ascii_uppercase() {
                c = c.to_ascii_lowercase();
            }
        }

        // Ctrl: map A-Z / a-z to control codes 0x01..0x1a.
        if self
            .modifiers
            .intersects(Modifiers::LEFT_CTRL | Modifiers::RIGHT_CTRL)
        {
            if (b'@'..b'@' + 32).contains(&c) {
                c -= b'@';
            } else if c.is_ascii_lowercase() {
                c -= b'a' - 1;
            }
        }

        // Left Alt: set high bit.
        if self.modifiers.contains(Modifiers::LEFT_ALT) {
            c |= 0x80;
        }

        out[0] = c;
        1
    }
}

/// Naked ISR stub for IRQ1 (keyboard interrupt).
///
/// Saves registers, sets up kernel data segments, calls the Rust handler,
/// then restores and returns via `iret`. Follows the same convention as
/// `timer_interrupt` in `task/timer.rs`.
#[naked]
pub extern "C" fn keyboard_interrupt() {
    unsafe {
        naked_asm!(
            "pushl %eax",
            "pushl %ebx",
            "pushl %ecx",
            "pushl %edx",
            "push %ds",
            "push %es",
            "movl $0x10, %eax",
            "movw %ax, %ds",
            "movw %ax, %es",
            "call {handler}",
            "pop %es",
            "pop %ds",
            "popl %edx",
            "popl %ecx",
            "popl %ebx",
            "popl %eax",
            "iret",
            handler = sym keyboard_handler,
            options(att_syntax),
        );
    }
}

/// Rust-side keyboard interrupt handler.
///
/// Reads the scan code, acknowledges the keyboard controller and PIC,
/// translates the scan code, pushes resulting bytes into the console TTY's
/// `raw_rx`, then invokes the line discipline via `on_interrupt`.
extern "C" fn keyboard_handler() {
    let scancode = inb(0x60);

    // Toggle keyboard controller acknowledge lines.
    let port_b = inb(0x61);
    outb(port_b | 0x80, 0x61);
    outb(port_b, 0x61);

    // Translate scan code to ASCII.
    let mut buf = [0u8; 8];
    let count = unsafe { KEYBOARD.exclusive_unchecked(|kb| kb.translate(scancode, &mut buf)) };

    if count > 0 {
        unsafe {
            Tty::device(0).state.exclusive_unchecked(|state| {
                for &b in &buf[..count] {
                    let _ = state.raw_rx.push(b);
                }
            });
        }
    }

    // Send End-Of-Interrupt to master PIC.
    outb(0x20, 0x20);

    // Process raw input through the line discipline.
    // This is called outside all KernelCell borrows to avoid nesting.
    // Safety: we are in IRQ context with interrupts masked by the interrupt gate.
    // on_interrupt internally uses exclusive() which nests properly.
    Tty::device(0).on_interrupt(0);
}
