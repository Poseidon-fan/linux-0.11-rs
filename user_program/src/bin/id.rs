//! `id` — print real and effective user and group IDs.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::fmt::Write as _;

use user_lib::{
    eprintln, fs,
    io::{self, Write},
    process::{self, ExitCode, GroupId, UserId},
};
use user_program::cli::cli_args;

cli_args! {
    /// Print user and group information for each specified USER, or the
    /// current process when USER is omitted.
    pub struct IdArgs {
        /// Ignore, for compatibility with other versions.
        pub ignored: bool        = ["-a"],
        /// Print only the effective user ID.
        pub user:    bool        = ["-u", "--user"],
        /// Print only the effective group ID.
        pub group:   bool        = ["-g", "--group"],
        /// Print all group IDs.
        pub groups:  bool        = ["-G", "--groups"],
        /// Print names instead of numbers with -u, -g, or -G.
        pub name:    bool        = ["-n", "--name"],
        /// Print real IDs instead of effective IDs with -u, -g, or -G.
        pub real:    bool        = ["-r", "--real"],
        /// Delimit entries with NUL characters, not whitespace.
        pub zero:    bool        = ["-z", "--zero"],
        /// Users to inspect.
        pub users:   Vec<String> = [..] @ "USER",
    }
}

#[derive(Clone)]
struct Identity {
    uid: UserId,
    euid: UserId,
    gid: GroupId,
    egid: GroupId,
    user_name: Option<String>,
    effective_user_name: Option<String>,
    group_name: Option<String>,
    effective_group_name: Option<String>,
}

