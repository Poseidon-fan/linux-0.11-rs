//! REPL driver: rustyline wrapper, prompt rendering, completion bridge.
//!
//! Everything visible to the user — prompt, history, completion suggestions
//! — flows through this module. Command logic lives under [`crate::commands`].

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::Result;
use rustyline::{
    CompletionType, Config, Editor, Helper,
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::Highlighter,
    hint::Hinter,
    history::FileHistory,
    validate::Validator,
};

use crate::{
    commands::{self, Outcome},
    path as shellpath,
    session::Session,
};

/// Runs the read-eval-print loop until the user exits or stdin closes.
pub fn run_repl(session: Session) -> Result<Session> {
    // The helper needs occasional read access to session state
    // (current cwds, the image fs for path completion) while rustyline
    // is sitting at the prompt. Wrap once and share between helper and
    // the dispatcher.
    let session = Rc::new(RefCell::new(session));

    let helper = ShellHelper {
        commands_snapshot: command_names(),
        session: Rc::clone(&session),
    };
    let config = Config::builder()
        .auto_add_history(true)
        // bash-like: first TAB extends to the longest common prefix; a
        // second TAB then lists every candidate. The default `Circular`
        // mode would silently rotate through one candidate per press,
        // which feels broken when there are many siblings under a
        // directory.
        .completion_type(CompletionType::List)
        .build();
    let mut editor: Editor<ShellHelper, FileHistory> = Editor::with_config(config)?;
    editor.set_helper(Some(helper));

    let history_path = session.borrow().history.clone();
    if let Some(history_path) = history_path.as_ref() {
        // Best-effort load; a missing file is fine, that just means a
        // first-time session.
        let _ = editor.load_history(history_path);
    }

    {
        let s = session.borrow();
        println!(
            "miniximg shell — {}{}",
            s.image_label(),
            if s.readonly { " (read-only)" } else { "" }
        );
    }
    println!("Paths default to the image. Prefix with `@` to use the host. `help` lists commands.");

    loop {
        let prompt = build_prompt(&session.borrow());
        match editor.readline(&prompt) {
            Ok(line) => {
                let outcome = commands::dispatch(&mut session.borrow_mut(), &line);
                if matches!(outcome, Outcome::Quit) {
                    break;
                }
            }
            Err(ReadlineError::Interrupted) => continue, // Ctrl-C → drop line
            Err(ReadlineError::Eof) => break,            // Ctrl-D on empty line
            Err(err) => {
                eprintln!("readline error: {err}");
                break;
            }
        }
    }

    if let Some(history_path) = history_path.as_ref() {
        if let Some(parent) = history_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = editor.save_history(history_path);
    }

    // Unwrap the session and return it. Helper is dropped together with
    // `editor` going out of scope, so the only remaining reference is
    // the one we hold here.
    drop(editor);
    Ok(Rc::try_unwrap(session)
        .map_err(|_| anyhow::anyhow!("session still borrowed at REPL exit"))?
        .into_inner())
}

fn build_prompt(session: &Session) -> String {
    let label = session.image_label();
    let cwd = session.image_cwd();
    let tag = if session.readonly { ":ro" } else { "" };
    format!("({}{}) {}> ", label, tag, cwd)
}

// ---------------------------------------------------------------------------
// Tab completion
// ---------------------------------------------------------------------------

/// Rustyline `Helper` impl. Owns a snapshot of command names plus a
/// shared handle to the session so path completion can consult the
/// active filesystem and cwds.
struct ShellHelper {
    commands_snapshot: Vec<&'static str>,
    session: Rc<RefCell<Session>>,
}

impl Helper for ShellHelper {}
impl Validator for ShellHelper {}
impl Highlighter for ShellHelper {}
impl Hinter for ShellHelper {
    type Hint = String;
}

impl Completer for ShellHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let (start, prefix) = current_word(line, pos);
        let is_first_word = line[..start].trim().is_empty();

        if is_first_word {
            let mut out: Vec<Pair> = Vec::new();
            for &name in &self.commands_snapshot {
                if name.starts_with(prefix) {
                    out.push(Pair {
                        display: name.to_string(),
                        replacement: format!("{} ", name),
                    });
                }
            }
            return Ok((start, out));
        }

        // The helper is invoked only at the prompt, between command
        // dispatches; `borrow_mut` should succeed. If it doesn't, fall
        // back to no completions rather than panicking.
        let Ok(mut session) = self.session.try_borrow_mut() else {
            return Ok((start, Vec::new()));
        };

        if let Some(rest) = prefix.strip_prefix('@') {
            let host_cwd = session.host_cwd().to_path_buf();
            // Skip the `@` sigil — `rest` lives at `start + 1`.
            return Ok(complete_host(start + 1, rest, &host_cwd));
        }
        let cwd = session.image_cwd().to_string();
        Ok(complete_image(start, prefix, &cwd, &mut session))
    }
}

