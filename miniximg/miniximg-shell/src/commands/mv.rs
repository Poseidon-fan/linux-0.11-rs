//! `mv` — rename / move (image-side only).
//!
//! Cross-filesystem moves (image ↔ host) are intentionally **not**
//! supported by `mv`; use `put` / `get` if you want to transfer.

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "mv";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "rename or move within the image";
pub const USAGE: &str = "mv SRC DST";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("mv")?;
    super::expect_args(args, 2, Some(2))?;
    let cwd = session.image_cwd().to_string();
    let host_cwd = session.host_cwd().to_path_buf();

    let src = path::resolve(&args[0], &cwd, &host_cwd);
    let dst = path::resolve(&args[1], &cwd, &host_cwd);
    if src.is_host() || dst.is_host() {
        anyhow::bail!("cross-filesystem mv: use `put`, `get`, or `cp`");
    }
    let path::AnyPath::Image(src) = src else {
        unreachable!()
    };
    let path::AnyPath::Image(dst) = dst else {
        unreachable!()
    };

    session.fs_mut().rename_path(&src, &dst)?;
    session.fs_mut().flush()?;
    Ok(())
}
