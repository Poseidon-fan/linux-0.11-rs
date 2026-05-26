//! `seq` — print a sequence of numbers (integer only — no FPU).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use user_lib::{
    eprintln,
    io::{self, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Print numbers from FIRST to LAST, in steps of INCREMENT.
    pub struct SeqArgs {
        /// Use STRING to separate numbers (default: \n).
        pub separator: Option<String> = ["-s", "--separator"] @ "STRING",
        /// Numbers: [FIRST [INCREMENT]] LAST.
        pub args:      Vec<String>    = [..] @ "NUMBER",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = SeqArgs::parse_env_or_exit();
    if cli.args.is_empty() {
        eprintln!("seq: missing operand");
        return ExitCode::FAILURE;
    }

    let (first, incr, last) = match parse_numbers(&cli.args) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("seq: {}", msg);
            return ExitCode::FAILURE;
        }
    };
    let sep = cli.separator.as_deref().unwrap_or("\n");

    let mut out = io::stdout();
    let mut buf = String::new();
    use core::fmt::Write as _;

    let mut val = first;
    let mut first_item = true;
    while (incr > 0 && val <= last) || (incr < 0 && val >= last) {
        if !first_item {
            buf.push_str(sep);
        }
        first_item = false;
        let _ = write!(buf, "{}", val);
        val = val.saturating_add(incr);
    }
    buf.push('\n');
    let _ = out.write_all(buf.as_bytes());
    ExitCode::SUCCESS
}

fn parse_numbers(args: &[String]) -> Result<(i64, i64, i64), String> {
    let parse = |s: &str| {
        s.parse::<i64>()
            .map_err(|_| format!("invalid number: '{}'", s))
    };
    match args.len() {
        1 => {
            let last = parse(&args[0])?;
            Ok((1, 1, last))
        }
        2 => {
            let first = parse(&args[0])?;
            let last = parse(&args[1])?;
            Ok((first, 1, last))
        }
        3 => {
            let first = parse(&args[0])?;
            let incr = parse(&args[1])?;
            let last = parse(&args[2])?;
            Ok((first, incr, last))
        }
        _ => Err("extra operand".into()),
    }
}
