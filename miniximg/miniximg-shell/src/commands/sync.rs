//! `sync` — flush any buffered writes to the image file.

use anyhow::Result;

use crate::session::Session;

pub const NAME: &str = "sync";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "flush pending writes to the image file";
pub const USAGE: &str = "sync";

pub fn run(session: &mut Session, _args: &[String]) -> Result<()> {
    session.fs_mut().flush()?;
    Ok(())
}
