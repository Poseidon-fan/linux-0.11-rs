//! N_TTY line discipline.
//!
//! Transforms raw input bytes (from hardware) into cooked input suitable for
//! canonical-mode reads. Handles input mapping, signal character detection,
//! flow control, line editing (erase/kill), echo, and line counting.
//!
//! Processing pipeline for each raw byte:
//!
//! ```text
//!  raw_rx.pop()
//!     │
//!     ▼
//!  Input mapping (CR↔NL, case folding)
//!     │
//!     ▼
//!  Flow control check (XOFF/XON)
//!     │
//!     ▼
//!  Signal character check (INTR/QUIT)
//!     │
//!     ▼
//!  Line editing (ERASE/KILL in canonical mode)
//!     │
//!     ▼
//!  Echo + enqueue to cooked_rx
//! ```

use user_lib::termios::*;

use super::Tty;
use super::TtyState;

pub struct LineDiscipline;

impl LineDiscipline {
    /// Drain `raw_rx`, process each byte through the line discipline, and push
    /// results into `cooked_rx`. Returns `true` if any echo data was written to
    /// `tx` (so the caller should flush output).
    pub fn process_raw_input(state: &mut TtyState) -> bool {
        let mut echoed = false;

        while !state.raw_rx.is_empty() && !state.cooked_rx.is_full() {
            let Some(mut c) = state.raw_rx.pop() else {
                break;
            };

            // --- Input mapping ---
            c = Self::map_input(state, c);
            if c == 0xff {
                continue; // IGNCR consumed the byte
            }

            // --- Canonical-mode flow control ---
            if state.termios.local_mode.contains(LocalMode::ICANON) {
                if c == state.termios.control_char(VSTOP) {
                    state.stopped = true;
                    continue;
                }
                if c == state.termios.control_char(VSTART) {
                    state.stopped = false;
                    continue;
                }
            }

            // --- Signal characters ---
            if state.termios.local_mode.contains(LocalMode::ISIG) && Self::check_signal(state, c) {
                continue;
            }

            // --- Canonical-mode line editing ---
            if state.termios.local_mode.contains(LocalMode::ICANON)
                && Self::handle_editing(state, c, &mut echoed)
            {
                continue;
            }

            // --- Line counting ---
            if c == b'\n' || c == state.termios.control_char(VEOF) {
                state.pending_lines += 1;
            }

            // --- Echo ---
            if state.termios.local_mode.contains(LocalMode::ECHO) {
                echoed |= Self::echo(state, c);
            }

            // --- Enqueue to cooked_rx ---
            state.cooked_rx.push(c);
        }

        echoed
    }

    /// Apply input-mode mappings (CR↔NL translation, case folding, parity stripping).
    /// Returns 0xff as a sentinel meaning "discard this byte".
    fn map_input(state: &TtyState, mut c: u8) -> u8 {
        if state.termios.input_mode.contains(InputMode::ISTRIP) {
            c &= 0x7f;
        }

        if c == b'\r' {
            if state.termios.input_mode.contains(InputMode::IGNCR) {
                return 0xff;
            }
            if state.termios.input_mode.contains(InputMode::ICRNL) {
                c = b'\n';
            }
        } else if c == b'\n' && state.termios.input_mode.contains(InputMode::INLCR) {
            c = b'\r';
        }

        if state.termios.input_mode.contains(InputMode::IUCLC) && c.is_ascii_uppercase() {
            c = c.to_ascii_lowercase();
        }

        c
    }

    /// Check if `c` is a signal character (INTR, QUIT). If so, deliver the
    /// signal and return `true` to discard the byte.
    fn check_signal(state: &TtyState, c: u8) -> bool {
        let intr_char = state.termios.control_char(VINTR);
        if c == intr_char {
            Tty::signal_foreground_group(
                state.foreground_group,
                1u32 << (user_lib::process::SIGINT - 1),
            );
            return true;
        }

        let quit_char = state.termios.control_char(VQUIT);
        if c == quit_char {
            Tty::signal_foreground_group(
                state.foreground_group,
                1u32 << (user_lib::process::SIGQUIT - 1),
            );
            return true;
        }

        false
    }

    /// Handle ERASE and KILL characters in canonical mode. Returns `true` if
    /// the byte was consumed by editing.
    fn handle_editing(state: &mut TtyState, c: u8, echoed: &mut bool) -> bool {
        let erase_char = state.termios.control_char(VERASE);
        let kill_char = state.termios.control_char(VKILL);
        let eof_char = state.termios.control_char(VEOF);

        if c == kill_char {
            // Delete the entire current line from cooked_rx.
            loop {
                if state.cooked_rx.is_empty() {
                    break;
                }
                let Some(last) = state.cooked_rx.peek_last() else {
                    break;
                };
                if last == b'\n' || last == eof_char {
                    break;
                }
                if state.termios.local_mode.contains(LocalMode::ECHO) {
                    // Echo DEL for each removed character.
                    if last < 32 {
                        state.tx.push(0x7f); // rubout the '^' caret
                    }
                    state.tx.push(0x7f);
                    *echoed = true;
                }
                state.cooked_rx.unpush();
            }
            return true;
        }

        if c == erase_char {
            if state.cooked_rx.is_empty() {
                return true;
            }
            let Some(last) = state.cooked_rx.peek_last() else {
                return true;
            };
            if last == b'\n' || last == eof_char {
                return true;
            }
            if state.termios.local_mode.contains(LocalMode::ECHO) {
                if last < 32 {
                    state.tx.push(0x7f);
                }
                state.tx.push(0x7f);
                *echoed = true;
            }
            state.cooked_rx.unpush();
            return true;
        }

        false
    }

    /// Echo a character to the `tx` queue. Returns `true` if anything was
    /// written.
    fn echo(state: &mut TtyState, c: u8) -> bool {
        if c == b'\n' {
            state.tx.push(b'\n');
            state.tx.push(b'\r');
            return true;
        }

        if c < 32 {
            if state.termios.local_mode.contains(LocalMode::ECHOCTL) {
                state.tx.push(b'^');
                state.tx.push(c + 64);
                return true;
            }
            return false;
        }

        state.tx.push(c);
        true
    }
}
