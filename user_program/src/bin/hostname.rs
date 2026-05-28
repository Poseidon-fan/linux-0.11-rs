//! `hostname` — show the system hostname.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::vec::Vec;

use user_lib::{
    fs,
    io::{self, Write},
    process::ExitCode,
    syscall,
};

#[user_lib::main]
fn main() -> ExitCode {
    // Prefer /etc/hostname (the persistent source of truth); fall back
    // to the kernel's nodename via uname() if it's missing.
    let name = if let Ok(s) = fs::read_to_string("/etc/hostname") {
        s.trim().as_bytes().to_vec()
    } else {
        let mut uts: syscall::process::UtsName = unsafe { core::mem::zeroed() };
        if syscall::process::uname(&mut uts).is_err() {
            return ExitCode::FAILURE;
        }
        let v: Vec<u8> = uts
            .nodename
            .iter()
            .copied()
            .take_while(|&b| b != 0)
            .collect();
        v
    };
    let _ = io::stdout().write_all(&name);
    let _ = io::stdout().write_all(b"\n");
    ExitCode::SUCCESS
}
