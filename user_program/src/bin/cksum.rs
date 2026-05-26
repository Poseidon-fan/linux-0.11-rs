//! `cksum` — display CRC checksum and byte counts.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{string::String, vec::Vec};

use user_lib::{
    fs::File,
    io::{self, Read, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! { pub struct CksumArgs { pub files: Vec<String> = [..] @ "FILE" } }

#[user_lib::main]
fn main() -> ExitCode {
    let cli = CksumArgs::parse_env_or_exit();
    let paths: Vec<&str> = if cli.files.is_empty() {
        alloc::vec!["-"]
    } else {
        cli.files.iter().map(String::as_str).collect()
    };
    let mut out = io::stdout();
    for path in &paths {
        cksum(path, &mut out);
    }
    ExitCode::SUCCESS
}

fn cksum(path: &str, out: &mut io::Stdout) {
    let mut data = Vec::new();
    if path == "-" {
        let _ = io::stdin().read_to_end(&mut data);
    } else if let Ok(mut f) = File::open(path) {
        let _ = f.read_to_end(&mut data);
    }
    let crc = crc32(&data);
    let len = data.len();
    let mut buf = String::new();
    use core::fmt::Write as _;
    let _ = writeln!(buf, "{} {}", crc, len);
    let _ = out.write_all(buf.as_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &byte in data {
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            if crc & 0x80000000 != 0 {
                crc = (crc << 1) ^ 0x04c11db7;
            } else {
                crc <<= 1;
            }
        }
    }
    let mut len = data.len() as u64;
    while len > 0 {
        crc ^= ((len & 0xff) as u32) << 24;
        for _ in 0..8 {
            if crc & 0x80000000 != 0 {
                crc = (crc << 1) ^ 0x04c11db7;
            } else {
                crc <<= 1;
            }
        }
        len >>= 8;
    }
    !crc
}
