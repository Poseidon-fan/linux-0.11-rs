//! `stat` — full metadata for an image or host path.

use anyhow::Result;
use miniximg::InodeType;

use crate::{path, session::Session};

pub const NAME: &str = "stat";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "show full metadata for a path";
pub const USAGE: &str = "stat PATH";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    super::expect_args(args, 1, Some(1))?;
    let cwd = session.image_cwd().to_string();
    let host_cwd = session.host_cwd().to_path_buf();
    match path::resolve(&args[0], &cwd, &host_cwd) {
        path::AnyPath::Image(p) => {
            let meta = session.fs_mut().stat(&p)?;
            println!("Path:        {}", meta.path);
            println!("Inode:       {}", meta.inode_number);
            println!("Type:        {}", type_label(meta.kind));
            println!("Mode:        {:04o}", meta.mode & 0o7777);
            println!("Owner:       uid={} gid={}", meta.uid, meta.gid);
            println!("Size:        {} bytes", meta.size);
            println!("Links:       {}", meta.link_count);
            println!("Modified:    {}", meta.modification_time);
            if let Some(dev) = meta.device_number {
                let major = (dev >> 8) & 0xff;
                let minor = dev & 0xff;
                println!("Device:      {},{} (raw 0x{:04x})", major, minor, dev);
            }
        }
        path::AnyPath::Host(p) => {
            let meta = std::fs::metadata(&p)?;
            println!("Path:        {} (host)", p.display());
            println!("Type:        {}", host_type_label(&meta));
            println!("Size:        {} bytes", meta.len());
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                println!("Mode:        {:04o}", meta.mode() & 0o7777);
                println!("Owner:       uid={} gid={}", meta.uid(), meta.gid());
            }
        }
    }
    Ok(())
}

fn type_label(kind: InodeType) -> &'static str {
    match kind {
        InodeType::Regular => "regular file",
        InodeType::Directory => "directory",
        InodeType::Fifo => "fifo",
        InodeType::BlockDevice => "block device",
        InodeType::CharacterDevice => "character device",
    }
}

fn host_type_label(meta: &std::fs::Metadata) -> &'static str {
    if meta.is_dir() {
        "directory"
    } else if meta.is_symlink() {
        "symlink"
    } else if meta.is_file() {
        "regular file"
    } else {
        "other"
    }
}
