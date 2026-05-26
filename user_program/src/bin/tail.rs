//! `tail` — output the last part of files.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    env, eprintln,
    fs::File,
    io::{self, Read, Write},
    process::{self, ExitCode},
};
use user_program::cli::{Arg, Error as CliError, Parser, cli_args};

cli_args! {
    /// Print the last 10 lines of each FILE to standard output. With more
    /// than one FILE, precede each with a header giving the file name. With
    /// no FILE, or when FILE is -, read standard input.
    pub struct TailArgs {
        /// Output the last NUM lines, or use +NUM to start at line NUM.
        pub lines:   Option<String> = ["-n", "--lines"] @ "NUM",
        /// Output the last NUM bytes, or use +NUM to start at byte NUM.
        pub bytes:   Option<String> = ["-c", "--bytes"] @ "NUM",
        /// Never output headers giving file names.
        pub quiet:   bool           = ["-q", "--quiet", "--silent"],
        /// Always output headers giving file names.
        pub verbose: bool           = ["-v", "--verbose"],
        /// Files to read.
        pub files:   Vec<String>    = [..] @ "FILE",
    }
}

#[derive(Clone, Copy)]
enum Mode {
    /// Select lines according to the parsed count specification.
    Lines(CountSpec),
    /// Select bytes according to the parsed count specification.
    Bytes(CountSpec),
}

#[derive(Clone, Copy)]
enum CountSpec {
    /// Output the last N units.
    FromEnd(u32),
    /// Output starting at one-based unit N.
    FromStart(u32),
}

struct Options {
    mode: Mode,
    quiet: bool,
    verbose: bool,
    files: Vec<String>,
}

#[user_lib::main]
fn main() -> ExitCode {
    let options = match parse_options() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("tail: {}", message);
            return ExitCode::FAILURE;
        }
    };

    let show_headers = match (options.verbose, options.quiet, options.files.len()) {
        (true, _, _) => true,
        (_, true, _) => false,
        (_, _, n) => n > 1,
    };

    let mut had_error = false;
    if options.files.is_empty() {
        if let Err(err) = tail_stdin(options.mode, show_headers, false) {
            eprintln!("tail: {}", err);
            had_error = true;
        }
    } else {
        let mut printed_any = false;
        for path in &options.files {
            let result = if path == "-" {
                tail_stdin(options.mode, show_headers, printed_any)
            } else {
                tail_file(path, options.mode, show_headers, printed_any)
            };
            if let Err(err) = result {
                eprintln!("tail: {}: {}", path, err);
                had_error = true;
            }
            printed_any = true;
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn parse_options() -> Result<Options, String> {
    match parse_options_inner() {
        Ok(options) => Ok(options),
        Err(err) => {
            err.print_with_hint(&user_program::cli::program_name());
            process::exit(2);
        }
    }
}

fn parse_options_inner() -> Result<Options, CliError> {
    let mut mode = Mode::Lines(CountSpec::FromEnd(10));
    let mut quiet = false;
    let mut verbose = false;
    let mut files = Vec::new();
    let mut parser = Parser::from_args(normalized_args());

    while let Some(arg) = parser.next_arg()? {
        match arg {
            Arg::Short('n') => mode = Mode::Lines(parse_count_arg(&mut parser)?),
            Arg::Short('c') => mode = Mode::Bytes(parse_count_arg(&mut parser)?),
            Arg::Short('q') => quiet = true,
            Arg::Short('v') => verbose = true,
            Arg::Short('h') => emit_and_exit(&TailArgs::usage()),
            Arg::Short(flag) => return Err(CliError::UnknownShort(flag)),
            Arg::Long(name) if name == "lines" => mode = Mode::Lines(parse_count_arg(&mut parser)?),
            Arg::Long(name) if name == "bytes" => mode = Mode::Bytes(parse_count_arg(&mut parser)?),
            Arg::Long(name) if name == "quiet" || name == "silent" => quiet = true,
            Arg::Long(name) if name == "verbose" => verbose = true,
            Arg::Long(name) if name == "help" => emit_and_exit(&TailArgs::usage()),
            Arg::Long(name) if name == "version" => emit_and_exit(&TailArgs::version_line()),
            Arg::Long(name) => return Err(CliError::UnknownLong(name)),
            Arg::Value(value) => files.push(value),
        }
    }

    Ok(Options {
        mode,
        quiet,
        verbose,
        files,
    })
}

fn normalized_args() -> Vec<String> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if let Some(first) = args.first().cloned() {
        if let Some(count) = first
            .strip_prefix('-')
            .filter(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        {
            args.remove(0);
            args.insert(0, count.to_string());
            args.insert(0, String::from("-n"));
        } else if first.starts_with('+')
            && first[1..].bytes().all(|b| b.is_ascii_digit())
            && first.len() > 1
        {
            let count = args.remove(0);
            args.insert(0, count);
            args.insert(0, String::from("-n"));
        }
    }
    args
}

fn parse_count_arg(parser: &mut Parser) -> Result<CountSpec, CliError> {
    let flag = parser.last_flag_name();
    let raw = parser.value()?;
    parse_count(raw.as_str()).ok_or(CliError::InvalidValue { flag, value: raw })
}

fn parse_count(raw: &str) -> Option<CountSpec> {
    let (origin, rest) = match raw.as_bytes().first().copied() {
        Some(b'+') => (CountOrigin::Start, &raw[1..]),
        Some(b'-') => (CountOrigin::End, &raw[1..]),
        _ => (CountOrigin::End, raw),
    };
    let count = parse_number_with_suffix(rest)?;
    Some(match origin {
        CountOrigin::Start => CountSpec::FromStart(count),
        CountOrigin::End => CountSpec::FromEnd(count),
    })
}

enum CountOrigin {
    Start,
    End,
}

fn parse_number_with_suffix(raw: &str) -> Option<u32> {
    let digit_len = raw
        .bytes()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(raw.len());
    if digit_len == 0 {
        return None;
    }
    let number = raw[..digit_len].parse::<u32>().ok()?;
    let multiplier = match &raw[digit_len..] {
        "" => 1,
        "b" => 512,
        "kB" => 1_000,
        "K" | "KiB" => 1_024,
        "MB" => 1_000_000,
        "M" | "MiB" => 1_048_576,
        "GB" => 1_000_000_000,
        "G" | "GiB" => 1_073_741_824,
        _ => return None,
    };
    number.checked_mul(multiplier)
}

fn tail_file(path: &str, mode: Mode, show_header: bool, after_first: bool) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::with_capacity(
        file.metadata()
            .map(|m| usize::try_from(m.len()).unwrap_or(0))
            .unwrap_or(0),
    );
    file.read_to_end(&mut bytes)?;
    write_tail(path, &bytes, mode, show_header, after_first)
}

fn tail_stdin(mode: Mode, show_header: bool, after_first: bool) -> io::Result<()> {
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes)?;
    write_tail("standard input", &bytes, mode, show_header, after_first)
}

