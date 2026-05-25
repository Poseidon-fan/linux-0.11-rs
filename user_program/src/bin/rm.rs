//! `rm` — remove files or directories.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use user_lib::{
    eprintln, fs,
    io::{self, ErrorKind},
    println,
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Remove files or directories.
    pub struct RmArgs {
        /// Ignore nonexistent files and arguments, never prompt.
        pub force:     bool        = ["-f", "--force"],
        /// Remove directories and their contents recursively.
        pub recursive: bool        = ["-r", "-R", "--recursive"],
        /// Explain what is being done.
        pub verbose:   bool        = ["-v", "--verbose"],
        /// Files or directories to remove.
        pub paths:     Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let args = RmArgs::parse_env_or_exit();
    if args.paths.is_empty() {
        if !args.force {
            eprintln!("rm: missing operand");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    let mut exit_code = ExitCode::SUCCESS;
    for path in &args.paths {
        if is_dot_or_dotdot(path) {
            eprintln!("rm: refusing to remove '.' or '..' directory: {}", path);
            exit_code = ExitCode::FAILURE;
            continue;
        }

        match remove_path(path, &args) {
            Ok(()) => {}
            Err(err) if args.force && err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                eprintln!("rm: {}: {}", path, err);
                exit_code = ExitCode::FAILURE;
            }
        }
    }

    exit_code
}

/// Removes one path, dispatching to recursive directory removal when needed.
fn remove_path(path: &str, args: &RmArgs) -> io::Result<()> {
    let metadata = fs::metadata(path)?;
    if metadata.is_dir() {
        if !args.recursive {
            return Err(io::Error::from(ErrorKind::IsADirectory));
        }
        remove_dir_recursive(path, args)?;
    } else {
        fs::remove_file(path)?;
        if args.verbose {
            println!("removed '{}'", path);
        }
    }
    Ok(())
}

/// Removes every child in a directory before removing the directory itself.
fn remove_dir_recursive(path: &str, args: &RmArgs) -> io::Result<()> {
    let mut children = Vec::new();
    for item in fs::read_dir(path)? {
        let entry = item?;
        children.push(entry.path().into_string());
    }
    children.sort();

    for child in children {
        remove_path(child.as_str(), args)?;
    }

    fs::remove_dir(path)?;
    if args.verbose {
        println!("removed directory '{}'", path);
    }
    Ok(())
}

/// Detects arguments whose final raw component is `.` or `..`.
fn is_dot_or_dotdot(path: &str) -> bool {
    let trimmed = path.trim_end_matches('/');
    let component = trimmed
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed);
    component == "." || component == ".."
}
