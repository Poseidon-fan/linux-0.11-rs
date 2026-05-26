//! `cat` — concatenate files (or stdin) to standard output.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::ToString, vec::Vec};

use anyhow::{Context, Result, bail};
use user_lib::{
    eprintln,
    fs::File,
    io::{self, Read, Write},
};
use user_program::cli::cli_args;

cli_args! {
    /// Concatenate FILE(s) to standard output. With no FILE, read stdin.
    pub struct CatArgs {
        /// Number all output lines.
        pub number: bool        = ["-n", "--number"],
        /// Files to read.
        pub files:  Vec<alloc::string::String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> Result<()> {
    let args = CatArgs::parse_env_or_exit();
    let mut state = CatState::new(args.number);
    let mut had_error = false;

    if args.files.is_empty() {
        state.cat_reader(&mut io::stdin()).context("stdin")?;
        return Ok(());
    }

    for path in &args.files {
        let result = if path == "-" {
            state
                .cat_reader(&mut io::stdin())
                .map_err(anyhow::Error::from)
        } else {
            cat_path(&mut state, path)
        };
        if let Err(err) = result {
            eprintln!("cat: {:#}", err);
            had_error = true;
        }
    }

    if had_error {
        bail!("one or more files failed");
    }
    Ok(())
}

/// Open + stream one named file. Context chain reads
/// `"<path>: <io error>"` when formatted with `{:#}`.
fn cat_path(state: &mut CatState, path: &str) -> Result<()> {
    let mut file = File::open(path).with_context(|| path.to_string())?;
    state
        .cat_reader(&mut file)
        .with_context(|| path.to_string())
}

struct CatState {
    number_lines: bool,
    next_line: u32,
    /// True between writes when the previous chunk did not end with `\n`.
    at_line_start: bool,
}

impl CatState {
    fn new(number_lines: bool) -> Self {
        Self {
            number_lines,
            next_line: 1,
            at_line_start: true,
        }
    }

    fn cat_reader<R: Read>(&mut self, reader: &mut R) -> io::Result<()> {
        if !self.number_lines {
            return io::copy(reader, &mut io::stdout()).map(|_| ());
        }
        let mut buf = [0u8; 1024];
        let mut stdout = io::stdout();
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            self.write_numbered(&mut stdout, &buf[..n])?;
        }
    }

    fn write_numbered<W: Write>(&mut self, out: &mut W, bytes: &[u8]) -> io::Result<()> {
        for &byte in bytes {
            if self.at_line_start {
                let mut number_buf = NumberBuf::default();
                let prefix = number_buf.format(self.next_line);
                out.write_all(prefix.as_bytes())?;
                self.next_line += 1;
                self.at_line_start = false;
            }
            out.write_all(core::slice::from_ref(&byte))?;
            if byte == b'\n' {
                self.at_line_start = true;
            }
        }
        Ok(())
    }
}

/// Stack-allocated formatter that produces `"   123\t"` style prefixes
/// without heap allocation.
#[derive(Default)]
struct NumberBuf {
    buf: [u8; 16],
    len: usize,
}

impl NumberBuf {
    fn format(&mut self, n: u32) -> &str {
        let mut digits = [0u8; 10];
        let mut digit_count = 0;
        let mut v = n;
        if v == 0 {
            digits[0] = b'0';
            digit_count = 1;
        } else {
            while v != 0 {
                digits[digit_count] = b'0' + (v % 10) as u8;
                digit_count += 1;
                v /= 10;
            }
        }

        let width = digit_count.max(6);
        let pad = width - digit_count;

        self.len = 0;
        for _ in 0..pad {
            self.buf[self.len] = b' ';
            self.len += 1;
        }
        for i in (0..digit_count).rev() {
            self.buf[self.len] = digits[i];
            self.len += 1;
        }
        self.buf[self.len] = b'\t';
        self.len += 1;

        // SAFETY: all bytes pushed are ASCII (`' '`, `'\t'`, `b'0'..=b'9'`).
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }
}
