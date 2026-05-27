//! `chmod` — change permission bits on an image-side path.
//!
//! The core library doesn't yet expose a standalone `chmod`. We
//! approximate it for regular files by rewriting their contents with new
//! mode bits but the same uid/gid/mtime. Directories and device nodes
//! aren't supported yet.

use anyhow::{Result, anyhow};
use miniximg::{CreateNodeOptions, InodeType};

use crate::{path, session::Session};

pub const NAME: &str = "chmod";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "change permission bits on an image path";
pub const USAGE: &str = "chmod MODE PATH...";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("chmod")?;
    super::expect_args(args, 2, None)?;
    let mode = parse_mode(&args[0])?;
    let cwd = session.image_cwd().to_string();
    let parent_opts = super::default_parent_options();

    for raw in &args[1..] {
        let resolved = path::resolve_image(raw, &cwd);
        let meta = session.fs_mut().stat(&resolved)?;
        if meta.kind != InodeType::Regular {
            anyhow::bail!("{}: chmod only supports regular files for now", resolved);
        }
        let data = session.fs_mut().read_file_at_path(&resolved)?;
        let opts = CreateNodeOptions {
            mode,
            uid: meta.uid,
            gid: meta.gid,
            mtime: meta.modification_time,
        };
        session
            .fs_mut()
            .write_file_at_path(&resolved, &data, &opts, true, &parent_opts)?;
    }
    session.fs_mut().flush()?;
    Ok(())
}

/// Parses an octal mode string, tolerating a leading `0` like `chmod(1)`.
fn parse_mode(s: &str) -> Result<u16> {
    let cleaned = s.trim_start_matches('0');
    let parsed = u16::from_str_radix(if cleaned.is_empty() { "0" } else { cleaned }, 8)
        .map_err(|_| anyhow!("bad octal mode: {}", s))?;
    Ok(parsed & 0o7777)
}
