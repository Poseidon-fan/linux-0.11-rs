//! `pwd` — print the active image-side and host-side cwds.

use anyhow::Result;

use crate::session::Session;

pub const NAME: &str = "pwd";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "show the image and host working directories";
pub const USAGE: &str = "pwd";

pub fn run(session: &mut Session, _args: &[String]) -> Result<()> {
    println!("image: {}", session.image_cwd());
    println!("host:  {}", session.host_cwd().display());
    Ok(())
}
