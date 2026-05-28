//! `ls` — list directory contents.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use user_lib::{
    eprintln, fs,
    io::{self, Write},
    process::ExitCode,
};
use user_program::cli::cli_args;

const DEFAULT_TERMINAL_WIDTH: usize = 80;
const COLUMN_PADDING: usize = 2;

cli_args! {
    /// List information about FILEs and directory contents.
    pub struct LsArgs {
        /// Do not ignore entries whose names start with `.`.
        pub all:       bool        = ["-a", "--all"],
        /// Use a long listing format.
        pub long:      bool        = ["-l"],
        /// List one file per line.
        pub one:       bool        = ["-1"],
        /// List entries by columns.
        pub columns:   bool        = ["-C"],
        /// List directories themselves, not their contents.
        pub directory: bool        = ["-d", "--directory"],
        /// Append a file-type indicator to each name.
        pub classify:  bool        = ["-F", "--classify"],
        /// Files or directories to list.
        pub paths:     Vec<String> = [..] @ "FILE",
    }
}

#[derive(Clone, Copy)]
enum Layout {
    /// One entry per output line.
    OnePerLine,
    /// A long listing with mode, size, inode, and name.
    Long,
    /// Multi-column display, filled down each column.
    Columns,
}

#[user_lib::main]
fn main() -> ExitCode {
    let mut args = LsArgs::parse_env_or_exit();
    if args.paths.is_empty() {
        args.paths.push(String::from("."));
    }

    let layout = choose_layout(&args);
    let mut out = io::stdout();
    match list_operands(&args, layout, &mut out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::FAILURE,
    }
}

/// Lists all command-line operands using Unix `ls` grouping rules.
fn list_operands(args: &LsArgs, layout: Layout, out: &mut impl Write) -> Result<(), ()> {
    let multiple = args.paths.len() > 1;
    let mut operands = Vec::new();
    let mut had_error = false;

    for path in &args.paths {
        match fs::metadata(path.as_str()) {
            Ok(metadata) => operands.push(Operand {
                path: path.clone(),
                metadata,
            }),
            Err(err) => {
                eprintln!("ls: cannot access '{}': {}", path, err);
                had_error = true;
            }
        }
    }

    operands.sort_by(|a, b| a.path.cmp(&b.path));

    let mut file_entries = Vec::new();
    let mut directories = Vec::new();
    for operand in operands {
        if !args.directory && operand.metadata.is_dir() {
            directories.push(operand);
        } else {
            file_entries.push(ListingEntry {
                name: operand.path,
                metadata: operand.metadata,
            });
        }
    }

    if !file_entries.is_empty() && write_entries(out, &file_entries, args, layout, false).is_err() {
        had_error = true;
    }

    let mut printed_any = !file_entries.is_empty();
    let show_headers = multiple && !args.directory;
    for directory in directories {
        if printed_any {
            let _ = out.write_all(b"\n");
        }
        if show_headers {
            let _ = writeln!(out, "{}:", directory.path);
        }
        if let Err(err) = list_directory(directory.path.as_str(), args, layout, out) {
            eprintln!("ls: {}: {}", directory.path, err);
            had_error = true;
        }
        printed_any = true;
    }

    if had_error { Err(()) } else { Ok(()) }
}

struct Operand {
    /// Path exactly as supplied on the command line.
    path: String,
    /// Metadata for deciding whether this operand is a directory.
    metadata: fs::Metadata,
}

/// Chooses the output layout from the requested format flags.
fn choose_layout(args: &LsArgs) -> Layout {
    if args.long {
        Layout::Long
    } else if args.one {
        Layout::OnePerLine
    } else {
        let _ = args.columns;
        Layout::Columns
    }
}

/// Lists the contents of one directory operand.
fn list_directory(
    path: &str,
    args: &LsArgs,
    layout: Layout,
    out: &mut impl Write,
) -> user_lib::io::Result<()> {
    let mut entries = read_entries(path, args.all)?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    write_entries(out, &entries, args, layout, true)?;
    Ok(())
}

struct ListingEntry {
    /// File name displayed for this entry.
    name: String,
    /// Metadata captured while reading the directory.
    metadata: fs::Metadata,
}

/// Reads directory entries, optionally synthesizing `.` and `..`.
fn read_entries(path: &str, include_hidden: bool) -> user_lib::io::Result<Vec<ListingEntry>> {
    let mut entries = Vec::new();

    if include_hidden {
        push_synthetic_entry(&mut entries, path, ".")?;
        push_synthetic_entry(&mut entries, path, "..")?;
    }

    for item in fs::read_dir(path)? {
        let entry = item?;
        let name = entry.file_name();
        if !include_hidden && name.starts_with('.') {
            continue;
        }
        entries.push(ListingEntry {
            metadata: entry.metadata()?,
            name,
        });
    }

    Ok(entries)
}

