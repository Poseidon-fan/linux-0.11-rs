//! `touch` — change file timestamps.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    eprintln, fs,
    io::{self, ErrorKind, Write},
    process::{self, ExitCode},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use user_program::cli::{Arg, Error as CliError, Parser, program_name};

#[derive(Clone, Copy)]
enum TimeSource {
    Now,
    Explicit(SystemTime),
    Reference {
        accessed: SystemTime,
        modified: SystemTime,
    },
}

struct TouchOptions {
    change_accessed: bool,
    change_modified: bool,
    no_create: bool,
    source: TimeSource,
    files: Vec<String>,
}

#[user_lib::main]
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn run() -> Result<(), ExitCode> {
    let mut options = parse_args().map_err(|err| {
        err.print_with_hint(&program_name());
        ExitCode::from(1)
    })?;

    if options.files.is_empty() {
        eprintln!("touch: missing file operand");
        eprintln!("Try 'touch --help' for more information.");
        return Err(ExitCode::FAILURE);
    }

    if !options.change_accessed && !options.change_modified {
        options.change_accessed = true;
        options.change_modified = true;
    }

    let mut exit_code = ExitCode::SUCCESS;
    for file in &options.files {
        if let Err(err) = touch_one(file, &options) {
            eprintln!("touch: cannot touch '{}': {}", file, err);
            exit_code = ExitCode::FAILURE;
        }
    }

    if exit_code == ExitCode::SUCCESS {
        Ok(())
    } else {
        Err(exit_code)
    }
}

fn parse_args() -> Result<TouchOptions, CliError> {
    let mut parser = Parser::from_env();
    let mut options = TouchOptions {
        change_accessed: false,
        change_modified: false,
        no_create: false,
        source: TimeSource::Now,
        files: Vec::new(),
    };

    while let Some(arg) = parser.next_arg()? {
        match arg {
            Arg::Short('a') => options.change_accessed = true,
            Arg::Short('m') => options.change_modified = true,
            Arg::Short('c') => options.no_create = true,
            Arg::Short('f') => {}
            Arg::Short('r') => {
                let reference = parser.value()?;
                options.source =
                    reference_times(&reference).map_err(|_| CliError::InvalidValue {
                        flag: "-r".to_string(),
                        value: reference,
                    })?;
            }
            Arg::Short('t') => {
                let stamp = parser.value()?;
                options.source = TimeSource::Explicit(parse_touch_time(&stamp).map_err(|_| {
                    CliError::InvalidValue {
                        flag: "-t".to_string(),
                        value: stamp,
                    }
                })?);
            }
            Arg::Short('h') => emit_and_exit(usage(), 0),
            Arg::Short(flag) => return Err(CliError::UnknownShort(flag)),
            Arg::Long(name) if name == "no-create" => options.no_create = true,
            Arg::Long(name) if name == "reference" => {
                let reference = parser.value()?;
                options.source =
                    reference_times(&reference).map_err(|_| CliError::InvalidValue {
                        flag: "--reference".to_string(),
                        value: reference,
                    })?;
            }
            Arg::Long(name) if name == "time" => apply_time_word(&mut options, parser.value()?)?,
            Arg::Long(name) if name == "help" => emit_and_exit(usage(), 0),
            Arg::Long(name) if name == "version" => emit_and_exit(version_line(), 0),
            Arg::Long(name) => return Err(CliError::UnknownLong(name)),
            Arg::Value(value) => options.files.push(value),
        }
    }

    Ok(options)
}

fn apply_time_word(options: &mut TouchOptions, value: String) -> Result<(), CliError> {
    match value.as_str() {
        "access" | "atime" | "use" => options.change_accessed = true,
        "modify" | "mtime" => options.change_modified = true,
        _ => {
            return Err(CliError::InvalidValue {
                flag: "--time".to_string(),
                value,
            });
        }
    }
    Ok(())
}

fn reference_times(path: &str) -> io::Result<TimeSource> {
    let metadata = fs::metadata(path)?;
    Ok(TimeSource::Reference {
        accessed: metadata.accessed()?,
        modified: metadata.modified()?,
    })
}

