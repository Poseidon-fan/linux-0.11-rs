//! `tee` — read stdin and write to stdout and zero or more files.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use anyhow::{Context, Result, bail};
use user_lib::{
    eprintln,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
};
use user_program::cli::cli_args;

cli_args! {
    /// Copy standard input to each FILE, and also to standard output.
    pub struct TeeArgs {
        /// Append to the given FILEs, do not overwrite.
        pub append: bool       = ["-a", "--append"],
        /// Ignore the SIGINT signal. (Currently a no-op on this kernel.)
        pub ignore_interrupts: bool = ["-i", "--ignore-interrupts"],
        /// Files to write to.
        pub files:  Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> Result<()> {
    let cli = TeeArgs::parse_env_or_exit();
    let _ = cli.ignore_interrupts; // accepted for compatibility; signal API TBD

    // Open every output up front so we can report which paths failed before
    // we waste any input bytes.
    let mut sinks: Vec<File> = Vec::with_capacity(cli.files.len());
    let mut had_open_error = false;
    for path in &cli.files {
        let result = open_sink(path, cli.append);
        match result {
            Ok(file) => sinks.push(file),
            Err(err) => {
                eprintln!("tee: {:#}", err);
                had_open_error = true;
            }
        }
    }

    let mut stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut buf = [0u8; 1024];
    let mut had_write_error = false;
    loop {
        let n = stdin.read(&mut buf).context("reading stdin")?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        if let Err(err) = stdout.write_all(chunk) {
            eprintln!("tee: stdout: {}", err);
            had_write_error = true;
        }
        for (path, sink) in cli.files.iter().zip(sinks.iter_mut()) {
            if let Err(err) = sink.write_all(chunk) {
                eprintln!("tee: {}: {}", path, err);
                had_write_error = true;
            }
        }
    }

    if had_open_error || had_write_error {
        bail!("one or more outputs failed");
    }
    Ok(())
}

fn open_sink(path: &str, append: bool) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true);
    if append {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    opts.open(path).with_context(|| path.to_string())
}

#[allow(dead_code)]
fn _retain_imports() -> String {
    String::new()
}
