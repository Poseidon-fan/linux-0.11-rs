//! Editor mode enum and per-mode persistent state.
//!
//! Lives in its own module so the dispatcher in `main.rs` doesn't
//! grow yet another enum at the top.

use alloc::string::String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Normal mode — every key is a command (`hjkl`, `i`, `dd`, …).
    Normal,
    /// Insert mode — typed bytes go into the buffer; `Esc` returns to
    /// Normal.
    Insert,
    /// After `r` — the next byte replaces the one under the cursor.
    ReplaceOne,
    /// Command-line mode: the user is composing a `:`/`/`/`?` line that
    /// runs (or searches) on Enter.
    CommandLine(CommandKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    /// Ex command (`:`).
    Ex,
    /// Forward search (`/`).
    SearchForward,
    /// Backward search (`?`).
    SearchBackward,
}

/// Buffer for the in-progress command line / search pattern.
#[derive(Default)]
pub struct CommandLine {
    pub buf: String,
}

impl CommandLine {
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    pub fn push(&mut self, ch: char) {
        self.buf.push(ch);
    }

    pub fn backspace(&mut self) {
        self.buf.pop();
    }
}
