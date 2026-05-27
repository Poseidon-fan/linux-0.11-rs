//! `touch` — create an empty file or update its mtime.

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "touch";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "create an empty file or update mtime";
pub const USAGE: &str = "touch [-c] PATH...";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("touch")?;
    let mut no_create = false;
    let mut paths: Vec<&str> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-c" => no_create = true,
            other => paths.push(other),
        }
    }
    if paths.is_empty() {
        anyhow::bail!("missing operand");
    }

    let cwd = session.image_cwd().to_string();
    let file_options = super::default_node_options(0o644);
    let parent_options = super::default_parent_options();

    for raw in paths {
        let resolved = path::resolve_image(raw, &cwd);
        let exists = session.fs_mut().stat(&resolved).is_ok();
        if !exists && no_create {
            continue;
        }
        // The core lib doesn't yet expose a "bump mtime only" op, so for
        // an existing file we rewrite its contents with itself. Newly
        // created files start out empty, like POSIX `touch`.
        let data = if exists {
            session.fs_mut().read_file_at_path(&resolved)?
        } else {
            Vec::new()
        };
        session.fs_mut().write_file_at_path(
            &resolved,
            &data,
            &file_options,
            true,
            &parent_options,
        )?;
    }
    session.fs_mut().flush()?;
    Ok(())
}
