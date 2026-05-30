//! `xargs` — build and run command lines from standard input.
//!
//! Reads whitespace- or NUL-separated items from stdin and appends them to a
//! command (default `echo`), running it in batches. Supports the common GNU
//! options for batch sizing (`-n`, `-L`), item replacement (`-I`), NUL input
//! (`-0`), and tracing (`-t`).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    env, eprintln, fs,
    io::{self, Read},
    process::{Command, ExitCode},
};
use user_program::cli::cli_args;

cli_args! {
    /// Run COMMAND with items read from standard input appended as arguments.
    pub struct XargsArgs {
        /// Input items are separated by NUL, not whitespace; quotes are literal.
        pub null:     bool           = ["-0", "--null"],
        /// Do not run the command if there are no input items.
        pub no_run:   bool           = ["-r", "--no-run-if-empty"],
        /// Print each command line on stderr before running it.
        pub trace:    bool           = ["-t", "--verbose"],
        /// Use at most MAX items per command line.
        pub max_args: Option<String> = ["-n", "--max-args"] @ "MAX",
        /// Use at most MAX input lines per command line.
        pub max_lines: Option<String> = ["-L", "--max-lines"] @ "MAX",
        /// Replace occurrences of REPL in the command with one input line.
        pub replace:  Option<String> = ["-I", "--replace"] @ "REPL",
        /// Command and its initial arguments (default: echo).
        pub command:  Vec<String>    = [..] @ "COMMAND",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = XargsArgs::parse_env_or_exit();

    let max_args = match parse_positive(cli.max_args.as_deref(), "-n") {
        Ok(value) => value,
        Err(code) => return code,
    };
    let max_lines = match parse_positive(cli.max_lines.as_deref(), "-L") {
        Ok(value) => value,
        Err(code) => return code,
    };

    // The base command: everything before the input items. Defaults to echo.
    let (program, base_args) = match cli.command.split_first() {
        Some((program, rest)) => (program.clone(), rest.to_vec()),
        None => ("echo".to_string(), Vec::new()),
    };

    let input = match read_input() {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("xargs: {}", err);
            return ExitCode::FAILURE;
        }
    };

    // `-I` splits input by line and replaces a token; otherwise items are
    // split by NUL (`-0`) or by shell-like whitespace with quote handling.
    let batches = if let Some(token) = cli.replace.as_deref() {
        replace_batches(&input, &base_args, token)
    } else {
        let items = if cli.null {
            split_nul(&input)
        } else {
            split_whitespace(&input)
        };
        item_batches(items, &base_args, max_args, max_lines)
    };

    if batches.is_empty() {
        if cli.replace.is_some() || cli.no_run {
            return ExitCode::SUCCESS;
        }
        // With no input and no `-r`, GNU xargs still runs the command once.
        return run_one(&program, &base_args, cli.trace);
    }

    let mut exit = ExitCode::SUCCESS;
    for args in &batches {
        let code = run_one(&program, args, cli.trace);
        if code != ExitCode::SUCCESS {
            exit = code;
        }
    }
    exit
}

/// Resolves `program` via `$PATH` and runs it with `args`, returning the
/// exit code xargs should propagate (123 if the command failed, 127 if it
/// could not be found or started).
fn run_one(program: &str, args: &[String], trace: bool) -> ExitCode {
    let Some(path) = lookup_in_path(program) else {
        eprintln!("xargs: {}: command not found", program);
        return ExitCode::from(127);
    };

    if trace {
        let mut line = program.to_string();
        for arg in args {
            line.push(' ');
            line.push_str(arg);
        }
        eprintln!("{}", line);
    }

    match Command::new(path).arg0(program).args(args).status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(123),
        Err(err) => {
            eprintln!("xargs: {}: {}", program, err);
            ExitCode::from(127)
        }
    }
}

