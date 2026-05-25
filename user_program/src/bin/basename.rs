//! `basename` — print the last component of a path.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use anyhow::Result;
use user_lib::{
    io::{self, Write},
    path::Path,
};
use user_program::cli::cli_args;

cli_args! {
    /// Print NAME with any leading directory components removed.
    /// If specified, also remove a trailing SUFFIX.
    pub struct BasenameArgs {
        /// Support multiple arguments and treat each as a NAME.
        pub multiple: bool       = ["-a", "--multiple"],
        /// Remove a trailing SUFFIX from each NAME.
        pub suffix:   Option<String> = ["-s", "--suffix"] @ "SUFFIX",
        /// Terminate each line with NUL, not newline.
        pub zero:     bool       = ["-z", "--zero"],
        /// Names to process.
        pub names:    Vec<String> = [..] @ "NAME",
    }
}

#[user_lib::main]
fn main() -> Result<()> {
    let cli = BasenameArgs::parse_env_or_exit();

    // POSIX: bare `basename NAME` or `basename NAME SUFFIX`.
    // `-a` / `-s` enables multi-argument processing.
    let multi = cli.multiple || cli.suffix.is_some();
    if cli.names.is_empty() {
        anyhow::bail!("missing operand");
    }

    let (names, suffix): (&[String], Option<&str>) = if multi {
        (cli.names.as_slice(), cli.suffix.as_deref())
    } else {
        // POSIX two-arg form: second arg is the suffix.
        match cli.names.len() {
            1 => (&cli.names[..], None),
            2 => (&cli.names[..1], Some(cli.names[1].as_str())),
            _ => anyhow::bail!("extra operand: '{}'", cli.names[2]),
        }
    };

    let term: u8 = if cli.zero { 0 } else { b'\n' };
    let mut out = io::stdout();
    let mut buf = String::new();
    for name in names {
        buf.clear();
        buf.push_str(&strip_basename(name, suffix));
        out.write_all(buf.as_bytes())?;
        out.write_all(&[term])?;
    }
    Ok(())
}

/// Returns the basename of `path`, optionally with one trailing
/// `suffix` removed (but never the entire result).
fn strip_basename(path: &str, suffix: Option<&str>) -> String {
    let p = Path::new(path);
    let base = p
        .file_name()
        .unwrap_or(if p.as_str() == "/" { "/" } else { path });
    if let Some(suf) = suffix {
        if base != suf && base.ends_with(suf) {
            return base[..base.len() - suf.len()].into();
        }
    }
    base.into()
}
