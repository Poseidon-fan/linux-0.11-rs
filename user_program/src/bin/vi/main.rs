//! `vi` — a small modal text editor for this kernel.
//!
//! Targets the busybox-vi feature set, adapted to what Linux 0.11 can
//! offer (no `TIOCGWINSZ`, no `SIGWINCH`, no `ftruncate`): we run on a
//! fixed 24×80 grid and rewrite the file on every `:w`.
//!
//! ## What it supports
//!
//! - **Modes**: Normal, Insert, Replace-one (`r`), Command-line
//!   (`:` / `/` / `?`).
//! - **Motions** with count prefix: `h j k l`, arrow keys, `w b e`,
//!   `0 ^ $`, `gg G`, `f F t T`, `%`, `Ctrl-f` / `Ctrl-b` (page down /
//!   up), `Home` / `End` / `PageUp` / `PageDown`.
//! - **Edits**: `i a I A o O`, `x X r`, `dd dw D yy p P J`, `cw`, `u`,
//!   `ZZ`.
//! - **Search**: `/pattern`, `?pattern`, `n`, `N` — uses the `regex`
//!   crate for full POSIX-extended regex syntax.
//! - **Ex commands**: `:w [PATH]`, `:q`, `:q!`, `:wq` / `:x`,
//!   `:e PATH`, `:r PATH`, `:s/pat/rep[/g]`, `:%s/pat/rep[/g]`,
//!   `:set nu` / `:set nonu`, `:NNN` (goto line), `:help`.
//!
//! ## What it doesn't
//!
//! - No syntax highlighting, no visual mode, no multi-window, no
//!   `.exrc`, no macros (`q`-record), no marks.
//! - No window-resize handling — the kernel doesn't deliver `SIGWINCH`
//!   and we can't ask for the real terminal size, so 24×80 is hardcoded.
//! - Undo keeps full buffer snapshots rather than a delta tree:
//!   simple, but memory-expensive on big buffers.

#![no_std]
#![no_main]

extern crate alloc;

mod buffer;
mod edit;
mod ex;
mod keys;
mod mode;
mod motion;
mod screen;
mod search;
mod tty;
mod undo;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    env,
    io::{self, Write},
    process::ExitCode,
};

use crate::{
    buffer::Buffer,
    keys::{Key, Reader},
    mode::{CommandKind, CommandLine, Mode},
    screen::Viewport,
    search::Search,
    tty::RawTty,
    undo::Undo,
};

const EXIT_OK: u8 = 0;
const EXIT_USAGE: u8 = 2;
const EXIT_NO_TTY: u8 = 3;

#[user_lib::main]
fn main() -> ExitCode {
    let argv: Vec<String> = env::args().collect();
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return ExitCode::from(EXIT_OK);
    }

    let buffer = match argv.get(1) {
        Some(path) => match Buffer::load(path) {
            Ok(b) => b,
            Err(err) => {
                let _ = writeln!(io::stderr(), "vi: {}: {}", path, err);
                return ExitCode::from(EXIT_USAGE);
            }
        },
        None => Buffer::empty(),
    };

    let Some(tty) = RawTty::enter() else {
        let _ = writeln!(io::stderr(), "vi: stdin is not a terminal");
        return ExitCode::from(EXIT_NO_TTY);
    };

    tty::print_raw(tty::ENTER_ALT_SCREEN);
    let exit_code = run(buffer);
    tty::print_raw(tty::LEAVE_ALT_SCREEN);
    drop(tty);
    ExitCode::from(exit_code as u8)
}

fn print_usage() {
    let _ = writeln!(
        io::stdout(),
        "usage: vi [FILE]\n\nA small modal editor. Type :help inside for a cheat sheet."
    );
}

// ---------------------------------------------------------------------------
// Editor state + main loop
// ---------------------------------------------------------------------------

struct Editor {
    buffer: Buffer,
    viewport: Viewport,
    mode: Mode,
    cmdline: CommandLine,
    undo: Undo,
    search: Search,
    /// Last-yanked text. A trailing `'\n'` marks it as line-wise.
    register: String,
    /// Status / transient message shown on the bottom row.
    status: String,
    /// `:set nu` toggle — currently only the wiring is in place; line
    /// numbers themselves aren't rendered yet.
    show_numbers: bool,
    /// Numeric prefix being typed in normal mode (`5dd`, `12G`).
    count: usize,
    /// Pending normal-mode operator awaiting a motion (`d`, `y`, `c`).
    pending_op: Option<u8>,
    /// Last text inserted in insert mode — reserved for a future `.`.
    last_insert: String,
    /// `gg` is two keystrokes; this remembers we saw the first `g`.
    g_prefix: bool,
    /// `f` / `F` / `t` / `T` need the next byte.
    pending_find: Option<u8>,
}

