//! `df` — report filesystem disk space usage.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use anyhow::Result;
use user_lib::{
    eprintln, fs,
    io::{self, Write},
    process::ExitCode,
    syscall,
    syscall::fs::Ustat,
};
use user_program::cli::cli_args;

cli_args! {
    /// Show filesystem disk space usage.
    pub struct DfArgs {
        /// Print sizes in human-readable format.
        pub human: bool        = ["-h", "--human-readable"],
        /// Filesystem device or mount points (default: all mounted).
        pub files: Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = DfArgs::parse_env_or_exit();
    let block_size = 1024u64;

    let _ = print_header();

    if cli.files.is_empty() {
        // Default: show root filesystem.
        let dev = match root_dev() {
            Some(d) => d,
            None => {
                eprintln!("df: cannot determine root device");
                return ExitCode::FAILURE;
            }
        };
        show_fs(dev, "rootfs", block_size, cli.human);
    } else {
        for path in &cli.files {
            match fs::metadata(path) {
                Ok(meta) => show_fs(meta.dev() as u32, path, block_size, cli.human),
                Err(err) => {
                    eprintln!("df: {}: {}", path, err);
                }
            }
        }
    }

    ExitCode::SUCCESS
}

fn root_dev() -> Option<u32> {
    fs::metadata("/").ok().map(|m| m.dev() as u32)
}

fn print_header() -> Result<()> {
    let mut out = io::stdout();
    out.write_all(b"Filesystem     1K-blocks      Used Available Use% Mounted on\n")?;
    Ok(())
}

fn show_fs(dev: u32, name: &str, _block_size: u64, human: bool) {
    let mut ubuf = Ustat::default();
    if syscall::fs::ustat(dev, &mut ubuf).is_err() {
        // ustat not available — just show what we can from stat.
        return;
    }

    let total = ubuf.f_blocks as u64;
    let free = ubuf.f_bfree as u64;
    let used = total.saturating_sub(free);
    let pct = if total > 0 {
        (used * 100 + total / 2) / total
    } else {
        0
    };

    let mut out = io::stdout();
    let mut line = String::new();
    use core::fmt::Write as _;

    let _ = write!(line, "{:<15}", name);
    if human {
        let _ = write!(
            line,
            "{:>11} {:>8} {:>9} {:>4}%",
            human_size_k(total),
            human_size_k(used),
            human_size_k(free),
            pct
        );
    } else {
        let _ = write!(line, "{:>11} {:>8} {:>9} {:>4}%", total, used, free, pct);
    }
    let _ = write!(line, " {}", name);
    line.push('\n');
    let _ = out.write_all(line.as_bytes());
}

fn human_size_k(blocks: u64) -> String {
    let bytes = blocks * 1024;
    // No FPU: use integer arithmetic with one decimal place.
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        let k = bytes / 1024;
        let frac = ((bytes % 1024) * 10) / 1024;
        format!("{}.{}K", k, frac)
    } else if bytes < 1024 * 1024 * 1024 {
        let m = bytes / (1024 * 1024);
        let frac = ((bytes % (1024 * 1024)) * 10) / (1024 * 1024);
        format!("{}.{}M", m, frac)
    } else {
        let g = bytes / (1024 * 1024 * 1024);
        let frac = ((bytes % (1024 * 1024 * 1024)) * 10) / (1024 * 1024 * 1024);
        format!("{}.{}G", g, frac)
    }
}
