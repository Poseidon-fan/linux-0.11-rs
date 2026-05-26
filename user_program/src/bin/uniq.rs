//! `uniq` — report or omit repeated lines.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use anyhow::Result;
use user_lib::{
    eprintln,
    fs::File,
    io::{self, BufRead, BufReader, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Filter adjacent matching lines from INPUT (or stdin), writing to stdout.
    pub struct UniqArgs {
        /// Prefix lines by the number of occurrences.
        pub count:          bool           = ["-c", "--count"],
        /// Only print duplicate lines.
        pub repeated:       bool           = ["-d", "--repeated"],
        /// Only print unique lines.
        pub unique:         bool           = ["-u", "--unique"],
        /// Input and optional output files.
        pub files:          Vec<String>    = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = UniqArgs::parse_env_or_exit();
    if cli.repeated && cli.unique {
        eprintln!("uniq: -d and -u are mutually exclusive");
        return ExitCode::from(1);
    }

    let input = cli.files.first().map(String::as_str).unwrap_or("-");
    let mut lines = match read_lines(input) {
        Ok(l) => l,
        Err(err) => {
            eprintln!("uniq: {:#}", err);
            return ExitCode::FAILURE;
        }
    };

    let mut out = io::stdout();
    let mut i = 0;
    while i < lines.len() {
        let mut count = 1usize;
        while i + count < lines.len() && lines[i + count] == lines[i] {
            count += 1;
        }
        let show = if cli.repeated {
            count > 1
        } else if cli.unique {
            count == 1
        } else {
            true
        };
        if show {
            let mut line_buf = String::new();
            use core::fmt::Write as _;
            if cli.count {
                let _ = write!(line_buf, "{:>7} {}", count, lines[i]);
            } else {
                line_buf.push_str(&lines[i]);
            }
            line_buf.push('\n');
            let _ = out.write_all(line_buf.as_bytes());
        }
        i += count as usize;
    }
    ExitCode::SUCCESS
}

fn read_lines(path: &str) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut line = String::new();
    if path == "-" {
        let mut reader = BufReader::new(io::stdin());
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            if line.ends_with('\n') {
                line.pop();
            }
            lines.push(line.clone());
        }
    } else {
        let mut reader = BufReader::new(File::open(path)?);
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            if line.ends_with('\n') {
                line.pop();
            }
            lines.push(line.clone());
        }
    }
    Ok(lines)
}
