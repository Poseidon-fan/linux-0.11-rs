//! `split` — split a file into pieces.

#![no_std]
#![no_main]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};

use user_lib::{
    fs::File,
    io::{self, Read, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    pub struct SplitArgs {
        pub lines: Option<String> = ["-l"] @ "NUM",
        pub files: Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = SplitArgs::parse_env_or_exit();
    let nlines: usize = cli
        .lines
        .as_ref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000)
        .max(1);
    let input = cli.files.first().map(String::as_str).unwrap_or("-");
    let prefix = cli.files.get(1).map(String::as_str).unwrap_or("x");

    let mut data = Vec::new();
    let _ = if input == "-" {
        io::stdin().read_to_end(&mut data)
    } else {
        File::open(input).and_then(|mut f| f.read_to_end(&mut data))
    };
    let text = core::str::from_utf8(&data).unwrap_or("");
    let lines: Vec<&str> = text.lines().collect();
    for (i, chunk) in lines.chunks(nlines).enumerate() {
        let name = format!(
            "{}{}{}",
            prefix,
            ((i / 26) as u8 + b'a') as char,
            ((i % 26) as u8 + b'a') as char
        );
        let mut f = File::create(&name).unwrap();
        for line in chunk {
            let _ = f.write_all(line.as_bytes());
            let _ = f.write_all(b"\n");
        }
    }
    ExitCode::SUCCESS
}