/// Adds one synthetic dot entry for `ls -a`.
fn push_synthetic_entry(
    entries: &mut Vec<ListingEntry>,
    dir: &str,
    name: &str,
) -> user_lib::io::Result<()> {
    let path = join_path(dir, name);
    entries.push(ListingEntry {
        metadata: fs::metadata(path.as_str())?,
        name: name.to_string(),
    });
    Ok(())
}

/// Writes a complete listing using the selected layout.
fn write_entries(
    out: &mut impl Write,
    entries: &[ListingEntry],
    args: &LsArgs,
    layout: Layout,
    directory_contents: bool,
) -> user_lib::io::Result<()> {
    match layout {
        Layout::Long => write_long_entries(out, entries, args, directory_contents),
        Layout::OnePerLine => write_one_per_line(out, entries, args),
        Layout::Columns => write_columns(out, entries, args),
    }
}

/// Writes entries in long format: `mode nlink owner group size mtime name`.
fn write_long_entries(
    out: &mut impl Write,
    entries: &[ListingEntry],
    args: &LsArgs,
    directory_contents: bool,
) -> user_lib::io::Result<()> {
    if directory_contents {
        writeln!(out, "total {}", total_blocks(entries))?;
    }

    let owner_lookup = OwnerLookup::load();
    let nlink_w = entries
        .iter()
        .map(|e| decimal_width(e.metadata.nlink()))
        .max()
        .unwrap_or(1);
    let size_w = entries
        .iter()
        .map(|e| decimal_width(e.metadata.len()))
        .max()
        .unwrap_or(1);
    let owner_w = entries
        .iter()
        .map(|e| owner_lookup.user(e.metadata.uid()).len())
        .max()
        .unwrap_or(1);
    let group_w = entries
        .iter()
        .map(|e| owner_lookup.group(e.metadata.gid()).len())
        .max()
        .unwrap_or(1);

    for entry in entries {
        let user = owner_lookup.user(entry.metadata.uid());
        let group = owner_lookup.group(entry.metadata.gid());
        let line = format!(
            "{} {:>nlink_w$} {:<owner_w$} {:<group_w$} {:>size_w$} {} {}\n",
            mode_string(&entry.metadata),
            entry.metadata.nlink(),
            user,
            group,
            entry.metadata.len(),
            format_mtime(entry.metadata.mtime()),
            decorated_name(entry.name.as_str(), &entry.metadata, args.classify),
            nlink_w = nlink_w,
            owner_w = owner_w,
            group_w = group_w,
            size_w = size_w,
        );
        out.write_all(line.as_bytes())?;
    }
    Ok(())
}

/// Looks up user and group names from `/etc/passwd` and `/etc/group`,
/// falling back to numeric ids when a name is missing or the files are
/// unreadable.
struct OwnerLookup {
    users: Vec<(u32, String)>,
    groups: Vec<(u32, String)>,
}

impl OwnerLookup {
    fn load() -> Self {
        Self {
            users: read_id_name("/etc/passwd"),
            groups: read_id_name("/etc/group"),
        }
    }

    fn user(&self, uid: u32) -> String {
        self.users
            .iter()
            .find(|(id, _)| *id == uid)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("{}", uid))
    }

    fn group(&self, gid: u32) -> String {
        self.groups
            .iter()
            .find(|(id, _)| *id == gid)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("{}", gid))
    }
}

/// Parses `name:_:id:...`-style colon files (passwd / group).
fn read_id_name(path: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let Ok(contents) = fs::read_to_string(path) else {
        return out;
    };
    for line in contents.lines() {
        let mut parts = line.splitn(4, ':');
        let Some(name) = parts.next() else { continue };
        let _x = parts.next();
        let Some(id_str) = parts.next() else { continue };
        let Ok(id) = id_str.parse::<u32>() else {
            continue;
        };
        out.push((id, name.to_string()));
    }
    out
}

/// Formats a Unix mtime (seconds since epoch) as `MMM DD HH:MM`, matching
/// the abbreviated form GNU `ls` uses for "recent" files. We have no
/// timezone or "is it within six months?" logic, so this is approximate.
fn format_mtime(secs: i64) -> String {
    let (year, month, day, hour, minute) = unix_to_calendar(secs);
    let month_name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(month - 1) as usize];
    let _ = year;
    format!("{} {:>2} {:02}:{:02}", month_name, day, hour, minute)
}

