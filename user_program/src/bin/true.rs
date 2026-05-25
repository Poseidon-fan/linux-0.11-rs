//! `true` — succeed.

#![no_std]
#![no_main]

extern crate alloc;

use user_lib::process::ExitCode;

#[user_lib::main]
fn main() -> ExitCode {
    ExitCode::SUCCESS
}
