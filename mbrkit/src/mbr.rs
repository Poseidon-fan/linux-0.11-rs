//! MBR data structures and sector encoding helpers.
//!
//! The Master Boot Record uses the first 512-byte sector of the disk:
//!
//! +----------------------+ Offset 0x000
//! | Bootstrap code       | 440 bytes
//! +----------------------+ Offset 0x1B8
//! | Disk signature       | 4 bytes
//! +----------------------+ Offset 0x1BC
//! | Reserved bytes       | 2 bytes
//! +----------------------+ Offset 0x1BE
//! | Partition entry #1   | 16 bytes
//! | Partition entry #2   | 16 bytes
//! | Partition entry #3   | 16 bytes
//! | Partition entry #4   | 16 bytes
//! +----------------------+ Offset 0x1FE
//! | Signature 0x55AA     | 2 bytes
//! +----------------------+ Offset 0x200

use std::fmt::Write as _;

use serde::{Serialize, Serializer};

use crate::error::{MbrkitError, Result};
use crate::layout::PartitionLayout;

/// The sector size supported by this tool.
pub const SECTOR_SIZE: usize = 512;
/// The executable bootstrap code before the disk signature field.
pub const MBR_BOOTSTRAP_CODE_SIZE: usize = 440;
/// The size of the on-disk disk signature field.
pub const DISK_SIGNATURE_SIZE: usize = 4;
/// The size of the reserved field that follows the disk signature.
pub const RESERVED_BYTES_SIZE: usize = 2;
/// The total prefix before the partition table.
pub const MBR_PREFIX_SIZE: usize =
    MBR_BOOTSTRAP_CODE_SIZE + DISK_SIGNATURE_SIZE + RESERVED_BYTES_SIZE;
/// The byte offset of the disk signature within the MBR sector.
pub const DISK_SIGNATURE_OFFSET: usize = MBR_BOOTSTRAP_CODE_SIZE;
/// The byte offset of the reserved bytes between disk signature and partition table.
pub const RESERVED_BYTES_OFFSET: usize = DISK_SIGNATURE_OFFSET + DISK_SIGNATURE_SIZE;
/// The byte offset of the partition table.
pub const PARTITION_TABLE_OFFSET: usize = MBR_PREFIX_SIZE;
/// The number of MBR partition slots.
pub const PARTITION_ENTRY_COUNT: usize = 4;
/// The size of one partition entry.
pub const PARTITION_ENTRY_SIZE: usize = 16;
/// The signature offset at the end of the MBR sector.
pub const MBR_SIGNATURE_OFFSET: usize = 510;
/// The required signature value for a valid MBR sector.
pub const MBR_SIGNATURE: u16 = 0xAA55;

/// A decoded MBR sector.
#[derive(Clone, Debug, Serialize)]
pub struct MbrHeader {
    /// The executable bootstrap code stored in bytes 0..440.
    #[serde(serialize_with = "serialize_bootstrap_code")]
    pub bootstrap_code: [u8; MBR_BOOTSTRAP_CODE_SIZE],
    /// The optional disk signature stored in bytes 440..444.
    pub disk_signature: u32,
    /// The reserved bytes stored in bytes 444..446.
    pub reserved: [u8; 2],
    /// The four primary partition entries.
    pub partitions: [PartitionEntry; PARTITION_ENTRY_COUNT],
    /// The raw signature stored in bytes 510..512.
    pub signature: u16,
}

/// A decoded MBR partition entry.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct PartitionEntry {
    /// The active flag. Expected values are 0x00 or 0x80.
    pub boot_indicator: u8,
    /// The encoded CHS start address.
    pub start_chs: [u8; 3],
    /// The partition type code.
    pub partition_type: u8,
    /// The encoded CHS end address.
    pub end_chs: [u8; 3],
    /// The starting LBA.
    pub starting_lba: u32,
    /// The partition sector count.
    pub sectors: u32,
}

/// A small CHS helper used for compatibility fields.
#[derive(Clone, Copy, Debug)]
struct ChsAddress {
    /// The cylinder component.
    cylinder: u16,
    /// The head component.
    head: u8,
    /// The sector component.
    sector: u8,
}

impl MbrHeader {
    /// Create an empty MBR header with the required signature.
    pub fn new(bootstrap_code: [u8; MBR_BOOTSTRAP_CODE_SIZE], disk_signature: u32) -> Self {
        Self {
            bootstrap_code,
            disk_signature,
            reserved: [0_u8; RESERVED_BYTES_SIZE],
            partitions: [PartitionEntry::default(); PARTITION_ENTRY_COUNT],
            signature: MBR_SIGNATURE,
        }
    }

