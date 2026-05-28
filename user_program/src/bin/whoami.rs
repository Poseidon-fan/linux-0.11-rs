//! `whoami` — print effective user name.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::string::ToString;

use user_lib::{
    fs,
    io::{self, Write},
    process::{self, ExitCode},
};

#[user_lib::main]
fn main() -> ExitCode {
    let euid = process::euid();
    let name = if let Ok(data) = fs::read_to_string("/etc/passwd") {
        data.lines()
            .find_map(|l| {
                let mut p = l.splitn(4, ':');
                let n = p.next()?;
                let _passwd = p.next()?;
                let uid_field = p.next()?;
                if uid_field.parse::<u32>().ok()? == euid {
                    Some(n.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| alloc::format!("{}", euid))
    } else {
        alloc::format!("{}", euid)
    };
    let _ = io::stdout().write_all(name.as_bytes());
    let _ = io::stdout().write_all(b"\n");
    ExitCode::SUCCESS
}
