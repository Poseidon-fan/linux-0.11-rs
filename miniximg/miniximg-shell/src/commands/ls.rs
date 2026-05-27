//! `ls` — list a directory in the image (or one file's metadata).

use anyhow::{Result, anyhow};
use miniximg::{DirectoryEntryInfo, InodeType, NodeMetadata};

use crate::{path, session::Session};

pub const NAME: &str = "ls";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "list directory contents (image-side)";
pub const USAGE: &str = "ls [-l] [-a] [PATH...]";

#[derive(Clone, Copy)]
struct Options {
    long: bool,
    show_hidden: bool,
}

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    let (opts, paths) = parse_args(args)?;
    let cwd = session.image_cwd().to_string();

    let targets: Vec<String> = if paths.is_empty() {
        vec![cwd]
    } else {
        paths.iter().map(|p| path::resolve_image(p, &cwd)).collect()
    };

    // Only print a `path:` banner before each block when the user asked
    // for multiple targets — matches `ls` from coreutils.
    let header = targets.len() > 1;
    for (i, target) in targets.iter().enumerate() {
        if header {
            if i > 0 {
                println!();
            }
            println!("{}:", target);
        }
        list_one(session, target, opts)?;
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<&str>)> {
    let mut opts = Options {
        long: false,
        show_hidden: false,
    };
    let mut paths: Vec<&str> = Vec::new();
    for arg in args {
        if let Some(rest) = arg.strip_prefix('-') {
            if rest.is_empty() {
                paths.push(arg);
                continue;
            }
            for ch in rest.chars() {
                match ch {
                    'l' => opts.long = true,
                    'a' => opts.show_hidden = true,
                    _ => return Err(anyhow!("unknown flag: -{}", ch)),
                }
            }
        } else {
            paths.push(arg);
        }
    }
    Ok((opts, paths))
}

fn list_one(session: &mut Session, path: &str, opts: Options) -> Result<()> {
    let meta = session.stat_image(path)?;
    if meta.kind != InodeType::Directory {
        // `ls FILE` prints one row using the supplied name, not the
        // basename — bash users expect to see the path they typed.
        let entry = DirectoryEntryInfo {
            name: crate::path::image_basename(path).to_string(),
            metadata: meta,
        };
        print_entry(&entry, opts.long);
        return Ok(());
    }

    let mut entries = session.fs_mut().list_path(path)?;
    if !opts.show_hidden {
        entries.retain(|e| !e.name.starts_with('.'));
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    if opts.long {
        for e in &entries {
            print_entry(e, true);
        }
    } else {
        print_columns(&entries);
    }
    Ok(())
}

fn print_entry(entry: &DirectoryEntryInfo, long: bool) {
    if long {
        println!(
            "{} {:>3} {:>8} {}",
            mode_string(&entry.metadata),
            entry.metadata.link_count,
            entry.metadata.size,
            decorated_name(&entry.name, &entry.metadata),
        );
    } else {
        println!("{}", decorated_name(&entry.name, &entry.metadata));
    }
}

/// Renders `entries` as a vertically-laid-out grid that fits in
/// `TERMINAL_WIDTH` columns, identical to coreutils `ls`'s default.
fn print_columns(entries: &[DirectoryEntryInfo]) {
    if entries.is_empty() {
        return;
    }
    const TERMINAL_WIDTH: usize = 80;
    const COLUMN_GUTTER: usize = 2;
    let names: Vec<String> = entries
        .iter()
        .map(|e| decorated_name(&e.name, &e.metadata))
        .collect();
    let column_w = names.iter().map(String::len).max().unwrap_or(0) + COLUMN_GUTTER;
    let cols = (TERMINAL_WIDTH / column_w.max(1)).max(1).min(names.len());
    let rows = names.len().div_ceil(cols);
    for row in 0..rows {
        for col in 0..cols {
            let idx = col * rows + row;
            let Some(name) = names.get(idx) else { continue };
            print!("{}", name);
            if col + 1 < cols && idx + rows < names.len() {
                for _ in name.len()..column_w {
                    print!(" ");
                }
            }
        }
        println!();
    }
}

/// Appends a one-character file-type suffix when applicable, matching the
/// `ls -F` convention. Always on in this shell — it costs one character
/// per row and helps disambiguate dirs from executables at a glance.
fn decorated_name(name: &str, meta: &NodeMetadata) -> String {
    let mut s = String::from(name);
    match meta.kind {
        InodeType::Directory => s.push('/'),
        InodeType::Fifo => s.push('|'),
        InodeType::Regular if meta.mode & 0o111 != 0 => s.push('*'),
        _ => {}
    }
    s
}

/// `drwxr-xr-x`-style mode string.
fn mode_string(meta: &NodeMetadata) -> String {
    let kind = match meta.kind {
        InodeType::Directory => 'd',
        InodeType::Regular => '-',
        InodeType::Fifo => 'p',
        InodeType::BlockDevice => 'b',
        InodeType::CharacterDevice => 'c',
    };
    let mut out = String::with_capacity(10);
    out.push(kind);
    for (mask, ch) in [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ] {
        out.push(if meta.mode & mask != 0 { ch } else { '-' });
    }
    out
}
