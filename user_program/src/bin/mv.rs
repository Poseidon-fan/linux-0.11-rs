//! `mv` — rename files and directories within the same filesystem.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use anyhow::Result;
use user_lib::{
    eprintln, fs,
    io::{self, ErrorKind, Read, Write},
    path::Path,
    println,
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Rename SOURCE to DEST, or move SOURCE(s) into DIRECTORY.
    pub struct MvArgs {
        /// Do not prompt before overwriting; ignore prior `-i`.
        pub force:       bool       = ["-f", "--force"],
        /// Prompt before overwriting an existing file.
        pub interactive: bool       = ["-i", "--interactive"],
        /// Do not overwrite an existing file.
        pub no_clobber:  bool       = ["-n", "--no-clobber"],
        /// Explain what is being done.
        pub verbose:     bool       = ["-v", "--verbose"],
        /// Files (or files + destination directory).
        pub paths:       Vec<String> = [..] @ "PATH",
    }
}

enum Overwrite {
    Force,
    NoClobber,
    Ask,
}

#[user_lib::main]
fn main() -> ExitCode {
    let cli = MvArgs::parse_env_or_exit();
    if cli.paths.len() < 2 {
        eprintln!("mv: missing destination operand");
        return ExitCode::FAILURE;
    }

    // POSIX precedence: -f overrides -i, -n overrides -i (last-write-wins for
    // -f vs -n in GNU; we keep it simple).
    let policy = if cli.force {
        Overwrite::Force
    } else if cli.no_clobber {
        Overwrite::NoClobber
    } else if cli.interactive {
        Overwrite::Ask
    } else {
        Overwrite::Force
    };

    let (sources, dest) = cli.paths.split_at(cli.paths.len() - 1);
    let dest = &dest[0];
    let dest_is_dir = matches!(fs::metadata(dest.as_str()), Ok(md) if md.is_dir());

    if sources.len() > 1 && !dest_is_dir {
        eprintln!("mv: target '{}' is not a directory", dest,);
        return ExitCode::FAILURE;
    }

    let mut exit_code = ExitCode::SUCCESS;
    for src in sources {
        let target = if dest_is_dir {
            join_into_dir(dest.as_str(), src.as_str())
        } else {
            dest.clone()
        };

        if let Err(err) = move_one(src.as_str(), target.as_str(), &policy, cli.verbose) {
            eprintln!("mv: {:#}", err);
            exit_code = ExitCode::FAILURE;
        }
    }
    exit_code
}

fn move_one(src: &str, dst: &str, policy: &Overwrite, verbose: bool) -> Result<()> {
    let src_md = match fs::metadata(src) {
        Ok(md) => md,
        Err(err) => anyhow::bail!("cannot stat '{}': {}", src, err),
    };

    // Pre-flight: src and dst already resolve to the same file. POSIX says
    // this is success / no-op, but printing a hint matches GNU mv and helps
    // the user notice the typo.
    if let Ok(dst_md) = fs::metadata(dst) {
        if dst_md.ino() == src_md.ino() && dst_md.dev() == src_md.dev() {
            anyhow::bail!("'{}' and '{}' are the same file", src, dst);
        }
    }

    // Pre-flight: would `src` end up nested inside itself? Walk dst's
    // ancestor chain checking for the src inode. Catches `mv foo foo/bar`
    // and `mv foo /abs/path/that/is/foo/sub` patterns *before* we issue a
    // rename(2) the kernel would have to reject with EINVAL.
    if src_md.is_dir() && would_create_loop(&src_md, dst) {
        anyhow::bail!(
            "cannot move '{}' to a subdirectory of itself, '{}'",
            src,
            dst
        );
    }

    if fs::metadata(dst).is_ok() {
        match policy {
            Overwrite::Force => {}
            Overwrite::NoClobber => return Ok(()),
            Overwrite::Ask => {
                if !prompt_yes(dst)? {
                    return Ok(());
                }
            }
        }
    }

    fs::rename(src, dst)
        .map_err(|err| anyhow::anyhow!("cannot move '{}' to '{}': {}", src, dst, err))?;

    if verbose {
        println!("renamed '{}' -> '{}'", src, dst);
    }
    Ok(())
}

/// Walks every ancestor directory of `dst` (excluding `dst` itself). If any
/// ancestor stats as the same inode/device as `src_md`, moving `src` to
/// `dst` would put it inside its own subtree.
fn would_create_loop(src_md: &fs::Metadata, dst: &str) -> bool {
    let mut current = Path::new(dst).to_path_buf();
    while let Some(parent) = current.parent() {
        let pstr = parent.as_str();
        if pstr.is_empty() {
            break;
        }
        if let Ok(md) = fs::metadata(pstr) {
            if md.ino() == src_md.ino() && md.dev() == src_md.dev() {
                return true;
            }
        }
        if pstr == "/" {
            break;
        }
        current = parent.to_path_buf();
    }
    false
}

/// Joins `basename(src)` onto `dir`, ensuring exactly one separator.
fn join_into_dir(dir: &str, src: &str) -> String {
    let base = Path::new(src).file_name().unwrap_or(src);
    let mut out = String::with_capacity(dir.len() + 1 + base.len());
    out.push_str(dir);
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(base);
    out
}

/// POSIX `-i`: prompt on stderr, accept "y"/"Y" prefix on stdin as yes.
/// Any other answer (including read error) means no.
fn prompt_yes(path: &str) -> Result<bool> {
    let mut out = io::stderr();
    out.write_all(b"mv: overwrite '")?;
    out.write_all(path.as_bytes())?;
    out.write_all(b"'? ")?;

    let mut buf = [0u8; 1];
    let mut stdin = io::stdin();
    let mut answer = b'\n';
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {
                if answer == b'\n' {
                    answer = buf[0];
                }
                if buf[0] == b'\n' {
                    break;
                }
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Ok(false),
        }
    }
    Ok(matches!(answer, b'y' | b'Y'))
}

#[allow(dead_code)]
fn _retain_string() -> String {
    String::new()
}

#[allow(dead_code)]
fn _retain_to_string<T: ToString>(t: T) -> String {
    t.to_string()
}
