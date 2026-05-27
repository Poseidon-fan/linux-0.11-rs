//! Command registry and dispatch.
//!
//! One module per command. Each command provides:
//!
//! - `pub const NAME: &str` — primary name used at the prompt.
//! - `pub const ALIASES: &[&str]` — optional alternate spellings.
//! - `pub const SUMMARY: &str` — one-line help text.
//! - `pub const USAGE: &str` — usage string shown by `help <cmd>`.
//! - `pub fn run(session: &mut Session, args: &[String]) -> Result<()>`
//!
//! [`COMMANDS`] is the static table consulted by [`dispatch`].

use std::fmt;

use anyhow::Result;

use crate::session::Session;

mod cat;
mod cd;
mod chmod;
mod clear;
mod cp;
mod diff;
mod edit;
mod exit;
mod fsck;
mod get;
mod help;
mod info;
mod lcd;
mod link;
mod lls;
mod lpwd;
mod ls;
mod mkdir;
mod mknod;
mod mv;
mod put;
mod pwd;
mod rm;
mod rmdir;
mod stat;
mod sync;
mod touch;
mod tree;

/// One entry in the command table.
pub struct Command {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
    pub usage: &'static str,
    pub run: fn(&mut Session, &[String]) -> Result<()>,
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Command").field("name", &self.name).finish()
    }
}

/// Builds the per-module `Command` entry by pulling each field out of
/// the named sub-module. Keeps the static command table free of dozens
/// of identical 5-line repetitions.
macro_rules! cmd {
    ($module:ident) => {
        Command {
            name: $module::NAME,
            aliases: $module::ALIASES,
            summary: $module::SUMMARY,
            usage: $module::USAGE,
            run: $module::run,
        }
    };
}

/// All commands the shell knows. Order is alphabetical by primary name.
pub const COMMANDS: &[Command] = &[
    cmd!(cat),
    cmd!(cd),
    cmd!(chmod),
    cmd!(clear),
    cmd!(cp),
    cmd!(diff),
    cmd!(edit),
    cmd!(exit),
    cmd!(fsck),
    cmd!(get),
    cmd!(help),
    cmd!(info),
    cmd!(lcd),
    cmd!(link),
    cmd!(lls),
    cmd!(lpwd),
    cmd!(ls),
    cmd!(mkdir),
    cmd!(mknod),
    cmd!(mv),
    cmd!(put),
    cmd!(pwd),
    cmd!(rm),
    cmd!(rmdir),
    cmd!(stat),
    cmd!(sync),
    cmd!(touch),
    cmd!(tree),
];

/// Resolves a typed command word to the static entry in [`COMMANDS`].
pub fn lookup(name: &str) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))
}

/// Outcome of one user line.
pub enum Outcome {
    /// Command finished normally (with or without side-effects).
    Continue,
    /// `exit` / `quit` — leave the REPL.
    Quit,
}

/// Parses `line`, dispatches to the matching command, and converts any
/// `Err` into a printed message so the REPL never aborts on a routine
/// command failure.
pub fn dispatch(session: &mut Session, line: &str) -> Outcome {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Outcome::Continue;
    }

    let tokens = match crate::parser::tokenize(trimmed) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("miniximg: parse error: {err}");
            return Outcome::Continue;
        }
    };
    let (name, args) = tokens.split_first().expect("non-empty after trim");

    let cmd = match lookup(name) {
        Some(c) => c,
        None => {
            eprintln!("miniximg: unknown command: {name} (try `help`)");
            return Outcome::Continue;
        }
    };

    match (cmd.run)(session, args) {
        Ok(()) => {
            // Some commands (notably `exit`) signal through an Err with a
            // sentinel message; we keep dispatch simple and check the
            // command name instead.
            if cmd.name == exit::NAME {
                Outcome::Quit
            } else {
                Outcome::Continue
            }
        }
        Err(err) => {
            eprintln!("{name}: {err:#}");
            Outcome::Continue
        }
    }
}

/// Argument helper: errors out when a command is called with too few or
/// too many positional arguments. Returns the slice unchanged on success.
pub fn expect_args(args: &[String], min: usize, max: Option<usize>) -> Result<&[String]> {
    if args.len() < min {
        anyhow::bail!("missing argument(s)");
    }
    if let Some(max) = max
        && args.len() > max
    {
        anyhow::bail!("too many arguments");
    }
    Ok(args)
}

// ---------------------------------------------------------------------------
// Shared filesystem helpers
// ---------------------------------------------------------------------------

/// Seconds since the Unix epoch, clamped to `u32` to match the Minix
/// on-disk timestamp width. Returns `0` if the system clock is set before
/// 1970 — unlikely, but `expect`/`panic` here would be silly.
pub fn now_secs() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

/// Builds default [`CreateNodeOptions`] for a node owned by root with the
/// given permission bits and the current time as its mtime. Most write
/// paths want the same shape; the `mode` parameter is the only thing
/// that changes per call site.
pub fn default_node_options(mode: u16) -> miniximg::CreateNodeOptions {
    miniximg::CreateNodeOptions {
        mode,
        uid: 0,
        gid: 0,
        mtime: now_secs(),
    }
}

/// Default [`CreateNodeOptions`] for any parent directory we have to
/// implicitly create on the way to a write target.
pub fn default_parent_options() -> miniximg::CreateNodeOptions {
    default_node_options(0o755)
}
