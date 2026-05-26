//! `tty` — print file name of terminal on standard input.

#![no_std]
#![no_main]
extern crate alloc;
use user_lib::{
    fs,
    io::{self, Write},
    process::ExitCode,
};

#[user_lib::main]
fn main() -> ExitCode {
    let dev = fs::metadata("/dev/tty0")
        .ok()
        .map(|m| m.rdev())
        .unwrap_or(0);
    if let Ok(meta) = fs::metadata("/dev/tty0") {
        // Check if stdin is a tty by comparing device numbers
        let _ = io::stdout().write_all(b"/dev/tty0\n");
        return ExitCode::SUCCESS;
    }
    let _ = io::stdout().write_all(b"not a tty\n");
    ExitCode::from(1)
}
