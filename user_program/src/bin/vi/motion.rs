//! Normal-mode motions.
//!
//! Each motion takes the buffer and applies one cursor move. Motions
//! that delete or yank text (`dd`, `dw`, …) live in
//! [`crate::edit`] — this module is strictly about moving the cursor.

use crate::buffer::Buffer;

/// Moves left by one column, stopping at column 0.
pub fn left(buf: &mut Buffer) {
    if buf.col > 0 {
        buf.col -= 1;
    }
}

/// Moves right by one column. In normal mode the cursor stays on the
/// last printable character, never one past.
pub fn right(buf: &mut Buffer) {
    let len = buf.lines[buf.row].len();
    if len > 0 && buf.col + 1 < len {
        buf.col += 1;
    }
}

/// Moves up one line, preserving column where possible.
pub fn up(buf: &mut Buffer) {
    if buf.row > 0 {
        buf.row -= 1;
        buf.clamp_col(false);
    }
}

/// Moves down one line.
pub fn down(buf: &mut Buffer) {
    if buf.row + 1 < buf.lines.len() {
        buf.row += 1;
        buf.clamp_col(false);
    }
}

/// `0` — go to column 0.
pub fn line_start(buf: &mut Buffer) {
    buf.col = 0;
}

/// `^` — first non-whitespace column.
pub fn first_non_blank(buf: &mut Buffer) {
    let line = buf.lines[buf.row].as_bytes();
    let first = line
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .unwrap_or(0);
    buf.col = first;
}

/// `$` — last character on the line.
pub fn line_end(buf: &mut Buffer) {
    let len = buf.lines[buf.row].len();
    buf.col = if len == 0 { 0 } else { len - 1 };
}

/// `gg` — top of buffer, first non-blank.
pub fn buf_start(buf: &mut Buffer) {
    buf.row = 0;
    first_non_blank(buf);
}

/// `G` — bottom of buffer (or to row `count - 1` if a count was given;
/// the caller handles that detail).
pub fn buf_end(buf: &mut Buffer) {
    buf.row = buf.lines.len() - 1;
    first_non_blank(buf);
}

/// `w` — next word start. A word boundary is the run break between
/// "word characters" (`[A-Za-z0-9_]`) and everything else, with
/// whitespace skipped over.
pub fn word_next(buf: &mut Buffer) {
    let (mut r, mut c) = (buf.row, buf.col);
    let start_class = char_class(byte_at(buf, r, c));
    advance_while(buf, &mut r, &mut c, |b| char_class(b) == start_class);
    advance_while(buf, &mut r, &mut c, |b| char_class(b) == CharClass::Space);
    buf.row = r;
    buf.col = c;
}

/// `b` — previous word start.
pub fn word_back(buf: &mut Buffer) {
    let (mut r, mut c) = (buf.row, buf.col);
    retreat_one(buf, &mut r, &mut c);
    retreat_while(buf, &mut r, &mut c, |b| char_class(b) == CharClass::Space);
    let class = char_class(byte_at(buf, r, c));
    while r > 0 || c > 0 {
        let mut tr = r;
        let mut tc = c;
        retreat_one(buf, &mut tr, &mut tc);
        if char_class(byte_at(buf, tr, tc)) != class {
            break;
        }
        r = tr;
        c = tc;
    }
    buf.row = r;
    buf.col = c;
}

/// `e` — end of current / next word.
pub fn word_end(buf: &mut Buffer) {
    let (mut r, mut c) = (buf.row, buf.col);
    advance_one(buf, &mut r, &mut c);
    advance_while(buf, &mut r, &mut c, |b| char_class(b) == CharClass::Space);
    let class = char_class(byte_at(buf, r, c));
    while next_is(buf, r, c, |b| char_class(b) == class) {
        advance_one(buf, &mut r, &mut c);
    }
    buf.row = r;
    buf.col = c;
}

