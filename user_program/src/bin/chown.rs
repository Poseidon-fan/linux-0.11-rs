//! `chown` — change file owner and group.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use user_lib::{eprintln, fs, process::ExitCode};
use user_program::cli::cli_args;

cli_args! {
    /// Change the owner and/or group of each FILE to OWNER[:[GROUP]].
    pub struct ChownArgs {
        /// Recursively change files and directories.
        pub recursive: bool        = ["-R", "--recursive"],
        /// Files and OWNER[:GROUP].
        pub args:      Vec<String> = [..] @ "ARG",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = ChownArgs::parse_env_or_exit();
    if cli.args.len() < 2 {
        eprintln!("chown: missing operand");
        return ExitCode::from(1);
    }
    let spec = &cli.args[0];
    let files = &cli.args[1..];
    let (uid, gid) = match parse_owner(spec) {
        Some(v) => v,
        None => {
            eprintln!("chown: invalid user: '{}'", spec);
            return ExitCode::FAILURE;
        }
    };

    let mut had_error = false;
    for path in files {
        if let Err(err) = change_owner(path, uid, gid, cli.recursive) {
            eprintln!("chown: {}: {}", path, err);
            had_error = true;
        }
    }
    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn change_owner(
    path: &str,
    uid: Option<u32>,
    gid: Option<u32>,
    recursive: bool,
) -> Result<(), user_lib::io::Error> {
    let meta = fs::metadata(path)?;
    if meta.is_dir() && recursive {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            change_owner(entry.path().as_str(), uid, gid, true)?;
        }
    }
    fs::chown(path, uid, gid)
}

fn parse_owner(raw: &str) -> Option<(Option<u32>, Option<u32>)> {
    if let Some(colon) = raw.find(':') {
        let user_part = &raw[..colon];
        let group_part = &raw[colon + 1..];
        let uid = if user_part.is_empty() {
            None
        } else {
            parse_uid(user_part)
        };
        let gid = if group_part.is_empty() {
            None
        } else {
            parse_gid(group_part)
        };
        if uid.is_some() || gid.is_some() {
            Some((uid, gid))
        } else {
            None
        }
    } else if raw.contains('.') {
        // Legacy user.group syntax
        let mut parts = raw.splitn(2, '.');
        let uid = parse_uid(parts.next().unwrap_or(""));
        let gid = parse_gid(parts.next().unwrap_or(""));
        Some((uid, gid))
    } else {
        let uid = parse_uid(raw)?;
        Some((Some(uid), None))
    }
}

fn parse_uid(raw: &str) -> Option<u32> {
    raw.parse::<u32>().ok().or_else(|| {
        let name = raw.trim();
        if let Ok(contents) = fs::read_to_string("/etc/passwd") {
            for line in contents.lines() {
                let mut parts = line.splitn(3, ':');
                let uname = parts.next().unwrap_or("");
                let _x = parts.next().unwrap_or("");
                let uid_str = parts.next().unwrap_or("");
                if uname == name {
                    return uid_str.parse::<u32>().ok();
                }
            }
        }
        None
    })
}

fn parse_gid(raw: &str) -> Option<u32> {
    raw.parse::<u32>().ok().or_else(|| {
        let name = raw.trim();
        if let Ok(contents) = fs::read_to_string("/etc/group") {
            for line in contents.lines() {
                let mut parts = line.splitn(3, ':');
                let gname = parts.next().unwrap_or("");
                let _passwd = parts.next().unwrap_or("");
                let gid_str = parts.next().unwrap_or("");
                if gname == name {
                    return gid_str.parse::<u32>().ok();
                }
            }
        }
        None
    })
}
