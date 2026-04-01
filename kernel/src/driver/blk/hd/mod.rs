mod controller;
mod interrupt;

use log::info;

use crate::{
    driver::blk::hd::controller::{AtaTaskFile, ControllerCommand, StatusFlags},
    fs::buffer::{self, BufferKey},
    pmio::{self, inb_p, outb, outb_p, port_write_words},
    println,
    segment::{get_fs_byte, get_fs_word},
    sync::KernelCell,
    trap::set_intr_gate,
};

static HARD_DISK_MANAGER: KernelCell<HardDiskManager> = KernelCell::new(HardDiskManager::new());

/// Block major reserved for ATA hard disks.
const HARD_DISK_MAJOR: usize = 3;
/// Supports at most two ATA hard disks.
const MAX_DRIVE_COUNT: usize = 2;
/// Each drive exposes up to four primary partitions.
const PRIMARY_PARTITION_COUNT: usize = 4;
/// Linux 0.11 exposes one whole-disk slot plus four partition slots per drive.
const PARTITION_SLOTS_PER_DRIVE: usize = PRIMARY_PARTITION_COUNT + 1;
/// One ATA sector is transferred as 256 16-bit words.
const SECTOR_WORD_COUNT: usize = super::SECTOR_SIZE / 2;
/// Maximum per-request error count from the original driver.
const MAX_REQUEST_ERRORS: u32 = 7;

/// Legacy CHS geometry reported for one ATA drive.
#[derive(Clone)]
struct DriveGeometry {
    /// Number of heads visible to the controller.
    pub head_count: u16,
    /// Number of sectors per track.
    pub sectors_per_track: u16,
    /// Number of cylinders.
    pub cylinder_count: u16,
    /// Write-precompensation cylinder programmed into the controller.
    pub write_precompensation: u16,
    /// Landing zone cylinder used by restore sequences.
    pub landing_zone: u16,
    /// Control byte written to the ATA control register.
    pub control: u8,
}

/// One addressable partition range on a drive.
struct DrivePartition {
    /// First 512-byte sector belonging to this partition.
    pub start_sector: u32,
    /// Number of addressable 512-byte sectors in this partition.
    pub sector_count: u32,
}

/// Recovery action requested before normal I/O may resume.
enum RecoveryState {
    /// No recovery step is pending.
    None,
    /// The controller must be reset before retrying the request.
    ResetPending,
    /// The drive must be recalibrated before retrying the request.
    RecalibrationPending,
}

/// Interrupt continuation expected for the current controller operation.
enum InterruptPhase {
    /// No hard disk interrupt is currently expected.
    None,
    /// A read completion interrupt is expected.
    Read,
    /// A write completion interrupt is expected.
    Write,
    /// A recalibration completion interrupt is expected.
    Recalibrate,
}

/// Static metadata for one drive slot.
struct DriveDescriptor {
    /// Geometry information for the detected drive.
    pub geometry: DriveGeometry,
    /// Whole-disk range for the drive.
    pub whole_disk: DrivePartition,
    /// Four primary partition slots.
    pub primary_partitions: [Option<DrivePartition>; PRIMARY_PARTITION_COUNT],
}

/// Shared hard disk driver state.
struct HardDiskManager {
    /// Static descriptors for both ATA drive slots.
    pub drives: [Option<DriveDescriptor>; MAX_DRIVE_COUNT],
    /// Indicates whether drive geometry has already been initialized.
    pub setup_completed: bool,
    /// Recovery step that must run before the next request retry.
    pub recovery_state: RecoveryState,
    /// Interrupt continuation currently expected from the controller.
    pub interrupt_phase: InterruptPhase,
}

impl HardDiskManager {
    const fn new() -> Self {
        Self {
            drives: [const { None }; MAX_DRIVE_COUNT],
            setup_completed: false,
            recovery_state: RecoveryState::None,
            interrupt_phase: InterruptPhase::None,
        }
    }
}

