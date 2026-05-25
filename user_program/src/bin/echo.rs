//! `echo` — write arguments to standard output.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use anyhow::Result;
use user_lib::io::{self, Write};
use user_program::cli::cli_args;

cli_args! {
    /// Write the ARGS to standard output, separated by spaces.
    pub struct EchoArgs {
        /// Do not output the trailing newline.
        pub no_newline: bool       = ["-n"],
        /// Enable interpretation of backslash escapes (`\n`, `\t`, `\\`, `\0`, `\a`, `\b`, `\r`, `\v`, `\f`).
        pub escapes:    bool       = ["-e"],
        /// Disable interpretation of backslash escapes (default).
        pub no_escapes: bool       = ["-E"],
        /// Arguments to print.
        pub args:       Vec<String> = [..] @ "ARG",
    }
}

#[user_lib::main]
fn main() -> Result<()> {
    let cli = EchoArgs::parse_env_or_exit();
    let interpret = cli.escapes && !cli.no_escapes;

    let mut out = String::new();
    for (i, arg) in cli.args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if interpret {
            if let Some(stop) = decode_escapes(arg, &mut out) {
                // `\c` halts output entirely; never print a trailing newline.
                let _ = stop;
                io::stdout().write_all(out.as_bytes())?;
                return Ok(());
            }
        } else {
            out.push_str(arg);
        }
    }
    if !cli.no_newline {
        out.push('\n');
    }

    io::stdout().write_all(out.as_bytes())?;
    Ok(())
}

/// Expands backslash escapes per POSIX `echo -e`. Returns `Some(())` if a
/// `\c` was encountered (which means "stop and suppress newline").
fn decode_escapes(input: &str, out: &mut String) -> Option<()> {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'\\' || i + 1 >= bytes.len() {
            out.push(b as char);
            i += 1;
            continue;
        }
        let esc = bytes[i + 1];
        i += 2;
        match esc {
            b'\\' => out.push('\\'),
            b'a' => out.push('\x07'),
            b'b' => out.push('\x08'),
            b'c' => return Some(()),
            b'f' => out.push('\x0c'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'v' => out.push('\x0b'),
            b'0' => {
                // Up to 3 more octal digits.
                let mut value: u32 = 0;
                let mut count = 0;
                while count < 3 && i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                    value = value * 8 + (bytes[i] - b'0') as u32;
                    i += 1;
                    count += 1;
                }
                let ch = char::from_u32(value).unwrap_or('\u{FFFD}');
                out.push(ch);
            }
            other => {
                out.push('\\');
                out.push(other as char);
            }
        }
    }
    None
}
