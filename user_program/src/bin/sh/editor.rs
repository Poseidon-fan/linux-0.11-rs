//! Interactive line editor: prompt, history, Tab completion.
//!
//! Driven from [`crate::run_interactive`] in place of a plain
//! [`Stdin::read_line`] call. The shell pre-configures the TTY into raw
//! mode (no `ICANON`, no `ECHO`) before entering [`Editor::read_line`] and
//! restores the previous settings on return; while inside the editor, the
//! shell owns every byte of input and is responsible for echoing it back
//! to the terminal.
//!
//! ## Supported keys
//!
//! - Printable bytes — inserted at the cursor.
//! - `Enter` (`\r` / `\n`) — submit the current line.
//! - `Backspace` (`0x08` / `0x7f`) — delete the byte before the cursor.
//! - `Ctrl-D` on an empty line — return EOF; otherwise ignored.
//! - `Ctrl-C` — abandon the current line, start fresh on the next one.
//! - `Ctrl-A` / `Ctrl-E` — move cursor to start / end of line.
//! - `Ctrl-B` / `Ctrl-F` — move cursor one byte left / right.
//! - `Ctrl-K` — delete from cursor to end of line.
//! - `Ctrl-U` — delete from start of line to cursor.
//! - `Ctrl-W` — delete the previous whitespace-delimited word.
//! - `Ctrl-L` — clear screen, redraw prompt and current line.
//! - `Tab` — file / command completion (see [`Completer`]).
//! - `Up` / `Down` (ANSI `ESC [ A` / `ESC [ B`) — walk the history.
//! - `Left` / `Right` (ANSI `ESC [ D` / `ESC [ C`) — move cursor.
//! - `Home` / `End` (`ESC [ H` / `ESC [ F`) — move to start / end.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    fs,
    io::{self, Read, Write},
};

use crate::state::State;

/// Outcome of one [`Editor::read_line`] call.
pub enum ReadLine {
    /// User pressed Enter; the assembled line is returned (without the
    /// trailing newline).
    Line(String),
    /// User pressed Ctrl-D on an empty line, or the underlying read hit
    /// real end-of-file.
    Eof,
    /// User pressed Ctrl-C. The shell should discard any pending
    /// continuation and re-prompt at PS1.
    Interrupted,
}

/// Persistent line-editor state across calls.
///
/// Holds the command history so that successive invocations of
/// [`Editor::read_line`] can walk it with the arrow keys.
pub struct Editor {
    history: Vec<String>,
    max_history: usize,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    /// Default backlog of history entries kept in memory.
    pub const DEFAULT_HISTORY: usize = 500;

    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            max_history: Self::DEFAULT_HISTORY,
        }
    }

    /// Pushes `line` onto the history, deduplicating against the most
    /// recent entry and dropping the oldest if we've hit the cap.
    pub fn record(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        if self.history.last().map(String::as_str) == Some(line) {
            return;
        }
        if self.history.len() == self.max_history {
            self.history.remove(0);
        }
        self.history.push(line.to_string());
    }

    /// Prompts and reads one line of input. Returns [`ReadLine`] to
    /// distinguish a real line from end-of-input or Ctrl-C.
    ///
    /// `secondary` selects the PS2 continuation prompt; the caller passes
    /// `true` after an incomplete-parse to ask for more input.
    pub fn read_line(&mut self, prompt: &str, st: &State, secondary: bool) -> ReadLine {
        let mut session = Session::new(prompt, &self.history, st, secondary);
        let outcome = session.run();
        match outcome {
            ReadLine::Line(ref line) if !secondary => self.record(line),
            _ => {}
        }
        outcome
    }
}

