//! `which` — locate a command.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use user_lib::{
    env, fs,
    io::{self, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Locate the executable file for each NAME in $PATH.
    pub struct WhichArgs {
        /// Names to search for.
        pub names: Vec<String> = [..] @ "NAME",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = WhichArgs::parse_env_or_exit();
    if cli.names.is_empty() {
        return ExitCode::from(1);
    }

    let path_var = env::var("PATH").unwrap_or_default();
    let mut all_found = true;
    let mut out = io::stdout();

    for name in &cli.names {
        if name.contains('/') {
            // Contains a slash — check directly.
            if is_executable(name) {
                let _ = writeln!(out, "{}", name);
            } else {
                all_found = false;
            }
        } else {
            let mut found = false;
            for dir in path_var.split(':') {
                let full = alloc::format!("{}/{}", dir, name);
                if is_executable(&full) {
                    let _ = writeln!(out, "{}", full);
                    found = true;
                    break;
                }
            }
            if !found {
                all_found = false;
            }
        }
    }
    if all_found {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn is_executable(path: &str) -> bool {
    if let Ok(meta) = fs::metadata(path) {
        if meta.is_file() {
            let mode = meta.mode();
            return mode & 0o111 != 0;
        }
    }
    false
}
