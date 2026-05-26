//! `date` — print or set the system date and time.
//!
//! This implementation only prints (no setting). Time is always UTC since
//! the kernel doesn't track timezones. `+FORMAT` is supported with a
//! subset of the POSIX `strftime` conversions.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::fmt::Write as _;

use user_lib::{
    eprintln,
    io::{self, Write},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

const WEEKDAY_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAY_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTH_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTH_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[user_lib::main]
fn main() -> ExitCode {
    // `date` only consumes leading flags (`-u`, `-h`, `--help`) and at most
    // one positional `+FORMAT` argument. cli_args! doesn't support
    // "stop at +" so we parse argv directly.
    let mut format: Option<String> = None;
    for arg in user_lib::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "-u" | "--utc" | "--universal" => {
                // We're always UTC; accept silently.
            }
            s if s.starts_with('+') => {
                if format.is_some() {
                    eprintln!("date: extra operand '{}'", s);
                    return ExitCode::FAILURE;
                }
                format = Some(s[1..].to_string());
            }
            s => {
                eprintln!("date: unknown operand '{}'", s);
                eprintln!("Try 'date --help' for more information.");
                return ExitCode::FAILURE;
            }
        }
    }

    let secs_since_epoch = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => 0,
    };
    let cal = Calendar::from_unix(secs_since_epoch);

    let mut out = String::new();
    match format.as_deref() {
        Some(fmt) => render(&cal, secs_since_epoch, fmt, &mut out),
        None => default_format(&cal, &mut out),
    }
    out.push('\n');
    let _ = io::stdout().write_all(out.as_bytes());
    ExitCode::SUCCESS
}

fn print_usage() {
    let _ = io::stdout().write_all(
        b"Usage: date [OPTION]... [+FORMAT]\n\
          Print the current time in the given FORMAT, or in `Day Mon DD HH:MM:SS UTC YYYY`\n\
          when none is given. Time is always UTC.\n\
          \n\
          Supported format specifiers:\n\
          \x20 %Y year         %m month (01-12)   %d day (01-31)\n\
          \x20 %H hour (00-23) %M minute (00-59)  %S second (00-60)\n\
          \x20 %A weekday name %a short weekday   %B month name   %b short month\n\
          \x20 %s seconds since epoch              %% literal %\n\
          \n\
          Options:\n\
          \x20 -u, --utc       accepted for compatibility (we are always UTC)\n\
          \x20 -h, --help      show this help\n",
    );
}

/// Default GNU/POSIX style: `Tue May 26 02:23:40 UTC 2026`.
fn default_format(cal: &Calendar, out: &mut String) {
    let _ = write!(
        out,
        "{} {} {:>2} {:02}:{:02}:{:02} UTC {}",
        WEEKDAY_SHORT[cal.weekday as usize],
        MONTH_SHORT[(cal.month - 1) as usize],
        cal.day,
        cal.hour,
        cal.minute,
        cal.second,
        cal.year,
    );
}

fn render(cal: &Calendar, epoch: i64, fmt: &str, out: &mut String) {
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => {
                let _ = write!(out, "{:04}", cal.year);
            }
            Some('m') => {
                let _ = write!(out, "{:02}", cal.month);
            }
            Some('d') => {
                let _ = write!(out, "{:02}", cal.day);
            }
            Some('H') => {
                let _ = write!(out, "{:02}", cal.hour);
            }
            Some('M') => {
                let _ = write!(out, "{:02}", cal.minute);
            }
            Some('S') => {
                let _ = write!(out, "{:02}", cal.second);
            }
            Some('A') => out.push_str(WEEKDAY_LONG[cal.weekday as usize]),
            Some('a') => out.push_str(WEEKDAY_SHORT[cal.weekday as usize]),
            Some('B') => out.push_str(MONTH_LONG[(cal.month - 1) as usize]),
            Some('b') => out.push_str(MONTH_SHORT[(cal.month - 1) as usize]),
            Some('s') => {
                let _ = write!(out, "{}", epoch);
            }
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
}

struct Calendar {
    year: i64,
    month: u32, // 1..=12
    day: u32,   // 1..=31
    hour: u32,
    minute: u32,
    second: u32,
    weekday: u32, // 0 = Sunday
}

impl Calendar {
    /// Convert Unix epoch seconds to UTC calendar fields using Howard
    /// Hinnant's `civil_from_days` algorithm.
    fn from_unix(secs: i64) -> Self {
        let days = secs.div_euclid(86_400);
        let seconds_of_day = secs.rem_euclid(86_400) as u64;
        let hour = (seconds_of_day / 3_600) as u32;
        let minute = ((seconds_of_day / 60) % 60) as u32;
        let second = (seconds_of_day % 60) as u32;

        // Shift epoch to 0000-03-01 so leap years align with era boundaries.
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64; // days of era
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y0 = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // days of year (Mar 1)
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month_offset = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = y0 + if month_offset <= 2 { 1 } else { 0 };
        let month = month_offset as u32;

        // 1970-01-01 was a Thursday (weekday 4).
        let weekday = ((days + 4).rem_euclid(7)) as u32;

        Calendar {
            year,
            month,
            day,
            hour,
            minute,
            second,
            weekday,
        }
    }
}

#[allow(dead_code)]
fn _retain_vec_string_imports() -> Vec<String> {
    Vec::new()
}
