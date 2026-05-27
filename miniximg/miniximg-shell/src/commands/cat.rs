//! `cat` — print files. Accepts both image and host paths.

use std::io::Write;

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "cat";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "concatenate files to stdout";
pub const USAGE: &str = "cat FILE...";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    super::expect_args(args, 1, None)?;
    let cwd = session.image_cwd().to_string();
    let host_cwd = session.host_cwd().to_path_buf();
    for raw in args {
        let resolved = path::resolve(raw, &cwd, &host_cwd);
        let data = match resolved {
            path::AnyPath::Image(p) => session.fs_mut().read_file_at_path(&p)?,
            path::AnyPath::Host(p) => std::fs::read(&p)?,
        };
        std::io::stdout().write_all(&data)?;
    }
    Ok(())
}