    /// Decode an MBR sector from a raw 512-byte buffer.
    pub fn from_sector(sector: &[u8]) -> Result<Self> {
        if sector.len() < SECTOR_SIZE {
            return Err(MbrkitError::InvalidMbr(
                "disk image is smaller than one sector".into(),
            ));
        }

        let mut bootstrap_code = [0_u8; MBR_BOOTSTRAP_CODE_SIZE];
        bootstrap_code.copy_from_slice(&sector[..MBR_BOOTSTRAP_CODE_SIZE]);

        let mut partitions = [PartitionEntry::default(); PARTITION_ENTRY_COUNT];

        for (index, entry) in partitions.iter_mut().enumerate() {
            let offset = PARTITION_TABLE_OFFSET + index * PARTITION_ENTRY_SIZE;
            *entry = PartitionEntry::from_bytes(&sector[offset..offset + PARTITION_ENTRY_SIZE]);
        }

        Ok(Self {
            bootstrap_code,
            disk_signature: u32::from_le_bytes([
                sector[DISK_SIGNATURE_OFFSET],
                sector[DISK_SIGNATURE_OFFSET + 1],
                sector[DISK_SIGNATURE_OFFSET + 2],
                sector[DISK_SIGNATURE_OFFSET + 3],
            ]),
            reserved: [
                sector[RESERVED_BYTES_OFFSET],
                sector[RESERVED_BYTES_OFFSET + 1],
            ],
            partitions,
            signature: u16::from_le_bytes([
                sector[MBR_SIGNATURE_OFFSET],
                sector[MBR_SIGNATURE_OFFSET + 1],
            ]),
        })
    }

    /// Encode the MBR header into a raw 512-byte sector.
    pub fn to_sector(&self) -> [u8; SECTOR_SIZE] {
        let mut sector = [0_u8; SECTOR_SIZE];
        sector[..MBR_BOOTSTRAP_CODE_SIZE].copy_from_slice(&self.bootstrap_code);
        sector[DISK_SIGNATURE_OFFSET..DISK_SIGNATURE_OFFSET + 4]
            .copy_from_slice(&self.disk_signature.to_le_bytes());
        sector[RESERVED_BYTES_OFFSET..RESERVED_BYTES_OFFSET + 2].copy_from_slice(&self.reserved);

        for (index, entry) in self.partitions.iter().enumerate() {
            let offset = PARTITION_TABLE_OFFSET + index * PARTITION_ENTRY_SIZE;
            sector[offset..offset + PARTITION_ENTRY_SIZE].copy_from_slice(&entry.to_bytes());
        }

        sector[MBR_SIGNATURE_OFFSET..MBR_SIGNATURE_OFFSET + 2]
            .copy_from_slice(&self.signature.to_le_bytes());

        sector
    }

    /// Replace one partition slot with a layout-derived entry.
    pub fn set_partition(&mut self, slot: usize, layout: &PartitionLayout) -> Result<()> {
        if slot >= PARTITION_ENTRY_COUNT {
            return Err(MbrkitError::InvalidArgument(format!(
                "partition slot {} is out of range",
                slot + 1
            )));
        }

        self.partitions[slot] = PartitionEntry::from_layout(layout)?;
        Ok(())
    }

    /// Return whether the MBR signature is correct.
    pub fn has_valid_signature(&self) -> bool {
        self.signature == MBR_SIGNATURE
    }
}

impl PartitionEntry {
    /// Decode a 16-byte partition entry.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            boot_indicator: bytes[0],
            start_chs: [bytes[1], bytes[2], bytes[3]],
            partition_type: bytes[4],
            end_chs: [bytes[5], bytes[6], bytes[7]],
            starting_lba: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            sectors: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        }
    }

    /// Encode the partition entry back into its 16-byte form.
    pub fn to_bytes(self) -> [u8; PARTITION_ENTRY_SIZE] {
        let mut bytes = [0_u8; PARTITION_ENTRY_SIZE];
        bytes[0] = self.boot_indicator;
        bytes[1..4].copy_from_slice(&self.start_chs);
        bytes[4] = self.partition_type;
        bytes[5..8].copy_from_slice(&self.end_chs);
        bytes[8..12].copy_from_slice(&self.starting_lba.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.sectors.to_le_bytes());
        bytes
    }

    /// Build an encoded partition entry from a resolved layout.
    pub fn from_layout(layout: &PartitionLayout) -> Result<Self> {
        let starting_lba = u32::try_from(layout.start_lba).map_err(|_| {
            MbrkitError::InvalidArgument(format!(
                "partition {} start LBA exceeds MBR limits",
                layout.slot
            ))
        })?;
        let sectors = u32::try_from(layout.sector_count).map_err(|_| {
            MbrkitError::InvalidArgument(format!(
                "partition {} size exceeds MBR limits",
                layout.slot
            ))
        })?;
        let end_lba = layout.end_lba();
        let start_chs = ChsAddress::from_lba(layout.start_lba).encode();
        let end_chs = ChsAddress::from_lba(end_lba).encode();

        Ok(Self {
            boot_indicator: if layout.bootable { 0x80 } else { 0x00 },
            start_chs,
            partition_type: layout.partition_type.0,
            end_chs,
            starting_lba,
            sectors,
        })
    }

    /// Return whether the slot is unused.
    pub fn is_empty(self) -> bool {
        self.partition_type == 0 && self.starting_lba == 0 && self.sectors == 0
    }

    /// Return whether the partition is marked active.
    pub fn is_bootable(self) -> bool {
        self.boot_indicator == 0x80
    }

    /// Return the inclusive end LBA when the entry is non-empty.
    pub fn end_lba(self) -> Option<u64> {
        if self.sectors == 0 {
            return None;
        }

        Some(self.starting_lba as u64 + self.sectors as u64 - 1)
    }
}

