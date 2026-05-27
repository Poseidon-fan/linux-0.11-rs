//! `lcd` — change the host-side cwd.

use std::path::PathBuf;

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "lcd";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "change the host-side directory";
pub const USAGE: &str = "lcd [PATH]";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    let target = match args.first().map(String::as_str) {
        None => default_home(),
        Some(raw) => {
            // Allow `lcd /tmp` without the user typing `@`; lcd is
            // unambiguously host-side, so we just expand-and-resolve.
            let with_at = format!("@{}", raw.trim_start_matches('@'));
            match path::resolve(&with_at, session.image_cwd(), session.host_cwd()) {
                path::AnyPath::Host(p) => p,
                path::AnyPath::Image(_) => unreachable!(),
            }
        }
    };
    session.set_host_cwd(target)
}

fn default_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}
