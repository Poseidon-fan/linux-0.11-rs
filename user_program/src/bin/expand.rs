//! `expand` — convert tabs to spaces.
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

cli_args! { pub struct ExpandArgs { pub tabs: Option<String> = ["-t"] @ "COLS", pub files: Vec<String> = [..] @ "FILE" } }

#[user_lib::main]
fn main() -> ExitCode {
    let cli = ExpandArgs::parse_env_or_exit();
    let ts: usize = cli
        .tabs
        .as_ref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8)
        .max(1);
    let paths: Vec<&str> = if cli.files.is_empty() {
        alloc::vec!["-"]
    } else {
        cli.files.iter().map(String::as_str).collect()
    };
    let mut out = io::stdout();
    for path in &paths {
        expand(path, ts, &mut out);
    }
    ExitCode::SUCCESS
}

fn expand(path: &str, ts: usize, out: &mut io::Stdout) {
    let mut line = String::new();
    if path == "-" {
        let mut r = BufReader::new(io::stdin());
        while r.read_line(&mut line).unwrap_or(0) > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            emit(&line, ts, out);
            line.clear();
        }
    } else if let Ok(f) = File::open(path) {
        let mut r = BufReader::new(f);
        while r.read_line(&mut line).unwrap_or(0) > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            emit(&line, ts, out);
            line.clear();
        }
    }
}
fn emit(line: &str, ts: usize, out: &mut io::Stdout) {
    let mut col = 0usize;
    for b in line.bytes() {
        if b == b'\t' {
            let n = ts - (col % ts);
            for _ in 0..n {
                let _ = out.write_all(b" ");
            }
            col += n;
        } else {
            let _ = out.write_all(&[b]);
            col += 1;
            if b > 127 {
                col += 1;
            }
        }
    }
    let _ = out.write_all(b"\n");
}
