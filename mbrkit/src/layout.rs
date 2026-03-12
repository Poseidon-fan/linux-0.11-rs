//! Layout models and argument parsing helpers for disk construction.
//!
//! The layout layer converts CLI strings and manifest values into concrete
//! sector-aligned partitions that the MBR encoder can write to disk.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli::PackArgs;
use crate::error::{MbrkitError, Result};
use crate::manifest::{PackManifest, PartitionManifest};
use crate::mbr::{MBR_BOOTSTRAP_CODE_SIZE, RESERVED_BYTES_SIZE, SECTOR_SIZE};

/// A fully resolved disk image layout.
#[derive(Clone, Debug)]
pub struct DiskLayout {
    /// The output disk image path.
    pub output: PathBuf,
    /// The final disk size in bytes.
    pub disk_size: u64,
    /// The disk signature written into the MBR sector.
    pub disk_signature: u32,
    /// The bootstrap code region for bytes 0..440.
    pub bootstrap_code: [u8; MBR_BOOTSTRAP_CODE_SIZE],
    /// The reserved bytes for bytes 444..446.
    pub reserved: [u8; RESERVED_BYTES_SIZE],
    /// The alignment used for auto-placed partitions.
    pub align_sectors: u64,
    /// The resolved partitions in table-slot order.
    pub partitions: Vec<PartitionLayout>,
}

/// A fully resolved partition entry.
#[derive(Clone, Debug)]
pub struct PartitionLayout {
    /// The one-based slot number in the partition table.
    pub slot: usize,
    /// The source payload file.
    pub file: PathBuf,
    /// The encoded partition type.
    pub partition_type: PartitionType,
    /// The active flag written into the MBR entry.
    pub bootable: bool,
    /// The starting sector of the partition.
    pub start_lba: u64,
    /// The number of sectors owned by the partition.
    pub sector_count: u64,
    /// The raw payload size before padding.
    pub file_size: u64,
}

/// A user-facing partition type wrapper.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PartitionType(pub u8);

/// A canonical partition type definition used by parsing and reporting.
#[derive(Clone, Copy, Debug)]
struct PartitionTypeDefinition {
    /// The raw MBR type byte.
    code: u8,
    /// The canonical display name used in reports.
    canonical_name: &'static str,
    /// Alternate names accepted by the CLI and manifest parser.
    aliases: &'static [&'static str],
}

/// A partition specification parsed from CLI flags or a manifest.
#[derive(Clone, Debug)]
pub struct PartitionSpec {
    /// The source payload path.
    pub file: PathBuf,
    /// The partition type.
    pub partition_type: PartitionType,
    /// Whether the partition is marked active.
    pub bootable: bool,
    /// An optional fixed start sector.
    pub start_lba: Option<u64>,
    /// An optional fixed partition size in bytes.
    pub size_bytes: Option<u64>,
}

impl DiskLayout {
    /// Build a disk layout from explicit CLI flags.
    pub fn from_pack_args(args: &PackArgs) -> Result<Self> {
        if let Some(path) = &args.manifest {
            let manifest = PackManifest::load(path)?;
            return Self::from_manifest(manifest);
        }

        let output = args
            .output
            .clone()
            .ok_or_else(|| MbrkitError::InvalidArgument("`--output` is required".into()))?;
        let disk_size =
            parse_size(&args.disk_size.clone().ok_or_else(|| {
                MbrkitError::InvalidArgument("`--disk-size` is required".into())
            })?)?;
        let disk_signature = args
            .disk_signature
            .as_deref()
            .map(parse_u32_value)
            .transpose()?
            .unwrap_or(0);
        let bootstrap_code = load_boot_code(args.boot_code.as_deref())?;
        let partition_specs = args
            .partition
            .iter()
            .map(|spec| PartitionSpec::parse(spec))
            .collect::<Result<Vec<_>>>()?;

        Self::from_parts(
            output,
            disk_size,
            bootstrap_code,
            disk_signature,
            args.align,
            partition_specs,
        )
    }