impl ChsAddress {
    /// Convert an LBA into a saturated CHS triplet using 255/63 geometry.
    fn from_lba(lba: u64) -> Self {
        let sectors_per_track = 63_u64;
        let heads = 255_u64;
        let max_lba = 1023_u64 * heads * sectors_per_track;

        if lba >= max_lba {
            return Self {
                cylinder: 1023,
                head: 254,
                sector: 63,
            };
        }

        let cylinder = lba / (heads * sectors_per_track);
        let head = (lba / sectors_per_track) % heads;
        let sector = (lba % sectors_per_track) + 1;

        Self {
            cylinder: cylinder as u16,
            head: head as u8,
            sector: sector as u8,
        }
    }

    /// Encode the CHS triplet into the packed MBR byte representation.
    fn encode(self) -> [u8; 3] {
        let cylinder_high = ((self.cylinder >> 8) & 0x03) as u8;
        let cylinder_low = (self.cylinder & 0xff) as u8;
        let sector = (self.sector & 0x3f) | (cylinder_high << 6);

        [self.head, sector, cylinder_low]
    }
}

/// Serialize the bootstrap code as a compact hexadecimal string.
fn serialize_bootstrap_code<S>(
    bootstrap_code: &[u8; MBR_BOOTSTRAP_CODE_SIZE],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut encoded = String::with_capacity(MBR_BOOTSTRAP_CODE_SIZE * 2);

    for byte in bootstrap_code {
        let _ = write!(&mut encoded, "{byte:02x}");
    }

    serializer.serialize_str(&encoded)
}

#[cfg(test)]
mod tests {
    use crate::layout::{PartitionLayout, PartitionType};

    use super::{MBR_SIGNATURE, MbrHeader, PartitionEntry};

    /// Check round-tripping of encoded MBR sectors.
    #[test]
    fn mbr_sector_round_trip_is_lossless() {
        let mut header = MbrHeader::new([0x90; super::MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
        let layout = PartitionLayout {
            slot: 1,
            file: "rootfs.img".into(),
            partition_type: PartitionType(0x81),
            bootable: true,
            start_lba: 2048,
            sector_count: 128,
            file_size: 65_536,
        };
        header.set_partition(0, &layout).unwrap();

        let sector = header.to_sector();
        let decoded = MbrHeader::from_sector(&sector).unwrap();

        assert_eq!(decoded.signature, MBR_SIGNATURE);
        assert_eq!(decoded.disk_signature, 0x12345678);
        assert_eq!(decoded.partitions[0].partition_type, 0x81);
        assert_eq!(decoded.partitions[0].starting_lba, 2048);
        assert_eq!(decoded.partitions[0].sectors, 128);
    }

    /// Confirm helper predicates match common expectations.
    #[test]
    fn partition_entry_predicates_match_flags() {
        let entry = PartitionEntry {
            boot_indicator: 0x80,
            partition_type: 0x83,
            starting_lba: 63,
            sectors: 1024,
            ..PartitionEntry::default()
        };

        assert!(entry.is_bootable());
        assert!(!entry.is_empty());
        assert_eq!(entry.end_lba(), Some(1086));
    }

    /// Confirm JSON reports serialize bootstrap code as a hexadecimal string.
    #[test]
    fn bootstrap_code_serializes_as_hex_string() {
        let mut header = MbrHeader::new([0_u8; super::MBR_BOOTSTRAP_CODE_SIZE], 0x12345678);
        header.bootstrap_code[0] = 0x01;
        header.bootstrap_code[1] = 0x23;
        header.bootstrap_code[2] = 0xff;

        let value = serde_json::to_value(&header).unwrap();
        let bootstrap_code = value["bootstrap_code"].as_str().unwrap();

        assert_eq!(bootstrap_code.len(), super::MBR_BOOTSTRAP_CODE_SIZE * 2);
        assert!(bootstrap_code.starts_with("0123ff"));
    }
}