fn touch_one(path: &str, options: &TouchOptions) -> io::Result<()> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            if options.no_create {
                return Ok(());
            }
            fs::File::create(path)?
        }
        Err(err) => return Err(err),
    };

    let (accessed, modified) = selected_times(options);
    let mut times = fs::FileTimes::new();
    if options.change_accessed {
        times = times.set_accessed(accessed);
    }
    if options.change_modified {
        times = times.set_modified(modified);
    }
    file.set_times(times)
}

fn selected_times(options: &TouchOptions) -> (SystemTime, SystemTime) {
    match options.source {
        TimeSource::Now => {
            let now = SystemTime::now();
            (now, now)
        }
        TimeSource::Explicit(time) => (time, time),
        TimeSource::Reference { accessed, modified } => (accessed, modified),
    }
}

fn parse_touch_time(raw: &str) -> Result<SystemTime, ()> {
    let (main, seconds) = match raw.find('.') {
        Some(dot) => (&raw[..dot], parse_fixed_digits(&raw[dot + 1..], 2)?),
        None => (raw, 0),
    };
    if !matches!(main.len(), 8 | 10 | 12) || !main.as_bytes().iter().all(u8::is_ascii_digit) {
        return Err(());
    }

    let (year, rest) = match main.len() {
        8 => (
            resolve_two_digit_year(parse_fixed_digits(&main[..2], 2)?),
            &main[2..],
        ),
        10 => (1900 + parse_fixed_digits(&main[..2], 2)?, &main[2..]),
        12 => (parse_fixed_digits(&main[..4], 4)?, &main[4..]),
        _ => return Err(()),
    };
    let month = parse_fixed_digits(&rest[..2], 2)?;
    let day = parse_fixed_digits(&rest[2..4], 2)?;
    let hour = parse_fixed_digits(&rest[4..6], 2)?;
    let minute = parse_fixed_digits(&rest[6..8], 2)?;

    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || seconds > 60
    {
        return Err(());
    }

    let unix = unix_seconds_utc(year, month, day, hour, minute, seconds)?;
    Ok(UNIX_EPOCH + Duration::from_secs(unix))
}

fn parse_fixed_digits(raw: &str, len: usize) -> Result<u32, ()> {
    if raw.len() != len || !raw.as_bytes().iter().all(u8::is_ascii_digit) {
        return Err(());
    }
    let mut value = 0u32;
    for byte in raw.bytes() {
        value = value * 10 + u32::from(byte - b'0');
    }
    Ok(value)
}

fn resolve_two_digit_year(year: u32) -> u32 {
    if year <= 68 { 2000 + year } else { 1900 + year }
}

fn unix_seconds_utc(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Result<u64, ()> {
    let days = days_from_civil(year as i64, month as i64, day as i64);
    if days < 0 {
        return Err(());
    }
    let seconds = days
        .checked_mul(86_400)
        .and_then(|s| s.checked_add(i64::from(hour) * 3600))
        .and_then(|s| s.checked_add(i64::from(minute) * 60))
        .and_then(|s| s.checked_add(i64::from(second)))
        .ok_or(())?;
    u64::try_from(seconds).map_err(|_| ())
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month_prime + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn usage() -> &'static str {
    "Usage: touch [OPTION]... FILE...\n\
     Update the access and modification times of each FILE to the current time.\n\
     \n\
     Options:\n\
     \x20 -a                     change only the access time\n\
     \x20 -c, --no-create        do not create files\n\
     \x20 -f                     ignored\n\
     \x20 -m                     change only the modification time\n\
     \x20 -r, --reference=FILE   use this file's times\n\
     \x20 -t STAMP               use [[CC]YY]MMDDhhmm[.ss]\n\
     \x20 --time=WORD            access, atime, use, modify, or mtime\n\
     \x20 -h, --help             show this help\n\
     \x20 --version              show version\n"
}

fn version_line() -> String {
    let mut out = program_name();
    out.push(' ');
    out.push_str(env!("CARGO_PKG_VERSION"));
    out.push('\n');
    out
}

fn emit_and_exit<T: AsRef<str>>(text: T, code: i32) -> ! {
    let mut out = io::stdout();
    let _ = out.write_all(text.as_ref().as_bytes());
    process::exit(code)
}
