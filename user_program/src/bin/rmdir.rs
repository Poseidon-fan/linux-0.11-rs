//! `rmdir` — remove empty directories.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use user_lib::{eprintln, fs, println, process::ExitCode};
use user_program::cli::cli_args;

cli_args! {
    /// Remove empty DIRECTORY(ies).
    pub struct RmdirArgs {
        /// Remove DIRECTORY and its ancestors when they become empty.
        pub parents: bool       = ["-p", "--parents"],
        /// Output a diagnostic for every directory processed.
        pub verbose: bool       = ["-v", "--verbose"],
        /// Directories to remove.
        pub dirs:    Vec<String> = [..] @ "DIRECTORY",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let args = RmdirArgs::parse_env_or_exit();
    if args.dirs.is_empty() {
        eprintln!("rmdir: missing operand");
        return ExitCode::FAILURE;
    }

    let mut exit_code = ExitCode::SUCCESS;
    for dir in &args.dirs {
        let result = if args.parents {
            remove_with_parents(dir, args.verbose)
        } else {
            remove_one(dir, args.verbose)
        };
        if let Err(err) = result {
            eprintln!("rmdir: {}: {}", dir, err);
            exit_code = ExitCode::FAILURE;
        }
    }
    exit_code
}

/// Removes a directory and then walks upward while parents become empty.
fn remove_with_parents(path: &str, verbose: bool) -> user_lib::io::Result<()> {
    let mut current = user_lib::path::Path::new(path).to_path_buf();
    loop {
        remove_one(current.as_str(), verbose)?;
        let Some(parent) = current.parent() else {
            break;
        };
        let parent = parent.as_str();
        if parent.is_empty() || parent == "/" {
            break;
        }
        current = parent.into();
    }
    Ok(())
}

/// Removes exactly one empty directory.
fn remove_one(path: &str, verbose: bool) -> user_lib::io::Result<()> {
    fs::remove_dir(path)?;
    if verbose {
        println!("rmdir: removed directory '{}'", path);
    }
    Ok(())
}
