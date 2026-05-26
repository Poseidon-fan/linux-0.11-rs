//! `od` — dump files in octal and other formats.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use anyhow::Result;
use user_lib::{
    eprintln,
    fs::File,
    io::{self, Read, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Dump files in octal and other formats.
    pub struct OdArgs {
        /// Output format: o(ctal), x(hex), d(ecimal), c(har), a(named char).
        pub format: Option<String> = ["-t", "--format"] @ "FMT",
        /// Skip BYTES bytes from the beginning.
        pub skip:   Option<String> = ["-j", "--skip-bytes"] @ "BYTES",
        /// Read at most BYTES bytes.
        pub count:  Option<String> = ["-N", "--read-bytes"] @ "BYTES",
        /// Files to read.
        pub files:  Vec<String>    = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = OdArgs::parse_env_or_exit();
    let fmt = cli.format.as_deref().unwrap_or("o");
    let skip: usize = cli
        .skip
        .as_ref()
        .and_then(|s| parse_usize_suffix(s))
        .unwrap_or(0);
    let limit: Option<usize> = cli.count.as_ref().and_then(|s| parse_usize_suffix(s));

    let mut had_error = false;
    let paths: Vec<&str> = if cli.files.is_empty() {
        alloc::vec!["-"]
    } else {
        cli.files.iter().map(String::as_str).collect()
    };
    let mut offset: usize = 0;
    for path in &paths {
        if let Err(err) = od(path, &mut offset, fmt, skip, limit) {
            eprintln!("od: {:#}", err);
            had_error = true;
        }
    }
    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn od(path: &str, offset: &mut usize, fmt: &str, skip: usize, limit: Option<usize>) -> Result<()> {
    let mut raw = Vec::new();
    if path == "-" {
        io::stdin().read_to_end(&mut raw)?;
    } else {
        File::open(path)?.read_to_end(&mut raw)?;
    }
    let data = if skip < raw.len() { &raw[skip..] } else { &[] };
    let data = if let Some(n) = limit {
        &data[..n.min(data.len())]
    } else {
        data
    };

    let out = &mut io::stdout();
    let mut buf = String::new();
    use core::fmt::Write as _;

    let bytes_per_line = 16usize;
    let mut i = 0;
    while i < data.len() {
        let _ = write!(buf, "{:07o} ", *offset + i);
        match fmt {
            "x" => {
                for j in 0..bytes_per_line {
                    if i + j < data.len() {
                        let _ = write!(buf, " {:02x}", data[i + j]);
                    } else {
                        buf.push_str("   ");
                    }
                }
                buf.push_str("  ");
                for j in 0..bytes_per_line {
                    if i + j < data.len() {
                        let b = data[i + j];
                        buf.push(if b.is_ascii_graphic() || b == b' ' {
                            b as char
                        } else {
                            '.'
                        });
                    }
                }
            }
            "c" => {
                for j in 0..bytes_per_line {
                    if i + j < data.len() {
                        let b = data[i + j];
                        match b {
                            b'\0' => buf.push_str(" \\0"),
                            b'\n' => buf.push_str(" \\n"),
                            b'\t' => buf.push_str(" \\t"),
                            b'\r' => buf.push_str(" \\r"),
                            _ if b.is_ascii_graphic() || b == b' ' => {
                                let _ = write!(buf, "  {}", b as char);
                            }
                            _ => {
                                let _ = write!(buf, " {:03o}", b);
                            }
                        }
                    }
                }
            }
            "d" => {
                let words = data[i..].chunks(2).take(8);
                for w in words {
                    let val = if w.len() == 2 {
                        u16::from_le_bytes([w[0], w[1]]) as u64
                    } else {
                        w[0] as u64
                    };
                    let _ = write!(buf, " {:>5}", val);
                }
            }
            _ => {
                // octal default
                for j in 0..bytes_per_line {
                    if i + j < data.len() {
                        let _ = write!(buf, " {:03o}", data[i + j]);
                    } else {
                        buf.push_str("    ");
                    }
                }
                buf.push_str("  ");
                for j in 0..bytes_per_line {
                    if i + j < data.len() {
                        let b = data[i + j];
                        buf.push(if b.is_ascii_graphic() || b == b' ' {
                            b as char
                        } else {
                            '.'
                        });
                    }
                }
            }
        }
        buf.push('\n');
        i += bytes_per_line;
    }
    let _ = writeln!(buf, "{:07o}", *offset + data.len());
    *offset += data.len();
    out.write_all(buf.as_bytes())?;
    Ok(())
}

fn parse_usize_suffix(s: &str) -> Option<usize> {
    let s = s.trim();
    let (num, mul): (u64, u64) = if let Some(r) = s.strip_suffix("KB") {
        (r.parse().ok()?, 1000)
    } else if let Some(r) = s.strip_suffix("K") {
        (r.parse().ok()?, 1024)
    } else if let Some(r) = s.strip_suffix("MB") {
        (r.parse().ok()?, 1000 * 1000)
    } else if let Some(r) = s.strip_suffix("M") {
        (r.parse().ok()?, 1024 * 1024)
    } else {
        (s.parse().ok()?, 1)
    };
    Some((num * mul) as usize)
}
