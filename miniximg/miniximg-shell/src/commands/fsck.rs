//! `fsck` — run the core's validation pass over the image.

use anyhow::Result;

use crate::session::Session;

pub const NAME: &str = "fsck";
pub const ALIASES: &[&str] = &["check"];
pub const SUMMARY: &str = "run validation checks on the image";
pub const USAGE: &str = "fsck";

pub fn run(session: &mut Session, _args: &[String]) -> Result<()> {
    let report = session.fs_mut().check()?;
    if report.issues.is_empty() {
        println!("ok");
        return Ok(());
    }
    println!("{} issue(s) found:", report.issues.len());
    for issue in report.issues {
        match issue.path {
            Some(p) => println!("  {}: {}", p, issue.message),
            None => println!("  {}", issue.message),
        }
    }
    Ok(())
}