    /// Build a disk layout from a TOML manifest.
    pub fn from_manifest(manifest: PackManifest) -> Result<Self> {
        let output = manifest.output;
        let disk_size = parse_size(&manifest.disk_size)?;
        let disk_signature = manifest
            .disk_signature
            .map(|value| parse_u32_value(&value))
            .transpose()?
            .unwrap_or(0);
        let bootstrap_code = load_boot_code(manifest.boot_code.as_deref())?;
        let partition_specs = manifest
            .partition
            .iter()
            .map(PartitionSpec::from_manifest)
            .collect::<Result<Vec<_>>>()?;

        Self::from_parts(
            output,
            disk_size,
            bootstrap_code,
            disk_signature,
            manifest.align_lba.unwrap_or(2048),
            partition_specs,
        )
    }

    /// Convert partially parsed values into a validated layout.
    fn from_parts(
        output: PathBuf,
        disk_size: u64,
        bootstrap_code: [u8; MBR_BOOTSTRAP_CODE_SIZE],
        disk_signature: u32,
        align_sectors: u64,
        partition_specs: Vec<PartitionSpec>,
    ) -> Result<Self> {
        if align_sectors == 0 {
            return Err(MbrkitError::InvalidArgument(
                "`--align` must be greater than zero".into(),
            ));
        }

        if disk_size < SECTOR_SIZE as u64 {
            return Err(MbrkitError::InvalidArgument(
                "disk size must be at least one sector".into(),
            ));
        }

        if partition_specs.is_empty() {
            return Err(MbrkitError::InvalidArgument(
                "at least one partition must be defined".into(),
            ));
        }

        if partition_specs.len() > 4 {
            return Err(MbrkitError::InvalidArgument(
                "MBR supports at most four primary partitions".into(),
            ));
        }

        let sector_count = disk_size / SECTOR_SIZE as u64;
        let mut next_auto_start = align_up(1, align_sectors);
        let mut partitions = Vec::with_capacity(partition_specs.len());

        for (index, spec) in partition_specs.into_iter().enumerate() {
            let metadata = fs::metadata(&spec.file).map_err(|source| {
                MbrkitError::io(
                    Some(spec.file.clone()),
                    "failed to read source image metadata",
                    source,
                )
            })?;
            let file_size = metadata.len();
            let requested_size = spec.size_bytes.unwrap_or(file_size);

            if requested_size < file_size {
                return Err(MbrkitError::InvalidArgument(format!(
                    "partition source `{}` is larger than the declared partition size",
                    spec.file.display()
                )));
            }

            let sector_count_for_partition = bytes_to_sectors(requested_size);
            let start_lba = spec.start_lba.unwrap_or(next_auto_start);

            if start_lba == 0 {
                return Err(MbrkitError::InvalidArgument(
                    "partition LBA 0 is reserved for the MBR sector".into(),
                ));
            }

            partitions.push(PartitionLayout {
                slot: index + 1,
                file: spec.file,
                partition_type: spec.partition_type,
                bootable: spec.bootable,
                start_lba,
                sector_count: sector_count_for_partition,
                file_size,
            });

            next_auto_start = align_up(
                start_lba
                    .checked_add(sector_count_for_partition)
                    .ok_or_else(|| {
                        MbrkitError::InvalidArgument("partition range overflowed".into())
                    })?,
                align_sectors,
            );
        }

        validate_partitions(&partitions, sector_count)?;

        Ok(Self {
            output,
            disk_size,
            disk_signature,
            bootstrap_code,
            reserved: [0_u8; RESERVED_BYTES_SIZE],
            align_sectors,
            partitions,
        })
    }
}

impl PartitionLayout {
    /// Return the inclusive end sector for the partition.
    pub fn end_lba(&self) -> u64 {
        self.start_lba + self.sector_count - 1
    }