/// State for a single in-progress line edit. Owns the buffer, cursor, and
/// transient history-navigation index.
struct Session<'a> {
    prompt: &'a str,
    /// In-progress line bytes. Always valid UTF-8; we only ever append
    /// ASCII or fully-formed Rust `char`s that the caller passes through.
    buf: String,
    /// Cursor position in `buf`, measured in bytes (we deal only in
    /// single-byte ASCII for cursor moves; multibyte characters get
    /// echoed but cursor arithmetic stays byte-based, which is fine for
    /// the bytes a kernel canonical-mode terminal actually delivers).
    cursor: usize,
    /// Snapshot of the shell's history; we never mutate it directly.
    history: &'a [String],
    /// Index into `history` while walking with Up/Down. Equals
    /// `history.len()` when the user is editing a brand-new line.
    hist_pos: usize,
    /// Buffer holding the user's in-progress line while they peek into
    /// history with Up, restored when they leave history with Down past
    /// the end.
    saved_buf: Option<String>,
    /// Shell state, used only to resolve `$PATH` during completion.
    st: &'a State,
    /// True when the prompt is the PS2 continuation prompt — disables
    /// command-name completion (a continuation never starts a command).
    secondary: bool,
}

impl<'a> Session<'a> {
    fn new(prompt: &'a str, history: &'a [String], st: &'a State, secondary: bool) -> Self {
        Self {
            prompt,
            buf: String::new(),
            cursor: 0,
            history,
            hist_pos: history.len(),
            saved_buf: None,
            st,
            secondary,
        }
    }

    fn run(&mut self) -> ReadLine {
        self.write_prompt();
        let mut byte = [0u8; 1];
        loop {
            let n = io::stdin().read(&mut byte).unwrap_or(0);
            if n == 0 {
                return if self.buf.is_empty() {
                    ReadLine::Eof
                } else {
                    self.finish_line();
                    ReadLine::Line(core::mem::take(&mut self.buf))
                };
            }
            match byte[0] {
                b'\n' | b'\r' => {
                    self.finish_line();
                    return ReadLine::Line(core::mem::take(&mut self.buf));
                }
                0x03 => {
                    // Ctrl-C — echo `^C` and abandon.
                    write_bytes(b"^C\r\n");
                    return ReadLine::Interrupted;
                }
                0x04 => {
                    if self.buf.is_empty() {
                        write_bytes(b"\r\n");
                        return ReadLine::Eof;
                    }
                    // Ctrl-D mid-line is ignored (bash deletes the byte
                    // under the cursor; we keep it simple).
                }
                0x08 | 0x7f => self.delete_left(),
                0x01 => self.move_home(),
                0x05 => self.move_end(),
                0x02 => self.move_left(),
                0x06 => self.move_right(),
                0x0b => self.delete_to_end(),
                0x15 => self.delete_to_start(),
                0x17 => self.delete_word_left(),
                0x0c => self.redraw_after_clear(),
                b'\t' => self.complete(),
                0x1b => self.handle_escape(),
                b if (b' '..0x7f).contains(&b) => self.insert_char(b as char),
                _ => {} // ignore other control bytes
            }
        }
    }

    // -----------------------------------------------------------------
    // Output primitives
    // -----------------------------------------------------------------

    fn write_prompt(&self) {
        write_bytes(self.prompt.as_bytes());
    }

    /// Repaints the current line from scratch: CR, prompt, buf, then
    /// moves the cursor back by `(buf.len() - cursor)` bytes.
    fn redraw(&self) {
        // CSI 2K = erase entire line; \r = move to col 0.
        write_bytes(b"\r\x1b[2K");
        write_bytes(self.prompt.as_bytes());
        write_bytes(self.buf.as_bytes());
        let tail = self.buf.len() - self.cursor;
        if tail > 0 {
            move_cursor_left(tail);
        }
    }

    fn finish_line(&self) {
        write_bytes(b"\r\n");
    }

    // -----------------------------------------------------------------
    // Editing actions
    // -----------------------------------------------------------------

    fn insert_char(&mut self, ch: char) {
        self.buf.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.redraw();
    }