fn write_tail(
    label: &str,
    bytes: &[u8],
    mode: Mode,
    show_header: bool,
    after_first: bool,
) -> io::Result<()> {
    let mut out = io::stdout();
    if show_header {
        if after_first {
            out.write_all(b"\n")?;
        }
        out.write_all(build_header(label).as_bytes())?;
    }
    let start = match mode {
        Mode::Lines(spec) => line_start(bytes, spec),
        Mode::Bytes(spec) => byte_start(bytes, spec),
    };
    out.write_all(&bytes[start..])
}

fn byte_start(bytes: &[u8], spec: CountSpec) -> usize {
    match spec {
        CountSpec::FromEnd(count) => bytes.len().saturating_sub(count as usize),
        CountSpec::FromStart(count) => count.saturating_sub(1).min(bytes.len() as u32) as usize,
    }
}

fn line_start(bytes: &[u8], spec: CountSpec) -> usize {
    match spec {
        CountSpec::FromEnd(0) => bytes.len(),
        CountSpec::FromEnd(count) => last_lines_start(bytes, count),
        CountSpec::FromStart(count) => first_line_start(bytes, count),
    }
}

fn last_lines_start(bytes: &[u8], count: u32) -> usize {
    let mut seen = 0;
    let mut end = bytes.len();
    if bytes.last().copied() == Some(b'\n') {
        end = end.saturating_sub(1);
    }

    for index in (0..end).rev() {
        if bytes[index] == b'\n' {
            seen += 1;
            if seen == count {
                return index + 1;
            }
        }
    }
    0
}

fn first_line_start(bytes: &[u8], count: u32) -> usize {
    if count <= 1 {
        return 0;
    }

    let mut skipped = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            skipped += 1;
            if skipped == count - 1 {
                return index + 1;
            }
        }
    }
    bytes.len()
}

fn build_header(path: &str) -> String {
    format!("==> {} <==\n", path)
}

fn emit_and_exit(text: &str) -> ! {
    let mut out = io::stdout();
    let _ = out.write_all(text.as_bytes());
    if !text.ends_with('\n') {
        let _ = out.write_all(b"\n");
    }
    process::exit(0)
}
