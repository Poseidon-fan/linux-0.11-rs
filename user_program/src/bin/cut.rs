//! `cut` — remove sections from each line of files.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use anyhow::Result;
use user_lib::{
    eprintln,
    fs::File,
    io::{self, BufRead, BufReader, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Print selected parts of lines from each FILE to standard output.
    pub struct CutArgs {
        /// Select only these characters.
        pub chars:       Option<String> = ["-c", "--characters"] @ "LIST",
        /// Select only these fields; also print lines with no delimiter.
        pub fields:      Option<String> = ["-f", "--fields"] @ "LIST",
        /// Use DELIM instead of TAB for field delimiter.
        pub delimiter:   Option<String> = ["-d", "--delimiter"] @ "DELIM",
        /// Complement the set of selected bytes, characters or fields.
        pub complement:  bool           = ["--complement"],
        /// Files to read.
        pub files:       Vec<String>    = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = CutArgs::parse_env_or_exit();
    let delim = cli
        .delimiter
        .as_ref()
        .map(|s| s.as_bytes()[0])
        .unwrap_or(b'\t');

    let ranges: Vec<(usize, usize)> =
        match parse_ranges(cli.chars.as_deref().or(cli.fields.as_deref())) {
            Ok(r) => r,
            Err(msg) => {
                eprintln!("cut: {}", msg);
                return ExitCode::FAILURE;
            }
        };

    let mode_field = cli.fields.is_some();

    let mut had_error = false;
    let paths: Vec<&str> = if cli.files.is_empty() {
        alloc::vec!["-"]
    } else {
        cli.files.iter().map(String::as_str).collect()
    };
    for path in &paths {
        if let Err(err) = cut(path, &ranges, delim, mode_field, cli.complement) {
            eprintln!("cut: {:#}", err);
            had_error = true;
        }
    }
    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn cut(
    path: &str,
    ranges: &[(usize, usize)],
    delim: u8,
    field_mode: bool,
    complement: bool,
) -> Result<()> {
    let mut out = io::stdout();
    if path == "-" {
        let mut reader = BufReader::new(io::stdin());
        cut_reader(&mut reader, ranges, delim, field_mode, complement, &mut out)
    } else {
        let mut reader = BufReader::new(File::open(path)?);
        cut_reader(&mut reader, ranges, delim, field_mode, complement, &mut out)
    }
}

fn cut_reader<R: BufRead>(
    reader: &mut R,
    ranges: &[(usize, usize)],
    delim: u8,
    field_mode: bool,
    complement: bool,
    out: &mut io::Stdout,
) -> Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line.ends_with('\n') {
            line.pop();
        }

        let result = if field_mode {
            let fields: Vec<&str> = line.split(delim as char).collect();
            select(&fields, ranges, complement, delim)
        } else {
            let chars: Vec<&str> = line.split("").filter(|s| !s.is_empty()).collect();
            select(&chars, ranges, complement, 0)
        };
        if !result.is_empty() || !complement {
            out.write_all(result.as_bytes())?;
        }
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn select(items: &[&str], ranges: &[(usize, usize)], complement: bool, delim: u8) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        let idx = i + 1; // 1-based
        let in_range = ranges.iter().any(|&(lo, hi)| idx >= lo && idx <= hi);
        if in_range != complement {
            if !out.is_empty() && delim != 0 {
                out.push(delim as char);
            }
            out.push_str(item);
        }
    }
    out
}

fn parse_ranges(raw: Option<&str>) -> Result<Vec<(usize, usize)>, String> {
    let Some(raw) = raw else {
        return Err("no list specified".into());
    };
    let mut ranges = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(dash) = part.find('-') {
            let lo: usize = if dash == 0 {
                1
            } else {
                part[..dash]
                    .parse()
                    .map_err(|_| format!("invalid range: {}", part))?
            };
            let hi: usize = if dash == part.len() - 1 {
                usize::MAX
            } else {
                part[dash + 1..]
                    .parse()
                    .map_err(|_| format!("invalid range: {}", part))?
            };
            if lo == 0 {
                return Err("fields/characters are numbered from 1".into());
            }
            ranges.push((lo, hi));
        } else {
            let n: usize = part
                .parse()
                .map_err(|_| format!("invalid range: {}", part))?;
            if n == 0 {
                return Err("fields/characters are numbered from 1".into());
            }
            ranges.push((n, n));
        }
    }
    Ok(ranges)
}
