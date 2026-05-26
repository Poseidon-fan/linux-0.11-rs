//! `cp` — copy files and directories.
//!
//! # Options
//!
//! | Flag              | Meaning |
//! |-------------------|---------|
//! | `-a`, `--archive` | Same as `-Rp`. Preserve mode, ownership, timestamps. |
//! | `-f`, `--force`   | If destination cannot be opened, remove it and retry. |
//! | `-i`, `--interactive` | Prompt before overwrite. |
//! | `-l`, `--link`    | Hard-link files instead of copying. |
//! | `-n`, `--no-clobber` | Do not overwrite an existing file. |
//! | `-R`, `-r`, `--recursive` | Copy directories recursively. |
//! | `--preserve[=ATTR]` | Preserve mode,ownership,timestamps (default: mode,timestamps). |
//! | `-T`, `--no-target-directory` | Treat DEST as a normal file. |
//! | `-t`, `--target-directory=DIR` | Copy all SOURCEEs into DIR. |
//! | `-u`, `--update`  | Copy only when SOURCE is newer or DEST is missing. |
//! | `-v`, `--verbose` | Explain what is being done. |
//! | `--parents`       | Use full source path under DEST. |

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use anyhow::{Context, Result, bail};
use user_lib::{
    eprintln,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};
use user_program::cli::cli_args;

