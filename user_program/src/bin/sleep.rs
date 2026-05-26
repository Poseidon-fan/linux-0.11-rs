//! `sleep` — pause for a specified amount of time.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use user_lib::{
    eprintln,
    process::ExitCode,
    time::{self, Duration},
};
use user_program::cli::cli_args;

cli_args! {
    /// Pause for NUMBER seconds. SUFFIX may be `s` for seconds (the
    /// default), `m` for minutes, `h` for hours, or `d` for days. With
    /// multiple arguments, pause for the sum of their values.
    pub struct SleepArgs {
        /// Time amounts to add together.
        pub durations: Vec<String> = [..] @ "NUMBER[SUFFIX]",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = SleepArgs::parse_env_or_exit();
    if cli.durations.is_empty() {
        eprintln!("sleep: missing operand");
        eprintln!("Try 'sleep --help' for more information.");
        return ExitCode::FAILURE;
    }

    let mut total = Duration::ZERO;
    for raw in &cli.durations {
        match parse_duration(raw.as_str()) {
            Some(d) => match total.checked_add(d) {
                Some(sum) => total = sum,
                None => {
                    eprintln!("sleep: time accumulator overflowed");
                    return ExitCode::FAILURE;
                }
            },
            None => {
                eprintln!("sleep: invalid time interval '{}'", raw);
                return ExitCode::FAILURE;
            }
        }
    }

    time::sleep(total);
    ExitCode::SUCCESS
}

/// Parses a `NUMBER[s|m|h|d]` token. Only non-negative integer NUMBER s
/// are accepted (the kernel has no sub-second `nanosleep`, so fractional
/// input is not really meaningful; GNU sleep accepts floats but we do not
/// pull in soft-float on this target).
fn parse_duration(raw: &str) -> Option<Duration> {
    if raw.is_empty() {
        return None;
    }
    let bytes = raw.as_bytes();
    let (digits, multiplier_secs): (&str, u64) = match bytes[bytes.len() - 1] {
        b's' => (&raw[..raw.len() - 1], 1),
        b'm' => (&raw[..raw.len() - 1], 60),
        b'h' => (&raw[..raw.len() - 1], 3600),
        b'd' => (&raw[..raw.len() - 1], 86_400),
        b'0'..=b'9' => (raw, 1),
        _ => return None,
    };
    let n: u64 = digits.parse().ok()?;
    n.checked_mul(multiplier_secs).map(Duration::from_secs)
}
