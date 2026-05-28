//! `join` — join lines of two files on a common field.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use anyhow::Result;
use user_lib::{
    eprintln,
    fs::File,
    io::{self, BufRead, BufReader, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    pub struct JoinArgs {
        pub ignore_case: bool = ["-i"], pub field1: Option<String> = ["-1"] @ "F", pub field2: Option<String> = ["-2"] @ "F",
        pub delim: Option<String> = ["-t"] @ "C", pub files: Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = JoinArgs::parse_env_or_exit();
    if cli.files.len() != 2 {
        eprintln!("join: need two files");
        return ExitCode::from(1);
    }
    let f1: usize = cli
        .field1
        .as_ref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1);
    let f2: usize = cli
        .field2
        .as_ref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1);
    let delim = cli.delim.as_ref().map(|s| s.as_bytes()[0]).unwrap_or(b' ');
    let a = read_lines(&cli.files[0]).unwrap_or_default();
    let b = read_lines(&cli.files[1]).unwrap_or_default();
    let key = |line: &str, f: usize| -> String {
        let parts: Vec<&str> = if delim == b' ' {
            line.split(|c: char| c == ' ' || c == '\t')
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            line.split(delim as char).collect()
        };
        let k = parts.get(f.saturating_sub(1)).unwrap_or(&"");
        if cli.ignore_case {
            k.to_ascii_lowercase()
        } else {
            k.to_string()
        }
    };
    // Split a line on the active delimiter. Mirrors `key`'s logic so
    // we can take both the key field and the remaining fields out.
    let split_line = |line: &str| -> Vec<String> {
        if delim == b' ' {
            line.split(|c: char| c == ' ' || c == '\t')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        } else {
            line.split(delim as char).map(|s| s.to_string()).collect()
        }
    };
    let join_with_delim = |fields: &[String]| -> String {
        let sep = if delim == b' ' { ' ' } else { delim as char };
        let mut s = String::new();
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                s.push(sep);
            }
            s.push_str(f);
        }
        s
    };
    let mut out = io::stdout();
    let mut buf = String::new();
    use core::fmt::Write as _;
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        let ka = key(&a[i], f1);
        let kb = key(&b[j], f2);
        if ka < kb {
            i += 1;
        } else if kb < ka {
            j += 1;
        } else {
            // GNU join output: `<key> <a's other fields> <b's other fields>`.
            let parts_a = split_line(&a[i]);
            let parts_b = split_line(&b[j]);
            let rest_a: Vec<String> = parts_a
                .iter()
                .enumerate()
                .filter_map(|(k, s)| (k + 1 != f1).then(|| s.clone()))
                .collect();
            let rest_b: Vec<String> = parts_b
                .iter()
                .enumerate()
                .filter_map(|(k, s)| (k + 1 != f2).then(|| s.clone()))
                .collect();
            let mut row = alloc::vec![ka.clone()];
            row.extend(rest_a);
            row.extend(rest_b);
            let _ = writeln!(buf, "{}", join_with_delim(&row));
            i += 1;
            j += 1;
        }
    }
    let _ = out.write_all(buf.as_bytes());
    ExitCode::SUCCESS
}

fn read_lines(path: &str) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut line = String::new();
    if path == "-" {
        let mut r = BufReader::new(io::stdin());
        while r.read_line(&mut line)? > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            lines.push(line.clone());
            line.clear();
        }
    } else {
        let mut r = BufReader::new(File::open(path)?);
        while r.read_line(&mut line)? > 0 {
            if line.ends_with('\n') {
                line.pop();
            }
            lines.push(line.clone());
            line.clear();
        }
    }
    Ok(lines)
}
