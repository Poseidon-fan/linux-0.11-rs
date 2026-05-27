//! Screen rendering: viewport, status line, cursor positioning.
//!
//! The editor doesn't maintain a per-cell terminal mirror — it just
//! repaints the visible window from scratch every time a change happens.
//! For the size of buffers vi works on this is fine and saves us the
//! diff-tracking complexity. Output goes through one `String` so the
//! whole frame lands on the terminal in a single `write(2)`, avoiding
//! flicker.

use alloc::{format, string::String};

use crate::{buffer::Buffer, tty};

/// Hard-coded terminal dimensions.
///
/// Linux 0.11 has neither `TIOCGWINSZ` nor `SIGWINCH`, so we can't ask
/// the kernel what size the console actually is and we can't be notified
/// when it changes. 24×80 is the universal VT100 default — every console
/// we care about either matches exactly (text-mode VGA, serial xterm
/// over `-nographic`) or is larger, in which case we just paint into the
/// upper-left rectangle.
pub const ROWS: usize = 24;
pub const COLS: usize = 80;

/// Number of rows reserved for the buffer (the last row is the status /
/// command line).
pub const TEXT_ROWS: usize = ROWS - 1;

/// Scrolling viewport. `top` is the first buffer row visible; `left`
/// is the first byte column.
#[derive(Default)]
pub struct Viewport {
    pub top: usize,
    pub left: usize,
}

impl Viewport {
    /// Scrolls the viewport so the cursor at `(cur_row, cur_col)`
    /// stays inside the visible rectangle.
    pub fn track(&mut self, cur_row: usize, cur_col: usize) {
        if cur_row < self.top {
            self.top = cur_row;
        } else if cur_row >= self.top + TEXT_ROWS {
            self.top = cur_row + 1 - TEXT_ROWS;
        }
        if cur_col < self.left {
            self.left = cur_col;
        } else if cur_col >= self.left + COLS {
            self.left = cur_col + 1 - COLS;
        }
    }
}

/// Renders the whole frame: buffer text, tilde rows for any visible
/// area past the end of the buffer, and the status / command line.
/// `status` is the message shown bottom-left in normal mode; in
/// command-line mode the caller passes the active `:`/`/` line so the
/// user can see what they're typing.
pub fn draw(buf: &Buffer, view: &Viewport, status: &str) {
    let mut frame = String::with_capacity(ROWS * (COLS + 8));
    frame.push_str(tty::HIDE_CURSOR);
    frame.push_str(&tty::move_to(1, 1));

    // Text area.
    for screen_row in 0..TEXT_ROWS {
        let buf_row = view.top + screen_row;
        if buf_row < buf.line_count() {
            let line = buf.line(buf_row);
            let slice = visible_slice(line, view.left, COLS);
            frame.push_str(slice);
        } else {
            frame.push('~');
        }
        frame.push_str(tty::CLEAR_EOL);
        frame.push_str("\r\n");
    }

    // Status / command line on the last row.
    draw_status(&mut frame, buf, status);

    // Place the hardware cursor at the buffer cursor.
    let cur_row = buf.row.saturating_sub(view.top) + 1;
    let cur_col = buf.col.saturating_sub(view.left) + 1;
    frame.push_str(&tty::move_to(cur_row as u16, cur_col as u16));
    frame.push_str(tty::SHOW_CURSOR);

    tty::print_raw(&frame);
}

/// Renders the bottom line. Two flavours, both fit in `COLS`:
///
/// - normal mode: filename `|--|` flag + `row,col` indicator on the
///   right, then any transient `status` message
/// - command line: the `status` string is the literal line the user is
///   typing (already includes the leading `:` or `/`)
fn draw_status(frame: &mut String, buf: &Buffer, status: &str) {
    frame.push_str(&tty::move_to(ROWS as u16, 1));
    frame.push_str(tty::CLEAR_EOL);

    if status.starts_with(':') || status.starts_with('/') || status.starts_with('?') {
        // Command-line mode: paint as-is, cursor will land at end.
        frame.push_str(status);
        return;
    }

    frame.push_str(tty::REVERSE_ON);
    let left = format!(
        " {}{} ",
        buf.display_name(),
        if buf.dirty { " [+]" } else { "" }
    );
    let right = format!(" {},{} ", buf.row + 1, buf.col + 1);
    frame.push_str(&left);
    let used = visible_width(&left) + visible_width(&right);
    if COLS > used {
        for _ in 0..(COLS - used) {
            frame.push(' ');
        }
    }
    frame.push_str(&right);
    frame.push_str(tty::ATTR_RESET);

    if !status.is_empty() {
        frame.push_str(&tty::move_to(ROWS as u16, 1));
        frame.push_str(tty::CLEAR_EOL);
        frame.push_str(status);
    }
}

/// Returns the substring of `line` that fits in `[left, left+width)`,
/// expanding tabs to spaces along the way. We keep the math byte-based
/// — multibyte text won't render perfectly but it won't crash either.
fn visible_slice(line: &str, left: usize, width: usize) -> &str {
    let bytes = line.as_bytes();
    if left >= bytes.len() {
        return "";
    }
    let end = (left + width).min(bytes.len());
    // `line` is `&str`, indexing with byte ranges is safe because we
    // checked the bounds — but only for ASCII. For non-ASCII bytes the
    // slice could end mid-codepoint; clamp to the nearest char boundary.
    let mut e = end;
    while e > left && !line.is_char_boundary(e) {
        e -= 1;
    }
    &line[left..e]
}

fn visible_width(s: &str) -> usize {
    s.len()
}
