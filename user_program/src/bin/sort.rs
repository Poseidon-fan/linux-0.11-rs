//! `sort` — sort lines of text.
//!
//! Supports the common GNU options including general numeric sort (`-g`),
//! which compares parsed floating-point values and so drives the x87 FPU.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::{cmp::Ordering, str::FromStr};

use user_lib::{
    eprintln,
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Write sorted concatenation of all FILE(s) to standard output.
    pub struct SortArgs {
        /// Compare according to string numerical value.
        pub numeric:    bool           = ["-n", "--numeric-sort"],
        /// Compare according to general numerical value (allows exponents).
        pub general:    bool           = ["-g", "--general-numeric-sort"],
        /// Reverse the result of comparisons.
        pub reverse:    bool           = ["-r", "--reverse"],
        /// Output only the first of an equal run.
        pub unique:     bool           = ["-u", "--unique"],
        /// Ignore leading blanks when finding sort keys.
        pub blanks:     bool           = ["-b", "--ignore-leading-blanks"],
        /// Fold lowercase to uppercase when comparing.
        pub fold_case:  bool           = ["-f", "--ignore-case"],
        /// Write result to FILE instead of standard output.
        pub output:     Option<String> = ["-o", "--output"] @ "FILE",
        /// Use SEP instead of whitespace runs as the field separator.
        pub separator:  Option<String> = ["-t", "--field-separator"] @ "SEP",
        /// Sort via a key; KEYDEF is F[.C][,F[.C]] (1-based, repeatable).
        pub keys:       Vec<String>    = ["-k", "--key"] @ "KEYDEF",
        /// Files to read; standard input if none or `-`.
        pub files:      Vec<String>    = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = SortArgs::parse_env_or_exit();

    if cli.numeric && cli.general {
        eprintln!("sort: -n and -g are mutually exclusive");
        return ExitCode::FAILURE;
    }

    let keys = match cli.keys.iter().map(|k| KeyDef::parse(k)).collect() {
        Ok(keys) => keys,
        Err(err) => {
            eprintln!("sort: {}", err);
            return ExitCode::FAILURE;
        }
    };
    let separator = cli.separator.as_ref().and_then(|s| s.bytes().next());

    let order = Order {
        mode: if cli.general {
            Mode::General
        } else if cli.numeric {
            Mode::Numeric
        } else {
            Mode::Lexical
        },
        reverse: cli.reverse,
        fold_case: cli.fold_case,
        blanks: cli.blanks,
        keys,
        separator,
    };

    let mut lines = match read_all(&cli.files) {
        Ok(lines) => lines,
        Err(err) => {
            eprintln!("sort: {}", err);
            return ExitCode::FAILURE;
        }
    };

    lines.sort_by(|a, b| order.compare(a, b));
    if cli.unique {
        lines.dedup_by(|a, b| order.compare_keys(a, b) == Ordering::Equal);
    }

    match write_all(cli.output.as_deref(), &lines) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("sort: {}", err);
            ExitCode::FAILURE
        }
    }
}

/// Resolved comparison configuration applied to every pair of lines.
struct Order {
    mode: Mode,
    reverse: bool,
    fold_case: bool,
    blanks: bool,
    keys: Vec<KeyDef>,
    separator: Option<u8>,
}

/// How key text is interpreted when comparing.
#[derive(Clone, Copy)]
enum Mode {
    Lexical,
    Numeric,
    General,
}

impl Order {
    /// Orders two whole lines: compares by keys, then breaks ties with a
    /// last-resort raw byte comparison of the full lines so the result is a
    /// total order. `-r` reverses the whole thing.
    fn compare(&self, a: &str, b: &str) -> Ordering {
        let ord = match self.compare_keys(a, b) {
            Ordering::Equal => a.cmp(b),
            ord => ord,
        };
        if self.reverse { ord.reverse() } else { ord }
    }

    /// Orders two lines by their sort keys alone (no last-resort fallback).
    /// This is what `-u` uses to decide whether two lines are duplicates.
    fn compare_keys(&self, a: &str, b: &str) -> Ordering {
        if self.keys.is_empty() {
            return self.compare_field(a, b);
        }
        for key in &self.keys {
            let ord =
                self.compare_field(key.slice(a, self.separator), key.slice(b, self.separator));
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    }

    /// Compares two key strings under the active mode.
    fn compare_field(&self, a: &str, b: &str) -> Ordering {
        let a = if self.blanks { a.trim_start() } else { a };
        let b = if self.blanks { b.trim_start() } else { b };
        match self.mode {
            Mode::Lexical => self.compare_lexical(a, b),
            Mode::Numeric => parse_numeric(a).total_cmp(&parse_numeric(b)),
            Mode::General => parse_general(a).total_cmp(&parse_general(b)),
        }
    }

    fn compare_lexical(&self, a: &str, b: &str) -> Ordering {
        if self.fold_case {
            let a = a.bytes().map(|c| c.to_ascii_uppercase());
            let b = b.bytes().map(|c| c.to_ascii_uppercase());
            a.cmp(b)
        } else {
            a.cmp(b)
        }
    }
}

/// A `-k` field selector: 1-based start/end field, optional character offsets.
struct KeyDef {
    start_field: usize,
    start_char: usize,
    end_field: Option<usize>,
    end_char: Option<usize>,
}

impl KeyDef {
    /// Parses `F[.C][,F[.C]]` into a [`KeyDef`] (1-based fields and chars).
    fn parse(spec: &str) -> Result<Self, &'static str> {
        let (start, end) = match spec.split_once(',') {
            Some((s, e)) => (s, Some(e)),
            None => (spec, None),
        };
        let (start_field, start_char) = parse_field_pos(start, 1)?;
        let (end_field, end_char) = match end {
            Some(e) => {
                let (f, c) = parse_field_pos(e, 0)?;
                (Some(f), if c == 0 { None } else { Some(c) })
            }
            None => (None, None),
        };
        Ok(Self {
            start_field,
            start_char,
            end_field,
            end_char,
        })
    }

