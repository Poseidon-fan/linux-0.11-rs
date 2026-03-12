//! Human-friendly and machine-friendly reports for inspection and verification.

use std::path::PathBuf;

use serde::Serialize;

use crate::error::Result;
use crate::layout::PartitionType;
use crate::mbr::MbrHeader;

/// The diagnostic severity levels produced by analysis commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticLevel {
    /// A hard failure.
    Error,
    /// A softer warning that becomes fatal in strict mode.
    Warning,
}

/// A structured diagnostic emitted during inspection or verification.
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    /// The severity of the diagnostic.
    pub level: DiagnosticLevel,
    /// A stable diagnostic code.
    pub code: &'static str,
    /// The human-friendly message.
    pub message: String,
}

/// The per-partition view used by `inspect` and `verify`.
#[derive(Clone, Debug, Serialize)]
pub struct PartitionReport {
    /// The one-based slot number.
    pub slot: usize,
    /// Whether the slot is populated.
    pub used: bool,
    /// Whether the slot is marked active.
    pub bootable: bool,
    /// The raw partition type code.
    pub partition_type: u8,
    /// A known partition type alias when available.
    pub partition_type_name: Option<&'static str>,
    /// The starting sector.
    pub start_lba: u64,
    /// The number of sectors.
    pub sectors: u64,
    /// The inclusive end sector.
    pub end_lba: Option<u64>,
    /// The starting byte offset.
    pub start_offset: u64,
    /// The reserved byte length.
    pub byte_len: u64,
}

/// The full report emitted by `inspect`.
#[derive(Clone, Debug, Serialize)]
pub struct InspectReport {
    /// The analyzed disk image path.
    pub disk: PathBuf,
    /// The logical disk size in bytes.
    pub disk_size: u64,
    /// The logical sector count derived from the file size.
    pub sector_count: u64,
    /// Whether the raw MBR signature is valid.
    pub mbr_signature_valid: bool,
    /// The stored disk signature.
    pub disk_signature: u32,
    /// The decoded MBR header.
    pub mbr: MbrHeader,
    /// The partition table view.
    pub partitions: Vec<PartitionReport>,
    /// Non-fatal diagnostics collected during inspection.
    pub diagnostics: Vec<Diagnostic>,
}

/// The full report emitted by `verify`.
#[derive(Clone, Debug, Serialize)]
pub struct VerifyReport {
    /// Whether the image passed validation under the selected mode.
    pub ok: bool,
    /// Whether strict mode was enabled.
    pub strict: bool,
    /// The inspection report that verification was based on.
    pub inspect: InspectReport,
    /// Validation diagnostics emitted by the verifier.
    pub diagnostics: Vec<Diagnostic>,
}

impl InspectReport {
    /// Render the report as indented JSON.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Render the report as a stable text table.
    pub fn to_text(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("Disk: {}\n", self.disk.display()));
        output.push_str(&format!("Size: {} bytes\n", self.disk_size));
        output.push_str(&format!("Sectors: {}\n", self.sector_count));
        output.push_str(&format!(
            "MBR signature: {}\n",
            if self.mbr_signature_valid {
                "valid"
            } else {
                "invalid"
            }
        ));
        output.push_str(&format!(
            "Disk signature: 0x{:08x}\n\n",
            self.disk_signature
        ));
        output.push_str("Slot  Boot  Type   Start    Sectors    End      Bytes\n");
        output.push_str("----  ----  -----  -------  ---------  -------  ---------\n");

        for partition in &self.partitions {
            let boot = if partition.bootable { "*" } else { "-" };
            let type_name = format!("0x{:02x}", partition.partition_type);
            let end_lba = partition
                .end_lba
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".into());
            output.push_str(&format!(
                "{:<4}  {:<4}  {:<5}  {:<7}  {:<9}  {:<7}  {}\n",
                partition.slot,
                boot,
                type_name,
                partition.start_lba,
                partition.sectors,
                end_lba,
                partition.byte_len
            ));
        }

        if !self.diagnostics.is_empty() {
            output.push_str("\nDiagnostics:\n");

            for diagnostic in &self.diagnostics {
                output.push_str(&format!(
                    "- {:?} {}: {}\n",
                    diagnostic.level, diagnostic.code, diagnostic.message
                ));
            }
        }

        output
    }
}

impl VerifyReport {
    /// Render the report as indented JSON.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Render the report as a stable text summary.
    pub fn to_text(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!(
            "Verification: {}\n",
            if self.ok { "ok" } else { "failed" }
        ));
        output.push_str(&format!("Strict mode: {}\n\n", self.strict));
        output.push_str(&self.inspect.to_text());

        if !self.diagnostics.is_empty() {
            output.push_str("\nVerification diagnostics:\n");

            for diagnostic in &self.diagnostics {
                output.push_str(&format!(
                    "- {:?} {}: {}\n",
                    diagnostic.level, diagnostic.code, diagnostic.message
                ));
            }
        }

        output
    }
}

/// Convert a raw MBR header into a stable partition report list.
pub fn build_partition_reports(mbr: &MbrHeader) -> Vec<PartitionReport> {
    mbr.partitions
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let partition_type = PartitionType(entry.partition_type);
            let start_lba = entry.starting_lba as u64;
            let sectors = entry.sectors as u64;

            PartitionReport {
                slot: index + 1,
                used: !entry.is_empty(),
                bootable: entry.is_bootable(),
                partition_type: entry.partition_type,
                partition_type_name: partition_type.known_name(),
                start_lba,
                sectors,
                end_lba: entry.end_lba(),
                start_offset: start_lba * crate::mbr::SECTOR_SIZE as u64,
                byte_len: sectors * crate::mbr::SECTOR_SIZE as u64,
            }
        })
        .collect()
}
