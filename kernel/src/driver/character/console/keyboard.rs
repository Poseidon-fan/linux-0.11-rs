//! PS/2 keyboard interrupt handler and scan-code translation.
//!
//! Scan codes read from port 0x60 are translated through static US keymap
//! tables and fed into the console TTY as ASCII (or escape sequences for the
//! cursor/keypad cluster):
//!
//! ```text
//!  IRQ1 ──► read 0x60 ──► translate(scancode) ──► tty::receive_input()
//!                               │
//!                               ├─ modifier keys: update state (+ LEDs)
//!                               ├─ normal keys: select map, apply Ctrl/Caps
//!                               └─ cursor keys: emit ESC [ <letter> sequence
//! ```
//!
//! Modifier and lock state lives in a [`KeyboardState`] behind a `KernelCell`.

use core::arch::naked_asm;

use bitflags::bitflags;

use super::super::tty;
use crate::{
    pmio::{inb, outb},
    sync::KernelCell,
};

/// Naked ISR stub for IRQ1 (keyboard).
///
/// Saves registers, loads the kernel data segments, calls the Rust handler,
/// then restores and returns via `iret`.
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

/// Shared keyboard controller state, guarded by a `KernelCell`.
static KEYBOARD: KernelCell<KeyboardState> = KernelCell::new(KeyboardState {
    modifiers: Modifiers::empty(),
    leds: Leds::NUM_LOCK,
    extended_prefix: false,
});

/// US keyboard layout — unshifted map. Index = scan code, value = ASCII
/// (`0` means no mapping).
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

/// US keyboard layout — shifted map.
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

/// Cursor/keypad scan codes (0x47..=0x53) to escape-sequence final character.
/// A value `> b'9'` emits `ESC [ <char>`; a value `<= b'9'` emits the tilde
/// form `ESC [ <char> ~`. `0` entries are non-cursor keys in this range.
#[rustfmt::skip]
static CURSOR_TABLE: [u8; 13] = [
    b'H', b'A', b'5', // Home, Up, PgUp
    0,                // 0x4A numpad -
    b'D', b'G', b'C', // Left, center, Right
    0,                // 0x4E numpad +
    b'Y', b'B', b'6', // End, Down, PgDn
    b'2', b'3',       // Ins, Del
];

/// Keyboard controller data/command port.
const KBD_DATA_PORT: u16 = 0x60;

/// Keyboard controller status/command port.
const KBD_STATUS_PORT: u16 = 0x64;

/// Status-register bit set while the controller input buffer is full.
const KBD_INPUT_FULL: u8 = 1 << 1;

/// Controller command: set keyboard LEDs (followed by an [`Leds`] byte).
const KBD_CMD_SET_LEDS: u8 = 0xed;

/// Keyboard controller port B (acknowledge toggling).
const KBD_CONTROL_PORT_B: u16 = 0x61;

/// Acknowledge bit pulsed on port B after reading a scan code.
const KBD_ACK_BIT: u8 = 1 << 7;

/// Master PIC command register.
const PIC_MASTER_COMMAND: u16 = 0x20;

/// End-of-interrupt command for the 8259A PIC.
const PIC_EOI: u8 = 0x20;

/// Extended scan-code prefix bytes (0xe0 / 0xe1).
const EXTENDED_PREFIXES: [u8; 2] = [0xe0, 0xe1];

/// Break (key-release) bit set in a scan code.
const BREAK_BIT: u8 = 0x80;

/// First scan code of the cursor/keypad cluster.
const CURSOR_FIRST: u8 = 0x47;
/// Last scan code of the cursor/keypad cluster.
const CURSOR_LAST: u8 = 0x53;

/// Maximum bytes one key press can translate to (longest escape sequence).
const MAX_TRANSLATION: usize = 8;

/// ASCII escape, leading byte of cursor-key sequences.
const ESC: u8 = 0x1b;

/// High bit OR-ed into a byte when Left Alt is held (meta prefix).
const ALT_HIGH_BIT: u8 = 0x80;

/// CapsLock make/break scan code.
const CAPS_LOCK_CODE: u8 = 0x3a;
/// NumLock make/break scan code.
const NUM_LOCK_CODE: u8 = 0x45;