    /// Return the byte offset of the partition payload in the disk image.
    pub fn start_offset(&self) -> u64 {
        self.start_lba * SECTOR_SIZE as u64
    }

    /// Return the exact byte length reserved for the partition.
    pub fn byte_len(&self) -> u64 {
        self.sector_count * SECTOR_SIZE as u64
    }
}

impl PartitionType {
    /// Parse a user-facing partition type string.
    pub fn parse(value: &str) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase();

        let err = || MbrkitError::InvalidPartitionType {
            value: value.into(),
        };

        let code = match PARTITION_TYPE_DEFINITIONS
            .iter()
            .find(|def| def.matches(&normalized))
        {
            Some(def) => def.code,
            None => normalized
                .strip_prefix("0x")
                .map(|hex| u8::from_str_radix(hex, 16))
                .unwrap_or_else(|| normalized.parse::<u8>())
                .map_err(|_| err())?,
        };

        Ok(Self(code))
    }

    /// Return a human-friendly alias when one is known.
    pub fn known_name(self) -> Option<&'static str> {
        PARTITION_TYPE_DEFINITIONS
            .iter()
            .find(|definition| definition.code == self.0)
            .map(|definition| definition.canonical_name)
    }
}

impl PartitionTypeDefinition {
    /// Return whether the definition accepts the provided alias.
    fn matches(self, value: &str) -> bool {
        self.canonical_name == value || self.aliases.contains(&value)
    }
}

/// The shared MBR partition type table.
const PARTITION_TYPE_DEFINITIONS: &[PartitionTypeDefinition] = &[
    PartitionTypeDefinition {
        code: 0x00,
        canonical_name: "empty",
        aliases: &["unused"],
    },
    PartitionTypeDefinition {
        code: 0x01,
        canonical_name: "fat12",
        aliases: &[],
    },
    PartitionTypeDefinition {
        code: 0x04,
        canonical_name: "fat16_small",
        aliases: &["fat16-16m", "fat16_16m"],
    },
    PartitionTypeDefinition {
        code: 0x05,
        canonical_name: "extended",
        aliases: &["chs_extended"],
    },
    PartitionTypeDefinition {
        code: 0x06,
        canonical_name: "fat16",
        aliases: &[],
    },
    PartitionTypeDefinition {
        code: 0x07,
        canonical_name: "ntfs",
        aliases: &["hpfs", "exfat"],
    },
    PartitionTypeDefinition {
        code: 0x0b,
        canonical_name: "fat32",
        aliases: &[],
    },
    PartitionTypeDefinition {
        code: 0x0c,
        canonical_name: "fat32_lba",
        aliases: &["fat32-lba"],
    },
    PartitionTypeDefinition {
        code: 0x0e,
        canonical_name: "fat16_lba",
        aliases: &["fat16-lba"],
    },
    PartitionTypeDefinition {
        code: 0x0f,
        canonical_name: "extended_lba",
        aliases: &["extended-lba", "lba_extended"],
    },
    PartitionTypeDefinition {
        code: 0x81,
        canonical_name: "minix",
        aliases: &[],
    },
    PartitionTypeDefinition {
        code: 0x82,
        canonical_name: "linux_swap",
        aliases: &["linux-swap", "swap"],
    },
    PartitionTypeDefinition {
        code: 0x83,
        canonical_name: "linux",
        aliases: &["linux_native"],
    },
];

