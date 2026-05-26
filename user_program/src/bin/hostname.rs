//! `hostname` — show the system hostname.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::vec::Vec;

use user_lib::{
    io::{self, Write},
    process::ExitCode,
    syscall,
};

#[user_lib::main]
fn main() -> ExitCode {
    let mut uts: syscall::process::UtsName = unsafe { core::mem::zeroed() };
    if syscall::process::uname(&mut uts).is_err() {
        return ExitCode::FAILURE;
    }
    let name: Vec<u8> = uts
        .nodename
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        .collect();
    let _ = io::stdout().write_all(&name);
    let _ = io::stdout().write_all(b"\n");
    ExitCode::SUCCESS
}
