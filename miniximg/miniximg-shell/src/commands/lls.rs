//! `lls` — list a host-side directory using the system `ls(1)` style.

use std::path::PathBuf;

use anyhow::Result;

use crate::{path, session::Session};

pub const NAME: &str = "lls";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "list directory contents (host-side)";
pub const USAGE: &str = "lls [PATH]";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    let target = match args.first() {
        Some(raw) => {
            let with_at = format!("@{}", raw.trim_start_matches('@'));
            match path::resolve(&with_at, session.image_cwd(), session.host_cwd()) {
                path::AnyPath::Host(p) => p,
                path::AnyPath::Image(_) => unreachable!(),
            }
        }
        None => session.host_cwd().to_path_buf(),
    };
    list_one_host(&target)
}

fn list_one_host(target: &PathBuf) -> Result<()> {
    let meta = std::fs::metadata(target)?;
    if !meta.is_dir() {
        println!("{}", target.display());
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(target)?.filter_map(|r| r.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        let suffix = entry
            .file_type()
            .ok()
            .and_then(|ft| if ft.is_dir() { Some('/') } else { None })
            .unwrap_or(' ');
        if suffix == '/' {
            println!("{}/", name.to_string_lossy());
        } else {
            println!("{}", name.to_string_lossy());
        }
    }
    Ok(())
}
