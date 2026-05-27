//! `ln` — hard link (image-side only).

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "ln";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "create a hard link inside the image";
pub const USAGE: &str = "ln SRC LINK";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("ln")?;
    super::expect_args(args, 2, Some(2))?;
    let cwd = session.image_cwd().to_string();
    let src = path::resolve_image(&args[0], &cwd);
    let dst = path::resolve_image(&args[1], &cwd);
    session.fs_mut().link_path(&src, &dst)?;
    session.fs_mut().flush()?;
    Ok(())
}
