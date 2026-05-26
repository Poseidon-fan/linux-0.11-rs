//! `cal` — display a calendar.
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
    pub struct CalArgs {
        pub args: Vec<String> = [..] @ "MONTH/YEAR",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = CalArgs::parse_env_or_exit();
    let (month, year) = match cli.args.len() {
        0 => {
            // Current month/year — we don't have localtime, use a hardcoded date
            // or read from the RTC. For now just show Jan 2026.
            (1u32, 2026u32)
        }
        1 => {
            let y: u32 = cli.args[0].parse().unwrap_or(2026);
            (0, y) // full year
        }
        _ => {
            let m: u32 = cli.args[0].parse().unwrap_or(1);
            let y: u32 = cli.args[1].parse().unwrap_or(2026);
            (m, y)
        }
    };

    let mut out = io::stdout();
    let mut buf = String::new();
    use core::fmt::Write as _;
    if month == 0 {
        let _ = writeln!(buf, "{:>32}", year);
        for m in 1..=12u32 {
            print_month(&mut buf, m, year);
        }
    } else {
        print_month(&mut buf, month, year);
    }
    let _ = out.write_all(buf.as_bytes());
    ExitCode::SUCCESS
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
    (q + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7
}