cli_args! {
    /// Copy SOURCE to DEST, or multiple SOURCE(s) into DIRECTORY.
    pub struct CpArgs {
        /// Same as -Rp. Preserve mode, ownership, timestamps.
        pub archive:     bool            = ["-a", "--archive"],
        /// Remove destination and retry if it cannot be opened.
        pub force:       bool            = ["-f", "--force"],
        /// Prompt before overwriting a file.
        pub interactive: bool            = ["-i", "--interactive"],
        /// Hard-link files instead of copying.
        pub link:        bool            = ["-l", "--link"],
        /// Do not overwrite existing files.
        pub no_clobber:  bool            = ["-n", "--no-clobber"],
        /// Copy directories recursively.
        pub recursive:   bool            = ["-r", "-R", "--recursive"],
        /// Preserve the specified attributes (mode,ownership,timestamps).
        pub preserve:    Option<String>  = ["--preserve"] @ "ATTR",
        /// Copy all SOURCEEs into DIRECTORY.
        pub target_directory: Option<String> = ["-t", "--target-directory"] @ "DIR",
        /// Treat DEST as a normal file, not a directory.
        pub no_target_dir:   bool            = ["-T", "--no-target-directory"],
        /// Copy only when SOURCE is newer than DEST or DEST is missing.
        pub update:      bool            = ["-u", "--update"],
        /// Explain what is being done.
        pub verbose:     bool            = ["-v", "--verbose"],
        /// Use full source path under DEST.
        pub parents:     bool            = ["--parents"],
        /// Sources and destination.
        pub paths:       Vec<String>     = [..] @ "PATH",
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[user_lib::main]
fn main() -> ExitCode {
    let mut args = CpArgs::parse_env_or_exit();

    if args.archive {
        args.recursive = true;
        args.preserve
            .get_or_insert(String::from("mode,ownership,timestamps"));
    }

    let preserve = Preserve::from_cli(&args);

    let (sources, dest, dest_is_dir) = match resolve_operands(&args) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("cp: {}", msg);
            return ExitCode::from(2);
        }
    };

    let mut had_error = false;

    if args.parents {
        for src in &sources {
            if let Err(err) = copy_with_parents(&args, &preserve, src, &dest) {
                eprintln!("cp: {:#}", err);
                had_error = true;
            }
        }
    } else if dest_is_dir {
        // Copy each source INTO the target directory.
        for src in &sources {
            let file_name = Path::new(src).file_name().unwrap_or(src);
            let target = dest.join(file_name);
            if let Err(err) = copy_one(&args, &preserve, src, &target) {
                eprintln!("cp: {:#}", err);
                had_error = true;
            }
        }
    } else {
        // Single source → single target.
        let src = &sources[0];
        if let Err(err) = copy_one(&args, &preserve, src, &dest) {
            eprintln!("cp: {:#}", err);
            had_error = true;
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// ---------------------------------------------------------------------------
// Operand resolution
// ---------------------------------------------------------------------------

/// Returns `(sources, dest, dest_is_directory_target)`.
fn resolve_operands(args: &CpArgs) -> Result<(Vec<String>, PathBuf, bool), String> {
    // -t DIR ... — all remaining operands are sources.
    if let Some(ref td) = args.target_directory {
        if args.paths.is_empty() {
            return Err("missing file operand".into());
        }
        return Ok((args.paths.clone(), PathBuf::from(td.as_str()), true));
    }

    if args.paths.len() < 2 {
        return Err("missing destination file operand after SOURCE".into());
    }

    let mut paths = args.paths.clone();
    let dest_raw = paths.pop().unwrap();

    if args.no_target_dir || paths.len() == 1 {
        // -T or single source: treat dest as a literal target path.
        // But if it's a single source and dest exists as a directory *and*
        // -T wasn't given, copy into it.
        if !args.no_target_dir {
            if let Ok(meta) = fs::metadata(&dest_raw) {
                if meta.is_dir() {
                    return Ok((paths, PathBuf::from(dest_raw), true));
                }
            }
        }
        Ok((paths, PathBuf::from(dest_raw), false))
    } else {
        // Multiple sources — dest must be an existing directory.
        Ok((paths, PathBuf::from(dest_raw), true))
    }
}

// ---------------------------------------------------------------------------
// Preserve flags
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Preserve {
    mode: bool,
    ownership: bool,
    timestamps: bool,
}

impl Preserve {
    fn from_cli(args: &CpArgs) -> Self {
        match &args.preserve {
            None => {
                // Default: preserve mode and timestamps.
                if args.archive {
                    Self {
                        mode: true,
                        ownership: true,
                        timestamps: true,
                    }
                } else {
                    Self {
                        mode: true,
                        ownership: false,
                        timestamps: true,
                    }
                }
            }
            Some(raw) => {
                let mut p = Preserve::default();
                for token in raw.split(',') {
                    let t = token.trim();
                    match t {
                        "mode" => p.mode = true,
                        "ownership" => p.ownership = true,
                        "timestamps" => p.timestamps = true,
                        "all" => {
                            p.mode = true;
                            p.ownership = true;
                            p.timestamps = true;
                        }
                        _ => {} // ignore unknown; GNU cp warns silently
                    }
                }
                p
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

fn copy_one(args: &CpArgs, preserve: &Preserve, src: &str, dest: &Path) -> Result<()> {
    let src_meta = fs::metadata(src).with_context(|| format!("cannot stat '{}'", src))?;

    if src_meta.is_dir() {
        if !args.recursive {
            bail!("-r not specified; omitting directory '{}'", src);
        }
        return copy_dir(args, preserve, src, dest, &src_meta);
    }

    // If dest exists and is a directory (and -T is not given), copy INTO it.
    if !args.no_target_dir {
        if let Ok(meta) = fs::metadata(dest) {
            if meta.is_dir() {
                let file_name = Path::new(src).file_name().unwrap_or(src);
                let into = dest.join(file_name);
                return copy_file(args, preserve, src, &into, &src_meta);
            }
        }
    }

    copy_file(args, preserve, src, dest, &src_meta)
}

// ---------------------------------------------------------------------------
// File copy
// ---------------------------------------------------------------------------

fn copy_file(
    args: &CpArgs,
    preserve: &Preserve,
    src: &str,
    dest: &Path,
    src_meta: &Metadata,
) -> Result<()> {
    // --link: hard link instead of copying data.
    if args.link {
        return link_mode(args, preserve, src, dest, src_meta);
    }

    // Check if destination exists.
    if let Ok(dest_meta) = fs::metadata(dest) {
        if same_inode(src_meta, &dest_meta) {
            if args.verbose {
                eprintln!("cp: '{}' and '{}' are the same file", src, dest);
            }
            return Ok(());
        }
        if args.no_clobber {
            return Ok(());
        }
        if args.update && !source_is_newer(src_meta, &dest_meta) {
            return Ok(());
        }
        if args.interactive && !prompt_yes(dest.as_str()) {
            return Ok(());
        }
        if args.force {
            let _ = fs::remove_file(dest);
        }
    }

    // Open source.
    let mut reader =
        File::open(src).with_context(|| format!("cannot open '{}' for reading", src))?;

    // Create destination with correct mode, or open for writing.
    let mode = if preserve.mode {
        src_meta.permissions().mode()
    } else {
        0o666
    };
    let mut writer = create_dest_file(dest, mode, args.force)
        .with_context(|| format!("cannot create '{}'", dest))?;

    io::copy(&mut reader, &mut writer)
        .with_context(|| format!("error copying '{}' to '{}'", src, dest))?;

    // Flush and drop writer before metadata tweaks.
    writer.flush().ok();
    drop(writer);

    apply_preserve(preserve, dest, src_meta)?;

    if args.verbose {
        eprintln!("'{}' -> '{}'", src, dest);
    }
    Ok(())
}

fn link_mode(
    args: &CpArgs,
    _preserve: &Preserve,
    src: &str,
    dest: &Path,
    _src_meta: &Metadata,
) -> Result<()> {
    if let Ok(dest_meta) = fs::metadata(dest) {
        if args.no_clobber {
            return Ok(());
        }
        if args.interactive && !prompt_yes(dest.as_str()) {
            return Ok(());
        }
        if args.force || args.interactive {
            if dest_meta.is_dir() {
                fs::remove_dir_all(dest)?;
            } else {
                fs::remove_file(dest)?;
            }
        }
    }
    fs::hard_link(src, dest)?;
    if args.verbose {
        eprintln!("'{}' -> '{}' (hard link)", src, dest);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Directory copy
// ---------------------------------------------------------------------------

fn copy_dir(
    args: &CpArgs,
    preserve: &Preserve,
    src: &str,
    dest: &Path,
    src_meta: &Metadata,
) -> Result<()> {
    // Detect copying a directory into itself.
    if let Ok(dest_meta) = fs::metadata(dest) {
        if same_inode(src_meta, &dest_meta) {
            bail!(
                "cannot copy a directory, '{}', into itself, '{}'",
                src,
                dest
            );
        }
        // If dest is an existing *file* (not dir), fail.
        if !dest_meta.is_dir() {
            bail!(
                "cannot overwrite non-directory '{}' with directory '{}'",
                dest,
                src
            );
        }
    }

    // Create destination directory if it doesn't exist.
    match fs::create_dir(dest) {
        Ok(()) => {}
        Err(ref e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => bail!("cannot create directory '{}': {}", dest, e),
    }

    let mut had_error = false;

    for entry in fs::read_dir(src).with_context(|| format!("cannot read directory '{}'", src))? {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                eprintln!("cp: {}", err);
                had_error = true;
                continue;
            }
        };
        let child_src = entry.path();
        let child_dest = dest.join(entry.file_name());
        let child_meta = entry.metadata()?;

        if child_meta.is_dir() {
            if let Err(err) = copy_dir(args, preserve, child_src.as_str(), &child_dest, &child_meta)
            {
                eprintln!("cp: {:#}", err);
                had_error = true;
            }
        } else if let Err(err) =
            copy_file(args, preserve, child_src.as_str(), &child_dest, &child_meta)
        {
            eprintln!("cp: {:#}", err);
            had_error = true;
        }
    }

    apply_preserve(preserve, dest, src_meta)?;

    if args.verbose {
        eprintln!("'{}' -> '{}' (directory)", src, dest);
    }

    if had_error {
        bail!("errors occurred while copying directory '{}'", src);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// --parents mode
// ---------------------------------------------------------------------------

fn copy_with_parents(args: &CpArgs, preserve: &Preserve, src: &str, base_dir: &Path) -> Result<()> {
    // Build the full destination: base_dir / src
    let full_dest = base_dir.join(src);

    // Ensure parent directories exist.
    if let Some(parent) = full_dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let src_meta = fs::metadata(src).with_context(|| format!("cannot stat '{}'", src))?;

    if src_meta.is_dir() {
        if !args.recursive {
            bail!("-r not specified; omitting directory '{}'", src);
        }
        copy_dir(args, preserve, src, &full_dest, &src_meta)
    } else {
        copy_file(args, preserve, src, &full_dest, &src_meta)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_dest_file(dest: &Path, mode: u32, force: bool) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(mode);
    match opts.open(dest) {
        Ok(f) => Ok(f),
        Err(_err) if force => {
            let _ = fs::remove_file(dest);
            Ok(opts.open(dest)?)
        }
        Err(err) => Err(err.into()),
    }
}

fn apply_preserve(preserve: &Preserve, path: &Path, src_meta: &Metadata) -> Result<()> {
    if preserve.mode {
        fs::set_permissions(path, src_meta.permissions()).ok();
    }
    if preserve.ownership {
        fs::chown(path, Some(src_meta.uid()), Some(src_meta.gid())).ok();
    }
    if preserve.timestamps {
        // Preserve both access and modification times.
        let accessed = src_meta.accessed().ok();
        let modified = src_meta.modified().ok();
        if let (Some(atime), Some(mtime)) = (accessed, modified) {
            let times = fs::FileTimes::new().set_accessed(atime).set_modified(mtime);
            // set_times requires the file to have been opened by path;
            // open/close a temp handle just for timestamp setting.
            if let Ok(f) = File::open(path) {
                f.set_times(times).ok();
            }
        }
    }
    Ok(())
}

fn same_inode(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}

fn source_is_newer(src: &Metadata, dest: &Metadata) -> bool {
    match (src.modified(), dest.modified()) {
        (Ok(s), Ok(d)) => s > d,
        _ => true, // if we can't read times, copy anyway
    }
}

fn prompt_yes(path: &str) -> bool {
    let mut stdout = io::stdout();
    let msg = format!("cp: overwrite '{}'? ", path);
    let _ = stdout.write_all(msg.as_bytes());
    let _ = stdout.flush();

    let mut buf = [0u8; 2];
    let mut stdin = io::stdin();
    match stdin.read(&mut buf) {
        Ok(1) | Ok(2) => {
            let first = buf[0];
            first == b'y' || first == b'Y'
        }
        _ => false,
    }
}
