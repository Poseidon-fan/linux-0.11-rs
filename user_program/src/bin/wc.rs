//! `wc` — print line, word, and byte counts for each file.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use anyhow::{Context, Result};
use user_lib::{
    eprintln,
    fs::File,
    io::{self, Read, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Print newline, word, and byte counts for each FILE, and a total line
    /// if more than one FILE is specified. With no FILE, read standard input.
    pub struct WcArgs {
        /// Print the newline counts.
        pub lines: bool        = ["-l", "--lines"],
        /// Print the word counts.
        pub words: bool        = ["-w", "--words"],
        /// Print the byte counts.
        pub bytes: bool        = ["-c", "--bytes"],
        /// Files to count.
        pub files: Vec<String> = [..] @ "FILE",
    }
}

#[derive(Clone, Copy, Default)]
struct Counts {
    lines: u32,
    words: u32,
    bytes: u32,
}

impl Counts {
    fn add(&mut self, other: &Counts) {
        self.lines += other.lines;
        self.words += other.words;
        self.bytes += other.bytes;
    }
}

/// Which counts the user wanted printed. Defaults to all three if no
/// `-l`/`-w`/`-c` flag is given (POSIX behaviour).
struct Mask {
    lines: bool,
    words: bool,
    bytes: bool,
}

impl Mask {
    fn from_cli(cli: &WcArgs) -> Self {
        if !cli.lines && !cli.words && !cli.bytes {
            Mask {
                lines: true,
                words: true,
                bytes: true,
            }
        } else {
            Mask {
                lines: cli.lines,
                words: cli.words,
                bytes: cli.bytes,
            }
        }
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = WcArgs::parse_env_or_exit();
    let mask = Mask::from_cli(&cli);
    let mut total = Counts::default();
    let mut had_error = false;

    if cli.files.is_empty() {
        match count_reader(&mut io::stdin()) {
            Ok(counts) => print_row(&mask, &counts, ""),
            Err(err) => {
                eprintln!("wc: {:#}", err);
                had_error = true;
            }
        }
    } else {
        for path in &cli.files {
            match count_file(path) {
                Ok(counts) => {
                    total.add(&counts);
                    print_row(&mask, &counts, path);
                }
                Err(err) => {
                    eprintln!("wc: {:#}", err);
                    had_error = true;
                }
            }
        }
        if cli.files.len() > 1 {
            print_row(&mask, &total, "total");
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn count_file(path: &str) -> Result<Counts> {
    let mut file = File::open(path).with_context(|| path.to_string())?;
    count_reader(&mut file).with_context(|| path.to_string())
}

fn count_reader<R: Read>(reader: &mut R) -> Result<Counts> {
    let mut buf = [0u8; 1024];
    let mut counts = Counts::default();
    let mut in_word = false;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        counts.bytes += n as u32;
        for &b in &buf[..n] {
            if b == b'\n' {
                counts.lines += 1;
            }
            if is_word_byte(b) {
                if !in_word {
                    counts.words += 1;
                    in_word = true;
                }
            } else {
                in_word = false;
            }
        }
    }
    Ok(counts)
}

fn is_word_byte(b: u8) -> bool {
    !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

fn print_row(mask: &Mask, counts: &Counts, label: &str) {
    let mut line = String::new();
    if mask.lines {
        push_count(&mut line, counts.lines);
    }
    if mask.words {
        push_count(&mut line, counts.words);
    }
    if mask.bytes {
        push_count(&mut line, counts.bytes);
    }
    if !label.is_empty() {
        line.push(' ');
        line.push_str(label);
    }
    line.push('\n');
    let _ = io::stdout().write_all(line.as_bytes());
}

fn push_count(line: &mut String, value: u32) {
    use core::fmt::Write as _;
    let _ = write!(line, "{:>7}", value);
}
