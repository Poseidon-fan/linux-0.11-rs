//! N_TTY line discipline.
//!
//! This module converts raw hardware input into cooked bytes consumed by
//! `read()`. It applies input mappings, canonical editing, signal characters,
//! flow control, echo, and complete-line accounting.

use user_lib::syscall::{
    signal::Signal,
    tty::{ControlChar, InputMode, LocalMode},
};

use super::{TtyState, signal_foreground_group};

/// Drain raw input through the line discipline.
///
/// Returns `true` when echo bytes were appended to the output queue.
pub fn process_raw_input(state: &mut TtyState) -> bool {
    let mut echoed = false;

    while !state.raw_input.is_empty() && !state.cooked_input.is_full() {
        let Some(byte) = state.raw_input.pop() else {
            break;
        };

        let byte = match map_input(state, byte) {
            MappedInput::Byte(byte) => byte,
            MappedInput::Discard => continue,
        };

        if state.termios.local_mode.contains(LocalMode::ICANON) && handle_flow_control(state, byte)
        {
            continue;
        }

        if state.termios.local_mode.contains(LocalMode::ISIG) && check_signal(state, byte) {
            continue;
        }

        if state.termios.local_mode.contains(LocalMode::ICANON)
            && handle_editing(state, byte, &mut echoed)
        {
            continue;
        }

        if byte == b'\n' || byte == state.termios.control_char(ControlChar::Eof) {
            state.pending_lines += 1;
        }

        if state.termios.local_mode.contains(LocalMode::ECHO) {
            echoed |= echo(state, byte);
        }

        state.cooked_input.push(byte);
    }

    echoed
}

/// Result of mapping one input byte through the input-mode flags.
enum MappedInput {
    /// Byte to continue processing.
    Byte(u8),
    /// Byte was consumed and should be dropped.
    Discard,
}

/// Apply input-mode translations (strip, CR/NL mapping, case folding).
fn map_input(state: &TtyState, mut byte: u8) -> MappedInput {
    if state.termios.input_mode.contains(InputMode::ISTRIP) {
        byte &= 0x7f;
    }

    if byte == b'\r' {
        if state.termios.input_mode.contains(InputMode::IGNCR) {
            return MappedInput::Discard;
        }
        if state.termios.input_mode.contains(InputMode::ICRNL) {
            byte = b'\n';
        }
    } else if byte == b'\n' && state.termios.input_mode.contains(InputMode::INLCR) {
        byte = b'\r';
    }

    if state.termios.input_mode.contains(InputMode::IUCLC) && byte.is_ascii_uppercase() {
        byte = byte.to_ascii_lowercase();
    }

    MappedInput::Byte(byte)
}

/// Handle XON/XOFF flow control; returns `true` if the byte was consumed.
fn handle_flow_control(state: &mut TtyState, byte: u8) -> bool {
    if byte == state.termios.control_char(ControlChar::Stop) {
        state.stopped = true;
        return true;
    }

    if byte == state.termios.control_char(ControlChar::Start) {
        state.stopped = false;
        return true;
    }

    false
}

/// Deliver INTR/QUIT signals; returns `true` if the byte was consumed.
fn check_signal(state: &TtyState, byte: u8) -> bool {
    if byte == state.termios.control_char(ControlChar::Intr) {
        signal_foreground_group(state.foreground_group, 1u32 << (Signal::Int as u32 - 1));
        return true;
    }

    if byte == state.termios.control_char(ControlChar::Quit) {
        signal_foreground_group(state.foreground_group, 1u32 << (Signal::Quit as u32 - 1));
        return true;
    }

    false
}

/// Apply canonical-mode ERASE/KILL editing; returns `true` if consumed.
fn handle_editing(state: &mut TtyState, byte: u8, echoed: &mut bool) -> bool {
    let erase_char = state.termios.control_char(ControlChar::Erase);
    let kill_char = state.termios.control_char(ControlChar::Kill);
    let eof_char = state.termios.control_char(ControlChar::Eof);

    if byte == kill_char {
        while let Some(last) = state.cooked_input.peek_last() {
            if last == b'\n' || last == eof_char {
                break;
            }
            echo_erase(state, last, echoed);
            state.cooked_input.unpush();
        }
        return true;
    }

    if byte == erase_char || is_erase_compat(byte, erase_char) {
        let Some(last) = state.cooked_input.peek_last() else {
            return true;
        };
        if last == b'\n' || last == eof_char {
            return true;
        }
        echo_erase(state, last, echoed);
        state.cooked_input.unpush();
        return true;
    }

    false
}

/// Treats both `^H` (0x08) and DEL (0x7f) as erase regardless of which one
/// is configured. Real terminals disagree on which byte the Backspace key
/// sends, so accepting both lets the line editor work without per-terminal
/// `stty` tweaking.
fn is_erase_compat(byte: u8, erase_char: u8) -> bool {
    matches!((erase_char, byte), (0x7f, 0x08) | (0x08, 0x7f))
}

/// Echo the rub-out sequence for an erased character when ECHO is enabled.
fn echo_erase(state: &mut TtyState, last: u8, echoed: &mut bool) {
    if !state.termios.local_mode.contains(LocalMode::ECHO) {
        return;
    }
    // ECHOE: rub out the prior glyph instead of just printing a DEL byte.
    // Terminals don't render `\x7f`, so the standard erase sequence is
    // `\b \b` (back up, write space, back up again). Control characters
    // were originally echoed as `^X` — two glyphs — so they need the
    // sequence twice.
    if state.termios.local_mode.contains(LocalMode::ECHOE) {
        let reps = if last < 32 || last == 0x7f { 2 } else { 1 };
        for _ in 0..reps {
            state.output.push(0x08);
            state.output.push(b' ');
            state.output.push(0x08);
        }
    } else {
        state.output.push(0x08);
    }
    *echoed = true;
}

/// Echo one input byte to the output queue; returns `true` if bytes were added.
fn echo(state: &mut TtyState, byte: u8) -> bool {
    if byte == b'\n' {
        state.output.push(b'\n');
        state.output.push(b'\r');
        return true;
    }

    if byte < 32 {
        if state.termios.local_mode.contains(LocalMode::ECHOCTL) {
            state.output.push(b'^');
            state.output.push(byte + 64);
            return true;
        }
        return false;
    }

    state.output.push(byte);
    true
}