    /// Extracts this key's substring from `line` given the field separator.
    fn slice<'a>(&self, line: &'a str, sep: Option<u8>) -> &'a str {
        let fields = split_fields(line, sep);
        if self.start_field == 0 || self.start_field > fields.len() {
            return "";
        }
        let (start_lo, _) = fields[self.start_field - 1];
        let begin = (start_lo + self.start_char.saturating_sub(1)).min(line.len());

        let end = match self.end_field {
            Some(0) | None => line.len(),
            Some(f) if f > fields.len() => line.len(),
            Some(f) => {
                let (lo, hi) = fields[f - 1];
                match self.end_char {
                    Some(c) => (lo + c).min(line.len()),
                    None => hi,
                }
            }
        };
        line.get(begin..end.max(begin)).unwrap_or("")
    }
}

/// Parses one `F[.C]` half of a key spec; `default_char` fills a missing `.C`.
fn parse_field_pos(spec: &str, default_char: usize) -> Result<(usize, usize), &'static str> {
    let (field, ch) = match spec.split_once('.') {
        Some((f, c)) => (f, Some(c)),
        None => (spec, None),
    };
    let field = field.parse::<usize>().map_err(|_| "invalid field number")?;
    let ch = match ch {
        Some(c) => c.parse::<usize>().map_err(|_| "invalid character offset")?,
        None => default_char,
    };
    Ok((field, ch))
}

/// Returns the `(start, end)` byte range of each field in `line`.
///
/// With an explicit separator, fields are the spans between separators. With
/// the default (whitespace) separator, each field begins at the transition
/// into non-blank text and includes the leading blank run, matching the way
/// the default sort key is taken.
fn split_fields(line: &str, sep: Option<u8>) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut fields = Vec::new();
    match sep {
        Some(sep) => {
            let mut start = 0;
            for (i, &b) in bytes.iter().enumerate() {
                if b == sep {
                    fields.push((start, i));
                    start = i + 1;
                }
            }
            fields.push((start, bytes.len()));
        }
        None => {
            let mut i = 0;
            while i < bytes.len() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                fields.push((start, i));
            }
        }
    }
    fields
}

/// Parses the leading numeric prefix of `s` (sign, digits, decimal point) as a
/// value for `-n`; unparsable or empty input compares as zero.
fn parse_numeric(s: &str) -> f64 {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    let mut end = 0;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let mut seen_dot = false;
    while end < bytes.len() {
        match bytes[end] {
            b'0'..=b'9' => end += 1,
            b'.' if !seen_dot => {
                seen_dot = true;
                end += 1;
            }
            _ => break,
        }
    }
    f64::from_str(&s[..end]).unwrap_or(0.0)
}

/// Parses the longest leading prefix of `s` accepted as a general number for
/// `-g` (including exponents, `inf`, `nan`); unparsable input compares as
/// negative infinity so it sorts before any real value.
fn parse_general(s: &str) -> f64 {
    let s = s.trim_start();
    let mut best = f64::NEG_INFINITY;
    // The longest parseable prefix wins, so `1.5e3x` parses as 1500.
    for (i, _) in s.char_indices().skip(1) {
        if let Ok(v) = f64::from_str(&s[..i]) {
            best = v;
        }
    }
    if let Ok(v) = f64::from_str(s) {
        best = v;
    }
    best
}

/// Reads every input path (or stdin for `-`/none) into a vector of lines with
/// trailing newlines stripped.
fn read_all(files: &[String]) -> Result<Vec<String>, io::Error> {
    let mut lines = Vec::new();
    if files.is_empty() {
        read_into(&mut BufReader::new(io::stdin()), &mut lines)?;
    } else {
        for path in files {
            if path == "-" {
                read_into(&mut BufReader::new(io::stdin()), &mut lines)?;
            } else {
                read_into(&mut BufReader::new(File::open(path)?), &mut lines)?;
            }
        }
    }
    Ok(lines)
}

fn read_into<R: BufRead>(reader: &mut R, lines: &mut Vec<String>) -> Result<(), io::Error> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line.ends_with('\n') {
            line.pop();
        }
        lines.push(core::mem::take(&mut line));
    }
    Ok(())
}

/// Writes each line followed by a newline to `-o FILE` or stdout.
fn write_all(output: Option<&str>, lines: &[String]) -> Result<(), io::Error> {
    match output {
        Some(path) => write_lines(BufWriter::new(File::create(path)?), lines),
        None => write_lines(BufWriter::new(io::stdout()), lines),
    }
}

fn write_lines<W: Write>(mut out: W, lines: &[String]) -> Result<(), io::Error> {
    for line in lines {
        out.write_all(line.as_bytes())?;
        out.write_all(b"\n")?;
    }
    out.flush()
}
