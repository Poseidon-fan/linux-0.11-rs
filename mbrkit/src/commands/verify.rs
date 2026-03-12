//! Implementation of `mbrkit verify`.

use crate::cli::{OutputFormat, VerifyArgs};
use crate::commands::inspect::inspect_disk;
use crate::error::{MbrkitError, Result};
use crate::report::{Diagnostic, DiagnosticLevel, VerifyReport};

/// Execute the `verify` command.
pub fn run(args: VerifyArgs) -> Result<()> {
    let report = verify_disk(&args)?;

    match args.format {
        OutputFormat::Table => println!("{}", report.to_text()),
        OutputFormat::Json => println!("{}", report.to_json()?),
    }

    if report.ok {
        Ok(())
    } else {
        Err(MbrkitError::SilentFailure(2))
    }
}

/// Validate a disk image and build the verification report.
pub fn verify_disk(args: &VerifyArgs) -> Result<VerifyReport> {
    let inspect = inspect_disk(&args.disk)?;
    let mut diagnostics = Vec::new();
    let mut used = inspect
        .partitions
        .iter()
        .filter(|partition| partition.used)
        .collect::<Vec<_>>();
    used.sort_by_key(|partition| partition.start_lba);

    if !inspect.mbr_signature_valid {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            code: "invalid_signature",
            message: "MBR signature is not 0x55AA".into(),
        });
    }

    for partition in &used {
        let boot_indicator = inspect.mbr.partitions[partition.slot - 1].boot_indicator;

        if boot_indicator != 0x00 && boot_indicator != 0x80 {
            diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Error,
                code: "invalid_boot_flag",
                message: format!(
                    "partition {} has an invalid boot flag 0x{:02x}",
                    partition.slot, boot_indicator
                ),
            });
        }

        if partition.start_offset + partition.byte_len > inspect.disk_size {
            diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Error,
                code: "out_of_bounds",
                message: format!(
                    "partition {} extends beyond the end of the disk image",
                    partition.slot
                ),
            });
        }

        if partition.partition_type_name.is_none() && partition.partition_type != 0x00 {
            diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Warning,
                code: "unknown_partition_type",
                message: format!(
                    "partition {} uses an unknown type 0x{:02x}",
                    partition.slot, partition.partition_type
                ),
            });
        }
    }

    for window in used.windows(2) {
        let left = window[0];
        let right = window[1];

        if left.start_lba <= right.start_lba && left.end_lba.unwrap_or(0) >= right.start_lba {
            diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Error,
                code: "overlap",
                message: format!("partitions {} and {} overlap", left.slot, right.slot),
            });
        }
    }

    let bootable_count = used.iter().filter(|partition| partition.bootable).count();

    if args.strict && inspect.disk_signature == 0 {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            code: "zero_disk_signature",
            message: "disk signature is zero".into(),
        });
    }

    if args.strict && bootable_count > 1 {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            code: "multiple_active_partitions",
            message: "more than one partition is marked active".into(),
        });
    }

    let ok = diagnostics.iter().all(|diagnostic| {
        diagnostic.level != DiagnosticLevel::Error
            && (!args.strict || diagnostic.level != DiagnosticLevel::Warning)
    });

    Ok(VerifyReport {
        ok,
        strict: args.strict,
        inspect,
        diagnostics,
    })
}