/// `f X` — to the next occurrence of `target` on the current line.
/// Returns `true` if the cursor moved.
pub fn find_forward(buf: &mut Buffer, target: u8) -> bool {
    let line = buf.lines[buf.row].as_bytes();
    if let Some(pos) = line
        .iter()
        .enumerate()
        .skip(buf.col + 1)
        .find(|(_, b)| **b == target)
        .map(|(i, _)| i)
    {
        buf.col = pos;
        true
    } else {
        false
    }
}

/// `F X` — to the previous occurrence on the current line.
pub fn find_backward(buf: &mut Buffer, target: u8) -> bool {
    let line = buf.lines[buf.row].as_bytes();
    if let Some(pos) = line[..buf.col].iter().rposition(|b| *b == target) {
        buf.col = pos;
        true
    } else {
        false
    }
}

/// `%` — to the matching bracket on the current line. Supports
/// `()/[]/{}` only; non-bracket targets just no-op.
pub fn matching_bracket(buf: &mut Buffer) {
    let line = buf.lines[buf.row].as_bytes();
    if buf.col >= line.len() {
        return;
    }
    let (open, close, forward): (u8, u8, bool) = match line[buf.col] {
        b'(' => (b'(', b')', true),
        b')' => (b'(', b')', false),
        b'[' => (b'[', b']', true),
        b']' => (b'[', b']', false),
        b'{' => (b'{', b'}', true),
        b'}' => (b'{', b'}', false),
        _ => return,
    };
    let mut depth = 1i32;
    if forward {
        for (i, b) in line.iter().enumerate().skip(buf.col + 1) {
            if *b == open {
                depth += 1;
            } else if *b == close {
                depth -= 1;
                if depth == 0 {
                    buf.col = i;
                    return;
                }
            }
        }
    } else {
        for i in (0..buf.col).rev() {
            let b = line[i];
            if b == close {
                depth += 1;
            } else if b == open {
                depth -= 1;
                if depth == 0 {
                    buf.col = i;
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Punct,
    Space,
}

fn char_class(b: u8) -> CharClass {
    match b {
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => CharClass::Word,
        b' ' | b'\t' => CharClass::Space,
        _ => CharClass::Punct,
    }
}

fn byte_at(buf: &Buffer, r: usize, c: usize) -> u8 {
    let line = buf.line(r).as_bytes();
    line.get(c).copied().unwrap_or(b' ')
}

fn advance_one(buf: &Buffer, r: &mut usize, c: &mut usize) {
    let len = buf.line(*r).len();
    if *c + 1 < len {
        *c += 1;
    } else if *r + 1 < buf.line_count() {
        *r += 1;
        *c = 0;
    } else {
        *c = len.saturating_sub(1);
    }
}

fn retreat_one(buf: &Buffer, r: &mut usize, c: &mut usize) {
    if *c > 0 {
        *c -= 1;
    } else if *r > 0 {
        *r -= 1;
        *c = buf.line(*r).len().saturating_sub(1);
    }
}

fn advance_while<F: Fn(u8) -> bool>(buf: &Buffer, r: &mut usize, c: &mut usize, pred: F) {
    while pred(byte_at(buf, *r, *c)) {
        let (pr, pc) = (*r, *c);
        advance_one(buf, r, c);
        if (*r, *c) == (pr, pc) {
            break;
        }
    }
}

fn retreat_while<F: Fn(u8) -> bool>(buf: &Buffer, r: &mut usize, c: &mut usize, pred: F) {
    while pred(byte_at(buf, *r, *c)) {
        if *r == 0 && *c == 0 {
            break;
        }
        let (pr, pc) = (*r, *c);
        retreat_one(buf, r, c);
        if (*r, *c) == (pr, pc) {
            break;
        }
    }
}

fn next_is<F: Fn(u8) -> bool>(buf: &Buffer, r: usize, c: usize, pred: F) -> bool {
    let mut nr = r;
    let mut nc = c;
    advance_one(buf, &mut nr, &mut nc);
    if (nr, nc) == (r, c) {
        return false;
    }
    pred(byte_at(buf, nr, nc))
}
