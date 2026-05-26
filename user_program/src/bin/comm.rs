//! `comm` — compare two sorted files line by line.

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
    pub struct CommArgs {
        pub suppress1: bool = ["-1"], pub suppress2: bool = ["-2"], pub suppress3: bool = ["-3"],
        pub files: Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = CommArgs::parse_env_or_exit();
    if cli.files.len() != 2 {
        eprintln!("comm: need two files");
        return ExitCode::from(1);
    }
    let a = read_lines(&cli.files[0]).unwrap_or_default();
    let b = read_lines(&cli.files[1]).unwrap_or_default();
    let (mut i, mut j) = (0usize, 0usize);
    let mut out = io::stdout();
    let mut buf = String::new();
    use core::fmt::Write as _;
    while i < a.len() || j < b.len() {
        if i < a.len() && (j >= b.len() || a[i] < b[j]) {
            if !cli.suppress1 {
                let _ = writeln!(buf, "{}", a[i]);
            }
            i += 1;
        } else if j < b.len() && (i >= a.len() || b[j] < a[i]) {
            if !cli.suppress2 {
                let _ = writeln!(buf, "\t{}", b[j]);
            }
            j += 1;
        } else {
            if !cli.suppress3 {
                let _ = writeln!(buf, "\t\t{}", a[i]);
            }
            i += 1;
            j += 1;
        }
    }
    let _ = out.write_all(buf.as_bytes());
    ExitCode::SUCCESS
}

fn read_lines(path: &str) -> Result<Vec<String>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut lines = Vec::new();
    let mut line = String::new();
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
    Ok(lines)
}
