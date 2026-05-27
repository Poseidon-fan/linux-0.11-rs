//! Edit operations triggered from normal mode.
//!
//! Each public function here mutates the buffer, optionally returning
//! text to populate the yank register. Snapshotting for undo happens at
//! the caller side, before we get here, so individual functions can
//! stay focused on the edit itself.

use alloc::string::{String, ToString};

use crate::{buffer::Buffer, motion};

/// `x` — delete one character under the cursor, returning it for the
/// yank register.
pub fn delete_char(buf: &mut Buffer) -> Option<String> {
    buf.delete_at_cursor().map(|c| c.to_string())
}

/// `X` — delete one character before the cursor.
pub fn backspace_char(buf: &mut Buffer) -> Option<String> {
    if buf.col == 0 {
        return None;
    }
    buf.col -= 1;
    buf.delete_at_cursor().map(|c| c.to_string())
}

/// `dd` — delete the current line.
pub fn delete_line(buf: &mut Buffer) -> String {
    buf.delete_line()
}

/// `yy` — copy the current line.
pub fn yank_line(buf: &Buffer) -> String {
    buf.line(buf.row).to_string()
}

/// `p` — paste after the cursor / line.
///
/// vi distinguishes line-wise from char-wise yanks: a register that
/// contains a trailing newline pastes as a fresh line below the cursor,
/// otherwise it pastes inline after the cursor. We mark line-wise text
/// in the register by appending `'\n'` on `yy` / `dd` and stripping it
/// here.
pub fn paste_after(buf: &mut Buffer, register: &str) {
    if let Some(stripped) = register.strip_suffix('\n') {
        // Line-wise paste.
        let row = buf.row + 1;
        for (i, line) in stripped.split('\n').enumerate() {
            buf.lines.insert(row + i, line.to_string());
        }
        buf.row = row;
        buf.col = 0;
        buf.dirty = true;
    } else {
        // Char-wise paste after the cursor.
        let insert_at = (buf.col + 1).min(buf.lines[buf.row].len());
        let line = &mut buf.lines[buf.row];
        line.insert_str(insert_at, register);
        buf.col = insert_at + register.len().saturating_sub(1);
        buf.dirty = true;
    }
}

/// `P` — paste before the cursor / line.
pub fn paste_before(buf: &mut Buffer, register: &str) {
    if let Some(stripped) = register.strip_suffix('\n') {
        let row = buf.row;
        for (i, line) in stripped.split('\n').enumerate() {
            buf.lines.insert(row + i, line.to_string());
        }
        buf.row = row;
        buf.col = 0;
        buf.dirty = true;
    } else {
        let line = &mut buf.lines[buf.row];
        line.insert_str(buf.col, register);
        // Cursor stays where it was (now at the start of the inserted
        // text), matching vi's `P` semantics.
        buf.dirty = true;
    }
}

/// `dw` — delete from the cursor up to (but not including) the start
/// of the next word, on the current line.
pub fn delete_word(buf: &mut Buffer) -> String {
    let start = buf.col;
    let row = buf.row;
    motion::word_next(buf);
    let end = if buf.row == row {
        buf.col
    } else {
        buf.lines[row].len()
    };
    buf.row = row;
    buf.col = start;
    let line = &mut buf.lines[row];
    let removed: String = line.drain(start..end).collect();
    buf.dirty = true;
    if buf.col >= line.len() && buf.col > 0 {
        buf.col = line.len().saturating_sub(1);
    }
    removed
}

/// `D` — delete from cursor to end of line.
pub fn delete_to_eol(buf: &mut Buffer) -> String {
    let row = buf.row;
    let start = buf.col;
    let line = &mut buf.lines[row];
    let removed: String = line.drain(start..).collect();
    if buf.col > 0 && buf.col >= buf.lines[row].len() {
        buf.col = buf.lines[row].len().saturating_sub(1);
    }
    buf.dirty = true;
    removed
}

/// `J` — join the next line onto the current one, separated by a single
/// space. Trailing whitespace on the upper line and leading whitespace
/// on the lower line both collapse.
pub fn join_lines(buf: &mut Buffer) {
    if buf.row + 1 >= buf.lines.len() {
        return;
    }
    let next = buf.lines.remove(buf.row + 1);
    let trimmed = next.trim_start();
    let line = &mut buf.lines[buf.row];
    let join_col = line.len();
    if !line.is_empty() && !line.ends_with(' ') && !trimmed.is_empty() {
        line.push(' ');
    }
    line.push_str(trimmed);
    buf.col = join_col;
    buf.dirty = true;
}

/// Helper for callers that want to repeat an action `count` times.
pub fn repeat<F: FnMut()>(count: usize, mut f: F) {
    for _ in 0..count.max(1) {
        f();
    }
}

/// Drains the trailing word from a `Vec<u8>` insert buffer — used by
/// `Ctrl-W` in insert mode.
pub fn drop_word_left(buf: &mut Buffer) {
    while buf.col > 0 && buf.lines[buf.row].as_bytes().get(buf.col - 1).copied() == Some(b' ') {
        buf.backspace();
    }
    while buf.col > 0
        && buf.lines[buf.row]
            .as_bytes()
            .get(buf.col - 1)
            .copied()
            .is_some_and(|b| b != b' ')
    {
        buf.backspace();
    }
}
