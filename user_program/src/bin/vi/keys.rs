//! Decoded keystrokes.
//!
//! Bytes read from the TTY get fed through [`Reader::next`] which
//! collapses ANSI escape sequences (arrow keys, Home, PgUp, function
//! keys we care about) into a single [`Key`] value. Anything we don't
//! recognise becomes [`Key::Unknown`] so the editor can ignore it
//! without aborting on garbage input.

/// One logical keystroke.
///
/// We model only the keys the editor actually reacts to. Adding more is
/// just one match arm in [`Reader::decode_escape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Printable ASCII byte (including space and `~`).
    Char(u8),
    /// `Ctrl-X` for some printable `X`. Carries the lowercased letter.
    Ctrl(u8),
    /// Enter (`\r` or `\n`).
    Enter,
    /// Backspace (DEL `0x7f` or `^H` `0x08`).
    Backspace,
    /// Tab.
    Tab,
    /// Lone Escape (the user pressed Esc with no follow-up byte before
    /// the read timeout — or stdin closed mid-sequence).
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// Delete (the `Delete` key, distinct from Backspace).
    Delete,
    /// Anything we don't have a name for. Carries the originating byte
    /// (or `0x1b` for a malformed escape sequence) for diagnostics.
    Unknown(u8),
}

/// Stateful reader. Sits on top of a byte-at-a-time input source —
/// typically `crate::tty::read_byte`.
pub struct Reader<F: FnMut() -> Option<u8>> {
    read: F,
    /// One-byte pushback used when escape-sequence decoding has to
    /// "un-read" a byte that doesn't belong to the sequence.
    pending: Option<u8>,
}

impl<F: FnMut() -> Option<u8>> Reader<F> {
    pub fn new(read: F) -> Self {
        Self {
            read,
            pending: None,
        }
    }

    fn read_one(&mut self) -> Option<u8> {
        self.pending.take().or_else(&mut self.read)
    }

    /// Blocks until one logical key is available. Returns `None` when
    /// the underlying source hits EOF.
    pub fn next(&mut self) -> Option<Key> {
        let b = self.read_one()?;
        Some(match b {
            b'\r' | b'\n' => Key::Enter,
            b'\t' => Key::Tab,
            0x7f | 0x08 => Key::Backspace,
            0x1b => self.decode_escape(),
            // Control chars `^A` (0x01) through `^Z` (0x1a), excluding
            // tab/enter/backspace/escape already handled above.
            c @ 0x01..=0x1a => Key::Ctrl(c + b'a' - 1),
            // Printable ASCII.
            c @ 0x20..=0x7e => Key::Char(c),
            other => Key::Unknown(other),
        })
    }

    /// Decodes the bytes that follow an ESC. We support the small set of
    /// CSI / SS3 sequences a VT100-compatible terminal sends for the
    /// arrow keys, navigation cluster, and Delete. Anything else turns
    /// into `Key::Esc` (lone Escape) plus pushback for the next byte.
    fn decode_escape(&mut self) -> Key {
        let Some(b1) = self.read_one() else {
            return Key::Esc;
        };
        if b1 != b'[' && b1 != b'O' {
            // Real "Esc then next key" — give the byte back so the next
            // `next()` sees it, and report Esc itself first.
            self.pending = Some(b1);
            return Key::Esc;
        }
        let Some(b2) = self.read_one() else {
            return Key::Esc;
        };
        match b2 {
            b'A' => Key::Up,
            b'B' => Key::Down,
            b'C' => Key::Right,
            b'D' => Key::Left,
            b'H' => Key::Home,
            b'F' => Key::End,
            // Numeric sequences like `ESC [ 3 ~` (Delete), `ESC [ 5 ~`
            // (PageUp), `ESC [ 6 ~` (PageDown). The trailing `~` (or any
            // alphabetic terminator) ends the sequence.
            d @ b'0'..=b'9' => {
                let mut number = String::new();
                number.push(d as char);
                loop {
                    let Some(b) = self.read_one() else {
                        return Key::Unknown(0x1b);
                    };
                    if b == b'~' || b.is_ascii_alphabetic() {
                        return match number.as_str() {
                            "1" | "7" => Key::Home,
                            "3" => Key::Delete,
                            "4" | "8" => Key::End,
                            "5" => Key::PageUp,
                            "6" => Key::PageDown,
                            _ => Key::Unknown(0x1b),
                        };
                    }
                    if !b.is_ascii_digit() && b != b';' {
                        return Key::Unknown(0x1b);
                    }
                    number.push(b as char);
                }
            }
            _ => Key::Unknown(0x1b),
        }
    }
}

use alloc::string::String;
