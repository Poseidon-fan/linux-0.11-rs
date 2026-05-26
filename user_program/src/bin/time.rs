//! `time` — run a command and print its resource usage.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::fmt::Write as _;

use user_lib::{
    eprintln,
    io::{self, Write},
    process::{Command, ExitCode},
    syscall::{self, process::Tms},
    time::Instant,
};

/// HZ matches `kernel::task::timer::HZ`; jiffies / HZ = seconds.
const HZ: u32 = 100;

#[user_lib::main]
fn main() -> ExitCode {
    // `time` wraps a child command, so we need argv parsing that stops at
    // the first non-flag argument — our cli_args! consumes flags anywhere,
    // which would steal `-l` from `time ls -l`. Walk argv manually.
    let argv: Vec<String> = user_lib::env::args().skip(1).collect();
    let mut idx = 0;
    while idx < argv.len() {
        match argv[idx].as_str() {
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "-p" | "--portable" => {
                // Always-on; the only output format we support.
                idx += 1;
            }
            "--" => {
                idx += 1;
                break;
            }
            s if s.starts_with('-') && s != "-" => {
                eprintln!("time: unknown option: {}", s);
                eprintln!("Try 'time --help' for more information.");
                return ExitCode::from(2);
            }
            _ => break,
        }
    }

    if idx >= argv.len() {
        eprintln!("time: missing COMMAND");
        eprintln!("Try 'time --help' for more information.");
        return ExitCode::from(2);
    }

    let program = argv[idx].clone();
    let args = argv[idx + 1..].to_vec();

    // Snapshot accumulated child times before spawning. After waitpid the
    // child's user / sys jiffies fold into our `child_*` counters; the
    // difference is what this child contributed.
    let mut before = empty_tms();
    let _ = syscall::process::times(&mut before);

    let real_start = Instant::now();
    let exit_code = match Command::new(&program).args(&args).status() {
        Ok(status) => {
            let real = real_start.elapsed();
            let mut after = empty_tms();
            let _ = syscall::process::times(&mut after);

            let user_j = after.child_user_time.wrapping_sub(before.child_user_time);
            let sys_j = after
                .child_system_time
                .wrapping_sub(before.child_system_time);

            let mut buf = String::new();
            let _ = writeln!(buf, "real     {}", format_secs_centi(real_to_centis(real)));
            let _ = writeln!(
                buf,
                "user     {}",
                format_secs_centi(jiffies_to_centis(user_j))
            );
            let _ = writeln!(
                buf,
                "sys      {}",
                format_secs_centi(jiffies_to_centis(sys_j))
            );
            let _ = io::stderr().write_all(buf.as_bytes());

            status
                .code()
                .map(|c| ExitCode::from(c as u8))
                .unwrap_or(ExitCode::FAILURE)
        }
        Err(err) => {
            eprintln!("time: cannot run '{}': {}", program, err);
            return ExitCode::from(127);
        }
    };

    exit_code
}

fn print_usage() {
    let _ = io::stdout().write_all(
        b"Usage: time [-p] COMMAND [ARG]...\n\
          Run COMMAND with ARGs, then print elapsed real, user, and system\n\
          time in seconds to standard error.\n\
          \n\
          Options:\n\
          \x20 -p, --portable   POSIX format (always used).\n\
          \x20 -h, --help       show this help.\n",
    );
}

fn empty_tms() -> Tms {
    Tms {
        user_time: 0,
        system_time: 0,
        child_user_time: 0,
        child_system_time: 0,
    }
}

/// Convert a `Duration` to total centiseconds, rounding down.
fn real_to_centis(d: core::time::Duration) -> u64 {
    let secs = d.as_secs();
    let centis = u64::from(d.subsec_nanos() / 10_000_000);
    secs.saturating_mul(100).saturating_add(centis)
}

fn jiffies_to_centis(j: u32) -> u64 {
    // 1 jiffy = 1 centisecond when HZ == 100. Keep the formula explicit
    // in case the kernel HZ changes.
    (u64::from(j) * 100) / u64::from(HZ)
}

fn format_secs_centi(total_centis: u64) -> String {
    let secs = total_centis / 100;
    let centis = total_centis % 100;
    let mut s = String::new();
    let _ = write!(s, "{}.{:02}", secs, centis);
    s
}
