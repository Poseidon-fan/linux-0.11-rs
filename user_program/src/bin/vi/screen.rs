//! Screen rendering: viewport, status line, cursor positioning.
//!
//! The editor doesn't maintain a per-cell terminal mirror — it just
//! repaints the visible window from scratch every time a change happens.
//! For the size of buffers vi works on this is fine and saves us the
//! diff-tracking complexity. Output goes through one `String` so the
//! whole frame lands on the terminal in a single `write(2)`, avoiding
//! flicker.
//!
//! ## Buffer vs screen columns
//!
//! The buffer stores raw bytes; the terminal renders them with variable
//! widths — a literal `\t` expands to the next tab stop, control bytes
//! print as `^X`, and runs past column 80 wrap. The editor cursor lives
//! in **byte space** ([`Buffer::col`]) so motions, deletes, and reads
//! work without surprises, and the renderer translates to **screen
//! space** at draw time via [`byte_col_to_screen`]. Without that
//! translation, a Tab in the buffer would put the screen cursor one
//! column past the `\t` while the eye sees it eight columns later,
//! making subsequent edits look broken.

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

/// Tab stop spacing in screen columns.
pub const TAB_WIDTH: usize = 8;

/// Scrolling viewport. `top` is the first buffer row visible; `left`
/// is the first **screen** column visible (i.e. tab-expanded).
#[derive(Default)]
pub struct Viewport {
    pub top: usize,
    pub left: usize,
}

impl Viewport {
    /// Scrolls the viewport so the cursor at buffer position
    /// `(cur_row, cur_byte_col)` stays inside the visible rectangle.
    ///
    /// `cur_byte_col` is byte-based but we convert to screen columns
    /// inside so vertical scrolling stays in sync with what the user
    /// actually sees.
    pub fn track(&mut self, buf: &Buffer, cur_row: usize, cur_byte_col: usize) {
        if cur_row < self.top {
            self.top = cur_row;
        } else if cur_row >= self.top + TEXT_ROWS {
            self.top = cur_row + 1 - TEXT_ROWS;
        }
        let screen_col = byte_col_to_screen(buf.line(cur_row), cur_byte_col);
        if screen_col < self.left {
            self.left = screen_col;
        } else if screen_col >= self.left + COLS {
            self.left = screen_col + 1 - COLS;
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
    frame.push_str(&tty::move_to(1, 1));

    // Text area.
    for screen_row in 0..TEXT_ROWS {
        let buf_row = view.top + screen_row;
        if buf_row < buf.line_count() {
            render_line_into(&mut frame, buf.line(buf_row), view.left, COLS);
        } else {
            frame.push('~');
        }
        frame.push_str(tty::CLEAR_EOL);
        frame.push_str("\r\n");
    }

    // Status / command line on the last row.
    draw_status(&mut frame, buf, status);

    // Place the hardware cursor at the buffer cursor, translated to
    // screen columns so it lands where the eye expects after tabs and
    // control bytes have been expanded.
    let cur_row_screen = buf.row.saturating_sub(view.top) + 1;
    let cur_col_screen =
        byte_col_to_screen(buf.line(buf.row), buf.col).saturating_sub(view.left) + 1;
    frame.push_str(&tty::move_to(cur_row_screen as u16, cur_col_screen as u16));

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
    let used = left.len() + right.len();
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

// ---------------------------------------------------------------------------
// Byte ↔ screen column translation
// ---------------------------------------------------------------------------

/// Returns the screen-column position of byte `byte_col` in `line`,
/// after tabs have been expanded and control bytes rendered as `^X`.
/// `byte_col` may sit one past the last byte (the standard
/// insert-cursor convention) — we treat that as the position the next
/// character would occupy.
pub fn byte_col_to_screen(line: &str, byte_col: usize) -> usize {
    let mut screen = 0usize;
    for (i, &b) in line.as_bytes().iter().enumerate() {
        if i == byte_col {
            return screen;
        }
        screen += display_width_of(b, screen);
    }
    screen
}

/// Appends the rendered form of `line[byte_left..]` to `frame`, clipped
/// so the result occupies at most `cols` screen columns starting at
/// screen column `screen_left`. Tabs become spaces up to the next tab
/// stop, control bytes render as `^X` (caret-X), DEL renders as `^?`.
fn render_line_into(frame: &mut String, line: &str, screen_left: usize, cols: usize) {
    let mut screen = 0usize;
    let max_screen = screen_left + cols;
    for &b in line.as_bytes() {
        let cell_w = display_width_of(b, screen);
        let next_screen = screen + cell_w;

        // Only emit the part of this cell that overlaps the viewport.
        if next_screen > screen_left && screen < max_screen {
            emit_cell(frame, b, screen, screen_left, max_screen);
        }
        screen = next_screen;
        if screen >= max_screen {
            break;
        }
    }
}

/// Width in screen columns of byte `b` when rendered at screen column
/// `screen` (tabs need the column to figure out the next tab stop).
fn display_width_of(b: u8, screen: usize) -> usize {
    match b {
        b'\t' => TAB_WIDTH - (screen % TAB_WIDTH),
        0..=0x1f | 0x7f => 2, // `^X` notation
        _ => 1,
    }
}

/// Emits the visible portion of one logical cell (a single byte, but
/// possibly expanded to many screen columns by tab or `^X`).
fn emit_cell(frame: &mut String, b: u8, screen: usize, screen_left: usize, max_screen: usize) {
    let cell_w = display_width_of(b, screen);
    // Determine how many of this cell's columns intersect the viewport.
    let visible_from = screen.max(screen_left) - screen;
    let visible_to = (screen + cell_w).min(max_screen) - screen;

    match b {
        b'\t' => {
            for _ in visible_from..visible_to {
                frame.push(' ');
            }
        }
        0..=0x1f => {
            // Render as `^X` (X is `b ^ 0x40`).
            let glyphs = [b'^', b ^ 0x40];
            for g in &glyphs[visible_from..visible_to] {
                frame.push(*g as char);
            }
        }
        0x7f => {
            let glyphs = [b'^', b'?'];
            for g in &glyphs[visible_from..visible_to] {
                frame.push(*g as char);
            }
        }
        _ => frame.push(b as char),
    }
}
