//! `rm` — remove files (and, with `-r`, recursive trees) on the image.

use anyhow::Result;
use miniximg::InodeType;

use crate::{path, session::Session};

pub const NAME: &str = "rm";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "remove files (or trees with -r)";
pub const USAGE: &str = "rm [-r] [-f] PATH...";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("rm")?;
    let mut recursive = false;
    let mut force = false;
    let mut paths: Vec<&str> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-r" | "-R" => recursive = true,
            "-f" => force = true,
            "-rf" | "-fr" => {
                recursive = true;
                force = true;
            }
            other => paths.push(other),
        }
    }
    if paths.is_empty() {
        anyhow::bail!("missing operand");
    }
    let cwd = session.image_cwd().to_string();
    for raw in paths {
        let resolved = path::resolve_image(raw, &cwd);
        if let Err(err) = remove_one(session, &resolved, recursive)
            && !force
        {
            return Err(err);
        }
    }
    session.fs_mut().flush()?;
    Ok(())
}

fn remove_one(session: &mut Session, path: &str, recursive: bool) -> Result<()> {
    let meta = session.fs_mut().stat(path)?;
    if meta.kind == InodeType::Directory {
        if !recursive {
            anyhow::bail!("{}: is a directory (use -r)", path);
        }
        let children: Vec<String> = session
            .fs_mut()
            .list_path(path)?
            .into_iter()
            .map(|e| e.metadata.path)
            .collect();
        for child in children {
            remove_one(session, &child, true)?;
        }
        session.fs_mut().remove_directory(path)?;
    } else {
        session.fs_mut().remove_file(path)?;
    }
    Ok(())
}
