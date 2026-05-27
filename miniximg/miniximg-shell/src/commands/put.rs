//! `put` — copy a host file into the image.

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "put";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "copy a host file into the image";
pub const USAGE: &str = "put HOST_SRC [IMAGE_DST]";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("put")?;
    super::expect_args(args, 1, Some(2))?;

    let host_cwd = session.host_cwd().to_path_buf();
    let cwd = session.image_cwd().to_string();

    // `put` always reads from the host, regardless of any `@` prefix
    // the user typed; strip it if present and force host resolution.
    let src = {
        let raw = args[0].strip_prefix('@').unwrap_or(&args[0]);
        let with_at = format!("@{}", raw);
        match path::resolve(&with_at, &cwd, &host_cwd) {
            path::AnyPath::Host(p) => p,
            path::AnyPath::Image(_) => unreachable!(),
        }
    };

    // And `put` always writes to the image — `@` on the destination is
    // a typo, not a direction change.
    let dst = match args.get(1) {
        Some(raw) => {
            let raw = raw.strip_prefix('@').unwrap_or(raw);
            path::resolve_image(raw, &cwd)
        }
        None => {
            let name = src
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("source has no file name"))?;
            path::resolve_image(&name.to_string_lossy(), &cwd)
        }
    };

    let data = std::fs::read(&src)?;
    session.fs_mut().write_file_at_path(
        &dst,
        &data,
        &super::default_node_options(0o644),
        true,
        &super::default_parent_options(),
    )?;
    session.fs_mut().flush()?;
    Ok(())
}
