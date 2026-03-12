//! Implementation of `mbrkit pack`.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};

use crate::cli::PackArgs;
use crate::error::{MbrkitError, Result};
use crate::layout::DiskLayout;
use crate::mbr::MbrHeader;

/// Execute the `pack` command.
pub fn run(args: PackArgs) -> Result<()> {
    let layout = DiskLayout::from_pack_args(&args)?;

    if args.dry_run {
        print_layout(&layout);
        return Ok(());
    }

    if layout.output.exists() && !args.force {
        return Err(MbrkitError::InvalidArgument(format!(
            "output image `{}` already exists; use --force to overwrite it",
            layout.output.display()
        )));
    }

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    let mut output = options.open(&layout.output).map_err(|source| {
        MbrkitError::io(
            Some(layout.output.clone()),
            "failed to create output disk image",
            source,
        )
    })?;
    output.set_len(layout.disk_size).map_err(|source| {
        MbrkitError::io(
            Some(layout.output.clone()),
            "failed to size output disk image",
            source,
        )
    })?;

    let mut header = MbrHeader::new(layout.bootstrap_code, layout.disk_signature);
    header.reserved = layout.reserved;

    for partition in &layout.partitions {
        header.set_partition(partition.slot - 1, partition)?;
    }

    output.seek(SeekFrom::Start(0)).map_err(|source| {
        MbrkitError::io(
            Some(layout.output.clone()),
            "failed to seek to the MBR sector",
            source,
        )
    })?;
    output.write_all(&header.to_sector()).map_err(|source| {
        MbrkitError::io(
            Some(layout.output.clone()),
            "failed to write the MBR sector",
            source,
        )
    })?;

    for partition in &layout.partitions {
        let mut input = File::open(&partition.file).map_err(|source| {
            MbrkitError::io(
                Some(partition.file.clone()),
                "failed to open partition payload",
                source,
            )
        })?;

        output
            .seek(SeekFrom::Start(partition.start_offset()))
            .map_err(|source| {
                MbrkitError::io(
                    Some(layout.output.clone()),
                    "failed to seek to partition offset",
                    source,
                )
            })?;
        copy_exact_with_padding(&mut input, &mut output, partition.byte_len())?;
    }

    output.flush().map_err(|source| {
        MbrkitError::io(
            Some(layout.output.clone()),
            "failed to flush output disk image",
            source,
        )
    })?;

    println!("Created {}", layout.output.display());
    Ok(())
}

/// Print the resolved disk layout for a dry run.
fn print_layout(layout: &DiskLayout) {
    println!("Output: {}", layout.output.display());
    println!("Disk size: {} bytes", layout.disk_size);
    println!("Disk signature: 0x{:08x}", layout.disk_signature);
    println!("Alignment: {} sectors", layout.align_sectors);
    println!();
    println!("Slot  Boot  Type   Start    Sectors    File");
    println!("----  ----  -----  -------  ---------  ----");

    for partition in &layout.partitions {
        println!(
            "{:<4}  {:<4}  0x{:02x}  {:<7}  {:<9}  {}",
            partition.slot,
            if partition.bootable { "*" } else { "-" },
            partition.partition_type.0,
            partition.start_lba,
            partition.sector_count,
            partition.file.display()
        );
    }
}

/// Copy a partition payload into the disk image and zero-pad the remaining space.
fn copy_exact_with_padding(input: &mut File, output: &mut File, target_len: u64) -> Result<()> {
    let copied = io_copy(input, output)?;

    if copied > target_len {
        return Err(MbrkitError::InvalidArgument(
            "copied payload exceeded the reserved partition size".into(),
        ));
    }

    let mut remaining = target_len - copied;
    let padding = [0_u8; 4096];

    while remaining > 0 {
        let chunk = remaining.min(padding.len() as u64) as usize;
        output
            .write_all(&padding[..chunk])
            .map_err(|source| MbrkitError::io(None, "failed to write partition padding", source))?;
        remaining -= chunk as u64;
    }

    Ok(())
}

/// Wrap `std::io::copy` so the command module keeps consistent error mapping.
fn io_copy(input: &mut File, output: &mut File) -> Result<u64> {
    std::io::copy(input, output)
        .map_err(|source| MbrkitError::io(None, "failed to copy partition payload", source))
}
