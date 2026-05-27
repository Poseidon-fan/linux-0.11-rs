//! `get` — copy an image file out to the host.

use std::io::Write;

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "get";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "copy an image file out to the host";
pub const USAGE: &str = "get IMAGE_SRC [HOST_DST]";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    super::expect_args(args, 1, Some(2))?;

    let cwd = session.image_cwd().to_string();
    let host_cwd = session.host_cwd().to_path_buf();

    // `get` always reads from the image.
    let src_raw = args[0].strip_prefix('@').unwrap_or(&args[0]);
    let src = path::resolve_image(src_raw, &cwd);

    let dst = match args.get(1) {
        Some(raw) => {
            // Same trick as `put`: dst is always host-side.
            let dst_raw = raw.strip_prefix('@').unwrap_or(raw);
            let with_at = format!("@{}", dst_raw);
            match path::resolve(&with_at, &cwd, &host_cwd) {
                path::AnyPath::Host(p) => p,
                path::AnyPath::Image(_) => unreachable!(),
            }
        }
        None => {
            let name = path::image_basename(&src);
            host_cwd.join(name)
        }
    };

    let data = session.fs_mut().read_file_at_path(&src)?;
    let mut file = std::fs::File::create(&dst)?;
    file.write_all(&data)?;
    Ok(())
}
