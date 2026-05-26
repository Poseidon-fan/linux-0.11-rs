//! `base64` — base64 encode/decode data.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{string::String, vec::Vec};

use user_lib::{
    io::{self, Read, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    pub struct Base64Args {
        pub decode: bool = ["-d", "--decode"],
        pub files: Vec<String> = [..] @ "FILE",
    }
}
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[user_lib::main]
fn main() -> ExitCode {
    let cli = Base64Args::parse_env_or_exit();
    let mut data = Vec::new();
    if cli.files.is_empty() {
        let _ = io::stdin().read_to_end(&mut data);
    } else {
        for p in &cli.files {
            if let Ok(mut f) = user_lib::fs::File::open(p) {
                let _ = f.read_to_end(&mut data);
            }
        }
    }
    let out = &mut io::stdout();
    if cli.decode {
        let mut decoded = Vec::new();
        let mut buf = 0u32;
        let mut bits = 0u8;
        for &b in &data {
            if b == b'\n' || b == b'\r' || b == b' ' {
                continue;
            }
            let val = ALPHABET.iter().position(|&c| c == b).unwrap_or(64) as u32;
            if val >= 64 {
                continue;
            }
            buf = (buf << 6) | val;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                decoded.push((buf >> bits) as u8);
                buf &= (1 << bits) - 1;
            }
        }
        let _ = out.write_all(&decoded);
    } else {
        let mut i = 0usize;
        while i < data.len() {
            let b0 = data[i];
            let b1 = if i + 1 < data.len() { data[i + 1] } else { 0 };
            let b2 = if i + 2 < data.len() { data[i + 2] } else { 0 };
            let triple = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
            let mut enc = [
                ALPHABET[((triple >> 18) & 0x3f) as usize],
                ALPHABET[((triple >> 12) & 0x3f) as usize],
                ALPHABET[((triple >> 6) & 0x3f) as usize],
                ALPHABET[(triple & 0x3f) as usize],
            ];
            let rem = data.len() - i;
            if rem == 1 {
                enc[2] = b'=';
                enc[3] = b'=';
            } else if rem == 2 {
                enc[3] = b'=';
            }
            let _ = out.write_all(&enc);
            i += 3;
        }
        let _ = out.write_all(b"\n");
    }
    ExitCode::SUCCESS
}
