//! `mknod` — create a block or character device node.

use anyhow::{Result, anyhow};
use miniximg::DeviceNodeKind;

use crate::{path, session::Session};

pub const NAME: &str = "mknod";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "create a block or character device node";
pub const USAGE: &str = "mknod PATH {b|c} MAJOR MINOR";

pub fn run(session: &mut Session, args: &[String]) -> Result<()> {
    session.require_writable("mknod")?;
    if args.len() != 4 {
        anyhow::bail!("mknod expects 4 arguments");
    }
    let kind = match args[1].as_str() {
        "b" | "block" => DeviceNodeKind::Block,
        "c" | "char" | "character" => DeviceNodeKind::Character,
        other => return Err(anyhow!("kind must be `b` or `c`, got {}", other)),
    };
    let major: u8 = args[2].parse()?;
    let minor: u8 = args[3].parse()?;
    // Minix v1 packs the device number into one 16-bit field: high
    // byte = major, low byte = minor.
    let device_number = ((major as u16) << 8) | (minor as u16);

    let cwd = session.image_cwd().to_string();
    let resolved = path::resolve_image(&args[0], &cwd);
    session.fs_mut().create_device_at_path(
        &resolved,
        kind,
        device_number,
        &super::default_node_options(0o644),
        &super::default_parent_options(),
    )?;
    session.fs_mut().flush()?;
    Ok(())
}
