//! The editing buffer: lines, cursor, dirty flag, file I/O.
//!
//! We model the buffer as `Vec<String>` (one entry per line, no trailing
//! `\n` stored). For an editor that targets ~megabyte files, this is
//! plenty — a rope would be over-engineering. Every edit goes through
//! this module, which keeps the dirty flag up to date and the cursor in
//! bounds.

use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

use user_lib::{fs, io::Result, path::PathBuf};

/// 1-based column for screen rendering. Mostly internal — file-facing
/// code uses raw byte offsets.
pub type Col = usize;
/// 0-based line index.
pub type Row = usize;

/// One open file's worth of state.
pub struct Buffer {
    /// File path the buffer is associated with. `None` when started
    /// from a blank slate (`vi` with no argument).
    pub path: Option<PathBuf>,
    /// Line contents. Always at least one entry (an empty document is
    /// `vec![""]`, not `vec![]`).
    pub lines: Vec<String>,
    /// Cursor row, 0-based.
    pub row: Row,
    /// Cursor column, 0-based byte offset within `lines[row]`. We let
    /// it transiently equal `lines[row].len()` (one past the last
    /// character) — useful in insert mode and when sitting at end of
    /// line.
    pub col: Col,
    /// True when the buffer has been modified since the last save.
    pub dirty: bool,
    /// Set when the buffer was opened from a path that didn't exist.
    pub new_file: bool,
}

impl Buffer {
    /// Builds a blank, unnamed buffer.
    pub fn empty() -> Self {
        Self {
            path: None,
            lines: vec![String::new()],
            row: 0,
            col: 0,
            dirty: false,
            new_file: true,
        }
    }

    /// Loads `path` from disk. A missing file isn't an error — it
    /// becomes a blank buffer associated with `path`, ready for `:w`,
    /// matching vi's behaviour.
    pub fn load(path: &str) -> Result<Self> {
        let pb = PathBuf::from(path);
        match fs::read_to_string(path) {
            Ok(text) => {
                let lines = split_keep_empty(&text);
                Ok(Self {
                    path: Some(pb),
                    lines,
                    row: 0,
                    col: 0,
                    dirty: false,
                    new_file: false,
                })
            }
            Err(err) if err.kind() == user_lib::io::ErrorKind::NotFound => Ok(Self {
                path: Some(pb),
                lines: vec![String::new()],
                row: 0,
                col: 0,
                dirty: false,
                new_file: true,
            }),
            Err(err) => Err(err),
        }
    }

    /// Writes the buffer back to its associated path. Clears the dirty
    /// flag on success.
    pub fn save(&mut self) -> Result<usize> {
        let Some(path) = self.path.clone() else {
            return Err(user_lib::io::Error::new(
                user_lib::io::ErrorKind::InvalidInput,
                "no file name",
            ));
        };
        self.save_as(path.as_path().as_str())
    }

    /// Writes the buffer to `path`, then makes that the buffer's
    /// associated path.
    pub fn save_as(&mut self, path: &str) -> Result<usize> {
        let mut blob: String = self.lines.join("\n");
        // POSIX text files end with a newline; vi preserves that
        // convention on write.
        blob.push('\n');
        let bytes = blob.into_bytes();
        let len = bytes.len();
        fs::write(path, &bytes)?;
        self.path = Some(PathBuf::from(path));
        self.dirty = false;
        self.new_file = false;
        Ok(len)
    }

    /// Returns the file name to show in the status line.
    pub fn display_name(&self) -> &str {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .unwrap_or("[No Name]")
    }

    /// Reads the current line, never panicking.
    pub fn line(&self, row: Row) -> &str {
        self.lines.get(row).map(String::as_str).unwrap_or("")
    }

    /// Number of lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Clamps the cursor column to a legal position for the given line.
    /// `allow_one_past` lets insert / append leave the cursor on the
    /// virtual column just after the last character (where typing
    /// extends the line); normal mode clamps to the last character.
    pub fn clamp_col(&mut self, allow_one_past: bool) {
        let len = self.lines[self.row].len();
        let max = if allow_one_past || len == 0 {
            len
        } else {
            len - 1
        };
        if self.col > max {
            self.col = max;
        }
    }

