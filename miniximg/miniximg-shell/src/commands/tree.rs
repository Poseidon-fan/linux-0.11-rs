//! `tree` — recursive listing inside the image.

use anyhow::Result;
use miniximg::InodeType;

use crate::{path, session::Session};

pub const NAME: &str = "tree";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "print an image subtree";
pub const USAGE: &str = "tree [PATH]";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    let target = match args.first() {
        Some(raw) => path::resolve_image(raw, session.image_cwd()),
        None => session.image_cwd().to_string(),
    };
    let entries = session.fs_mut().tree(&target)?;
    for entry in entries {
        let indent = "  ".repeat(entry.depth);
        let suffix = if entry.metadata.kind == InodeType::Directory {
            "/"
        } else {
            ""
        };
        println!("{}{}{}", indent, entry.metadata.path, suffix);
    }
    Ok(())
}
