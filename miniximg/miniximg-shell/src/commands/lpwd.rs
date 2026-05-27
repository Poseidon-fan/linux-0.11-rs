//! `lpwd` — print only the host-side cwd.

use anyhow::Result;

use crate::session::Session;

pub const NAME: &str = "lpwd";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "print the host-side working directory";
pub const USAGE: &str = "lpwd";

pub fn run(session: &mut Session, _args: &[String]) -> Result<()> {
    println!("{}", session.host_cwd().display());
    Ok(())
}