    fn delete_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Find the start of the prior char (handle multibyte gracefully).
        let mut new = self.cursor - 1;
        while !self.buf.is_char_boundary(new) && new > 0 {
            new -= 1;
        }
        self.buf.replace_range(new..self.cursor, "");
        self.cursor = new;
        self.redraw();
    }

    fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut new = self.cursor - 1;
        while !self.buf.is_char_boundary(new) && new > 0 {
            new -= 1;
        }
        let delta = self.cursor - new;
        self.cursor = new;
        move_cursor_left(delta);
    }

    fn move_right(&mut self) {
        if self.cursor >= self.buf.len() {
            return;
        }
        let mut new = self.cursor + 1;
        while new < self.buf.len() && !self.buf.is_char_boundary(new) {
            new += 1;
        }
        let delta = new - self.cursor;
        self.cursor = new;
        move_cursor_right(delta);
    }

    fn move_home(&mut self) {
        if self.cursor != 0 {
            move_cursor_left(self.cursor);
            self.cursor = 0;
        }
    }

    fn move_end(&mut self) {
        if self.cursor < self.buf.len() {
            let delta = self.buf.len() - self.cursor;
            move_cursor_right(delta);
            self.cursor = self.buf.len();
        }
    }

    fn delete_to_end(&mut self) {
        if self.cursor < self.buf.len() {
            self.buf.truncate(self.cursor);
            self.redraw();
        }
    }

    fn delete_to_start(&mut self) {
        if self.cursor > 0 {
            self.buf.replace_range(..self.cursor, "");
            self.cursor = 0;
            self.redraw();
        }
    }

    fn delete_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let bytes = self.buf.as_bytes();
        let mut i = self.cursor;
        // Skip trailing whitespace.
        while i > 0 && bytes[i - 1] == b' ' {
            i -= 1;
        }
        // Drop one word.
        while i > 0 && bytes[i - 1] != b' ' {
            i -= 1;
        }
        self.buf.replace_range(i..self.cursor, "");
        self.cursor = i;
        self.redraw();
    }

    fn redraw_after_clear(&self) {
        // ESC [2J = clear screen; ESC [H = home cursor.
        write_bytes(b"\x1b[2J\x1b[H");
        self.redraw();
    }

    // -----------------------------------------------------------------
    // History navigation
    // -----------------------------------------------------------------

    fn history_prev(&mut self) {
        if self.hist_pos == 0 {
            return;
        }
        if self.hist_pos == self.history.len() {
            self.saved_buf = Some(self.buf.clone());
        }
        self.hist_pos -= 1;
        self.buf = self.history[self.hist_pos].clone();
        self.cursor = self.buf.len();
        self.redraw();
    }

    fn history_next(&mut self) {
        if self.hist_pos >= self.history.len() {
            return;
        }
        self.hist_pos += 1;
        self.buf = if self.hist_pos == self.history.len() {
            self.saved_buf.take().unwrap_or_default()
        } else {
            self.history[self.hist_pos].clone()
        };
        self.cursor = self.buf.len();
        self.redraw();
    }

    // -----------------------------------------------------------------
    // ANSI escape decoding — only the few sequences we care about.
    // -----------------------------------------------------------------

    fn handle_escape(&mut self) {
        let mut b = [0u8; 1];
        if io::stdin().read(&mut b).unwrap_or(0) == 0 {
            return;
        }
        if b[0] != b'[' && b[0] != b'O' {
            return; // unknown — drop the lead-in
        }
        if io::stdin().read(&mut b).unwrap_or(0) == 0 {
            return;
        }
        match b[0] {
            b'A' => self.history_prev(),
            b'B' => self.history_next(),
            b'C' => self.move_right(),
            b'D' => self.move_left(),
            b'H' => self.move_home(),
            b'F' => self.move_end(),
            b'1' | b'3' | b'4' | b'7' | b'8' => {
                // Numbered sequences like `ESC [ 3 ~` (Delete). Swallow
                // the trailing `~` so it doesn't become a literal char.
                let mut last = [0u8; 1];
                while io::stdin().read(&mut last).unwrap_or(0) == 1 && last[0] != b'~' {
                    if last[0].is_ascii_alphabetic() {
                        break;
                    }
                }
                if b[0] == b'3' {
                    self.delete_under_cursor();
                }
            }
            _ => {}
        }
    }

    fn delete_under_cursor(&mut self) {
        if self.cursor >= self.buf.len() {
            return;
        }
        let mut end = self.cursor + 1;
        while end < self.buf.len() && !self.buf.is_char_boundary(end) {
            end += 1;
        }
        self.buf.replace_range(self.cursor..end, "");
        self.redraw();
    }

    // -----------------------------------------------------------------
    // Tab completion
    // -----------------------------------------------------------------

    fn complete(&mut self) {
        let (start, prefix) = self.completion_prefix();
        let candidates = if start == 0 && !self.secondary {
            command_candidates(prefix, self.st)
        } else {
            path_candidates(prefix)
        };
        match candidates.len() {
            0 => {} // no completions — silent
            1 => {
                // Single match: insert the remainder. If the completed
                // entry is a directory we append `/`; otherwise a space.
                let only = &candidates[0];
                if only.text.len() > prefix.len() {
                    let extra = &only.text[prefix.len()..];
                    self.buf.insert_str(self.cursor, extra);
                    self.cursor += extra.len();
                }
                if only.kind == CandidateKind::Directory {
                    if !self.buf[..self.cursor].ends_with('/') {
                        self.buf.insert(self.cursor, '/');
                        self.cursor += 1;
                    }
                } else if only.kind == CandidateKind::File || only.kind == CandidateKind::Executable
                {
                    let needs_space =
                        self.cursor == self.buf.len() || self.buf.as_bytes()[self.cursor] != b' ';
                    if needs_space {
                        self.buf.insert(self.cursor, ' ');
                        self.cursor += 1;
                    }
                }
                self.redraw();
            }
            _ => {
                // Multiple matches: extend by the longest common prefix
                // and, if nothing changed, list the candidates beneath
                // the prompt and redraw.
                let lcp = longest_common_prefix(&candidates);
                if lcp.len() > prefix.len() {
                    let extra = &lcp[prefix.len()..];
                    self.buf.insert_str(self.cursor, extra);
                    self.cursor += extra.len();
                    self.redraw();
                } else {
                    write_bytes(b"\r\n");
                    write_columns(&candidates);
                    self.redraw();
                }
            }
        }
    }

    /// Returns `(byte offset where the current word starts, prefix already
    /// typed)`. The word boundary is whitespace or any shell metacharacter
    /// that ends a word.
    fn completion_prefix(&self) -> (usize, &str) {
        let bytes = self.buf.as_bytes();
        let mut start = self.cursor;
        while start > 0 {
            let c = bytes[start - 1];
            if matches!(
                c,
                b' ' | b'\t' | b';' | b'|' | b'&' | b'<' | b'>' | b'(' | b')'
            ) {
                break;
            }
            start -= 1;
        }
        (start, &self.buf[start..self.cursor])
    }
}

