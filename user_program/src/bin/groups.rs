//! `groups` — print the groups a user is in.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    fs,
    io::{self, Write},
    process::{self, ExitCode},
};

#[user_lib::main]
fn main() -> ExitCode {
    let uid = process::uid();
    let gid = process::gid();
    let mut names = Vec::new();
    let uname = user_name(uid);
    if let Ok(data) = fs::read_to_string("/etc/group") {
        for line in data.lines() {
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 3 {
                if let Ok(g) = parts[2].parse::<u32>() {
                    if g == gid {
                        names.push(parts[0].to_string());
                    }
                    if parts.len() >= 4 && !parts[3].is_empty() {
                        if let Some(ref n) = uname {
                            if parts[3].split(',').any(|m| m == n.as_str()) {
                                names.push(parts[0].to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    let mut out = io::stdout();
    for (i, n) in names.iter().enumerate() {
        if i > 0 {
            let _ = out.write_all(b" ");
        }
        let _ = out.write_all(n.as_bytes());
    }
    let _ = out.write_all(b"\n");
    ExitCode::SUCCESS
}
fn user_name(uid: u32) -> Option<String> {
    let data = fs::read_to_string("/etc/passwd").ok()?;
    data.lines().find_map(|l| {
        let mut p = l.splitn(3, ':');
        let n = p.next()?;
        let _ = p.next()?;
        if p.next()?.parse::<u32>().ok()? == uid {
            Some(n.to_string())
        } else {
            None
        }
    })
}
