//! Implementation of `mbrkit extract`.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};

use crate::cli::ExtractArgs;
use crate::commands::inspect::inspect_disk;
use crate::error::{MbrkitError, Result};

/// Execute the `extract` command.
pub fn run(args: ExtractArgs) -> Result<()> {
    if args.partition == 0 || args.partition > 4 {
        return Err(MbrkitError::InvalidArgument(
            "`--partition` must be in the range 1..=4".into(),
        ));
    }

    if args.output.exists() && !args.force {
        return Err(MbrkitError::InvalidArgument(format!(
            "output image `{}` already exists; use --force to overwrite it",
            args.output.display()
        )));
    }

    let report = inspect_disk(&args.disk)?;
    let partition = report
        .partitions
        .iter()
        .find(|partition| partition.slot == args.partition)
        .ok_or_else(|| {
            MbrkitError::InvalidArgument("requested partition slot does not exist".into())
        })?;

    if !partition.used {
        return Err(MbrkitError::InvalidArgument(format!(
            "partition {} is empty",
            args.partition
        )));
    }

    let data = fs::read(&args.disk).map_err(|source| {
        MbrkitError::io(
            Some(args.disk.clone()),
            "failed to read disk image for extraction",
            source,
        )
    })?;
    let start = usize::try_from(partition.start_offset).map_err(|_| {
        MbrkitError::InvalidArgument("partition offset exceeds addressable memory".into())
    })?;
    let end = start
        .checked_add(usize::try_from(partition.byte_len).map_err(|_| {
            MbrkitError::InvalidArgument("partition size exceeds addressable memory".into())
        })?)
        .ok_or_else(|| MbrkitError::InvalidArgument("partition range overflowed".into()))?;

    if end > data.len() {
        return Err(MbrkitError::InvalidArgument(format!(
            "partition {} extends past the end of the disk image",
            args.partition
        )));
    }

    let mut output = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&args.output)
        .map_err(|source| {
            MbrkitError::io(
                Some(args.output.clone()),
                "failed to create extracted image",
                source,
            )
        })?;
    output.seek(SeekFrom::Start(0)).map_err(|source| {
        MbrkitError::io(
            Some(args.output.clone()),
            "failed to seek extracted image",
            source,
        )
    })?;
    output.write_all(&data[start..end]).map_err(|source| {
        MbrkitError::io(
            Some(args.output.clone()),
            "failed to write extracted image",
            source,
        )
    })?;
    output.flush().map_err(|source| {
        MbrkitError::io(
            Some(args.output.clone()),
            "failed to flush extracted image",
            source,
        )
    })?;

    println!(
        "Extracted partition {} to {}",
        args.partition,
        args.output.display()
    );
    Ok(())
}