// ---------------------------------------------------------------------------
// Completion candidates
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum CandidateKind {
    /// External command found on `$PATH`, or a builtin / function name.
    Executable,
    /// Regular file on disk.
    File,
    /// Directory; the completion appends a trailing `/`.
    Directory,
    /// Anything else (device node, FIFO, etc.).
    Other,
}

struct Candidate {
    /// Full matched name, including any prefix path component the user
    /// typed. This is what gets spliced back into the line.
    text: String,
    /// Just the filename portion of `text`, used for the multi-column
    /// listing so the user sees `foo` rather than `subdir/foo`.
    display: String,
    kind: CandidateKind,
}

/// Completes a command name: tries every `$PATH` entry plus the list of
/// shell built-ins.
fn command_candidates(prefix: &str, st: &State) -> Vec<Candidate> {
    // A name containing `/` is really a path — fall through to path
    // completion so `./scrip<TAB>` works.
    if prefix.contains('/') {
        return path_candidates(prefix);
    }

    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for name in crate::builtin::all_names() {
        if name.starts_with(prefix) && !seen.contains(&name.to_string()) {
            seen.push(name.to_string());
            out.push(Candidate {
                text: name.to_string(),
                display: name.to_string(),
                kind: CandidateKind::Executable,
            });
        }
    }

    let path = st.get("PATH").unwrap_or("/bin:/usr/bin");
    for dir in path.split(':') {
        let dir = if dir.is_empty() { "." } else { dir };
        let Ok(rd) = fs::read_dir(dir) else { continue };
        for entry in rd.flatten() {
            let name = entry.file_name();
            if !name.starts_with(prefix) || seen.contains(&name) {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.is_file() {
                continue;
            }
            seen.push(name.clone());
            out.push(Candidate {
                text: name.clone(),
                display: name,
                kind: CandidateKind::Executable,
            });
        }
    }

    out.sort_by(|a, b| a.text.cmp(&b.text));
    out
}

/// Completes a path. `prefix` may be absolute or relative; we split off the
/// directory component, list it, and keep entries whose name starts with
/// the remaining basename.
fn path_candidates(prefix: &str) -> Vec<Candidate> {
    let (dir, base) = match prefix.rfind('/') {
        Some(idx) => (&prefix[..=idx], &prefix[idx + 1..]),
        None => ("", prefix),
    };
    let listing_dir = if dir.is_empty() { "." } else { dir };

    let Ok(rd) = fs::read_dir(listing_dir) else {
        return Vec::new();
    };
    let mut out: Vec<Candidate> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name();
        if !name.starts_with(base) {
            continue;
        }
        // Skip dotfiles unless the user explicitly typed a leading `.`.
        if name.starts_with('.') && !base.starts_with('.') {
            continue;
        }
        let kind = entry
            .metadata()
            .map(|m| {
                let ft = m.file_type();
                if ft.is_dir() {
                    CandidateKind::Directory
                } else if ft.is_file() {
                    CandidateKind::File
                } else {
                    CandidateKind::Other
                }
            })
            .unwrap_or(CandidateKind::Other);
        let mut text = String::with_capacity(dir.len() + name.len());
        text.push_str(dir);
        text.push_str(&name);
        out.push(Candidate {
            text,
            display: name,
            kind,
        });
    }
    out.sort_by(|a, b| a.display.cmp(&b.display));
    out
}

