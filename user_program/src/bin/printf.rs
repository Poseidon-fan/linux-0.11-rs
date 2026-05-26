//! `printf` — format and print data.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use user_lib::{
    io::{self, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// FORMAT controls the output as in C printf.
    pub struct PrintfArgs {
        /// Format string and arguments.
        pub args: Vec<String> = [..] @ "ARG",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = PrintfArgs::parse_env_or_exit();
    if cli.args.is_empty() {
        return ExitCode::SUCCESS;
    }
    let fmt = &cli.args[0];
    let mut arg_idx = 1usize;
    let mut out_buf = String::new();
    let bytes = fmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'n' => out_buf.push('\n'),
                b't' => out_buf.push('\t'),
                b'r' => out_buf.push('\r'),
                b'\\' => out_buf.push('\\'),
                b'a' => out_buf.push('\x07'),
                b'b' => out_buf.push('\x08'),
                b'v' => out_buf.push('\x0b'),
                b'f' => out_buf.push('\x0c'),
                b'0' => {
                    let mut val = 0u32;
                    for _ in 0..3 {
                        if i + 1 < bytes.len() && (b'0'..=b'7').contains(&bytes[i + 1]) {
                            i += 1;
                            val = val * 8 + (bytes[i] - b'0') as u32;
                        } else {
                            break;
                        }
                    }
                    if let Some(c) = char::from_u32(val & 0xff) {
                        out_buf.push(c);
                    }
                }
                b'x' => {
                    let mut val = 0u32;
                    for _ in 0..2 {
                        if i + 1 < bytes.len() && bytes[i + 1].is_ascii_hexdigit() {
                            i += 1;
                            let d = bytes[i];
                            val = val * 16
                                + if d.is_ascii_digit() {
                                    d - b'0'
                                } else {
                                    d.to_ascii_lowercase() - b'a' + 10
                                } as u32;
                        } else {
                            break;
                        }
                    }
                    if let Some(c) = char::from_u32(val) {
                        out_buf.push(c);
                    }
                }
                _ => out_buf.push(bytes[i] as char),
            }
            i += 1;
        } else if bytes[i] == b'%' {
            i += 1;
            arg_idx += format_spec(&cli.args, &mut out_buf, bytes, &mut i, &mut arg_idx);
        } else {
            out_buf.push(bytes[i] as char);
            i += 1;
        }
    }
    let mut out = io::stdout();
    let _ = out.write_all(out_buf.as_bytes());
    ExitCode::SUCCESS
}

fn format_spec(
    args: &[String],
    out: &mut String,
    fmt: &[u8],
    i: &mut usize,
    arg_idx: &mut usize,
) -> usize {
    let val = if *arg_idx < args.len() {
        &args[*arg_idx]
    } else {
        ""
    };
    if *i >= fmt.len() {
        return 0;
    }
    match fmt[*i] {
        b's' => {
            out.push_str(val);
            *i += 1;
            1
        }
        b'd' | b'i' => {
            if let Ok(v) = val.parse::<i64>() {
                use core::fmt::Write as _;
                let _ = write!(out, "{}", v);
            }
            *i += 1;
            1
        }
        b'u' => {
            if let Ok(v) = val.parse::<u64>() {
                use core::fmt::Write as _;
                let _ = write!(out, "{}", v);
            }
            *i += 1;
            1
        }
        b'x' => {
            if let Ok(v) = val.parse::<u64>() {
                use core::fmt::Write as _;
                let _ = write!(out, "{:x}", v);
            }
            *i += 1;
            1
        }
        b'X' => {
            if let Ok(v) = val.parse::<u64>() {
                use core::fmt::Write as _;
                let _ = write!(out, "{:X}", v);
            }
            *i += 1;
            1
        }
        b'o' => {
            if let Ok(v) = val.parse::<u64>() {
                use core::fmt::Write as _;
                let _ = write!(out, "{:o}", v);
            }
            *i += 1;
            1
        }
        b'c' => {
            if let Some(c) = val.as_bytes().first() {
                out.push(*c as char);
            }
            *i += 1;
            1
        }
        b'%' => {
            out.push('%');
            *i += 1;
            0
        }
        _ => {
            *i += 1;
            0
        }
    }
}
