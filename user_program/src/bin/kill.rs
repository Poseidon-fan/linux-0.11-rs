//! `kill` — send a signal to a process, or list signal names.

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
    io::{self, Write},
    process::ExitCode,
    syscall::{
        Errno,
        signal::{self, SIGNULL, Signal},
    },
};
use user_program::cli::cli_args;

cli_args! {
    /// Send a signal to a process, or list signal names.
    /// If no signal is specified, SIGTERM is used.
    pub struct KillArgs {
        /// List signal names (optional: show number for SIGNAL).
        pub list:  bool           = ["-l", "--list"],
        /// List signal names in a compact table.
        pub table: bool           = ["-L", "--table"],
        /// Signal to send (name or number). Overrides the default SIGTERM.
        pub signal: Option<String> = ["-s", "--signal"] @ "SIGNAL",
        /// PIDs to signal.
        pub pids:  Vec<String>    = [..] @ "PID",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    // Normalize legacy forms: -NUMBER / -SIGNAME → -s NUMBER/SIGNAME.
    let normalized = normalize_args();

    let args = match KillArgs::parse_from(normalized) {
        Ok(a) => a,
        Err(err) => {
            err.print_with_hint(&user_program::cli::program_name());
            return ExitCode::from(2);
        }
    };

    // List mode.
    if args.list || args.table {
        return if args.table {
            print_signal_table()
        } else {
            list_signal(args.pids.first().map(String::as_str))
        };
    }

    // Resolve signal (default: TERM).
    let spec = args.signal.as_deref().unwrap_or("TERM");
    let (sig_name, sig_num) = match parse_signal(spec) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("kill: {}", msg);
            return ExitCode::FAILURE;
        }
    };

    // Parse PIDs.
    let pids: Vec<i32> = args
        .pids
        .iter()
        .filter_map(|s| s.parse::<i32>().ok())
        .collect();

    if pids.is_empty() {
        eprintln!("kill: not enough arguments");
        return ExitCode::from(1);
    }

    let mut had_error = false;
    for pid in &pids {
        let result = if sig_num == SIGNULL {
            signal::kill_raw(*pid, SIGNULL).map(|_| ())
        } else if let Some(sig) = Signal::from_u32(sig_num) {
            signal::kill(*pid, sig).map(|_| ())
        } else {
            signal::kill_raw(*pid, sig_num).map(|_| ())
        };

        if let Err(errno) = result {
            eprintln!("kill: ({}) - {}: {}", sig_name, *pid, errno_str(errno));
            had_error = true;
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// ---------------------------------------------------------------------------
// Argument normalization
// ---------------------------------------------------------------------------

/// Convert legacy `-NUMBER`, `-SIGNAME`, `-NAME` to `-s NUMBER` / `-s NAME`.
/// `-0` is also recognized as the null signal.
fn normalize_args() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let args: Vec<String> = env::args().skip(1).collect();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        // Recognise: -9, -0, -SIGTERM, -TERM (but NOT -l, -L, -s, -h, --*)
        if arg.len() > 1 && arg.starts_with('-') && !arg.starts_with("--") && arg.len() >= 2 {
            let ch = arg.as_bytes()[1];
            if ch != b'l' && ch != b'L' && ch != b's' && ch != b'h' {
                let rest = &arg[1..];
                if rest == "0"
                    || Signal::parse(rest).is_some()
                    || rest.bytes().all(|b| b.is_ascii_digit())
                {
                    out.push("-s".to_string());
                    out.push(rest.to_string());
                    i += 1;
                    continue;
                }
            }
        }
        out.push(arg.clone());
        i += 1;
    }

    out
}

// ---------------------------------------------------------------------------
// Signal parsing
// ---------------------------------------------------------------------------

fn parse_signal(raw: &str) -> Result<(String, u32), String> {
    if let Ok(num) = raw.parse::<u32>() {
        if num == SIGNULL {
            return Ok(("0".to_string(), SIGNULL));
        }
        return match Signal::from_u32(num) {
            Some(s) => Ok((s.name().to_string(), num)),
            None => Err(format!("invalid signal: {}", raw)),
        };
    }

    Signal::parse(raw)
        .map(|s| (s.name().to_string(), s.number()))
        .ok_or_else(|| format!("unknown signal: {}", raw))
}

// ---------------------------------------------------------------------------
// List mode
// ---------------------------------------------------------------------------

fn list_signal(target: Option<&str>) -> ExitCode {
    if let Some(name) = target {
        if let Some(sig) = Signal::parse(name) {
            eprintln!("{}", sig.number());
        } else if let Ok(num) = name.parse::<u32>() {
            if let Some(sig) = Signal::from_u32(num) {
                eprintln!("{}", sig.name());
            } else {
                eprintln!("{}", num);
            }
        } else {
            eprintln!("kill: {}: unknown signal", name);
            return ExitCode::FAILURE;
        }
    } else {
        let mut out = io::stdout();
        for i in 1..=31 {
            if let Some(sig) = Signal::from_u32(i) {
                let _ = out.write_all(sig.name().as_bytes());
                if i < 31 {
                    let _ = out.write_all(b" ");
                }
            }
        }
        let _ = out.write_all(b"\n");
    }
    ExitCode::SUCCESS
}

fn print_signal_table() -> ExitCode {
    let mut out = io::stdout();
    for i in (1..=31).step_by(7) {
        let mut line = String::new();
        for j in i..(i + 7).min(32) {
            use core::fmt::Write as _;
            if let Some(sig) = Signal::from_u32(j) {
                let _ = write!(line, "{:>2} {:<8}", j, sig.name());
            }
        }
        line.push('\n');
        let _ = out.write_all(line.as_bytes());
    }
    let _ = out.write_all(b" 0 EXIT\n");
    ExitCode::SUCCESS
}
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn errno_str(errno: Errno) -> &'static str {
    match errno.code() {
        1 => "Operation not permitted",
        3 => "No such process",
        22 => "Invalid argument",
        _ => "Unknown error",
    }
}