impl DriveGeometry {
    /// Parse drive geometry from a BIOS drive-info table entry (in user segment).
    /// Returns `None` when the entry describes no drive (cylinder count is zero).
    unsafe fn from_bios_entry(entry_addr: *const u8) -> Option<Self> {
        // SAFETY: caller guarantees `entry_addr` points to a valid BIOS drive-info entry.
        let geo = unsafe {
            Self {
                cylinder_count: get_fs_word(entry_addr.cast::<u16>()),
                head_count: u16::from(get_fs_byte(entry_addr.add(2))),
                write_precompensation: get_fs_word(entry_addr.add(5).cast::<u16>()),
                control: get_fs_byte(entry_addr.add(8)),
                landing_zone: get_fs_word(entry_addr.add(12).cast::<u16>()),
                sectors_per_track: u16::from(get_fs_byte(entry_addr.add(14))),
            }
        };
        (geo.cylinder_count != 0).then_some(geo)
    }

    /// Total addressable sectors for this CHS geometry (saturating arithmetic).
    fn total_sectors(&self) -> u32 {
        [self.head_count, self.sectors_per_track, self.cylinder_count]
            .into_iter()
            .fold(1u32, |acc, v| acc.saturating_mul(u32::from(v)))
    }
}

impl DrivePartition {
    /// Parse one partition entry from an MBR sector. Returns `None` for empty entries.
    fn from_mbr_entry(sector: &[u8], index: usize) -> Option<Self> {
        /// First partition entry offset inside an MBR sector.
        const PARTITION_TABLE_OFFSET: usize = 0x1BE;
        /// Size of one DOS partition table entry.
        const PARTITION_TABLE_ENTRY_SIZE: usize = 16;
        /// Offset of the little-endian start-sector field in one entry.
        const PARTITION_START_SECTOR_OFFSET: usize = 8;
        /// Offset of the little-endian sector-count field in one entry.
        const PARTITION_SECTOR_COUNT_OFFSET: usize = 12;

        let off = PARTITION_TABLE_OFFSET + index * PARTITION_TABLE_ENTRY_SIZE;
        let start_sector = u32::from_le_bytes(
            sector[off + PARTITION_START_SECTOR_OFFSET..][..4]
                .try_into()
                .unwrap(),
        );
        let sector_count = u32::from_le_bytes(
            sector[off + PARTITION_SECTOR_COUNT_OFFSET..][..4]
                .try_into()
                .unwrap(),
        );
        (sector_count != 0).then_some(Self {
            start_sector,
            sector_count,
        })
    }
}

impl DriveDescriptor {
    /// Build a descriptor from BIOS geometry with empty partition slots.
    fn from_geometry(geometry: DriveGeometry) -> Self {
        Self {
            whole_disk: DrivePartition {
                start_sector: 0,
                sector_count: geometry.total_sectors(),
            },
            geometry,
            primary_partitions: [const { None }; PRIMARY_PARTITION_COUNT],
        }
    }
}

/// Register the hard disk block device and install its interrupt gate.
pub fn init() {
    super::register_device(HARD_DISK_MAJOR, handle_request, None, None);
    set_intr_gate(0x2E, interrupt::hd_interrupt);

    // Keep the cascade IRQ enabled on the master PIC and unmask IRQ14 on the slave PIC.
    outb_p(inb_p(0x21) & !0x04, 0x21);
    outb(inb_p(0xA1) & !0x40, 0xA1);
}

