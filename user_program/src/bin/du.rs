//! `du` — estimate file space usage.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use anyhow::{Context, Result};
use user_lib::{
    eprintln, fs,
    io::{self, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Summarize disk usage of each FILE, recursively for directories.
    pub struct DuArgs {
        /// Write counts for all files, not just directories.
        pub all:       bool           = ["-a", "--all"],
        /// Print a grand total.
        pub total:     bool           = ["-c", "--total"],
        /// Print sizes in human-readable format (e.g., 1K 234M 2G).
        pub human:     bool           = ["-h", "--human-readable"],
        /// Display only a total for each argument.
        pub summarize: bool           = ["-s", "--summarize"],
        /// Files or directories.
        pub files:     Vec<String>    = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = DuArgs::parse_env_or_exit();
    let paths: Vec<&str> = if cli.files.is_empty() {
        alloc::vec!["."]
    } else {
        cli.files.iter().map(String::as_str).collect()
    };

    let block_size = 1024u64;
    let mut grand_total: u64 = 0;
    let mut had_error = false;

    for path in &paths {
        match du(path, &cli, block_size) {
            Ok(total) => grand_total += total,
            Err(err) => {
                eprintln!("du: {:#}", err);
                had_error = true;
            }
        }
    }

    if cli.total {
        print_size(grand_total, "total", block_size, cli.human);
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn du(path_str: &str, cli: &DuArgs, block_size: u64) -> Result<u64> {
    let meta = fs::metadata(path_str).with_context(|| path_str.to_string())?;
    if !meta.is_dir() {
        // GNU du prints non-directory arguments too: only the `-a` and
        // recursion bits skip files _under_ directories, but a file
        // passed directly is always reported.
        let blocks = blocks_used(meta.len(), block_size);
        print_size(blocks * block_size, path_str, block_size, cli.human);
        return Ok(blocks);
    }

    if cli.summarize {
        let total = du_dir(path_str, cli, block_size)?;
        print_size(total * block_size, path_str, block_size, cli.human);
        return Ok(total);
    }

    let total = du_dir(path_str, cli, block_size)?;
    print_size(total * block_size, path_str, block_size, cli.human);
    Ok(total)
}

fn du_dir(dir: &str, cli: &DuArgs, block_size: u64) -> Result<u64> {
    let mut total: u64 = 0;

    // The directory entry itself uses blocks.
    if let Ok(meta) = fs::metadata(dir) {
        total += blocks_used(meta.len(), block_size);
    }

    for entry in fs::read_dir(dir).with_context(|| dir.to_string())? {
        let entry = entry?;
        let child_meta = entry.metadata()?;
        if child_meta.is_dir() {
            total += du_dir(entry.path().as_str(), cli, block_size)?;
        } else {
            let blocks = blocks_used(child_meta.len(), block_size);
            if cli.all {
                let p = entry.path();
                print_size(blocks * block_size, p.as_str(), block_size, cli.human);
            }
            total += blocks;
        }
    }

    Ok(total)
}

fn blocks_used(size_bytes: u64, block_size: u64) -> u64 {
    size_bytes.div_ceil(block_size)
}

fn print_size(bytes: u64, label: &str, block_size: u64, human: bool) {
    let mut out = io::stdout();
    let mut line = String::new();
    use core::fmt::Write as _;

    if human {
        let _ = write!(line, "{}\t{}", human_size(bytes), label);
    } else {
        let blocks = bytes.div_ceil(block_size);
        let _ = write!(line, "{}\t{}", blocks, label);
    }
    line.push('\n');
    let _ = out.write_all(line.as_bytes());
}

fn human_size(bytes: u64) -> String {
    let units = [
        ("", 1u64),
        ("K", 1024),
        ("M", 1024 * 1024),
        ("G", 1024 * 1024 * 1024),
    ];
    let mut unit = 0;
    while unit < units.len() - 1 && bytes >= units[unit + 1].1 {
        unit += 1;
    }
    let (suffix, divisor) = units[unit];
    if unit == 0 {
        alloc::format!("{}", bytes)
    } else {
        let whole = bytes / divisor;
        let frac = ((bytes % divisor) * 10) / divisor;
        if whole < 10 {
            alloc::format!("{}.{}{}", whole, frac, suffix)
        } else {
            alloc::format!("{}{}", whole, suffix)
        }
    }
}
