//! `chgrp` — change group ownership of files.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use user_lib::{eprintln, fs, process::ExitCode};
use user_program::cli::cli_args;

cli_args! {
    /// Change the group of each FILE to GROUP.
    pub struct ChgrpArgs {
        /// Recursively change files and directories.
        pub recursive: bool        = ["-R", "--recursive"],
        /// Files and GROUP.
        pub args:      Vec<String> = [..] @ "ARG",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = ChgrpArgs::parse_env_or_exit();
    if cli.args.len() < 2 {
        eprintln!("chgrp: missing operand");
        return ExitCode::from(1);
    }
    let group_arg = &cli.args[0];
    let files = &cli.args[1..];
    let gid = match parse_gid(group_arg) {
        Some(g) => g,
        None => {
            eprintln!("chgrp: invalid group: '{}'", group_arg);
            return ExitCode::FAILURE;
        }
    };

    let mut had_error = false;
    for path in files {
        if let Err(err) = change_group(path, gid, cli.recursive) {
            eprintln!("chgrp: {}: {}", path, err);
            had_error = true;
        }
    }
    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn change_group(path: &str, gid: u32, recursive: bool) -> Result<(), user_lib::io::Error> {
    let meta = fs::metadata(path)?;
    if meta.is_dir() && recursive {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            change_group(entry.path().as_str(), gid, true)?;
        }
    }
    fs::chown(path, None, Some(gid))
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
