//! `dirname` — print the directory component of a path.

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
    /// Strip the last component from each NAME, defaulting to `.` if NAME
    /// contains no `/` characters.
    pub struct DirnameArgs {
        /// Terminate each line with NUL, not newline.
        pub zero:  bool       = ["-z", "--zero"],
        /// Names to process.
        pub names: Vec<String> = [..] @ "NAME",
    }
}

#[user_lib::main]
fn main() -> Result<()> {
    let cli = DirnameArgs::parse_env_or_exit();
    if cli.names.is_empty() {
        anyhow::bail!("missing operand");
    }

    let term: u8 = if cli.zero { 0 } else { b'\n' };
    let mut out = io::stdout();
    let mut buf = String::new();

    for name in &cli.names {
        buf.clear();
        buf.push_str(&dirname(name));
        out.write_all(buf.as_bytes())?;
        out.write_all(&[term])?;
    }
    Ok(())
}

/// POSIX dirname semantics: return everything before the final `/`,
/// or `.` if there is no slash, or `/` if input is `/`.
fn dirname(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let p = Path::new(path);
    match p.parent() {
        Some(parent) if parent.as_str().is_empty() => ".".into(),
        Some(parent) => parent.as_str().into(),
        None => {
            if path == "/" {
                "/".into()
            } else {
                ".".into()
            }
        }
    }
}