/// Naive Unix-time → (year, month, day, hour, minute) conversion in UTC.
fn unix_to_calendar(secs: i64) -> (i32, u32, u32, u32, u32) {
    let mut days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400) as u32;
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;

    let mut year: i32 = 1970;
    loop {
        let dy = if is_leap_year(year) { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let months = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month: u32 = 1;
    for &m in &months {
        let dm = if month == 2 && is_leap_year(year) {
            29
        } else {
            m
        };
        if days < dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    let day = days as u32 + 1;
    (year, month, day, hour, minute)
}

fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Writes entries one per line.
fn write_one_per_line(
    out: &mut impl Write,
    entries: &[ListingEntry],
    args: &LsArgs,
) -> user_lib::io::Result<()> {
    for entry in entries {
        let line = format!(
            "{}\n",
            decorated_name(entry.name.as_str(), &entry.metadata, args.classify)
        );
        out.write_all(line.as_bytes())?;
    }
    Ok(())
}

/// Writes entries in vertical columns, matching the usual interactive `ls`.
fn write_columns(
    out: &mut impl Write,
    entries: &[ListingEntry],
    args: &LsArgs,
) -> user_lib::io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let names = decorated_names(entries, args.classify);
    let name_width = names
        .iter()
        .map(|name| name.len())
        .max()
        .unwrap_or(0)
        .saturating_add(COLUMN_PADDING);
    let column_width = name_width.max(1);
    let columns = (DEFAULT_TERMINAL_WIDTH / column_width)
        .max(1)
        .min(names.len());
    let rows = names.len().div_ceil(columns);

    for row in 0..rows {
        for column in 0..columns {
            let index = column * rows + row;
            if index >= names.len() {
                continue;
            }
            let name = &names[index];
            out.write_all(name.as_bytes())?;
            if column + 1 < columns && index + rows < names.len() {
                write_padding(out, column_width.saturating_sub(name.len()))?;
            }
        }
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// Builds display names after applying file-kind suffixes.
fn decorated_names(entries: &[ListingEntry], classify: bool) -> Vec<String> {
    entries
        .iter()
        .map(|entry| decorated_name(entry.name.as_str(), &entry.metadata, classify))
        .collect()
}

/// Writes spaces used to align multi-column output.
fn write_padding(out: &mut impl Write, mut count: usize) -> user_lib::io::Result<()> {
    while count > 0 {
        out.write_all(b" ")?;
        count -= 1;
    }
    Ok(())
}

/// Estimates the block count used by `ls -l`'s total line.
fn total_blocks(entries: &[ListingEntry]) -> u64 {
    entries
        .iter()
        .map(|entry| (entry.metadata.len() + 1023) / 1024)
        .sum()
}

/// Returns the number of decimal digits needed to print `value`.
fn decimal_width(mut value: u64) -> usize {
    let mut width = 1;
    while value >= 10 {
        value /= 10;
        width += 1;
    }
    width
}

/// Adds the `ls -F` file-kind suffix when requested.
fn decorated_name(name: &str, metadata: &fs::Metadata, classify: bool) -> String {
    let mut out = String::from(name);
    if classify {
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            out.push('/');
        } else if file_type.is_fifo() {
            out.push('|');
        } else if file_type.is_char_device() || file_type.is_block_device() {
            out.push('=');
        } else if metadata.permissions().mode() & 0o111 != 0 {
            out.push('*');
        }
    }
    out
}

/// Formats Unix permission bits in the familiar `drwxr-xr-x` style.
fn mode_string(metadata: &fs::Metadata) -> String {
    let file_type = metadata.file_type();
    let kind = if file_type.is_dir() {
        'd'
    } else if file_type.is_char_device() {
        'c'
    } else if file_type.is_block_device() {
        'b'
    } else if file_type.is_fifo() {
        'p'
    } else {
        '-'
    };

    let mode = metadata.permissions().mode();
    let mut out = String::with_capacity(10);
    out.push(kind);
    for bit in [
        0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001,
    ] {
        let ch = match bit {
            0o400 | 0o040 | 0o004 => 'r',
            0o200 | 0o020 | 0o002 => 'w',
            _ => 'x',
        };
        out.push(if mode & bit != 0 { ch } else { '-' });
    }
    out
}

/// Joins a directory path with one child name.
fn join_path(dir: &str, name: &str) -> String {
    let mut path = String::with_capacity(dir.len() + 1 + name.len());
    path.push_str(dir);
    if !path.ends_with('/') {
        path.push('/');
    }
    path.push_str(name);
    path
}
