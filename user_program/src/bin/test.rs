//! `test` / `[` — check file types and compare values.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{string::String, vec::Vec};

use user_lib::{env, fs, process::ExitCode};

fn args_vec() -> Vec<String> {
    let mut v: Vec<String> = env::args().collect();
    if v.len() > 1 && v.last().map(String::as_str) == Some("]") {
        v.pop();
    }
    v.remove(0);
    v
}

#[user_lib::main]
fn main() -> ExitCode {
    let args = args_vec();
    if eval(&args) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn eval(args: &[String]) -> bool {
    if args.is_empty() {
        return false;
    }
    match args[0].as_str() {
        "!" => !eval(&args[1..]),
        _ => {
            if args.len() == 1 {
                return !args[0].is_empty();
            }
            if args.len() == 2 && args[0] == "-n" {
                return !args[1].is_empty();
            }
            if args.len() == 2 && args[0] == "-z" {
                return args[1].is_empty();
            }
            if args.len() == 2 {
                return unary(&args[0], &args[1]);
            }
            if args.len() == 3 {
                return binary(&args[0], &args[1], &args[2]);
            }
            if args.len() == 4 && (args[1] == "-a" || args[1] == "-o") {
                let left = eval(&args[..1]);
                let right = eval(&args[2..]);
                return if args[1] == "-a" {
                    left && right
                } else {
                    left || right
                };
            }
            false
        }
    }
}
fn unary(op: &str, a: &str) -> bool {
    match op {
        "-f" => fs::metadata(a).map(|m| m.is_file()).unwrap_or(false),
        "-d" => fs::metadata(a).map(|m| m.is_dir()).unwrap_or(false),
        "-e" => fs::metadata(a).is_ok(),
        "-r" => fs::metadata(a)
            .map(|m| m.mode() & 0o400 != 0)
            .unwrap_or(false),
        "-w" => fs::metadata(a)
            .map(|m| m.mode() & 0o200 != 0)
            .unwrap_or(false),
        "-x" => fs::metadata(a)
            .map(|m| m.mode() & 0o100 != 0)
            .unwrap_or(false),
        "-s" => fs::metadata(a).map(|m| !m.is_empty()).unwrap_or(false),
        _ => false,
    }
}
fn binary(a: &str, op: &str, b: &str) -> bool {
    let ai = a.parse::<i64>();
    let bi = b.parse::<i64>();
    match op {
        "=" | "==" => a == b,
        "!=" => a != b,
        "-eq" => ai.ok() == bi.ok(),
        "-ne" => ai.ok() != bi.ok(),
        "-lt" => ai.ok() < bi.ok(),
        "-le" => ai.ok() <= bi.ok(),
        "-gt" => ai.ok() > bi.ok(),
        "-ge" => ai.ok() >= bi.ok(),
        _ => false,
    }
}
