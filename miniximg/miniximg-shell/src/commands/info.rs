//! `info` — image-level summary (size, inode usage, free blocks).

use anyhow::Result;

use crate::session::Session;

pub const NAME: &str = "info";
pub const ALIASES: &[&str] = &[];
pub const SUMMARY: &str = "show image-level metadata";
pub const USAGE: &str = "info";

pub fn run(session: &mut Session, _args: &[String]) -> Result<()> {
    let label = session.image_label();
    let report = session.fs_mut().inspect()?;
    let used_inodes = (report.inode_count as usize).saturating_sub(report.free_inodes);
    let used_zones = (report.zone_count as usize).saturating_sub(report.free_zones);
    println!("Image:       {}", label);
    println!("Block size:  {} bytes", report.block_size);
    println!(
        "Zones:       {} total, {} used, {} free",
        report.zone_count, used_zones, report.free_zones
    );
    println!(
        "Inodes:      {} total, {} used, {} free",
        report.inode_count, used_inodes, report.free_inodes
    );
    println!("Max file:    {} bytes", report.max_file_size);
    Ok(())
}