/// Initialize hard disk geometry from the BIOS drive table.
pub fn setup_from_bios(drive_info_addr: *const u8) -> Result<(), ()> {
    /// One BIOS hard disk geometry entry occupies 16 bytes.
    const BIOS_DRIVE_INFO_STRIDE: usize = 16;
    /// CMOS register containing the installed AT hard disk types.
    const CMOS_DISK_TYPE_REGISTER: u8 = 0x12;
    /// Offset of the 0x55AA boot signature in an MBR sector.
    const MBR_SIGNATURE_OFFSET: usize = 510;

    if HARD_DISK_MANAGER.exclusive(|m| m.setup_completed) {
        return Err(());
    }

    // Stage 1: load BIOS geometry so whole-disk minors can serve MBR reads.
    let mut drives: [Option<DriveDescriptor>; MAX_DRIVE_COUNT] = core::array::from_fn(|i| {
        let addr = unsafe { drive_info_addr.add(i * BIOS_DRIVE_INFO_STRIDE) };
        unsafe { DriveGeometry::from_bios_entry(addr) }.map(DriveDescriptor::from_geometry)
    });

    let cmos_disks = pmio::read_cmos(CMOS_DISK_TYPE_REGISTER);
    let drive_count = match (cmos_disks & 0xF0 != 0, cmos_disks & 0x0F != 0) {
        (false, _) => 0,
        (true, false) => 1,
        (true, true) => 2,
    }
    .min(drives.iter().flatten().count());

    for slot in &mut drives[drive_count..] {
        *slot = None;
    }

    HARD_DISK_MANAGER.exclusive(|m| {
        if m.setup_completed {
            return Err(());
        }
        m.drives = drives;
        Ok(())
    })?;

    // Stage 2: read each MBR and fill the four primary partition slots.
    for drive_index in 0..drive_count {
        let dev = crate::driver::DevNum::new(
            HARD_DISK_MAJOR as u8,
            (drive_index * PARTITION_SLOTS_PER_DRIVE) as u8,
        );
        let Some(handle) = buffer::read_block(BufferKey { dev, block_nr: 0 }) else {
            println!("Unable to read partition table of drive {}", drive_index);
            return Err(());
        };
        let sector =
            unsafe { core::slice::from_raw_parts(handle.data.as_ptr(), super::SECTOR_SIZE) };
        let partitions: Result<[_; PRIMARY_PARTITION_COUNT], ()> =
            if sector[MBR_SIGNATURE_OFFSET..][..2] != [0x55, 0xAA] {
                println!("Bad partition table on drive {}", drive_index);
                Err(())
            } else {
                Ok(core::array::from_fn(|i| {
                    DrivePartition::from_mbr_entry(sector, i)
                }))
            };
        buffer::release_block(handle);

        HARD_DISK_MANAGER.exclusive(|m| {
            m.drives[drive_index].as_mut().ok_or(())?.primary_partitions = partitions?;
            Ok(())
        })?;
    }

    HARD_DISK_MANAGER.exclusive(|m| m.setup_completed = true);

    if drive_count != 0 {
        info!(
            "Partition table{} ok.",
            if drive_count > 1 { "s" } else { "" }
        );
    }

    Ok(())
}

fn handle_request() {
    /// Maximum DRQ wait loop for the first write sector.
    const WRITE_DATA_READY_RETRIES: usize = 3_000;
    loop {
        let Some(request) = super::BLOCK_MANAGER.exclusive(|manager| {
            let request_slot =
                manager.devices[HARD_DISK_MAJOR].and_then(|device| device.current_request)?;
            Some(manager.request(request_slot).io.clone())
        }) else {
            return;
        };

        let Some((geometry, task_file, interrupt_phase)) = translate_request(
            request.dev.minor(),
            request.first_sector,
            request.sector_count,
            request.ty,
        ) else {
            super::complete_current_request(HARD_DISK_MAJOR, false);
            continue;
        };

        match HARD_DISK_MANAGER.exclusive(|manager| {
            match core::mem::replace(&mut manager.recovery_state, RecoveryState::None) {
                RecoveryState::ResetPending => {
                    manager.recovery_state = RecoveryState::RecalibrationPending;
                    RecoveryState::ResetPending
                }
                state => state,
            }
        }) {
            RecoveryState::ResetPending => {
                reset_drive(task_file.drive_index);
                return;
            }
            RecoveryState::RecalibrationPending => {
                let task_file = AtaTaskFile {
                    drive_index: task_file.drive_index,
                    sector_count: geometry.sectors_per_track as u8,
                    sector: 0,
                    head: 0,
                    cylinder: 0,
                    command: ControllerCommand::Restore,
                };
                controller::issue_command(geometry, task_file, InterruptPhase::Recalibrate);
                return;
            }
            RecoveryState::None => {}
        }

        controller::issue_command(geometry, task_file, interrupt_phase);

        match request.ty {
            super::BlockRequestType::Read => return,
            super::BlockRequestType::Write => {
                if controller::wait_for_status(WRITE_DATA_READY_RETRIES, |status| {
                    status.contains(StatusFlags::DATA_REQUEST)
                })
                .is_none()
                {
                    mark_request_error();
                    continue;
                }

                port_write_words(
                    controller::DATA_PORT,
                    request.data_addr.cast::<u16>().as_ptr(),
                    SECTOR_WORD_COUNT,
                );
                return;
            }
        }
    }
}