fn longest_common_prefix(cands: &[Candidate]) -> String {
    let first = &cands[0].text;
    let mut len = first.len();
    for c in &cands[1..] {
        let max = len.min(c.text.len());
        let common = first
            .as_bytes()
            .iter()
            .zip(c.text.as_bytes().iter())
            .take(max)
            .take_while(|(a, b)| a == b)
            .count();
        len = common;
    }
    while !first.is_char_boundary(len) && len > 0 {
        len -= 1;
    }
    first[..len].to_string()
}

/// Writes `cands` to the terminal as a multi-column listing. Each row is
/// terminated with CRLF so a raw-mode TTY still advances correctly.
fn write_columns(cands: &[Candidate]) {
    const WIDTH: usize = 80;
    const GUTTER: usize = 2;
    let widest = cands.iter().map(|c| c.display.len()).max().unwrap_or(0);
    let col_w = widest + GUTTER;
    let cols = (WIDTH / col_w.max(1)).max(1);
    let mut out = io::stdout();
    for (i, c) in cands.iter().enumerate() {
        let _ = out.write_all(c.display.as_bytes());
        let pad = col_w.saturating_sub(c.display.len());
        if (i + 1) % cols == 0 || i + 1 == cands.len() {
            let _ = out.write_all(b"\r\n");
        } else {
            for _ in 0..pad {
                let _ = out.write_all(b" ");
            }
        }
    }
    let _ = out.flush();
}

// ---------------------------------------------------------------------------
// Terminal output helpers
// ---------------------------------------------------------------------------

fn write_bytes(bytes: &[u8]) {
    let mut out = io::stdout();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

/// Emits CSI `n D` to step the cursor `n` columns left.
fn move_cursor_left(n: usize) {
    if n == 0 {
        return;
    }
    let mut buf = [0u8; 16];
    let s = format_csi(&mut buf, n, b'D');
    write_bytes(s);
}

/// Emits CSI `n C` to step the cursor `n` columns right.
fn move_cursor_right(n: usize) {
    if n == 0 {
        return;
    }
    let mut buf = [0u8; 16];
    let s = format_csi(&mut buf, n, b'C');
    write_bytes(s);
}

/// Formats a CSI sequence `ESC [ n FINAL` into the supplied buffer and
/// returns the populated slice. Avoids any heap allocation.
fn format_csi(buf: &mut [u8; 16], n: usize, final_byte: u8) -> &[u8] {
    let mut digits = [0u8; 10];
    let mut count = 0;
    let mut v = n;
    if v == 0 {
        digits[0] = b'0';
        count = 1;
    } else {
        while v != 0 {
            digits[count] = b'0' + (v % 10) as u8;
            count += 1;
            v /= 10;
        }
    }
    buf[0] = 0x1b;
    buf[1] = b'[';
    let mut len = 2;
    for i in (0..count).rev() {
        buf[len] = digits[i];
        len += 1;
    }
    buf[len] = final_byte;
    len += 1;
    &buf[..len]
}
