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
        pub ignore_case: bool = ["-i", "--ignore-case"],
        pub field1: Option<String> = ["-1"] @ "FIELD",
        pub field2: Option<String> = ["-2"] @ "FIELD",
        pub delim: Option<String>  = ["-t"] @ "CHAR",
        pub files: Vec<String> = [..] @ "FILE",
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

    let get_key = |line: &str, f: usize, d: u8| -> String {
        let parts: Vec<&str> = if d == b' ' {
            line.split(|c: char| c == ' ' || c == '\t')
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            line.split(d as char).collect()
        };
        let k = parts.get(f.saturating_sub(1)).unwrap_or(&"");
        if cli.ignore_case {
            k.to_ascii_lowercase()
        } else {
            k.to_string()
        }
    };

    let mut out = io::stdout();
    let mut buf = String::new();
    use core::fmt::Write as _;
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        let ka = get_key(&a[i], f1, delim);
        let kb = get_key(&b[j], f2, delim);
        if ka < kb {
            i += 1;
        } else if kb < ka {
            j += 1;
        } else {
            let _ = writeln!(buf, "{} {} {}", a[i], delim as char, b[j]);
            i += 1;
            j += 1;
        }
    }
    let _ = out.write_all(buf.as_bytes());
    ExitCode::SUCCESS
}

fn read_lines(path: &str) -> Result<Vec<String>> {
    let mut r = BufReader::new(File::open(path)?);
    let mut ls = Vec::new();
    let mut l = String::new();
    loop {
        l.clear();
        if r.read_line(&mut l)? == 0 {
            break;
        }
        if l.ends_with('\n') {
            l.pop();
        }
        ls.push(l.clone());
    }
    Ok(ls)
}
