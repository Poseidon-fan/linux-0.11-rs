//! `exit` / `quit` — leave the shell.
//!
//! Dispatch checks the command name and converts `Ok(())` returned here
//! into `Outcome::Quit`, so the body just succeeds silently.

use anyhow::Result;

use crate::session::Session;

pub const NAME: &str = "exit";
pub const ALIASES: &[&str] = &["quit"];
pub const SUMMARY: &str = "leave the shell";
pub const USAGE: &str = "exit";

pub fn run(_session: &mut Session, _args: &[String]) -> Result<()> {
    Ok(())
}
