//! `cp` — copy files. Direction follows the `@` prefixes on its operands.
//!
//! | src    | dst    | behaviour                                      |
//! |--------|--------|------------------------------------------------|
//! | image  | image  | image→image (recursive with `-r`)              |
//! | host   | host   | host→host (delegated to `std::fs::copy`)       |
//! | host   | image  | host→image (same as `put`)                     |
//! | image  | host   | image→host (same as `get`)                     |
//!
//! Recursive (`-r`) copies are supported for image→image transfers only;
//! recursive cross-filesystem trees are intentionally out of scope.

use std::{io::Write, path::Path};

use anyhow::Result;
use miniximg::{CreateNodeOptions, InodeType};

use crate::{path as shellpath, session::Session};

pub const NAME: &str = "cp";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "copy a file (auto-detects host/image direction)";
pub const USAGE: &str = "cp [-r] SRC DST";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    let mut recursive = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-r" | "-R" => recursive = true,
            other => positional.push(other),
        }
    }
    if positional.len() != 2 {
        anyhow::bail!("cp expects two operands (got {})", positional.len());
    }

    let cwd = session.image_cwd().to_string();
    let host_cwd = session.host_cwd().to_path_buf();
    let src = shellpath::resolve(positional[0], &cwd, &host_cwd);
    let dst = shellpath::resolve(positional[1], &cwd, &host_cwd);

    use shellpath::AnyPath::{Host, Image};
    match (src, dst) {
        (Host(s), Host(d)) => {
            std::fs::copy(&s, &d)?;
        }
        (Host(s), Image(d)) => {
            session.require_writable("cp")?;
            host_to_image(session, &s, &d)?;
            session.fs_mut().flush()?;
        }
        (Image(s), Host(d)) => image_to_host(session, &s, &d)?,
        (Image(s), Image(d)) => {
            session.require_writable("cp")?;
            image_to_image(session, &s, &d, recursive)?;
            session.fs_mut().flush()?;
        }
    }
    Ok(())
}

fn host_to_image(session: &mut Session, src: &Path, dst: &str) -> Result<()> {
    let data = std::fs::read(src)?;
    session.fs_mut().write_file_at_path(
        dst,
        &data,
        &super::default_node_options(0o644),
        true,
        &super::default_parent_options(),
    )?;
    Ok(())
}

fn image_to_host(session: &mut Session, src: &str, dst: &Path) -> Result<()> {
    let data = session.fs_mut().read_file_at_path(src)?;
    let mut out = std::fs::File::create(dst)?;
    out.write_all(&data)?;
    Ok(())
}

fn image_to_image(session: &mut Session, src: &str, dst: &str, recursive: bool) -> Result<()> {
    let meta = session.fs_mut().stat(src)?;
    if meta.kind == InodeType::Directory {
        if !recursive {
            anyhow::bail!("{}: is a directory (use -r)", src);
        }
        let parent_opts = super::default_parent_options();
        session.fs_mut().mkdir_all(dst, &parent_opts)?;
        // Snapshot children before recursing — `image_to_image` will
        // re-borrow `session.fs_mut()` and we can't keep an iterator
        // live across the recursive call.
        let children: Vec<(String, String)> = session
            .fs_mut()
            .list_path(src)?
            .into_iter()
            .map(|e| (e.metadata.path, join_image(dst, &e.name)))
            .collect();
        for (s, d) in children {
            image_to_image(session, &s, &d, true)?;
        }
    } else {
        let data = session.fs_mut().read_file_at_path(src)?;
        // Preserve the source's mode bits and owners; mtime advances to
        // the time of the copy.
        let opts = CreateNodeOptions {
            mode: meta.mode & 0o7777,
            uid: meta.uid,
            gid: meta.gid,
            mtime: super::now_secs(),
        };
        session.fs_mut().write_file_at_path(
            dst,
            &data,
            &opts,
            true,
            &super::default_parent_options(),
        )?;
    }
    Ok(())
}

fn join_image(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}
