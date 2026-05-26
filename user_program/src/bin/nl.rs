//! `nl` — number lines of files.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{string::String, vec::Vec};

use user_lib::{
    fs::File,
    io::{self, BufRead, BufReader, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! { pub struct NlArgs { pub files: Vec<String> = [..] @ "FILE" } }

#[user_lib::main]
fn main() -> ExitCode {
    let cli = NlArgs::parse_env_or_exit();
    let paths: Vec<&str> = if cli.files.is_empty() {
        alloc::vec!["-"]
    } else {
        cli.files.iter().map(String::as_str).collect()
    };
    let mut out = io::stdout();
    let mut buf = String::new();
    let mut n = 1usize;
    use core::fmt::Write as _;
    for path in &paths {
        if let Some(lines) = read_lines(path) {
            for line in &lines {
                let _ = writeln!(buf, "{:>6}\t{}", n, line);
                n += 1;
            }
        }
    }
    let _ = out.write_all(buf.as_bytes());
    ExitCode::SUCCESS
}
fn read_lines(path: &str) -> Option<Vec<String>> {
    let mut lines = Vec::new();
    let mut line = String::new();
    if path == "-" {
        let mut r = BufReader::new(io::stdin());
        while r.read_line(&mut line).ok()? > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            lines.push(line.clone());
            line.clear();
        }
    } else {
        let mut r = BufReader::new(File::open(path).ok()?);
        while r.read_line(&mut line).ok()? > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            lines.push(line.clone());
            line.clear();
        }
    }
    Some(lines)
}