fn run(buffer: Buffer) -> i32 {
    let mut ed = Editor {
        buffer,
        viewport: Viewport::default(),
        mode: Mode::Normal,
        cmdline: CommandLine::default(),
        undo: Undo::new(),
        search: Search::new(),
        register: String::new(),
        status: String::new(),
        show_numbers: false,
        count: 0,
        pending_op: None,
        last_insert: String::new(),
        g_prefix: false,
        pending_find: None,
    };

    ed.status = if ed.buffer.new_file {
        alloc::format!("\"{}\" [new file]", ed.buffer.display_name())
    } else {
        alloc::format!(
            "\"{}\" {}L",
            ed.buffer.display_name(),
            ed.buffer.line_count()
        )
    };

    let mut reader = Reader::new(tty::read_byte);
    loop {
        ed.viewport.track(ed.buffer.row, ed.buffer.col);
        let visible_status = match ed.mode {
            Mode::CommandLine(kind) => {
                let prefix = match kind {
                    CommandKind::Ex => ':',
                    CommandKind::SearchForward => '/',
                    CommandKind::SearchBackward => '?',
                };
                let mut s = String::with_capacity(ed.cmdline.buf.len() + 1);
                s.push(prefix);
                s.push_str(&ed.cmdline.buf);
                s
            }
            _ => core::mem::take(&mut ed.status),
        };
        screen::draw(&ed.buffer, &ed.viewport, &visible_status);

        let Some(key) = reader.next() else {
            return EXIT_OK as i32;
        };
        match ed.mode {
            Mode::Normal => {
                if let Some(code) = handle_normal(&mut ed, key) {
                    return code;
                }
            }
            Mode::Insert => handle_insert(&mut ed, key),
            Mode::ReplaceOne => handle_replace(&mut ed, key),
            Mode::CommandLine(kind) => {
                if let Some(code) = handle_cmdline(&mut ed, key, kind) {
                    return code;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Normal mode
// ---------------------------------------------------------------------------

fn handle_normal(ed: &mut Editor, key: Key) -> Option<i32> {
    // `f` / `F` / `t` / `T` need the next byte. Capture it before
    // anything else interprets the key.
    if let Some(verb) = ed.pending_find.take() {
        if let Key::Char(b) = key {
            execute_find(ed, verb, b);
        }
        ed.count = 0;
        return None;
    }

    // Digits build up the numeric count prefix. Leading `0` is the
    // line-start motion, not a digit.
    if let Key::Char(b) = key {
        if b.is_ascii_digit() && !(b == b'0' && ed.count == 0) {
            ed.count = ed.count.saturating_mul(10) + (b - b'0') as usize;
            return None;
        }
    }

    // `gg` — second `g` after we recorded the first.
    if ed.g_prefix {
        ed.g_prefix = false;
        if matches!(key, Key::Char(b'g')) {
            motion::buf_start(&mut ed.buffer);
            ed.count = 0;
            return None;
        }
    }

    // Operator-pending (`dd`, `dw`, `yy`, `ZZ`, …).
    if let Some(op) = ed.pending_op.take() {
        execute_operator(ed, op, key);
        ed.count = 0;
        return None;
    }

    let count = ed.count.max(1);
    let had_explicit_count = ed.count > 0;
    ed.count = 0;

    match key {
        Key::Char(b'h') | Key::Left | Key::Backspace => {
            edit::repeat(count, || motion::left(&mut ed.buffer));
        }
        Key::Char(b'l') | Key::Right | Key::Char(b' ') => {
            edit::repeat(count, || motion::right(&mut ed.buffer));
        }
        Key::Char(b'k') | Key::Up => edit::repeat(count, || motion::up(&mut ed.buffer)),
        Key::Char(b'j') | Key::Down | Key::Enter => {
            edit::repeat(count, || motion::down(&mut ed.buffer));
        }
        Key::Char(b'w') => edit::repeat(count, || motion::word_next(&mut ed.buffer)),
        Key::Char(b'b') => edit::repeat(count, || motion::word_back(&mut ed.buffer)),
        Key::Char(b'e') => edit::repeat(count, || motion::word_end(&mut ed.buffer)),
        Key::Char(b'0') => motion::line_start(&mut ed.buffer),
        Key::Char(b'^') | Key::Home => motion::first_non_blank(&mut ed.buffer),
        Key::Char(b'$') | Key::End => motion::line_end(&mut ed.buffer),
        Key::Char(b'g') => ed.g_prefix = true,
        Key::Char(b'G') => {
            if had_explicit_count {
                ed.buffer.row = (count - 1).min(ed.buffer.line_count() - 1);
                ed.buffer.col = 0;
            } else {
                motion::buf_end(&mut ed.buffer);
            }
        }
        Key::Char(b'%') => motion::matching_bracket(&mut ed.buffer),
        Key::Char(b'f') => ed.pending_find = Some(b'f'),
        Key::Char(b'F') => ed.pending_find = Some(b'F'),
        Key::Char(b't') => ed.pending_find = Some(b't'),
        Key::Char(b'T') => ed.pending_find = Some(b'T'),
        Key::PageDown | Key::Ctrl(b'f') => edit::repeat(count, || {
            for _ in 0..screen::TEXT_ROWS {
                motion::down(&mut ed.buffer);
            }
        }),
        Key::PageUp | Key::Ctrl(b'b') => edit::repeat(count, || {
            for _ in 0..screen::TEXT_ROWS {
                motion::up(&mut ed.buffer);
            }
        }),

        Key::Char(b'i') => enter_insert(ed),
        Key::Char(b'I') => {
            motion::first_non_blank(&mut ed.buffer);
            enter_insert(ed);
        }
        Key::Char(b'a') => {
            if !ed.buffer.lines[ed.buffer.row].is_empty() {
                ed.buffer.col += 1;
            }
            ed.buffer.clamp_col(true);
            enter_insert(ed);
        }
        Key::Char(b'A') => {
            ed.buffer.col = ed.buffer.lines[ed.buffer.row].len();
            enter_insert(ed);
        }
        Key::Char(b'o') => {
            ed.undo.snapshot(&ed.buffer);
            ed.buffer.open_below();
            ed.mode = Mode::Insert;
            ed.last_insert.clear();
        }
        Key::Char(b'O') => {
            ed.undo.snapshot(&ed.buffer);
            ed.buffer.open_above();
            ed.mode = Mode::Insert;
            ed.last_insert.clear();
        }

        Key::Char(b'x') => {
            ed.undo.snapshot(&ed.buffer);
            edit::repeat(count, || {
                let _ = edit::delete_char(&mut ed.buffer);
            });
        }
        Key::Char(b'X') => {
            ed.undo.snapshot(&ed.buffer);
            edit::repeat(count, || {
                let _ = edit::backspace_char(&mut ed.buffer);
            });
        }
        Key::Char(b'r') => ed.mode = Mode::ReplaceOne,
        Key::Char(b'D') => {
            ed.undo.snapshot(&ed.buffer);
            ed.register = edit::delete_to_eol(&mut ed.buffer);
        }
        Key::Char(b'J') => {
            ed.undo.snapshot(&ed.buffer);
            edit::repeat(count, || edit::join_lines(&mut ed.buffer));
        }

        Key::Char(b'd') => ed.pending_op = Some(b'd'),
        Key::Char(b'y') => ed.pending_op = Some(b'y'),
        Key::Char(b'c') => ed.pending_op = Some(b'c'),
        Key::Char(b'Z') => ed.pending_op = Some(b'Z'),

        Key::Char(b'p') => {
            if !ed.register.is_empty() {
                ed.undo.snapshot(&ed.buffer);
                edit::paste_after(&mut ed.buffer, &ed.register);
            }
        }
        Key::Char(b'P') => {
            if !ed.register.is_empty() {
                ed.undo.snapshot(&ed.buffer);
                edit::paste_before(&mut ed.buffer, &ed.register);
            }
        }

        Key::Char(b'u') => {
            if !ed.undo.pop_into(&mut ed.buffer) {
                ed.status = "already at oldest change".to_string();
            }
        }

        Key::Char(b':') => {
            ed.mode = Mode::CommandLine(CommandKind::Ex);
            ed.cmdline.clear();
        }
        Key::Char(b'/') => {
            ed.mode = Mode::CommandLine(CommandKind::SearchForward);
            ed.cmdline.clear();
        }
        Key::Char(b'?') => {
            ed.mode = Mode::CommandLine(CommandKind::SearchBackward);
            ed.cmdline.clear();
        }
        Key::Char(b'n') => {
            if !ed.search.find_next(&mut ed.buffer) {
                ed.status = "pattern not found".to_string();
            }
        }
        Key::Char(b'N') => {
            if !ed.search.find_prev(&mut ed.buffer) {
                ed.status = "pattern not found".to_string();
            }
        }

        Key::Ctrl(b'l') => tty::print_raw(tty::CLEAR_SCREEN),
        _ => {}
    }
    None
}

/// Carries out operator + motion combos (`dd`, `yy`, `cw`, `ZZ`, …).
fn execute_operator(ed: &mut Editor, op: u8, motion_key: Key) {
    match (op, motion_key) {
        (b'd', Key::Char(b'd')) => {
            ed.undo.snapshot(&ed.buffer);
            let mut removed = edit::delete_line(&mut ed.buffer);
            removed.push('\n');
            ed.register = removed;
        }
        (b'y', Key::Char(b'y')) => {
            let mut s = edit::yank_line(&ed.buffer);
            s.push('\n');
            ed.register = s;
        }
        (b'd', Key::Char(b'w')) => {
            ed.undo.snapshot(&ed.buffer);
            ed.register = edit::delete_word(&mut ed.buffer);
        }
        (b'c', Key::Char(b'w')) => {
            ed.undo.snapshot(&ed.buffer);
            ed.register = edit::delete_word(&mut ed.buffer);
            enter_insert(ed);
        }
        (b'Z', Key::Char(b'Z')) => {
            let _ = ex::dispatch("wq", &mut ed.buffer, &mut ed.show_numbers);
            ed.status = "ZZ".to_string();
        }
        _ => {}
    }
}

fn enter_insert(ed: &mut Editor) {
    ed.mode = Mode::Insert;
    ed.last_insert.clear();
    ed.undo.snapshot(&ed.buffer);
}

/// Carries out `f X` / `F X` / `t X` / `T X`. `t` lands one column
/// before the target, `f` lands on it.
fn execute_find(ed: &mut Editor, verb: u8, target: u8) {
    let moved = match verb {
        b'f' => motion::find_forward(&mut ed.buffer, target),
        b'F' => motion::find_backward(&mut ed.buffer, target),
        b't' => {
            let r = motion::find_forward(&mut ed.buffer, target);
            if r && ed.buffer.col > 0 {
                ed.buffer.col -= 1;
            }
            r
        }
        b'T' => {
            let r = motion::find_backward(&mut ed.buffer, target);
            if r {
                ed.buffer.col += 1;
            }
            r
        }
        _ => false,
    };
    if !moved {
        ed.status = alloc::format!("{} not found on this line", target as char);
    }
}

// ---------------------------------------------------------------------------
// Insert mode
// ---------------------------------------------------------------------------

fn handle_insert(ed: &mut Editor, key: Key) {
    match key {
        Key::Esc => {
            ed.mode = Mode::Normal;
            ed.buffer.clamp_col(false);
        }
        Key::Enter => {
            ed.buffer.split_line();
            ed.last_insert.push('\n');
        }
        Key::Backspace => ed.buffer.backspace(),
        Key::Char(b) => {
            ed.buffer.insert_byte(b);
            ed.last_insert.push(b as char);
        }
        Key::Tab => {
            ed.buffer.insert_byte(b'\t');
            ed.last_insert.push('\t');
        }
        Key::Left => motion::left(&mut ed.buffer),
        Key::Right => motion::right(&mut ed.buffer),
        Key::Up => motion::up(&mut ed.buffer),
        Key::Down => motion::down(&mut ed.buffer),
        Key::Ctrl(b'w') => edit::drop_word_left(&mut ed.buffer),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// `r` — replace one byte
// ---------------------------------------------------------------------------

fn handle_replace(ed: &mut Editor, key: Key) {
    if let Key::Char(b) = key {
        ed.undo.snapshot(&ed.buffer);
        ed.buffer.replace_at_cursor(b);
    }
    ed.mode = Mode::Normal;
}

// ---------------------------------------------------------------------------
// Command-line mode (`:`, `/`, `?`)
// ---------------------------------------------------------------------------

fn handle_cmdline(ed: &mut Editor, key: Key, kind: CommandKind) -> Option<i32> {
    match key {
        Key::Enter => {
            let line = core::mem::take(&mut ed.cmdline.buf);
            ed.mode = Mode::Normal;
            match kind {
                CommandKind::Ex => {
                    match ex::dispatch(&line, &mut ed.buffer, &mut ed.show_numbers) {
                        ex::ExResult::Continue(msg) => {
                            if let Some(m) = msg {
                                ed.status = m;
                            }
                        }
                        ex::ExResult::Quit(code) => return Some(code),
                        ex::ExResult::Replace(new_buf, msg) => {
                            ed.buffer = new_buf;
                            ed.viewport = Viewport::default();
                            ed.undo = Undo::new();
                            if let Some(m) = msg {
                                ed.status = m;
                            }
                        }
                    }
                }
                CommandKind::SearchForward | CommandKind::SearchBackward => {
                    let forward = matches!(kind, CommandKind::SearchForward);
                    match ed.search.set(&line, forward) {
                        Ok(()) => {
                            if !ed.search.find_next(&mut ed.buffer) {
                                ed.status = "pattern not found".to_string();
                            }
                        }
                        Err(err) => ed.status = err,
                    }
                }
            }
        }
        Key::Esc => {
            ed.cmdline.clear();
            ed.mode = Mode::Normal;
        }
        Key::Backspace => {
            if ed.cmdline.buf.is_empty() {
                ed.mode = Mode::Normal;
            } else {
                ed.cmdline.backspace();
            }
        }
        Key::Char(b) => ed.cmdline.push(b as char),
        _ => {}
    }
    None
}