bitflags! {
    /// Active modifier and lock state, tracked across make/break scan codes.
    #[derive(Clone, Copy)]
    struct Modifiers: u8 {
        const LEFT_SHIFT   = 1 << 0;
        const RIGHT_SHIFT  = 1 << 1;
        const LEFT_CTRL    = 1 << 2;
        const RIGHT_CTRL   = 1 << 3;
        const LEFT_ALT     = 1 << 4;
        const RIGHT_ALT    = 1 << 5;
        /// CapsLock toggle is currently active.
        const CAPS_LOCK    = 1 << 6;
        /// CapsLock key is held down (debounces the toggle).
        const CAPS_PRESSED = 1 << 7;

        /// Either Shift key.
        const SHIFT = Self::LEFT_SHIFT.bits() | Self::RIGHT_SHIFT.bits();
        /// Either Ctrl key.
        const CTRL = Self::LEFT_CTRL.bits() | Self::RIGHT_CTRL.bits();
    }
}

bitflags! {
    /// Keyboard LED state sent to the controller with the set-LEDs command.
    #[derive(Clone, Copy)]
    struct Leds: u8 {
        const SCROLL_LOCK = 1 << 0;
        const NUM_LOCK    = 1 << 1;
        const CAPS_LOCK   = 1 << 2;
    }
}

/// Keyboard controller state tracked across interrupts.
struct KeyboardState {
    /// Active modifier and lock keys.
    modifiers: Modifiers,
    /// Current LED state mirrored to the controller.
    leds: Leds,
    /// Whether the previous scan code was an extended (0xe0/0xe1) prefix.
    extended_prefix: bool,
}

/// Outcome of translating one scan code.
enum Key {
    /// Code was consumed without producing output (modifier, prefix, break).
    Consumed,
    /// Code produced the given number of bytes in the output buffer.
    Bytes(usize),
}

/// Rust-side keyboard interrupt handler: read the scan code, acknowledge the
/// controller and PIC, translate, and feed the result into the console TTY.
extern "C" fn keyboard_handler() {
    let scancode = inb(KBD_DATA_PORT);

    // Pulse the controller acknowledge line.
    let port_b = inb(KBD_CONTROL_PORT_B);
    outb(port_b | KBD_ACK_BIT, KBD_CONTROL_PORT_B);
    outb(port_b, KBD_CONTROL_PORT_B);

    let mut buf = [0u8; MAX_TRANSLATION];
    // SAFETY: runs inside the IRQ handler with interrupts masked, so no
    // reentrant access to KEYBOARD can occur.
    let count = unsafe { KEYBOARD.exclusive_unchecked(|kb| kb.translate(scancode, &mut buf)) };

    outb(PIC_EOI, PIC_MASTER_COMMAND);

    if count > 0 {
        tty::receive_input(0, &buf[..count]);
    }
}

/// Map a modifier/lock scan code to the [`Modifiers`] bit it controls.
///
/// CapsLock and NumLock have no single [`Modifiers`] bit of their own (they are
/// handled specially in [`KeyboardState::apply_modifier`]) but still resolve
/// here so the caller routes them as modifier keys.
fn modifier_for(code: u8, was_extended: bool) -> Option<Modifiers> {
    Some(match code {
        0x2a => Modifiers::LEFT_SHIFT,
        0x36 => Modifiers::RIGHT_SHIFT,
        0x1d if was_extended => Modifiers::RIGHT_CTRL,
        0x1d => Modifiers::LEFT_CTRL,
        0x38 if was_extended => Modifiers::RIGHT_ALT,
        0x38 => Modifiers::LEFT_ALT,
        CAPS_LOCK_CODE | NUM_LOCK_CODE => Modifiers::empty(),
        _ => return None,
    })
}

/// Swap the case of an ASCII letter, leaving other bytes unchanged.
fn swap_ascii_case(byte: u8) -> u8 {
    if byte.is_ascii_lowercase() {
        byte.to_ascii_uppercase()
    } else if byte.is_ascii_uppercase() {
        byte.to_ascii_lowercase()
    } else {
        byte
    }
}

/// Map a byte to its Ctrl-modified control code (`Ctrl-A` == 0x01), leaving
/// bytes outside the control range unchanged.
fn to_control(byte: u8) -> u8 {
    match byte {
        b'@'..=b'_' => byte - b'@',
        b'a'..=b'z' => byte - b'a' + 1,
        _ => byte,
    }
}

/// Wait until the keyboard controller input buffer is empty.
fn kbd_wait() {
    while inb(KBD_STATUS_PORT) & KBD_INPUT_FULL != 0 {}
}

