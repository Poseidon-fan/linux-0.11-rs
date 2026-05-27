//! `rmdir` — remove empty directories on the image.

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "rmdir";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "remove empty directories";
pub const USAGE: &str = "rmdir PATH...";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("rmdir")?;
    super::expect_args(args, 1, None)?;
    let cwd = session.image_cwd().to_string();
    for raw in args {
        let resolved = path::resolve_image(raw, &cwd);
        session.fs_mut().remove_directory(&resolved)?;
    }
    session.fs_mut().flush()?;
    Ok(())
}
