//! Terminal control: raw mode setup, ANSI output primitives.
//!
//! `RawTty` flips the controlling terminal into raw mode on construction
//! and restores the previous settings on drop. While the guard is alive
//! the editor sees every keystroke as a raw byte (no canonical line
//! buffering, no echo, no signal generation from `^C`/`^\`), and is
//! responsible for echoing every character it wants on the screen.
//!
//! The logic mirrors POSIX `cfmakeraw(3)` — see
//! `man 3 cfmakeraw` for the exact bit list — but adapted to the
//! flag set this kernel ships in [`user_lib::syscall::tty`].

use alloc::string::String;

use user_lib::{
    io::{self, Write},
    syscall::{
        self,
        tty::{ControlMode, InputMode, LocalMode, OutputMode, Termios, TtyRequest},
    },
};

/// Guard that restores the terminal to its previous settings on drop.
///
/// Always construct one of these before the first call to [`read_key`]
/// and let it live for the entire editor session. Panics in the middle
/// of editing still flush through `Drop`, so the user's prompt isn't
/// left in raw mode.
pub struct RawTty {
    /// Settings captured at entry — restored verbatim on drop.
    saved: Option<Termios>,
}

impl RawTty {
    /// Enters raw mode and returns the guard. Returns `None` if fd 0
    /// isn't a TTY (e.g. stdin is a pipe), in which case the caller
    /// should bail out — vi without a terminal is meaningless.
    pub fn enter() -> Option<Self> {
        let saved = get_termios()?;
        let mut raw = saved;
        // Input: every byte is delivered verbatim. We do still want CR→LF
        // translation off (we'll interpret CR explicitly).
        raw.input_mode &= !(InputMode::ICRNL
            | InputMode::IGNCR
            | InputMode::INLCR
            | InputMode::ISTRIP
            | InputMode::IXON);
        // Output: leave OPOST/ONLCR in place — the editor writes CRLF
        // explicitly anyway, but keeping post-processing on means stray
        // `\n` from error paths still moves to column zero.
        raw.output_mode |= OutputMode::OPOST | OutputMode::ONLCR;
        // Local: no canonical line editing, no echo, no signal generation.
        raw.local_mode &= !(LocalMode::ICANON
            | LocalMode::ECHO
            | LocalMode::ECHOE
            | LocalMode::ECHOK
            | LocalMode::ECHOCTL
            | LocalMode::ECHOKE
            | LocalMode::ISIG
            | LocalMode::IEXTEN);
        // Control: 8-bit characters, no parity, receiver on.
        raw.control_mode |= ControlMode::CS8 | ControlMode::CREAD;
        set_termios(&raw);
        Some(Self { saved: Some(saved) })
    }
}

impl Drop for RawTty {
    fn drop(&mut self) {
        if let Some(prev) = self.saved.take() {
            set_termios(&prev);
            // Reset graphic attributes and put us on a fresh line so the
            // next shell prompt doesn't share a row with our last
            // status line. We intentionally avoid `\x1b[?25h` because
            // this kernel's VGA driver doesn't implement DEC-private
            // CSI sequences — the cursor is already visible by default.
            print_raw("\x1b[m\r\n");
        }
    }
}

/// Reads one keystroke worth of bytes from stdin, blocking until at
/// least one is available. Returns `None` on EOF.
///
/// Callers feed the returned bytes through [`crate::keys::decode`] to
/// turn ANSI escape sequences (arrow keys, Home, PgUp …) into a single
/// logical [`crate::keys::Key`].
pub fn read_byte() -> Option<u8> {
    let mut buf = [0u8; 1];
    let n = syscall::fs::read(0, buf.as_mut_ptr(), 1).ok()?;
    if n == 0 { None } else { Some(buf[0]) }
}

// ---------------------------------------------------------------------------
// ANSI output primitives
// ---------------------------------------------------------------------------

/// Writes `s` straight to stdout, flushing immediately.
///
/// vi spends most of its time issuing short CSI sequences; we bypass
/// `print!` so we never have to think about whether stdout was buffered.
pub fn print_raw(s: &str) {
    let mut out = io::stdout();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

// `\x1b[?25l` / `\x1b[?25h` (hide / show cursor) would be the natural
// way to hide the cursor between repaints to avoid flicker. We don't
// emit them: this kernel's VGA driver doesn't parse DEC-private CSI
// sequences, so the show-cursor on exit would be silently dropped and
// the cursor would stay invisible across the editor's lifetime.

/// Clears the screen and homes the cursor.
pub const CLEAR_SCREEN: &str = "\x1b[2J\x1b[H";

/// Clears the current line from the cursor to the end.
pub const CLEAR_EOL: &str = "\x1b[K";

/// Reverse-video on / off (used for the status bar).
pub const REVERSE_ON: &str = "\x1b[7m";
pub const ATTR_RESET: &str = "\x1b[m";

/// Returns the CSI sequence that moves the cursor to `(row, col)`,
/// 1-based, matching the VT100 convention.
pub fn move_to(row: u16, col: u16) -> String {
    alloc::format!("\x1b[{};{}H", row, col)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn get_termios() -> Option<Termios> {
    let mut t = Termios::console_default();
    syscall::fs::ioctl(0, TtyRequest::GetTermios as u32, &mut t as *mut _ as u32).ok()?;
    Some(t)
}

fn set_termios(t: &Termios) {
    let _ = syscall::fs::ioctl(0, TtyRequest::SetTermios as u32, t as *const _ as u32);
}