    // -----------------------------------------------------------------
    // Mutations
    // -----------------------------------------------------------------

    /// Inserts a single byte (ASCII assumed — multibyte chars get
    /// handled at the caller side, but we don't care about codepoint
    /// boundaries since we only ever splice at the cursor.) Advances
    /// the cursor one column.
    pub fn insert_byte(&mut self, byte: u8) {
        let line = &mut self.lines[self.row];
        line.insert(self.col, byte as char);
        self.col += 1;
        self.dirty = true;
    }

    /// Splits the current line at the cursor, leaving the cursor at
    /// column 0 of the new line. Used for Enter in insert mode and `o`/`O`.
    pub fn split_line(&mut self) {
        let line = &mut self.lines[self.row];
        let tail = line.split_off(self.col);
        self.row += 1;
        self.col = 0;
        self.lines.insert(self.row, tail);
        self.dirty = true;
    }

    /// Deletes the byte before the cursor. At column 0 it joins the
    /// current line onto the previous one. Backspace in insert mode.
    pub fn backspace(&mut self) {
        if self.col == 0 {
            if self.row == 0 {
                return;
            }
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].len();
            self.lines[self.row].push_str(&cur);
        } else {
            let line = &mut self.lines[self.row];
            self.col -= 1;
            line.remove(self.col);
        }
        self.dirty = true;
    }

    /// Deletes the byte under the cursor (vi `x`). Does nothing on an
    /// empty line.
    pub fn delete_at_cursor(&mut self) -> Option<char> {
        let line = &mut self.lines[self.row];
        if line.is_empty() || self.col >= line.len() {
            return None;
        }
        let ch = line.remove(self.col);
        if self.col >= line.len() && self.col > 0 {
            self.col -= 1;
        }
        self.dirty = true;
        Some(ch)
    }

    /// Deletes the whole current line. The deleted text (without
    /// trailing newline) is returned so callers can populate the
    /// yank register.
    pub fn delete_line(&mut self) -> String {
        let removed = if self.lines.len() == 1 {
            core::mem::take(&mut self.lines[0])
        } else {
            self.lines.remove(self.row)
        };
        if self.row >= self.lines.len() {
            self.row = self.lines.len() - 1;
        }
        self.col = 0;
        self.dirty = true;
        removed
    }

    /// Replaces the current line with the empty string, keeping the
    /// line slot in the buffer. Used by `cc`. Returns the old contents
    /// so the caller can store them in a register.
    pub fn clear_current_line(&mut self) -> String {
        let row = self.row;
        let old = core::mem::take(&mut self.lines[row]);
        self.col = 0;
        self.dirty = true;
        old
    }

    /// Inserts a fresh empty line below the cursor and moves the cursor
    /// to its column 0 — the `o` command.
    pub fn open_below(&mut self) {
        self.row += 1;
        self.lines.insert(self.row, String::new());
        self.col = 0;
        self.dirty = true;
    }

    /// Inserts a fresh empty line above the cursor — the `O` command.
    pub fn open_above(&mut self) {
        self.lines.insert(self.row, String::new());
        self.col = 0;
        self.dirty = true;
    }

    /// Replaces the byte under the cursor with `byte` (vi `r`).
    pub fn replace_at_cursor(&mut self, byte: u8) {
        let line = &mut self.lines[self.row];
        if self.col >= line.len() {
            return;
        }
        // String::remove + insert handles UTF-8 boundaries safely.
        line.remove(self.col);
        line.insert(self.col, byte as char);
        self.dirty = true;
    }
}

/// Splits `text` into lines, preserving every line including the empty
/// tail after a trailing `\n`. Mirrors what `str::lines` does **except**
/// that we keep an empty last entry so a file ending with `\n` doesn't
/// look the same as one that doesn't.
fn split_keep_empty(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut out: Vec<String> = text.split('\n').map(ToString::to_string).collect();
    // `str::split('\n')` always produces N+1 entries when there are N
    // newlines — for a file that ends with `\n`, the trailing entry is
    // empty and we want to drop it so the buffer doesn't show a phantom
    // blank line at the bottom.
    if out.last().is_some_and(String::is_empty) {
        out.pop();
        if out.is_empty() {
            out.push(String::new());
        }
    }
    out
}