/// Groups freestanding `items` into command-argument batches, honouring the
/// `-n` (max items) and `-L` (max lines) limits. With neither limit set all
/// items go into a single batch. Each batch is prefixed with `base_args`.
fn item_batches(
    items: Vec<String>,
    base_args: &[String],
    max_args: Option<usize>,
    max_lines: Option<usize>,
) -> Vec<Vec<String>> {
    let mut batches = Vec::new();
    if items.is_empty() {
        return batches;
    }

    // `-L` caps items per batch as well (each NUL/whitespace item counts as a
    // line for our purposes, matching how the items were split).
    let limit = match (max_args, max_lines) {
        (Some(a), Some(l)) => Some(a.min(l)),
        (Some(a), None) => Some(a),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    };

    match limit {
        Some(limit) => {
            for chunk in items.chunks(limit) {
                let mut args = base_args.to_vec();
                args.extend_from_slice(chunk);
                batches.push(args);
            }
        }
        None => {
            let mut args = base_args.to_vec();
            args.extend(items);
            batches.push(args);
        }
    }
    batches
}

/// Builds one batch per input line, substituting every occurrence of `token`
/// in `base_args` (and, if absent there, appending the line) — the `-I` mode.
fn replace_batches(input: &[u8], base_args: &[String], token: &str) -> Vec<Vec<String>> {
    let mut batches = Vec::new();
    for line in split_lines(input) {
        let mut args: Vec<String> = base_args
            .iter()
            .map(|arg| arg.replace(token, &line))
            .collect();
        if !base_args.iter().any(|arg| arg.contains(token)) {
            args.push(line);
        }
        batches.push(args);
    }
    batches
}

/// Reads all of stdin into a byte buffer.
fn read_input() -> Result<Vec<u8>, io::Error> {
    let mut buf = Vec::new();
    io::stdin().read_to_end(&mut buf)?;
    Ok(buf)
}

/// Splits input on NUL bytes, dropping a trailing empty item.
fn split_nul(input: &[u8]) -> Vec<String> {
    input
        .split(|&b| b == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect()
}

/// Splits input into non-empty lines (trailing newline ignored), used by `-I`.
fn split_lines(input: &[u8]) -> Vec<String> {
    input
        .split(|&b| b == b'\n')
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect()
}

/// Splits input into items on unquoted whitespace, honouring single quotes,
/// double quotes, and backslash escapes the way GNU xargs does by default.
fn split_whitespace(input: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(input);
    let mut items = Vec::new();
    let mut current = String::new();
    let mut has_item = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if has_item {
                    items.push(core::mem::take(&mut current));
                    has_item = false;
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                    has_item = true;
                }
            }
            '\'' | '"' => {
                has_item = true;
                for inner in chars.by_ref() {
                    if inner == c {
                        break;
                    }
                    current.push(inner);
                }
            }
            other => {
                current.push(other);
                has_item = true;
            }
        }
    }
    if has_item {
        items.push(current);
    }
    items
}

/// Parses an optional positive-integer option value, reporting a usage error
/// as the returned [`ExitCode`] on failure.
fn parse_positive(value: Option<&str>, flag: &str) -> Result<Option<usize>, ExitCode> {
    match value {
        None => Ok(None),
        Some(text) => match text.parse::<usize>() {
            Ok(0) | Err(_) => {
                eprintln!("xargs: invalid number for {}: {}", flag, text);
                Err(ExitCode::from(2))
            }
            Ok(n) => Ok(Some(n)),
        },
    }
}

/// Resolves a command name to a runnable path. Names containing `/` are used
/// as-is; bare names are searched in each `$PATH` element.
fn lookup_in_path(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if name.contains('/') {
        return is_file(name).then(|| name.to_string());
    }
    let path = env::var("PATH").unwrap_or_else(|_| "/bin:/usr/bin".to_string());
    for dir in path.split(':') {
        let dir = if dir.is_empty() { "." } else { dir };
        let candidate = if dir.ends_with('/') {
            alloc::format!("{}{}", dir, name)
        } else {
            alloc::format!("{}/{}", dir, name)
        };
        if is_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_file(path: &str) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}