impl PartitionSpec {
    /// Parse a CLI partition specification.
    pub fn parse(spec: &str) -> Result<Self> {
        let mut file = None;
        let mut partition_type = PartitionType(0x83);
        let mut bootable = false;
        let mut start_lba = None;
        let mut size_bytes = None;

        for item in spec.split(',') {
            let trimmed = item.trim();

            if trimmed.eq_ignore_ascii_case("bootable") {
                bootable = true;
                continue;
            }

            let (key, value) =
                trimmed
                    .split_once('=')
                    .ok_or_else(|| MbrkitError::InvalidPartitionSpec {
                        spec: spec.into(),
                        message: "expected `key=value` items".into(),
                    })?;

            match key.trim() {
                "file" => file = Some(PathBuf::from(value.trim())),
                "type" => partition_type = PartitionType::parse(value.trim())?,
                "start" => {
                    let parsed = if value.trim().eq_ignore_ascii_case("auto") {
                        None
                    } else {
                        Some(value.trim().parse::<u64>().map_err(|_| {
                            MbrkitError::InvalidPartitionSpec {
                                spec: spec.into(),
                                message: "invalid `start` value".into(),
                            }
                        })?)
                    };
                    start_lba = parsed;
                }
                "size" => size_bytes = Some(parse_size(value.trim())?),
                _ => {
                    return Err(MbrkitError::InvalidPartitionSpec {
                        spec: spec.into(),
                        message: format!("unknown key `{}`", key.trim()),
                    });
                }
            }
        }

        let file = file.ok_or_else(|| MbrkitError::InvalidPartitionSpec {
            spec: spec.into(),
            message: "missing `file` entry".into(),
        })?;

        Ok(Self {
            file,
            partition_type,
            bootable,
            start_lba,
            size_bytes,
        })
    }

    /// Convert a manifest partition into the shared internal representation.
    pub fn from_manifest(partition: &PartitionManifest) -> Result<Self> {
        Ok(Self {
            file: partition.file.clone(),
            partition_type: partition
                .partition_type
                .as_deref()
                .map(PartitionType::parse)
                .transpose()?
                .unwrap_or(PartitionType(0x83)),
            bootable: partition.bootable.unwrap_or(false),
            start_lba: partition.start_lba,
            size_bytes: partition.size.as_deref().map(parse_size).transpose()?,
        })
    }
}

/// Parse a human-friendly size string into bytes.
pub fn parse_size(value: &str) -> Result<u64> {
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return Err(MbrkitError::InvalidSize {
            value: value.into(),
        });
    }

    let digits_end = trimmed
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (number_text, suffix_text) = trimmed.split_at(digits_end);

    if number_text.is_empty() {
        return Err(MbrkitError::InvalidSize {
            value: value.into(),
        });
    }

    let number = number_text
        .parse::<u64>()
        .map_err(|_| MbrkitError::InvalidSize {
            value: value.into(),
        })?;
    let multiplier = match suffix_text.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" => 1_000,
        "m" | "mb" => 1_000_000,
        "g" | "gb" => 1_000_000_000,
        "kib" => 1024,
        "mib" => 1024 * 1024,
        "gib" => 1024 * 1024 * 1024,
        _ => {
            return Err(MbrkitError::InvalidSize {
                value: value.into(),
            });
        }
    };

    number
        .checked_mul(multiplier)
        .ok_or_else(|| MbrkitError::InvalidSize {
            value: value.into(),
        })
}

/// Convert bytes into a sector count by rounding up to the next full sector.
pub fn bytes_to_sectors(bytes: u64) -> u64 {
    bytes.div_ceil(SECTOR_SIZE as u64)
}

/// Read and validate an optional bootstrap code image.
pub fn load_boot_code(path: Option<&Path>) -> Result<[u8; MBR_BOOTSTRAP_CODE_SIZE]> {
    let mut bootstrap_code = [0_u8; MBR_BOOTSTRAP_CODE_SIZE];

    if let Some(path) = path {
        let data = fs::read(path).map_err(|source| {
            MbrkitError::io(
                Some(path.to_path_buf()),
                "failed to read bootstrap code image",
                source,
            )
        })?;

        if data.len() > MBR_BOOTSTRAP_CODE_SIZE {
            return Err(MbrkitError::InvalidArgument(format!(
                "bootstrap code `{}` exceeds {} bytes",
                path.display(),
                MBR_BOOTSTRAP_CODE_SIZE
            )));
        }

        // The remaining bytes stay zero-filled so short bootstrap stubs are accepted.
        bootstrap_code[..data.len()].copy_from_slice(&data);
    }

    Ok(bootstrap_code)
}

