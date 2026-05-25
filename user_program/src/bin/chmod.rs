//! `chmod` — change file mode bits.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use user_lib::{
    eprintln, fs,
    io::{self, ErrorKind},
    process::ExitCode,
    syscall,
};
use user_program::cli::cli_args;

const USER_RWX: u32 = 0o700;
const GROUP_RWX: u32 = 0o070;
const OTHER_RWX: u32 = 0o007;
const ALL_RWX: u32 = USER_RWX | GROUP_RWX | OTHER_RWX;
const SPECIAL_BITS: u32 = 0o7000;
const USER_SPECIAL: u32 = 0o4000;
const GROUP_SPECIAL: u32 = 0o2000;
const OTHER_SPECIAL: u32 = 0o1000;

cli_args! {
    /// Change the mode of each FILE to MODE.
    pub struct ChmodArgs {
        /// Change files and directories recursively.
        pub recursive: bool        = ["-R", "--recursive"],
        /// Output a diagnostic for every file processed.
        pub verbose:   bool        = ["-v", "--verbose"],
        /// Like verbose, but report only when a change is made.
        pub changes:   bool        = ["-c", "--changes"],
        /// Mode followed by files.
        pub operands:  Vec<String> = [..] @ "MODE FILE",
    }
}

#[user_lib::main]
fn main() -> ExitCode {
    let args = ChmodArgs::parse_env_or_exit();
    let (mode, paths) = match split_operands(&args.operands) {
        Ok(parts) => parts,
        Err(message) => {
            eprintln!("chmod: {}", message);
            eprintln!("Try 'chmod --help' for more information.");
            return ExitCode::FAILURE;
        }
    };

    let mut exit_code = ExitCode::SUCCESS;
    for path in paths {
        if let Err(err) = chmod_path(path, mode, &args) {
            eprintln!("chmod: {}: {}", path, err);
            exit_code = ExitCode::FAILURE;
        }
    }
    exit_code
}

fn split_operands(operands: &[String]) -> Result<(&str, &[String]), &'static str> {
    match operands {
        [] => Err("missing operand"),
        [_mode] => Err("missing operand after mode"),
        [mode, paths @ ..] => Ok((mode.as_str(), paths)),
    }
}

fn chmod_path(path: &str, mode_spec: &str, args: &ChmodArgs) -> io::Result<()> {
    let metadata = fs::metadata(path)?;
    let old_mode = metadata.permissions().mode();
    let new_mode = parse_mode(mode_spec, old_mode, metadata.is_dir())?;

    if old_mode != new_mode {
        fs::set_permissions(path, fs::Permissions::from_mode(new_mode))?;
        if args.verbose || args.changes {
            user_lib::println!("mode of '{}' changed to {:04o}", path, new_mode);
        }
    } else if args.verbose {
        user_lib::println!("mode of '{}' retained as {:04o}", path, old_mode);
    }

    if args.recursive && metadata.is_dir() {
        chmod_children(path, mode_spec, args)?;
    }

    Ok(())
}

fn chmod_children(path: &str, mode_spec: &str, args: &ChmodArgs) -> io::Result<()> {
    let mut children = Vec::new();
    for item in fs::read_dir(path)? {
        children.push(item?.path().into_string());
    }
    children.sort();

    for child in children {
        if let Err(err) = chmod_path(child.as_str(), mode_spec, args) {
            eprintln!("chmod: {}: {}", child, err);
            if err.kind() != ErrorKind::NotFound {
                return Err(err);
            }
        }
    }

    Ok(())
}

fn parse_mode(spec: &str, current: u32, is_dir: bool) -> io::Result<u32> {
    if is_octal_mode(spec) {
        return u32::from_str_radix(spec, 8)
            .ok()
            .filter(|mode| *mode <= 0o7777)
            .ok_or_else(invalid_mode);
    }

    let umask = current_umask();
    let mut mode = current & 0o7777;
    for clause in spec.split(',') {
        if clause.is_empty() {
            return Err(invalid_mode());
        }
        mode = apply_symbolic_clause(mode, clause, is_dir, umask)?;
    }
    Ok(mode & 0o7777)
}

