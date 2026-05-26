//! `unexpand` — convert spaces to tabs.
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
    pub struct UnexpandArgs {
        pub all:  bool           = ["-a", "--all"],
        pub tabs: Option<String> = ["-t"] @ "COLS",
        pub files: Vec<String>   = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = UnexpandArgs::parse_env_or_exit();
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
        unexpand(path, ts, cli.all, &mut out);
    }
    ExitCode::SUCCESS
}

fn unexpand(path: &str, ts: usize, all: bool, out: &mut io::Stdout) {
    let mut line = String::new();
    if path == "-" {
        let mut r = BufReader::new(io::stdin());
        while r.read_line(&mut line).unwrap_or(0) > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            convert(&line, ts, all, out);
            line.clear();
        }
    } else if let Ok(f) = File::open(path) {
        let mut r = BufReader::new(f);
        while r.read_line(&mut line).unwrap_or(0) > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            convert(&line, ts, all, out);
            line.clear();
        }
    }
}

fn convert(line: &str, ts: usize, all: bool, out: &mut io::Stdout) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut col = 0usize;
    let mut spaces = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b' ' {
            spaces += 1;
            col += 1;
            if col % ts == 0 && (all || spaces > 1) {
                // Replace the preceding spaces (including this one) with a tab.
                // Backtrack: remove the spaces we already emitted.
                let _ = out.write_all(b"\t");
                spaces = 0;
            }
        } else {
            // Flush any buffered spaces that weren't converted.
            for _ in 0..spaces {
                let _ = out.write_all(b" ");
            }
            spaces = 0;
            if b == b'\t' {
                col = ((col / ts) + 1) * ts;
            } else {
                col += 1;
            }
            let _ = out.write_all(&[b]);
        }
        i += 1;
    }
    for _ in 0..spaces {
        let _ = out.write_all(b" ");
    }
    let _ = out.write_all(b"\n");
}