/// Parse a decimal or hexadecimal 32-bit integer.
pub fn parse_u32_value(value: &str) -> Result<u32> {
    if let Some(hex) = value.trim().strip_prefix("0x") {
        return u32::from_str_radix(hex, 16)
            .map_err(|_| MbrkitError::InvalidArgument(format!("invalid numeric value `{value}`")));
    }

    value
        .trim()
        .parse::<u32>()
        .map_err(|_| MbrkitError::InvalidArgument(format!("invalid numeric value `{value}`")))
}

/// Align a sector number upward to the requested boundary.
pub fn align_up(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}

/// Validate the final partition placements against disk boundaries.
pub fn validate_partitions(partitions: &[PartitionLayout], disk_sectors: u64) -> Result<()> {
    let mut ordered = partitions.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|partition| partition.start_lba);

    for window in ordered.windows(2) {
        let left = window[0];
        let right = window[1];

        if left.end_lba() >= right.start_lba {
            return Err(MbrkitError::InvalidArgument(format!(
                "partitions {} and {} overlap",
                left.slot, right.slot
            )));
        }
    }

    for partition in partitions {
        if partition.sector_count == 0 {
            return Err(MbrkitError::InvalidArgument(format!(
                "partition {} has zero sectors",
                partition.slot
            )));
        }

        let end_lba = partition.end_lba();

        if end_lba >= disk_sectors {
            return Err(MbrkitError::InvalidArgument(format!(
                "partition {} exceeds the declared disk size",
                partition.slot
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PartitionSpec, PartitionType, align_up, bytes_to_sectors, parse_size};

    /// Check the supported size suffixes.
    #[test]
    fn parse_size_supports_common_suffixes() {
        assert_eq!(parse_size("512").unwrap(), 512);
        assert_eq!(parse_size("1KiB").unwrap(), 1024);
        assert_eq!(parse_size("2MiB").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_size("3M").unwrap(), 3_000_000);
    }

    /// Confirm the alias table stays stable.
    #[test]
    fn partition_type_aliases_are_supported() {
        assert_eq!(PartitionType::parse("empty").unwrap(), PartitionType(0x00));
        assert_eq!(PartitionType::parse("ntfs").unwrap(), PartitionType(0x07));
        assert_eq!(
            PartitionType::parse("linux_swap").unwrap(),
            PartitionType(0x82)
        );
        assert_eq!(PartitionType::parse("swap").unwrap(), PartitionType(0x82));
        assert_eq!(PartitionType::parse("minix").unwrap(), PartitionType(0x81));
        assert_eq!(PartitionType::parse("linux").unwrap(), PartitionType(0x83));
        assert_eq!(PartitionType::parse("0x83").unwrap(), PartitionType(0x83));
        assert_eq!(PartitionType(0x0f).known_name(), Some("extended_lba"));
    }

    /// Verify CLI partition specifications parse as expected.
    #[test]
    fn partition_spec_parses_flags() {
        let spec = PartitionSpec::parse("file=rootfs.img,type=minix,bootable,start=2048,size=4MiB")
            .unwrap();

        assert_eq!(spec.file.to_string_lossy(), "rootfs.img");
        assert_eq!(spec.partition_type, PartitionType(0x81));
        assert!(spec.bootable);
        assert_eq!(spec.start_lba, Some(2048));
        assert_eq!(spec.size_bytes, Some(4 * 1024 * 1024));
    }

    /// Confirm helper functions match expected rounding semantics.
    #[test]
    fn alignment_and_sector_rounding_are_stable() {
        assert_eq!(bytes_to_sectors(1), 1);
        assert_eq!(bytes_to_sectors(512), 1);
        assert_eq!(bytes_to_sectors(513), 2);
        assert_eq!(align_up(1, 2048), 2048);
        assert_eq!(align_up(4096, 2048), 4096);
    }
}
