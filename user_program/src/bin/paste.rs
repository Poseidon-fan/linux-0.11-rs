//! `paste` — merge lines of files.

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
    /// Write lines consisting of the sequentially corresponding lines from
    /// each FILE, separated by TABs, to standard output.
    pub struct PasteArgs {
        /// Reuse characters from DELIM-LIST instead of TABs.
        pub delimiters: Option<String> = ["-d", "--delimiters"] @ "LIST",
        /// Files to merge.
        pub files:      Vec<String>    = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = PasteArgs::parse_env_or_exit();
    let delims: Vec<u8> = cli
        .delimiters
        .as_ref()
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_else(|| alloc::vec![b'\t']);

    let path_list: Vec<&str> = if cli.files.is_empty() {
        alloc::vec!["-"]
    } else {
        cli.files.iter().map(String::as_str).collect()
    };

    let mut had_error = false;
    let mut file_lines: Vec<Vec<String>> = Vec::new();
    for path in &path_list {
        match read_lines(path) {
            Ok(lines) => file_lines.push(lines),
            Err(err) => {
                eprintln!("paste: {:#}", err);
                had_error = true;
            }
        }
    }

    if file_lines.is_empty() {
        return ExitCode::FAILURE;
    }

    let max_lines = file_lines.iter().map(|v| v.len()).max().unwrap_or(0);
    let mut out = io::stdout();
    for row in 0..max_lines {
        let mut line = String::new();
        for (fi, lines) in file_lines.iter().enumerate() {
            if fi > 0 {
                let d = delims[fi.saturating_sub(1) % delims.len()];
                line.push(d as char);
            }
            if row < lines.len() {
                line.push_str(&lines[row]);
            }
        }
        line.push('\n');
        let _ = out.write_all(line.as_bytes());
    }
    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn read_lines(path: &str) -> Result<Vec<String>> {
    let mut reader: BufReader<File> = BufReader::new(File::open(path)?);
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
