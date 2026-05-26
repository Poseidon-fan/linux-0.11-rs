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

cli_args! {
    pub struct NlArgs { pub files: Vec<String> = [..] @ "FILE" }
}

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
        let lines: Vec<String> = {
            let mut lines = Vec::new();
            let mut line = String::new();
            let mut reader = BufReader::new(File::open(*path).unwrap_or_else(|_| panic!("open")));
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line.ends_with('\n') {
                    line.pop();
                }
                lines.push(line.clone());
            }
            lines
        };
        for line in &lines {
            let _ = writeln!(buf, "{:>6}\t{}", n, line);
            n += 1;
        }
    }
    let _ = out.write_all(buf.as_bytes());
    ExitCode::SUCCESS
}
