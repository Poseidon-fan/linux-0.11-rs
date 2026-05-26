//! `mkdir` — make directories.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use anyhow::{Context, Result, bail};
use user_lib::{eprintln, fs, io::ErrorKind, path::Path, process::ExitCode};
use user_program::cli::cli_args;

cli_args! {
    /// Create the DIRECTORY(ies), if they do not already exist.
    pub struct MkdirArgs {
        /// No error if existing, make parent directories as needed.
        pub parents: bool       = ["-p", "--parents"],
        /// Print a message for each created directory.
        pub verbose: bool       = ["-v", "--verbose"],
        /// Directories to create.
        pub dirs:    Vec<String> = [..] @ "DIRECTORY",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = MkdirArgs::parse_env_or_exit();
    if cli.dirs.is_empty() {
        eprintln!("mkdir: missing operand");
        eprintln!("Try 'mkdir --help' for more information.");
        return ExitCode::FAILURE;
    }

    let mut exit_code = ExitCode::SUCCESS;
    for dir in &cli.dirs {
        let result = if cli.parents && !cli.verbose {
            fs::create_dir_all(dir.as_str()).with_context(|| dir.to_string())
        } else if cli.parents {
            create_with_parents(dir, cli.verbose)
        } else {
            create_one(dir, cli.verbose, false)
        };
        if let Err(err) = result {
            eprintln!("mkdir: {:#}", err);
            exit_code = ExitCode::FAILURE;
        }
    }
    exit_code
}

fn create_one(path: &str, verbose: bool, idempotent: bool) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {
            if verbose {
                user_lib::println!("mkdir: created directory '{}'", path);
            }
            Ok(())
        }
        Err(err) if idempotent && err.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err).with_context(|| path.to_string()),
    }
}

/// `mkdir -p`: create every missing component along `path`.
fn create_with_parents(path: &str, verbose: bool) -> Result<()> {
    if path.is_empty() {
        bail!("cannot create directory '': No such file or directory");
    }

    let p = Path::new(path);
    if p.is_absolute() {
        create_chain("/", p.as_str().trim_start_matches('/'), verbose)?;
    } else {
        create_chain("", p.as_str(), verbose)?;
    }
    Ok(())
}

fn create_chain(prefix: &str, rest: &str, verbose: bool) -> Result<()> {
    let mut current = String::from(prefix);
    for component in rest.split('/') {
        if component.is_empty() {
            continue;
        }
        if !current.is_empty() && !current.ends_with('/') {
            current.push('/');
        }
        current.push_str(component);
        create_one(&current, verbose, true)?;
    }
    Ok(())
}
