//! `head` — output the first part of files.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use anyhow::Result;
use user_lib::{
    eprintln,
    fs::File,
    io::{self, BufRead, BufReader, Read, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Print the first 10 lines of each FILE to standard output. With more
    /// than one FILE, precede each with a header giving the file name. With
    /// no FILE, read standard input.
    pub struct HeadArgs {
        /// Print the first NUM lines instead of the first 10.
        pub lines: u32        = ["-n", "--lines"] @ "NUM" = 10,
        /// Print the first NUM bytes of each file.
        pub bytes: u32        = ["-c", "--bytes"] @ "NUM" = 0,
        /// Never print headers giving file names.
        pub quiet: bool       = ["-q", "--quiet"],
        /// Always print headers giving file names.
        pub verbose: bool     = ["-v", "--verbose"],
        /// Files to read.
        pub files: Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = HeadArgs::parse_env_or_exit();

    // -c and -n are mutually exclusive in this minimal impl; -c wins if both
    // were supplied (matches GNU head).
    let mode = if cli.bytes > 0 {
        Mode::Bytes(cli.bytes)
    } else {
        Mode::Lines(cli.lines)
    };

    let show_headers = match (cli.verbose, cli.quiet, cli.files.len()) {
        (true, _, _) => true,
        (_, true, _) => false,
        (_, _, n) => n > 1,
    };

    let mut had_error = false;

    if cli.files.is_empty() {
        if let Err(err) = head_reader(io::stdin(), mode) {
            eprintln!("head: {:#}", err);
            had_error = true;
        }
        return if had_error {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

    let mut first_emitted = false;
    for path in &cli.files {
        match File::open(path.as_str()) {
            Ok(file) => {
                if show_headers {
                    let mut out = io::stdout();
                    if first_emitted {
                        let _ = out.write_all(b"\n");
                    }
                    let _ = out.write_all(build_header(path).as_bytes());
                }
                first_emitted = true;
                if let Err(err) = head_reader(file, mode) {
                    eprintln!("head: {}: {}", path, err);
                    had_error = true;
                }
            }
            Err(err) => {
                eprintln!("head: {}: {}", path, err);
                had_error = true;
            }
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Lines(u32),
    Bytes(u32),
}

/// Streams `reader` through the configured mode to stdout. Takes ownership
/// of the reader so that `Lines` mode can wrap it in a `BufReader` without
/// dragging an extra lifetime around.
fn head_reader<R: Read>(reader: R, mode: Mode) -> Result<()> {
    let mut stdout = io::stdout();
    match mode {
        Mode::Bytes(limit) => {
            let mut reader = reader;
            let mut buf = [0u8; 1024];
            let mut remaining = limit;
            while remaining > 0 {
                let want = core::cmp::min(remaining, buf.len() as u32) as usize;
                let n = reader.read(&mut buf[..want])?;
                if n == 0 {
                    break;
                }
                stdout.write_all(&buf[..n])?;
                remaining -= n as u32;
            }
        }
        Mode::Lines(limit) => {
            if limit == 0 {
                return Ok(());
            }
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            let mut printed: u32 = 0;
            while printed < limit {
                line.clear();
                if reader.read_line(&mut line)? == 0 {
                    break;
                }
                stdout.write_all(line.as_bytes())?;
                printed += 1;
            }
        }
    }
    Ok(())
}

fn build_header(path: &str) -> String {
    let mut s = String::with_capacity(path.len() + 6);
    s.push_str("==> ");
    s.push_str(path);
    s.push_str(" <==\n");
    s
}
