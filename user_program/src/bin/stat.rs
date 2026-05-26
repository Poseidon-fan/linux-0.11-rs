//! `stat` — display file or filesystem status.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    format,
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
    /// Display file or filesystem status.
    pub struct StatArgs {
        /// Follow symbolic links (accepted for compatibility).
        pub dereference: bool        = ["-L", "--dereference"],
        /// Files to stat.
        pub files:       Vec<String> = [..] @ "FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = StatArgs::parse_env_or_exit();
    let _ = cli.dereference;
    let mut had_error = false;

    for path in &cli.files {
        if let Err(err) = stat_one(path) {
            eprintln!("stat: {:#}", err);
            had_error = true;
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn stat_one(path: &str) -> Result<()> {
    let meta = fs::metadata(path).with_context(|| path.to_string())?;

    let mut out = io::stdout();
    let mut buf = String::new();
    use core::fmt::Write as _;

    let _ = writeln!(buf, "  File: {}", path);
    let _ = writeln!(
        buf,
        "  Size: {:<12} Blocks: {:<8} IO Block: 1024",
        meta.len(),
        meta.blocks()
    );
    let _ = write!(buf, "  ");
    if meta.is_dir() {
        let _ = write!(buf, "directory");
    } else if meta.is_file() {
        let _ = write!(buf, "regular file");
    } else if meta.file_type().is_char_device() {
        let _ = write!(buf, "character device");
    } else if meta.file_type().is_block_device() {
        let _ = write!(buf, "block device");
    } else if meta.file_type().is_fifo() {
        let _ = write!(buf, "fifo");
    } else {
        let _ = write!(buf, "unknown");
    }
    let _ = writeln!(buf);

    let _ = writeln!(
        buf,
        "Device: {},{}   Inode: {:<8}  Links: {}",
        meta.dev() >> 8,
        meta.dev() & 0xff,
        meta.ino(),
        meta.nlink()
    );

    let mode = meta.mode();
    let _ = write!(buf, "Access: ({:04o}/", mode & 0o7777);
    let _ = write!(buf, "{}", mode_string(mode));
    let _ = writeln!(buf, ")");

    let _ = writeln!(
        buf,
        "  Uid: ({:>4}/unknown)   Gid: ({:>4}/unknown)",
        meta.uid(),
        meta.gid()
    );

    let _ = writeln!(buf, "Access: {}", fmt_time(meta.atime()));
    let _ = writeln!(buf, "Modify: {}", fmt_time(meta.mtime()));
    let _ = writeln!(buf, "Change: {}", fmt_time(meta.ctime()));

    out.write_all(buf.as_bytes())?;
    Ok(())
}

fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(10);
    s.push(type_char(mode));
    s.push(perm_char(mode, 0o400, b'r'));
    s.push(perm_char(mode, 0o200, b'w'));
    s.push(special_char(mode, 0o4000, 0o100, b's', b'x'));
    s.push(perm_char(mode, 0o040, b'r'));
    s.push(perm_char(mode, 0o020, b'w'));
    s.push(special_char(mode, 0o2000, 0o010, b's', b'x'));
    s.push(perm_char(mode, 0o004, b'r'));
    s.push(perm_char(mode, 0o002, b'w'));
    s.push(special_char(mode, 0o1000, 0o001, b't', b'x'));
    s
}

fn type_char(mode: u32) -> char {
    match (mode >> 12) as u8 & 0o17 {
        0o04 => 'd',
        0o02 => 'c',
        0o06 => 'b',
        0o01 => 'p',
        _ => '-',
    }
}

fn perm_char(mode: u32, bit: u32, ch: u8) -> char {
    if mode & bit != 0 { ch as char } else { '-' }
}

fn special_char(mode: u32, special: u32, exec: u32, special_ch: u8, exec_ch: u8) -> char {
    if mode & special != 0 {
        if mode & exec != 0 {
            special_ch.to_ascii_uppercase() as char
        } else {
            special_ch.to_ascii_lowercase() as char
        }
    } else if mode & exec != 0 {
        exec_ch as char
    } else {
        '-'
    }
}

fn fmt_time(unix_secs: i64) -> String {
    if unix_secs < 0 {
        return String::from("(before epoch)");
    }
    let secs = unix_secs as u64;
    let day_secs = secs % 86400;
    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;

    let days = (secs / 86400) as i64;
    let (year, month, day) = civil_from_days(days);

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, month, day, hour, minute, second
    )
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z as u64 } else { 0 } / 146097;
    let doe = z.wrapping_sub((era * 146097) as i64) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era as i64 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}
