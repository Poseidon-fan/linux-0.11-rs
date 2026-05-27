//! Ex commands typed after `:`.
//!
//! We support what `busybox vi` calls "the useful set":
//!
//! - `:w [PATH]`        — write
//! - `:q` / `:q!`       — quit (the bang form skips the dirty check)
//! - `:wq` / `:x`       — write and quit
//! - `:e PATH`          — abandon current buffer, open PATH
//! - `:r PATH`          — read PATH below the current line
//! - `:set nu`/`:set nonu` — toggle line numbers
//! - `:NNN`             — jump to line NNN
//! - `:s/pat/rep/[g]`   — substitute on the current line
//! - `:%s/pat/rep/[g]`  — substitute across the whole buffer
//! - `:help`            — print a one-screen cheat sheet
//!
//! Unknown commands report `not an editor command: ...` and otherwise
//! leave the buffer alone.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use regex::Regex;

use crate::buffer::Buffer;

/// Result of dispatching one ex command.
pub enum ExResult {
    /// Continue editing, optionally with a status-line message.
    Continue(Option<String>),
    /// Leave the editor with the given exit code.
    Quit(i32),
    /// Replace the current buffer with the supplied one (`:e PATH`).
    Replace(Buffer, Option<String>),
}

/// Parses and runs one `:`-prefixed line. The leading colon must
/// already be stripped before calling.
pub fn dispatch(line: &str, buf: &mut Buffer, show_numbers: &mut bool) -> ExResult {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ExResult::Continue(None);
    }

    // Pure number → goto-line.
    if let Ok(n) = trimmed.parse::<usize>() {
        let target = n.saturating_sub(1).min(buf.line_count() - 1);
        buf.row = target;
        buf.col = 0;
        return ExResult::Continue(None);
    }

    // Substitution: `:s/pat/rep[/g]` or `:%s/...`.
    if trimmed.starts_with("s/") || trimmed.starts_with("%s/") {
        return substitute(trimmed, buf);
    }

    // Split off the verb (up to the first space or `!`).
    let (verb, rest) = split_verb(trimmed);
    let arg = rest.trim_start();

    match verb {
        "w" => write(buf, arg),
        "q" => quit(buf, false),
        "q!" => quit(buf, true),
        "wq" | "x" => match write(buf, arg) {
            ExResult::Continue(_) => ExResult::Quit(0),
            other => other,
        },
        "e" => edit(arg),
        "r" => read_into(buf, arg),
        "set" => set_option(arg, show_numbers),
        "help" => ExResult::Continue(Some(help_line())),
        other => ExResult::Continue(Some(alloc::format!("not an editor command: {}", other))),
    }
}

fn split_verb(line: &str) -> (&str, &str) {
    if let Some(rest) = line.strip_prefix("q!") {
        return ("q!", rest);
    }
    match line.find([' ', '\t']) {
        Some(idx) => line.split_at(idx),
        None => (line, ""),
    }
}

fn write(buf: &mut Buffer, arg: &str) -> ExResult {
    let result = if arg.is_empty() {
        buf.save()
    } else {
        buf.save_as(arg)
    };
    match result {
        Ok(bytes) => {
            let name = buf.display_name().to_string();
            ExResult::Continue(Some(alloc::format!(
                "\"{}\" {}L, {}B written",
                name,
                buf.line_count(),
                bytes
            )))
        }
        Err(err) => ExResult::Continue(Some(alloc::format!("write failed: {}", err))),
    }
}

fn quit(buf: &Buffer, force: bool) -> ExResult {
    if buf.dirty && !force {
        return ExResult::Continue(Some(
            "no write since last change (add ! to override)".to_string(),
        ));
    }
    ExResult::Quit(0)
}

fn edit(arg: &str) -> ExResult {
    if arg.is_empty() {
        return ExResult::Continue(Some("usage: :e PATH".to_string()));
    }
    match Buffer::load(arg) {
        Ok(b) => {
            let msg = if b.new_file {
                Some(alloc::format!("\"{}\" [new file]", arg))
            } else {
                Some(alloc::format!("\"{}\" {}L", arg, b.line_count()))
            };
            ExResult::Replace(b, msg)
        }
        Err(err) => ExResult::Continue(Some(alloc::format!("can't open {}: {}", arg, err))),
    }
}

fn read_into(buf: &mut Buffer, arg: &str) -> ExResult {
    if arg.is_empty() {
        return ExResult::Continue(Some("usage: :r PATH".to_string()));
    }
    match user_lib::fs::read_to_string(arg) {
        Ok(text) => {
            let inserted_lines: Vec<String> = text.lines().map(ToString::to_string).collect();
            let count = inserted_lines.len();
            let row = buf.row + 1;
            for (i, line) in inserted_lines.into_iter().enumerate() {
                buf.lines.insert(row + i, line);
            }
            buf.dirty = true;
            ExResult::Continue(Some(alloc::format!("\"{}\" {}L read", arg, count)))
        }
        Err(err) => ExResult::Continue(Some(alloc::format!("can't read {}: {}", arg, err))),
    }
}

fn set_option(arg: &str, show_numbers: &mut bool) -> ExResult {
    match arg {
        "nu" | "number" => {
            *show_numbers = true;
            ExResult::Continue(None)
        }
        "nonu" | "nonumber" => {
            *show_numbers = false;
            ExResult::Continue(None)
        }
        other => ExResult::Continue(Some(alloc::format!("unknown option: {}", other))),
    }
}

/// Handles `s/pat/rep[/g]` (current line) and `%s/...` (whole buffer).
fn substitute(line: &str, buf: &mut Buffer) -> ExResult {
    let (global_lines, body) = if let Some(rest) = line.strip_prefix("%s/") {
        (true, rest)
    } else {
        (false, &line[2..]) // strip leading "s/"
    };

    // Split on unescaped `/`.
    let parts = split_unescaped(body, '/');
    if parts.len() < 2 {
        return ExResult::Continue(Some("usage: :s/pattern/replacement/[g]".to_string()));
    }
    let pattern = &parts[0];
    let replacement = &parts[1];
    let flags = parts.get(2).cloned().unwrap_or_default();
    let global_in_line = flags.contains('g');

    let re = match Regex::new(pattern) {
        Ok(re) => re,
        Err(err) => return ExResult::Continue(Some(alloc::format!("bad pattern: {}", err))),
    };

    let mut count = 0usize;
    let rows: Vec<usize> = if global_lines {
        (0..buf.line_count()).collect()
    } else {
        alloc::vec![buf.row]
    };
    for r in rows {
        let line = buf.lines[r].clone();
        let new = if global_in_line {
            re.replace_all(&line, replacement.as_str()).into_owned()
        } else {
            re.replacen(&line, 1, replacement.as_str()).into_owned()
        };
        if new != line {
            count += if global_in_line {
                re.find_iter(&line).count()
            } else {
                1
            };
            buf.lines[r] = new;
            buf.dirty = true;
        }
    }

    ExResult::Continue(Some(alloc::format!("{} substitutions", count)))
}

fn split_unescaped(s: &str, sep: char) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut iter = s.chars().peekable();
    while let Some(c) = iter.next() {
        if c == '\\' && iter.peek() == Some(&sep) {
            cur.push(sep);
            iter.next();
        } else if c == sep {
            out.push(core::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn help_line() -> String {
    "h/j/k/l move | i insert | x del | dd cut | yy/p copy/paste | u undo | /pat search | :w :q :wq"
        .to_string()
}