fn is_octal_mode(spec: &str) -> bool {
    !spec.is_empty()
        && spec
            .as_bytes()
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'7'))
}

fn current_umask() -> u32 {
    let old = syscall::fs::umask(0).unwrap_or(0);
    let _ = syscall::fs::umask(old);
    old & 0o777
}

fn apply_symbolic_clause(current: u32, clause: &str, is_dir: bool, umask: u32) -> io::Result<u32> {
    let bytes = clause.as_bytes();
    let mut index = 0;
    let mut who = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'u' => who |= USER_RWX | USER_SPECIAL,
            b'g' => who |= GROUP_RWX | GROUP_SPECIAL,
            b'o' => who |= OTHER_RWX | OTHER_SPECIAL,
            b'a' => who |= ALL_RWX | SPECIAL_BITS,
            _ => break,
        }
        index += 1;
    }

    let who_was_omitted = who == 0;
    if who_was_omitted {
        who = (ALL_RWX & !umask) | SPECIAL_BITS;
    }

    let mut mode = current;
    while index < bytes.len() {
        let op = bytes[index];
        if !matches!(op, b'+' | b'-' | b'=') {
            return Err(invalid_mode());
        }
        index += 1;

        let start = index;
        while index < bytes.len() && !matches!(bytes[index], b'+' | b'-' | b'=') {
            index += 1;
        }
        let value = symbolic_value(current, &bytes[start..index], who, is_dir)?;
        match op {
            b'+' => mode |= value,
            b'-' => mode &= !value,
            b'=' => mode = (mode & !who) | value,
            _ => unreachable!(),
        }
    }

    Ok(mode)
}

fn symbolic_value(current: u32, symbols: &[u8], who: u32, is_dir: bool) -> io::Result<u32> {
    let mut value = 0;
    for symbol in symbols {
        match *symbol {
            b'r' => value |= class_bits(who, 0o400, 0o040, 0o004),
            b'w' => value |= class_bits(who, 0o200, 0o020, 0o002),
            b'x' => value |= class_bits(who, 0o100, 0o010, 0o001),
            b'X' => {
                if is_dir || current & 0o111 != 0 {
                    value |= class_bits(who, 0o100, 0o010, 0o001);
                }
            }
            b's' => value |= who & (USER_SPECIAL | GROUP_SPECIAL),
            b't' => value |= who & OTHER_SPECIAL,
            b'u' => value |= copy_class(current, who, USER_RWX),
            b'g' => value |= copy_class(current, who, GROUP_RWX),
            b'o' => value |= copy_class(current, who, OTHER_RWX),
            _ => return Err(invalid_mode()),
        }
    }
    Ok(value & who)
}

fn class_bits(who: u32, user: u32, group: u32, other: u32) -> u32 {
    let mut bits = 0;
    if who & USER_RWX != 0 {
        bits |= user;
    }
    if who & GROUP_RWX != 0 {
        bits |= group;
    }
    if who & OTHER_RWX != 0 {
        bits |= other;
    }
    bits
}

fn copy_class(current: u32, who: u32, source_class: u32) -> u32 {
    let source = current & source_class;
    let normalized = match source_class {
        USER_RWX => source >> 6,
        GROUP_RWX => source >> 3,
        OTHER_RWX => source,
        _ => 0,
    };

    let mut bits = 0;
    if who & USER_RWX != 0 {
        bits |= normalized << 6;
    }
    if who & GROUP_RWX != 0 {
        bits |= normalized << 3;
    }
    if who & OTHER_RWX != 0 {
        bits |= normalized;
    }
    bits
}

fn invalid_mode() -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, "invalid mode")
}