/// Returns `(start_byte_offset, current_partial_word)`.
fn current_word(line: &str, pos: usize) -> (usize, &str) {
    let bytes = line.as_bytes();
    let mut start = pos;
    while start > 0 {
        let c = bytes[start - 1];
        if matches!(c, b' ' | b'\t') {
            break;
        }
        start -= 1;
    }
    (start, &line[start..pos])
}

/// Completes a path inside the image. Splits `prefix` into directory and
/// basename, lists the directory through the loaded filesystem, and
/// returns entries whose name extends the basename.
///
/// We return `start = <where the basename begins>` rather than the start
/// of the whole word, so each candidate's `replacement` is just the
/// entry name plus its trailing `/` or ` ` separator. That matches what
/// rustyline expects: replacing only the unfinished basename, not the
/// directory portion the user already typed.
fn complete_image(
    word_start: usize,
    prefix: &str,
    image_cwd: &str,
    session: &mut Session,
) -> (usize, Vec<Pair>) {
    let (typed_dir, base) = split_dir_base(prefix);
    let base_start = word_start + typed_dir.len();

    let listing_dir = if typed_dir.is_empty() {
        image_cwd.to_string()
    } else {
        shellpath::resolve_image(typed_dir, image_cwd)
    };

    let Ok(entries) = session.fs_mut().list_path(&listing_dir) else {
        return (base_start, Vec::new());
    };

    let pairs = entries
        .into_iter()
        .filter(|e| keep_entry(&e.name, base))
        .map(|e| make_pair(&e.name, e.metadata.kind == miniximg::InodeType::Directory));
    (base_start, sorted(pairs))
}

fn complete_host(word_start: usize, prefix: &str, host_cwd: &Path) -> (usize, Vec<Pair>) {
    let (typed_dir, base) = split_dir_base(prefix);
    // `prefix` does not include the leading `@`; rustyline's `start`
    // for the helper already points past it, so adding `typed_dir.len()`
    // gives the basename's byte offset within the full input line.
    let base_start = word_start + typed_dir.len();

    let listing_dir: PathBuf = if typed_dir.is_empty() {
        host_cwd.to_path_buf()
    } else {
        let expanded = expand_tilde(typed_dir);
        if expanded.is_absolute() {
            expanded
        } else {
            host_cwd.join(expanded)
        }
    };

    let Ok(entries) = std::fs::read_dir(&listing_dir) else {
        return (base_start, Vec::new());
    };

    let pairs = entries.flatten().filter_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !keep_entry(&name, base) {
            return None;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        Some(make_pair(&name, is_dir))
    });
    (base_start, sorted(pairs))
}

/// Splits a typed path fragment into `(directory prefix including the
/// trailing slash, basename)`. The basename can be empty when the user
/// has just typed a slash and is waiting for completions.
fn split_dir_base(prefix: &str) -> (&str, &str) {
    match prefix.rfind('/') {
        Some(idx) => (&prefix[..=idx], &prefix[idx + 1..]),
        None => ("", prefix),
    }
}

/// Filter rule shared by image and host completion: an entry matches
/// when its name begins with `base`, and dotfiles are skipped unless
/// `base` itself starts with a dot.
fn keep_entry(name: &str, base: &str) -> bool {
    name.starts_with(base) && (!name.starts_with('.') || base.starts_with('.'))
}

/// Builds the `Pair` shown by rustyline for one entry. Directories get
/// a trailing `/`; everything else gets a trailing space so the cursor
/// lands ready for the next argument.
fn make_pair(name: &str, is_dir: bool) -> Pair {
    let suffix = if is_dir { '/' } else { ' ' };
    let mut replacement = String::with_capacity(name.len() + 1);
    replacement.push_str(name);
    replacement.push(suffix);
    let display = if is_dir {
        format!("{}/", name)
    } else {
        name.to_string()
    };
    Pair {
        display,
        replacement,
    }
}

fn sorted(pairs: impl IntoIterator<Item = Pair>) -> Vec<Pair> {
    let mut out: Vec<Pair> = pairs.into_iter().collect();
    out.sort_by(|a, b| a.display.cmp(&b.display));
    out
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Path::new(&home).join(rest);
        }
    } else if p == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    PathBuf::from(p)
}

fn command_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for cmd in commands::COMMANDS {
        v.push(cmd.name);
        for alias in cmd.aliases {
            v.push(alias);
        }
    }
    v.sort();
    v.dedup();
    v
}
