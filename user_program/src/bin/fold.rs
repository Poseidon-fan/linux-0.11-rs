//! `fold` — wrap each input line to fit in specified width.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{string::String, vec::Vec};

use user_lib::{
    io::{self, BufRead, BufReader, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! { pub struct FoldArgs { pub width: Option<String> = ["-w"] @ "COLS", pub files: Vec<String> = [..] @ "FILE" } }

#[user_lib::main]
fn main() -> ExitCode {
    let cli = FoldArgs::parse_env_or_exit();
    let w = cli
        .width
        .as_ref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(80)
        .max(1);
    let paths: Vec<&str> = if cli.files.is_empty() {
        alloc::vec!["-"]
    } else {
        cli.files.iter().map(String::as_str).collect()
    };
    let mut out = io::stdout();
    for path in &paths {
        fold_path(path, w, &mut out);
    }
    ExitCode::SUCCESS
}
fn fold_path(path: &str, w: usize, out: &mut io::Stdout) {
    let mut line = String::new();
    if path == "-" {
        let mut r = BufReader::new(io::stdin());
        while r.read_line(&mut line).unwrap_or(0) > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            fold_line(&line, w, out);
            line.clear();
        }
    } else if let Ok(f) = user_lib::fs::File::open(path) {
        let mut r = BufReader::new(f);
        while r.read_line(&mut line).unwrap_or(0) > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            fold_line(&line, w, out);
            line.clear();
        }
    }
}
fn fold_line(line: &str, w: usize, out: &mut io::Stdout) {
    let b = line.as_bytes();
    let mut p = 0;
    while p < b.len() {
        let e = (p + w).min(b.len());
        let _ = out.write_all(&b[p..e]);
        let _ = out.write_all(b"\n");
        p = e;
    }
}