/// Reset one drive and reprogram its geometry into the controller.
fn reset_drive(drive_index: usize) {
    let geometry = HARD_DISK_MANAGER.exclusive(|manager| {
        manager.drives[drive_index]
            .as_ref()
            .unwrap()
            .geometry
            .clone()
    });

    controller::reset_controller(geometry.control);

    let sector_count = geometry.sectors_per_track as u8;
    let task_file = AtaTaskFile {
        drive_index,
        sector_count,
        sector: sector_count,
        head: (geometry.head_count - 1) as u8,
        cylinder: geometry.cylinder_count,
        command: ControllerCommand::Specify,
    };

    controller::issue_command(geometry, task_file, InterruptPhase::Recalibrate);
}

/// Translate one block request into ATA task-file parameters.
fn translate_request(
    minor: u8,
    first_sector: u32,
    sector_count: u32,
    request_ty: super::BlockRequestType,
) -> Option<(DriveGeometry, AtaTaskFile, InterruptPhase)> {
    HARD_DISK_MANAGER.exclusive(|manager| {
        let drive_index = usize::from(minor) / PARTITION_SLOTS_PER_DRIVE;
        let partition_index = usize::from(minor) % PARTITION_SLOTS_PER_DRIVE;
        let drive = manager.drives.get(drive_index)?.as_ref()?;
        let partition = match partition_index {
            0 => Some(&drive.whole_disk),
            slot => drive.primary_partitions.get(slot - 1)?.as_ref(),
        }?;

        let request_end = first_sector.checked_add(sector_count)?;
        if request_end > partition.sector_count {
            return None;
        }

        let absolute_sector = partition.start_sector.checked_add(first_sector)?;
        let sectors_per_track = u32::from(drive.geometry.sectors_per_track);
        let head_count = u32::from(drive.geometry.head_count);
        if sectors_per_track == 0 || head_count == 0 {
            return None;
        }

        let sector = (absolute_sector % sectors_per_track) + 1;
        let track = absolute_sector / sectors_per_track;
        let head = track % head_count;
        let cylinder = track / head_count;

        let (command, interrupt_phase) = match request_ty {
            super::BlockRequestType::Read => (ControllerCommand::Read, InterruptPhase::Read),
            super::BlockRequestType::Write => (ControllerCommand::Write, InterruptPhase::Write),
        };

        Some((
            drive.geometry.clone(),
            AtaTaskFile {
                drive_index,
                sector_count: sector_count as u8,
                sector: sector as u8,
                head: head as u8,
                cylinder: cylinder as u16,
                command,
            },
            interrupt_phase,
        ))
    })
}

/// Increase the current request error count and request a reset when needed.
fn mark_request_error() {
    let (must_fail, must_reset) = super::BLOCK_MANAGER.exclusive(|manager| {
        let Some(request) = manager.current_request_mut(HARD_DISK_MAJOR) else {
            return (false, false);
        };

        request.error_count += 1;
        (
            request.error_count >= MAX_REQUEST_ERRORS,
            request.error_count > MAX_REQUEST_ERRORS / 2,
        )
    });

    if must_reset {
        HARD_DISK_MANAGER.exclusive(|manager| {
            manager.recovery_state = RecoveryState::ResetPending;
        });
    }

    if must_fail {
        super::complete_current_request(HARD_DISK_MAJOR, false);
    }
}