impl KeyboardState {
    /// Translate one scan code into ASCII bytes written to `out`, returning the
    /// number of bytes produced.
    fn translate(&mut self, scancode: u8, out: &mut [u8; MAX_TRANSLATION]) -> usize {
        if EXTENDED_PREFIXES.contains(&scancode) {
            self.extended_prefix = true;
            return 0;
        }

        let is_break = scancode & BREAK_BIT != 0;
        let code = scancode & !BREAK_BIT;
        let was_extended = self.extended_prefix;
        self.extended_prefix = false;

        let key = if let Some(modifier) = modifier_for(code, was_extended) {
            self.apply_modifier(code, modifier, is_break);
            Key::Consumed
        } else if is_break {
            Key::Consumed
        } else if (CURSOR_FIRST..=CURSOR_LAST).contains(&code) {
            self.translate_cursor(code, was_extended, out)
        } else {
            self.translate_normal(code, out)
        };

        match key {
            Key::Consumed => 0,
            Key::Bytes(n) => n,
        }
    }

    /// Update modifier/lock state for a modifier key.
    fn apply_modifier(&mut self, code: u8, modifier: Modifiers, is_break: bool) {
        match code {
            CAPS_LOCK_CODE => self.toggle_caps_lock(is_break),
            NUM_LOCK_CODE => {
                if !is_break {
                    self.leds.toggle(Leds::NUM_LOCK);
                    self.update_leds();
                }
            }
            _ => self.modifiers.set(modifier, !is_break),
        }
    }

    /// Toggle CapsLock on key-down, debouncing auto-repeat, and refresh the LED.
    fn toggle_caps_lock(&mut self, is_break: bool) {
        if is_break {
            self.modifiers.remove(Modifiers::CAPS_PRESSED);
        } else if !self.modifiers.contains(Modifiers::CAPS_PRESSED) {
            self.modifiers.toggle(Modifiers::CAPS_LOCK);
            self.modifiers.insert(Modifiers::CAPS_PRESSED);
            self.leds.set(
                Leds::CAPS_LOCK,
                self.modifiers.contains(Modifiers::CAPS_LOCK),
            );
            self.update_leds();
        }
    }

    /// Translate a cursor/keypad key, emitting an escape sequence when the
    /// cursor interpretation applies (extended key, NumLock off, or Shift).
    fn translate_cursor(
        &self,
        code: u8,
        was_extended: bool,
        out: &mut [u8; MAX_TRANSLATION],
    ) -> Key {
        let num_lock = self.leds.contains(Leds::NUM_LOCK);
        let as_cursor = was_extended || !num_lock || self.modifiers.intersects(Modifiers::SHIFT);
        if !as_cursor {
            return self.translate_normal(code, out);
        }

        let final_char = CURSOR_TABLE[(code - CURSOR_FIRST) as usize];
        if final_char == 0 {
            return Key::Consumed;
        }

        // `ESC [ <char>`, with a trailing `~` for keys whose code is a digit.
        out[0] = ESC;
        out[1] = b'[';
        out[2] = final_char;
        if final_char > b'9' {
            Key::Bytes(3)
        } else {
            out[3] = b'~';
            Key::Bytes(4)
        }
    }

    /// Translate a normal key through the keymap, applying CapsLock, Ctrl, and
    /// Alt modifiers.
    fn translate_normal(&self, code: u8, out: &mut [u8; MAX_TRANSLATION]) -> Key {
        let Some(&base) = self.active_map().get(code as usize) else {
            return Key::Consumed;
        };
        if base == 0 {
            return Key::Consumed;
        }

        let mut ch = base;
        if self.modifiers.contains(Modifiers::CAPS_LOCK) {
            ch = swap_ascii_case(ch);
        }
        if self.modifiers.intersects(Modifiers::CTRL) {
            ch = to_control(ch);
        }
        if self.modifiers.contains(Modifiers::LEFT_ALT) {
            ch |= ALT_HIGH_BIT;
        }

        out[0] = ch;
        Key::Bytes(1)
    }

    /// Select the keymap matching the current Shift state.
    fn active_map(&self) -> &'static [u8; 89] {
        if self.modifiers.intersects(Modifiers::SHIFT) {
            &SHIFT_MAP
        } else {
            &NORMAL_MAP
        }
    }

    /// Send the current LED state to the keyboard controller.
    fn update_leds(&self) {
        kbd_wait();
        outb(KBD_CMD_SET_LEDS, KBD_DATA_PORT);
        kbd_wait();
        outb(self.leds.bits(), KBD_DATA_PORT);
    }
}
