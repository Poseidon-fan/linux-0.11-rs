//! Implementation of `mbrkit inspect`.

use std::fs;
use std::path::Path;

use crate::cli::{InspectArgs, OutputFormat};
use crate::error::{MbrkitError, Result};
use crate::mbr::{MbrHeader, SECTOR_SIZE};
use crate::report::{Diagnostic, DiagnosticLevel, InspectReport, build_partition_reports};

/// Execute the `inspect` command.
pub fn run(args: InspectArgs) -> Result<()> {
    let report = inspect_disk(&args.disk)?;

    match args.format {
        OutputFormat::Table => println!("{}", report.to_text()),
        OutputFormat::Json => println!("{}", report.to_json()?),
    }

    Ok(())
}

/// Inspect a disk image and return the structured report.
pub fn inspect_disk(path: &Path) -> Result<InspectReport> {
    let data = fs::read(path).map_err(|source| {
        MbrkitError::io(
            Some(path.to_path_buf()),
            "failed to read disk image",
            source,
        )
    })?;
    let disk_size = data.len() as u64;
    let mbr = MbrHeader::from_sector(&data)?;
    let mut diagnostics = Vec::new();

    if !mbr.has_valid_signature() {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            code: "invalid_signature",
            message: "MBR signature is not 0x55AA".into(),
        });
    }

    Ok(InspectReport {
        disk: path.to_path_buf(),
        disk_size,
        sector_count: disk_size / SECTOR_SIZE as u64,
        mbr_signature_valid: mbr.has_valid_signature(),
        disk_signature: mbr.disk_signature,
        partitions: build_partition_reports(&mbr),
        mbr,
        diagnostics,
    })
}
