//! `sh` — POSIX-subset shell for this kernel.
//!
//! Supports pipelines, redirections, `&&` / `||` / `;` / `&`, single and
//! double quoting, `\` escapes, parameter and command substitution,
//! arithmetic expansion, globbing, `if` / `while` / `for`, functions, and
//! the usual builtins (cd, pwd, exit, export, unset, set, shift, read,
//! echo, eval, exec, `.`/source, true, false, break, continue, return,
//! umask, type, wait, command, test, `[`).
//!
//! ## Module layout
//!
//! - [`lexer`] — splits source bytes into [`Token`](lexer::Token) stream
//!   while preserving quote / expansion structure.
//! - [`parser`] — recursive-descent parser → [`Cmd`](ast::Cmd) AST.
//! - [`expand`] — word expansion: tilde, parameter, command, arithmetic,
//!   field splitting, pathname expansion.
//! - [`exec`] — runs the AST: fork / pipe / dup2 for external commands,
//!   in-process dispatch for builtins.
//! - [`builtin`] — every shell built-in.
//! - [`state`] — variables, functions, positional parameters, options.

#![no_std]
#![no_main]

extern crate alloc;

mod ast;
mod builtin;
mod editor;
mod exec;
mod expand;
mod lexer;
mod parser;
mod state;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    env, fs,
    io::{self, Read, Write},
    process::ExitCode,
    syscall::{
        self,
        tty::{ControlMode, InputMode, LocalMode, OutputMode, Termios, TtyRequest},
    },
};

use crate::{exec::ExecError, state::State};

/// Largest single line accepted at the interactive prompt. Anything longer
/// is dropped with a diagnostic — keeps a runaway producer from filling
/// the user-space heap.
const MAX_LINE: usize = 16 * 1024;

#[user_lib::main]
fn main() -> ExitCode {
    let argv: Vec<String> = env::args().collect();
    let opts = Opts::parse(&argv);

    let arg0 = argv.first().cloned().unwrap_or_else(|| "sh".to_string());
    let is_login = arg0.starts_with('-');
    let Opts {
        mode,
        positional,
        force_interactive,
    } = opts;
    let mut st = State::from_env(arg0, positional);

    if is_login {
        source_login_profiles(&mut st);
    }

    let result = run(mode, force_interactive, &mut st);

    let exit_status = match result {
        Ok(()) => st.last_status,
        Err(ExecError::Exit(s)) => s,
        Err(ExecError::Fatal(msg)) => {
            let _ = writeln!(io::stderr(), "sh: {}", msg);
            2
        }
        Err(_) => 2,
    };
    ExitCode::from((exit_status & 0xff) as u8)
}

/// Where the shell pulls commands from for this invocation.
enum RunMode {
    /// `-c "..."` — execute the string and exit.
    InlineString(String),
    /// `sh script.sh ...` — read commands from a file.
    File(String),
    /// No filename: read from stdin (interactive if it's a TTY).
    Stdin,
}

/// Parsed command-line options.
struct Opts {
    mode: RunMode,
    positional: Vec<String>,
    /// Force interactive (`-i`); when `None`, auto-detect from stdin.
    /// `Some(true)` overrides the `isatty` check.
    force_interactive: Option<bool>,
}

impl Opts {
    /// Minimal POSIX argv parser: `-c STR`, `-s`, `-i`, `--`, plus a script
    /// filename. Unknown `-X` flags are silently ignored for compatibility.
    fn parse(argv: &[String]) -> Self {
        let mut mode = RunMode::Stdin;
        let mut positional: Vec<String> = Vec::new();
        let mut force_interactive = None;

        let mut iter = argv.iter().skip(1).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-c" => {
                    if let Some(src) = iter.next() {
                        mode = RunMode::InlineString(src.clone());
                    }
                }
                "-s" => {} // explicit "read from stdin" — default
                "-i" => force_interactive = Some(true),
                "--" => {
                    positional.extend(iter.by_ref().cloned());
                    break;
                }
                other if other.starts_with('-') => {} // unknown flag → ignore
                _ => {
                    mode = RunMode::File(arg.clone());
                    positional.extend(iter.by_ref().cloned());
                    break;
                }
            }
        }

        Self {
            mode,
            positional,
            force_interactive,
        }
    }
}

/// Sources `/etc/profile` and then `$HOME/.profile`. Missing files are
/// silently skipped — both are commonly absent.
fn source_login_profiles(st: &mut State) {
    let _ = source_if_exists("/etc/profile", st);
    if let Some(home) = st.get("HOME").map(ToString::to_string) {
        let path = format!("{}/.profile", home);
        let _ = source_if_exists(&path, st);
    }
}

