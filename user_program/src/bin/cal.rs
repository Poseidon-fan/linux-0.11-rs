//! `cal` — display a calendar.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{string::String, vec::Vec};

use user_lib::{
    eprintln,
    io::{self, Write},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};
use user_program::cli::cli_args;

cli_args! {
    pub struct CalArgs {
        pub args: Vec<String> = [..] @ "MONTH/YEAR",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = CalArgs::parse_env_or_exit();
    let mode = match parse_mode(&cli.args) {
        Ok(mode) => mode,
        Err(code) => return code,
    };

    let mut out = io::stdout();
    let mut buf = String::new();
    use core::fmt::Write as _;
    match mode {
        DisplayMode::Year(year) => {
            let _ = writeln!(buf, "{:>32}", year);
            for m in 1..=12u32 {
                print_month(&mut buf, m, year);
            }
        }
        DisplayMode::MonthYear(month, year) => {
            print_month(&mut buf, month, year);
        }
    }
    let _ = out.write_all(buf.as_bytes());
    ExitCode::SUCCESS
}

const EXIT_USAGE: u8 = 64;

enum DisplayMode {
    MonthYear(u32, u32),
    Year(u32),
}

fn parse_mode(args: &[String]) -> Result<DisplayMode, ExitCode> {
    match args.len() {
        0 => {
            let (month, year) = current_month_year();
            Ok(DisplayMode::MonthYear(month, year))
        }
        1 => {
            let year = parse_year(&args[0]).ok_or_else(|| invalid_year(&args[0]))?;
            Ok(DisplayMode::Year(year))
        }
        2 => {
            let month = parse_month(&args[0]).ok_or_else(|| invalid_month(&args[0]))?;
            let year = parse_year(&args[1]).ok_or_else(|| invalid_year(&args[1]))?;
            Ok(DisplayMode::MonthYear(month, year))
        }
        _ => Err(usage()),
    }
}

fn parse_month(raw: &str) -> Option<u32> {
    let month = parse_u32(raw)?;
    (1..=12).contains(&month).then_some(month)
}

fn parse_year(raw: &str) -> Option<u32> {
    let year = parse_u32(raw)?;
    (1..=9999).contains(&year).then_some(year)
}

fn parse_u32(raw: &str) -> Option<u32> {
    if raw.is_empty() || !raw.as_bytes().iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

fn usage() -> ExitCode {
    eprintln!("Usage: cal [[month] year]");
    ExitCode::from(EXIT_USAGE)
}

fn invalid_month(raw: &str) -> ExitCode {
    eprintln!("cal: not a valid month {}", raw);
    usage()
}

fn invalid_year(raw: &str) -> ExitCode {
    eprintln!("cal: not a valid year {}", raw);
    usage()
}

fn current_month_year() -> (u32, u32) {
    let secs_since_epoch = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => 0,
    };
    let (year, month, _) = civil_from_days(secs_since_epoch.div_euclid(86_400));
    let year = if (1..=9999).contains(&year) {
        year as u32
    } else {
        1970
    };
    (month, year)
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y0 = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = y0 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

fn print_month(buf: &mut String, month: u32, year: u32) {
    use core::fmt::Write as _;
    let months = [
        "",
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
    let _ = writeln!(buf, "     {} {}", months[month as usize], year);
    let _ = writeln!(buf, "Su Mo Tu We Th Fr Sa");
    let days = days_in_month(month, year);
    let first_wday = day_of_week(year, month, 1);
    for _ in 0..first_wday {
        buf.push_str("   ");
    }
    for d in 1..=days {
        let _ = write!(buf, "{:>2} ", d);
        if (first_wday + d) % 7 == 0 {
            buf.push('\n');
        }
    }
    if (first_wday + days) % 7 != 0 {
        buf.push('\n');
    }
    buf.push('\n');
}

fn days_in_month(month: u32, year: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 31,
    }
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Zeller's congruence — returns 0=Sun..6=Sat
fn day_of_week(y: u32, m: u32, d: u32) -> u32 {
    let (m, y) = if m < 3 { (m + 12, y - 1) } else { (m, y) };
    let q = d;
    let k = y % 100;
    let j = y / 100;
    let h = (q + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    (h + 6) % 7
}
