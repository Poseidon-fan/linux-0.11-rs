//! `mkdir` — create directories on the image.

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "mkdir";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "create directories";
pub const USAGE: &str = "mkdir [-p] PATH...";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("mkdir")?;
    let mut parents = false;
    let mut paths: Vec<&str> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-p" => parents = true,
            other => paths.push(other),
        }
    }
    if paths.is_empty() {
        anyhow::bail!("missing operand");
    }

    let cwd = session.image_cwd().to_string();
    let options = super::default_node_options(0o755);

    for raw in paths {
        let resolved = path::resolve_image(raw, &cwd);
        // Without `-p`, refuse to overwrite an existing entry — the core
        // `mkdir_all` is silent in that case, but POSIX `mkdir` is not.
        if !parents && session.fs_mut().stat(&resolved).is_ok() {
            anyhow::bail!("{}: exists", resolved);
        }
        session.fs_mut().mkdir_all(&resolved, &options)?;
    }
    session.fs_mut().flush()?;
    Ok(())
}
