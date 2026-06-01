//! N_TTY line discipline.
//!
//! Converts raw hardware input into cooked bytes that `read()` consumes,
//! applying input-mode mapping, canonical editing, signal characters, XON/XOFF
//! flow control, echo, and complete-line accounting.

use user_lib::syscall::{
    signal::Signal,
    tty::{ControlChar, InputMode, LocalMode},
};

use super::{TtyState, signal_foreground_group};

/// Drain raw input through the line discipline.
///
/// Returns `true` when the backend should be kicked to flush output — either
/// because bytes were echoed or because an XON resumed a stopped transmitter.
pub fn process_raw_input(state: &mut TtyState) -> bool {
    let mut needs_flush = false;

    while !state.raw_input.is_empty() && !state.cooked_input.is_full() {
        let Some(raw) = state.raw_input.pop() else {
            break;
        };
        let Some(byte) = map_input(state, raw) else {
            continue;
        };

        let canonical = state.is_canonical();
        if canonical && handle_flow_control(state, byte, &mut needs_flush) {
            continue;
        }
        if state.termios.local_mode.contains(LocalMode::ISIG) && handle_signal(state, byte) {
            continue;
        }
        if canonical && handle_editing(state, byte, &mut needs_flush) {
            continue;
        }

        if state.is_line_boundary(byte) {
            state.pending_lines += 1;
        }
        if state.termios.local_mode.contains(LocalMode::ECHO) {
            needs_flush |= echo(state, byte);
        }
        state.cooked_input.push(byte);
    }

    needs_flush
}

/// Apply input-mode translations, returning `None` if the byte is discarded.
fn map_input(state: &TtyState, mut byte: u8) -> Option<u8> {
    let input_mode = state.termios.input_mode;

    if input_mode.contains(InputMode::ISTRIP) {
        byte &= 0x7f;
    }

    byte = match byte {
        b'\r' if input_mode.contains(InputMode::IGNCR) => return None,
        b'\r' if input_mode.contains(InputMode::ICRNL) => b'\n',
        b'\n' if input_mode.contains(InputMode::INLCR) => b'\r',
        _ => byte,
    };

    if input_mode.contains(InputMode::IUCLC) {
        byte = byte.to_ascii_lowercase();
    }

    Some(byte)
}

/// Handle XON/XOFF flow control; returns `true` if the byte was consumed.
///
/// Resuming output sets `needs_flush` so the caller restarts the transmitter.
fn handle_flow_control(state: &mut TtyState, byte: u8, needs_flush: &mut bool) -> bool {
    if byte == state.termios.control_char(ControlChar::Stop) {
        state.stopped = true;
        true
    } else if byte == state.termios.control_char(ControlChar::Start) {
        state.stopped = false;
        *needs_flush = true;
        true
    } else {
        false
    }
}

/// Deliver INTR/QUIT signals to the foreground group; returns `true` if the
/// byte was consumed.
fn handle_signal(state: &TtyState, byte: u8) -> bool {
    let signal = if byte == state.termios.control_char(ControlChar::Intr) {
        Signal::Int
    } else if byte == state.termios.control_char(ControlChar::Quit) {
        Signal::Quit
    } else {
        return false;
    };

    signal_foreground_group(state.foreground_group, signal as u32);
    true
}

/// Apply canonical-mode ERASE/KILL editing; returns `true` if the byte was
/// consumed.
fn handle_editing(state: &mut TtyState, byte: u8, echoed: &mut bool) -> bool {
    let erase = state.termios.control_char(ControlChar::Erase);
    let kill = state.termios.control_char(ControlChar::Kill);

    if byte == kill {
        while erase_last(state, echoed) {}
        true
    } else if byte == erase || is_backspace_alias(byte, erase) {
        erase_last(state, echoed);
        true
    } else {
        false
    }
}

/// Erase the last editable character from the cooked queue, echoing the rub-out
/// if enabled. Returns `false` once the line start (or a committed boundary) is
/// reached.
fn erase_last(state: &mut TtyState, echoed: &mut bool) -> bool {
    match state.cooked_input.last() {
        Some(last) if !state.is_line_boundary(last) => {
            echo_erase(state, last, echoed);
            state.cooked_input.pop_last();
            true
        }
        _ => false,
    }
}

/// Treat both `^H` (0x08) and DEL (0x7f) as erase regardless of which one is
/// configured: real terminals disagree on which byte Backspace sends, so
/// accepting both lets the line editor work without per-terminal `stty` tweaks.
fn is_backspace_alias(byte: u8, erase: u8) -> bool {
    matches!((erase, byte), (0x7f, 0x08) | (0x08, 0x7f))
}

/// Echo the rub-out sequence for an erased character when ECHO is enabled.
fn echo_erase(state: &mut TtyState, erased: u8, echoed: &mut bool) {
    if !state.termios.local_mode.contains(LocalMode::ECHO) {
        return;
    }

    if state.termios.local_mode.contains(LocalMode::ECHOE) {
        // Terminals don't render a bare DEL, so rub out visually with
        // backspace-space-backspace. A character echoed as `^X` occupies two
        // glyphs, so its rub-out needs the sequence twice.
        let glyphs = if echoed_as_caret(erased) { 2 } else { 1 };
        for _ in 0..glyphs {
            push_erase_sequence(state);
        }
    } else {
        state.output.push(BACKSPACE);
    }
    *echoed = true;
}

/// Echo one input byte to the output queue; returns `true` if bytes were added.
fn echo(state: &mut TtyState, byte: u8) -> bool {
    match byte {
        b'\n' => {
            state.output.push(b'\n');
            state.output.push(b'\r');
            true
        }
        _ if is_control(byte) => {
            if state.termios.local_mode.contains(LocalMode::ECHOCTL) {
                state.output.push(b'^');
                state.output.push(byte + CARET_OFFSET);
                true
            } else {
                false
            }
        }
        _ => {
            state.output.push(byte);
            true
        }
    }
}

/// Push one backspace-space-backspace rub-out to the output queue.
fn push_erase_sequence(state: &mut TtyState) {
    state.output.push(BACKSPACE);
    state.output.push(b' ');
    state.output.push(BACKSPACE);
}

/// Whether `byte` is a C0 control character echoed in `^X` caret notation.
fn is_control(byte: u8) -> bool {
    byte < 0x20
}

/// Whether `byte`, when echoed, occupies two glyphs on screen.
///
/// Control characters render as `^X`; DEL also rubs out as a two-column cell,
/// so both need the erase sequence applied twice.
fn echoed_as_caret(byte: u8) -> bool {
    is_control(byte) || byte == DELETE
}

/// ASCII backspace.
const BACKSPACE: u8 = 0x08;

/// ASCII delete.
const DELETE: u8 = 0x7f;

/// Offset mapping a control code to its caret-notation letter (`^A` == 0x01 + 64).
const CARET_OFFSET: u8 = 64;
