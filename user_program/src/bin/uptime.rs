//! `uptime` — tell how long the system has been running.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use user_lib::{
    io::{self, Write},
    process::ExitCode,
    time,
};
use user_program::cli::cli_args;

cli_args! {
    /// Show how long the system has been running.
    pub struct UptimeArgs {
        /// Show uptime in a human-friendly long form.
        pub pretty: bool = ["-p", "--pretty"],
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = UptimeArgs::parse_env_or_exit();
    let secs = time::uptime().as_secs();

    let line = if cli.pretty {
        pretty(secs)
    } else {
        compact(secs)
    };

    let _ = writeln!(io::stdout(), "{}", line);
    ExitCode::SUCCESS
}

/// `up HH:MM:SS` or `up N days, HH:MM:SS`.
fn compact(total: u64) -> String {
    let days = total / 86_400;
    let hours = (total / 3_600) % 24;
    let minutes = (total / 60) % 60;
    let seconds = total % 60;
    if days > 0 {
        format!(
            "up {} day{}, {:02}:{:02}:{:02}",
            days,
            if days == 1 { "" } else { "s" },
            hours,
            minutes,
            seconds,
        )
    } else {
        format!("up {:02}:{:02}:{:02}", hours, minutes, seconds)
    }
}

/// `up 2 days, 3 hours, 14 minutes` — drops zero leading components,
/// always includes at least "X minutes".
fn pretty(total: u64) -> String {
    let weeks = total / (86_400 * 7);
    let days = (total / 86_400) % 7;
    let hours = (total / 3_600) % 24;
    let minutes = (total / 60) % 60;

    let mut parts: Vec<String> = Vec::new();
    let mut push = |value: u64, name: &str| {
        if value == 0 && !parts.is_empty() {
            return;
        }
        if value == 0 && name != "minute" {
            return;
        }
        parts.push(format!(
            "{} {}{}",
            value,
            name,
            if value == 1 { "" } else { "s" }
        ));
    };
    push(weeks, "week");
    push(days, "day");
    push(hours, "hour");
    push(minutes, "minute");

    if parts.is_empty() {
        parts.push(String::from("0 minutes"));
    }

    let mut out = String::from("up ");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(part);
    }
    out
}
