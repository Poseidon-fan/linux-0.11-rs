//! Manifest parsing support for `mbrkit pack`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{MbrkitError, Result};

/// The top-level TOML schema for `mbrkit pack --manifest`.
#[derive(Clone, Debug, Deserialize)]
pub struct PackManifest {
    /// The output disk image path.
    pub output: PathBuf,
    /// The final disk size such as `32MiB`.
    pub disk_size: String,
    /// The optional MBR bootstrap code file.
    pub boot_code: Option<PathBuf>,
    /// The optional disk signature in decimal or hexadecimal notation.
    pub disk_signature: Option<String>,
    /// The optional alignment in sectors for auto-placed partitions.
    pub align_lba: Option<u64>,
    /// The declared partition list in table-slot order.
    pub partition: Vec<PartitionManifest>,
}

/// A single partition entry inside a TOML manifest.
#[derive(Clone, Debug, Deserialize)]
pub struct PartitionManifest {
    /// The source payload path.
    pub file: PathBuf,
    /// The optional partition type alias or hex value.
    #[serde(rename = "type")]
    pub partition_type: Option<String>,
    /// Whether the partition should be marked active.
    pub bootable: Option<bool>,
    /// The optional fixed starting LBA.
    pub start_lba: Option<u64>,
    /// The optional partition size string.
    pub size: Option<String>,
}

impl PackManifest {
    /// Load a TOML manifest from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path).map_err(|source| {
            MbrkitError::io(
                Some(path.to_path_buf()),
                "failed to read pack manifest",
                source,
            )
        })?;

        toml::from_str(&content).map_err(|source| MbrkitError::Manifest {
            path: path.display().to_string(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PackManifest;

    /// Ensure the manifest schema remains deserializable.
    #[test]
    fn manifest_deserializes_partition_entries() {
        let manifest = toml::from_str::<PackManifest>(
            r#"
output = "disk.img"
disk_size = "32MiB"
boot_code = "mbr.bin"
disk_signature = "0x1234"
align_lba = 2048

[[partition]]
file = "rootfs.img"
type = "minix"
bootable = true
start_lba = 2048
size = "4MiB"
"#,
        )
        .unwrap();

        assert_eq!(manifest.partition.len(), 1);
        assert_eq!(manifest.partition[0].file.to_string_lossy(), "rootfs.img");
        assert_eq!(
            manifest.partition[0].partition_type.as_deref(),
            Some("minix")
        );
        assert_eq!(manifest.partition[0].start_lba, Some(2048));
    }
}
