//! `printenv` — print all or part of the environment.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use user_lib::{
    env,
    io::{self, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Print values of environment variables.
    pub struct PrintenvArgs {
        /// End each output item with NUL, not newline.
        pub zero:  bool        = ["-0", "--null"],
        /// Variable names to print.
        pub names: Vec<String> = [..] @ "NAME",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let args = PrintenvArgs::parse_env_or_exit();
    let terminator = if args.zero { 0 } else { b'\n' };

    let mut out = io::stdout();
    let mut exit_code = ExitCode::SUCCESS;

    if args.names.is_empty() {
        for (name, value) in env::vars() {
            if write_env_pair(&mut out, name.as_str(), value.as_str(), terminator).is_err() {
                return ExitCode::FAILURE;
            }
        }
        return ExitCode::SUCCESS;
    }

    for name in &args.names {
        match env::var(name.as_str()) {
            Ok(value) => {
                if write_value(&mut out, value.as_str(), terminator).is_err() {
                    return ExitCode::FAILURE;
                }
            }
            Err(_) => exit_code = ExitCode::FAILURE,
        }
    }

    exit_code
}

/// Writes `NAME=VALUE` followed by the requested terminator.
fn write_env_pair(out: &mut impl Write, name: &str, value: &str, terminator: u8) -> io::Result<()> {
    out.write_all(name.as_bytes())?;
    out.write_all(b"=")?;
    write_value(out, value, terminator)
}

/// Writes a variable value followed by the requested terminator.
fn write_value(out: &mut impl Write, value: &str, terminator: u8) -> io::Result<()> {
    out.write_all(value.as_bytes())?;
    out.write_all(&[terminator])
}