/// Top-level dispatcher: pick the right entry point for the run mode and
/// run it. Handles the interactive-mode TTY setup / restore for `Stdin`.
fn run(mode: RunMode, force_interactive: Option<bool>, st: &mut State) -> Result<(), ExecError> {
    match mode {
        RunMode::InlineString(src) => run_string(&src, st),
        RunMode::File(path) => run_file(&path, st),
        RunMode::Stdin => {
            let is_tty = force_interactive.unwrap_or_else(stdin_is_tty);
            if is_tty {
                let saved = configure_interactive_termios();
                let r = run_interactive(st);
                if let Some(prev) = saved {
                    set_termios(&prev);
                }
                r
            } else {
                run_stdin_script(st)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Script / inline-string entry points
// ---------------------------------------------------------------------------

fn run_string(src: &str, st: &mut State) -> Result<(), ExecError> {
    match exec::run_source(src, st) {
        Ok(_) => Ok(()),
        Err(ExecError::Exit(s)) => {
            st.last_status = s;
            Ok(())
        }
        Err(other) => Err(other),
    }
}

fn run_file(path: &str, st: &mut State) -> Result<(), ExecError> {
    match fs::read_to_string(path) {
        Ok(src) => run_string(&src, st),
        Err(err) => {
            let _ = writeln!(io::stderr(), "sh: {}: {}", path, err);
            st.last_status = 127;
            Ok(())
        }
    }
}

/// Like [`run_file`] but silently does nothing when the file is missing;
/// used for `/etc/profile` and `~/.profile` autoload at login.
fn source_if_exists(path: &str, st: &mut State) -> Result<(), ExecError> {
    if fs::metadata(path).is_err() {
        return Ok(());
    }
    run_file(path, st)
}

fn run_stdin_script(st: &mut State) -> Result<(), ExecError> {
    let mut buf = String::new();
    let _ = io::stdin().read_to_string(&mut buf);
    run_string(&buf, st)
}

// ---------------------------------------------------------------------------
// Interactive REPL
// ---------------------------------------------------------------------------

/// Interactive read-evaluate-print loop.
///
/// Uses [`editor::Editor`] to provide raw-mode line editing (history,
/// arrow keys, Tab completion, Ctrl-A/E/U/W/K/L). The editor returns
/// either a complete line, end-of-input, or Ctrl-C; on Ctrl-C we discard
/// any pending continuation and re-prompt at PS1.
fn run_interactive(st: &mut State) -> Result<(), ExecError> {
    let mut editor = editor::Editor::new();
    let mut pending: Option<String> = None;

    loop {
        let prompt = if pending.is_some() {
            st.get("PS2").unwrap_or("> ").to_string()
        } else {
            let template = st.get("PS1").unwrap_or("$ ").to_string();
            expand_prompt(&template, st)
        };

        let line = match editor.read_line(&prompt, st, pending.is_some()) {
            editor::ReadLine::Line(line) => line,
            editor::ReadLine::Eof => return Ok(()),
            editor::ReadLine::Interrupted => {
                pending = None;
                continue;
            }
        };

        if line.len() > MAX_LINE {
            let _ = writeln!(io::stderr(), "sh: input line too long");
            pending = None;
            continue;
        }

        let mut combined = pending.take().unwrap_or_default();
        combined.push_str(&line);
        combined.push('\n');

        match parse_and_run(&combined, st) {
            ParseOutcome::Ran => {}
            ParseOutcome::Exit(s) => {
                st.last_status = s;
                return Ok(());
            }
            ParseOutcome::Incomplete => pending = Some(combined),
            ParseOutcome::ParseError(msg) => {
                let _ = writeln!(io::stderr(), "sh: parse error: {}", msg);
            }
            ParseOutcome::RuntimeError(msg) => {
                let _ = writeln!(io::stderr(), "sh: {}", msg);
            }
        }
    }
}

/// Distinct outcomes the REPL needs to react to. Keeping them named makes
/// the loop body read top-down rather than as a four-deep `match`.
enum ParseOutcome {
    Ran,
    Exit(i32),
    Incomplete,
    ParseError(String),
    RuntimeError(String),
}

fn parse_and_run(src: &str, st: &mut State) -> ParseOutcome {
    let mut parser = match parser::Parser::new(src) {
        Ok(p) => p,
        Err(err) if err.incomplete => return ParseOutcome::Incomplete,
        Err(err) => return ParseOutcome::ParseError(err.msg),
    };
    let prog = match parser.parse_program() {
        Ok(p) => p,
        Err(err) if err.incomplete => return ParseOutcome::Incomplete,
        Err(err) => return ParseOutcome::ParseError(err.msg),
    };
    match exec::run_cmd(&prog, st) {
        Ok(_) => ParseOutcome::Ran,
        Err(ExecError::Exit(s)) => ParseOutcome::Exit(s),
        Err(ExecError::Fatal(m)) => ParseOutcome::RuntimeError(m),
        Err(_) => ParseOutcome::Ran,
    }
}

/// Expands the prompt template before display.
///
/// Two layers happen here, mirroring what bash does on every prompt:
///
/// 1. Backslash escapes (`\u`, `\h`, `\w`, `\$`, `\n`, `\\`) are
///    substituted from the current shell state.
/// 2. The result is then re-parsed as a double-quoted shell word so any
///    `$var`, `${var}`, `$(cmd)`, or `` `cmd` `` inside PS1 gets evaluated
///    fresh on every prompt (e.g. `PS1='[`pwd`]# '` shows the live cwd).
fn expand_prompt(template: &str, st: &mut State) -> String {
    let escaped = substitute_backslash_escapes(template, st);

    // Re-tokenise as a double-quoted word so embedded `$()` / backticks /
    // `$var` evaluate every time. Failures fall back to the pre-expansion
    // text rather than aborting.
    let quoted = format!("\"{}\"", escaped.replace('"', "\\\""));
    let mut lex = lexer::Lexer::new(&quoted);
    match lex.next_token() {
        Ok(lexer::Token::Word(w)) => expand::expand_word_unsplit(&w, st).unwrap_or(escaped),
        _ => escaped,
    }
}

fn substitute_backslash_escapes(template: &str, st: &State) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'u' => out.push_str(st.get("USER").unwrap_or("root")),
                b'h' => out.push_str(st.get("HOSTNAME").unwrap_or("linux")),
                b'w' => out.push_str(
                    &env::current_dir()
                        .map(|p| p.into_string())
                        .unwrap_or_else(|_| "?".to_string()),
                ),
                b'$' => out.push(if user_lib::process::uid() == 0 {
                    '#'
                } else {
                    '$'
                }),
                b'n' => out.push('\n'),
                b'\\' => out.push('\\'),
                other => {
                    out.push('\\');
                    out.push(other as char);
                }
            }
            i += 2;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// TTY mode handling
// ---------------------------------------------------------------------------

/// Returns `true` if fd 0 is a terminal. Tested by issuing `GetTermios`,
/// which succeeds only for character devices that carry a tty discipline.
///
/// Honors `-i` as an override: forcing interactive mode reports `true`
/// even when stdin is a regular file.
fn stdin_is_tty() -> bool {
    let mut t = Termios::console_default();
    syscall::fs::ioctl(0, TtyRequest::GetTermios as u32, &mut t as *mut _ as u32).is_ok()
}

fn get_termios() -> Option<Termios> {
    let mut t = Termios::console_default();
    syscall::fs::ioctl(0, TtyRequest::GetTermios as u32, &mut t as *mut _ as u32)
        .ok()
        .map(|_| t)
}

fn set_termios(t: &Termios) {
    let _ = syscall::fs::ioctl(0, TtyRequest::SetTermios as u32, t as *const _ as u32);
}

/// Switches fd 0 into raw input mode for the interactive line editor:
/// `ICANON` and `ECHO` are turned off so the shell sees every keystroke
/// immediately and renders its own line buffer. Output post-processing
/// (CR-LF translation) stays on so prompts and command output continue
/// to render correctly. Returns the previous settings so the caller can
/// restore them on exit.
fn configure_interactive_termios() -> Option<Termios> {
    let old = get_termios()?;
    let mut new = old;

    // Input: translate CR to LF so Enter shows up as `\n`.
    new.input_mode |= InputMode::ICRNL;
    new.input_mode &= !(InputMode::INLCR | InputMode::IGNCR);

    // Output: post-process and expand LF into CRLF.
    new.output_mode |= OutputMode::OPOST | OutputMode::ONLCR;

    // Local: raw input — kernel hands every keystroke straight to us.
    // We keep `ISIG` so Ctrl-C / Ctrl-\ still produce signals (the
    // editor exits cleanly via the signal path), and disable kernel-side
    // line editing and echo so the editor owns the display.
    new.local_mode |= LocalMode::ISIG;
    new.local_mode &= !(LocalMode::ICANON
        | LocalMode::ECHO
        | LocalMode::ECHOE
        | LocalMode::ECHOK
        | LocalMode::ECHOCTL
        | LocalMode::ECHOKE);

    // Control: ensure characters can actually be received.
    new.control_mode |= ControlMode::CREAD;

    set_termios(&new);
    Some(old)
}