#[user_lib::main]
fn main() -> ExitCode {
    let args = IdArgs::parse_env_or_exit();
    let _ = args.ignored;

    if let Err(message) = validate_args(&args) {
        eprintln!("id: {}", message);
        return ExitCode::FAILURE;
    }

    let mut out = io::stdout();
    let mut exit_code = ExitCode::SUCCESS;

    if args.users.is_empty() {
        let identity = current_identity();
        if print_identity(&args, &identity, &mut out).is_err() {
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    for user in &args.users {
        match identity_for_user(user) {
            Some(identity) => {
                if print_identity(&args, &identity, &mut out).is_err() {
                    return ExitCode::FAILURE;
                }
            }
            None => {
                eprintln!("id: '{}': no such user", user);
                exit_code = ExitCode::FAILURE;
            }
        }
    }

    exit_code
}

fn validate_args(args: &IdArgs) -> Result<(), &'static str> {
    let only_count = [args.user, args.group, args.groups]
        .iter()
        .filter(|selected| **selected)
        .count();
    if only_count > 1 {
        return Err("cannot print \"only\" of more than one choice");
    }
    if (args.name || args.real) && only_count == 0 {
        return Err("printing only names or real IDs requires -u, -g, or -G");
    }
    if args.zero && only_count == 0 {
        return Err("option --zero not permitted in default format");
    }
    Ok(())
}

fn current_identity() -> Identity {
    let uid = process::uid();
    let euid = process::euid();
    let gid = process::gid();
    let egid = process::egid();
    Identity {
        uid,
        euid,
        gid,
        egid,
        user_name: lookup_user_name(uid),
        effective_user_name: lookup_user_name(euid),
        group_name: lookup_group_name(gid),
        effective_group_name: lookup_group_name(egid),
    }
}

fn identity_for_user(user: &str) -> Option<Identity> {
    let record = match parse_u32(user) {
        Some(uid) => lookup_user_by_uid(uid)?,
        None => lookup_user_by_name(user)?,
    };
    let group_name = lookup_group_name(record.gid);
    Some(Identity {
        uid: record.uid,
        euid: record.uid,
        gid: record.gid,
        egid: record.gid,
        user_name: Some(record.name.clone()),
        effective_user_name: Some(record.name),
        group_name: group_name.clone(),
        effective_group_name: group_name,
    })
}

fn print_identity(args: &IdArgs, identity: &Identity, out: &mut impl Write) -> io::Result<()> {
    if args.user {
        let id = if args.real {
            identity.uid
        } else {
            identity.euid
        };
        let name = if args.real {
            identity.user_name.as_deref()
        } else {
            identity.effective_user_name.as_deref()
        };
        return print_one(id, name, args.name, args.zero, "user ID", out);
    }
    if args.group {
        let id = if args.real {
            identity.gid
        } else {
            identity.egid
        };
        let name = if args.real {
            identity.group_name.as_deref()
        } else {
            identity.effective_group_name.as_deref()
        };
        return print_one(id, name, args.name, args.zero, "group ID", out);
    }
    if args.groups {
        let id = if args.real {
            identity.gid
        } else {
            identity.egid
        };
        let name = if args.real {
            identity.group_name.as_deref()
        } else {
            identity.effective_group_name.as_deref()
        };
        return print_group_list(id, name, args.name, args.zero, out);
    }

    let mut line = String::new();
    push_labeled_id(
        &mut line,
        "uid",
        identity.uid,
        identity.user_name.as_deref(),
    );
    if identity.euid != identity.uid {
        line.push(' ');
        push_labeled_id(
            &mut line,
            "euid",
            identity.euid,
            identity.effective_user_name.as_deref(),
        );
    }
    line.push(' ');
    push_labeled_id(
        &mut line,
        "gid",
        identity.gid,
        identity.group_name.as_deref(),
    );
    if identity.egid != identity.gid {
        line.push(' ');
        push_labeled_id(
            &mut line,
            "egid",
            identity.egid,
            identity.effective_group_name.as_deref(),
        );
    }
    line.push(' ');
    push_labeled_id(
        &mut line,
        "groups",
        identity.gid,
        identity.group_name.as_deref(),
    );
    line.push('\n');
    out.write_all(line.as_bytes())
}

fn print_one(
    id: u32,
    name: Option<&str>,
    use_name: bool,
    zero: bool,
    label: &str,
    out: &mut impl Write,
) -> io::Result<()> {
    let terminator = if zero { 0 } else { b'\n' };
    if use_name {
        let Some(name) = name else {
            eprintln!("id: cannot find name for {} {}", label, id);
            return Err(io::Error::from(io::ErrorKind::NotFound));
        };
        out.write_all(name.as_bytes())?;
    } else {
        out.write_all(id.to_string().as_bytes())?;
    }
    out.write_all(&[terminator])
}

fn print_group_list(
    id: u32,
    name: Option<&str>,
    use_name: bool,
    zero: bool,
    out: &mut impl Write,
) -> io::Result<()> {
    print_one(id, name, use_name, zero, "group ID", out)
}

fn push_labeled_id(out: &mut String, label: &str, id: u32, name: Option<&str>) {
    let _ = write!(out, "{}={}", label, id);
    if let Some(name) = name {
        out.push('(');
        out.push_str(name);
        out.push(')');
    }
}

#[derive(Clone)]
struct UserRecord {
    name: String,
    uid: UserId,
    gid: GroupId,
}

fn lookup_user_name(uid: UserId) -> Option<String> {
    lookup_user_by_uid(uid).map(|record| record.name)
}

fn lookup_user_by_uid(uid: UserId) -> Option<UserRecord> {
    read_passwd()
        .into_iter()
        .find(|record| record.uid == uid)
        .or_else(|| {
            (uid == 0).then(|| UserRecord {
                name: String::from("root"),
                uid: 0,
                gid: 0,
            })
        })
}

fn lookup_user_by_name(name: &str) -> Option<UserRecord> {
    read_passwd()
        .into_iter()
        .find(|record| record.name == name)
        .or_else(|| {
            (name == "root").then(|| UserRecord {
                name: String::from("root"),
                uid: 0,
                gid: 0,
            })
        })
}

fn read_passwd() -> Vec<UserRecord> {
    let Ok(contents) = fs::read_to_string("/etc/passwd") else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for line in contents.lines() {
        let mut fields = line.split(':');
        let Some(name) = fields.next() else {
            continue;
        };
        let _password = fields.next();
        let Some(uid) = fields.next().and_then(parse_u32) else {
            continue;
        };
        let Some(gid) = fields.next().and_then(parse_u32) else {
            continue;
        };
        records.push(UserRecord {
            name: name.to_string(),
            uid,
            gid,
        });
    }
    records
}

fn lookup_group_name(gid: GroupId) -> Option<String> {
    read_group()
        .into_iter()
        .find(|(record_gid, _)| *record_gid == gid)
        .map(|(_, name)| name)
        .or_else(|| (gid == 0).then(|| String::from("root")))
}

fn read_group() -> Vec<(GroupId, String)> {
    let Ok(contents) = fs::read_to_string("/etc/group") else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for line in contents.lines() {
        let mut fields = line.split(':');
        let Some(name) = fields.next() else {
            continue;
        };
        let _password = fields.next();
        let Some(gid) = fields.next().and_then(parse_u32) else {
            continue;
        };
        records.push((gid, name.to_string()));
    }
    records
}

fn parse_u32(text: &str) -> Option<u32> {
    text.parse::<u32>().ok()
}
