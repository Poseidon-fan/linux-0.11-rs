//! ATA hard disk driver — partition probing, interrupt-driven read/write.
//!
//! The block layer owns request ordering and buffer lifetime. This module owns
//! ATA-specific state: disk geometry, partition ranges, controller recovery,
//! and the interrupt continuation currently expected from the hardware.

mod controller;
mod interrupt;

use core::ptr::NonNull;

use controller::{AtaTaskFile, ControllerCommand, StatusFlags, command_succeeded};
use log::info;

use crate::{
    driver::{
        DevNum,
        block::{self, BlockDriverOps, BlockRequestType, QueueAfterComplete, RequestErrorAction},
    },
    error::{Errno, Result},
    fs::buffer::{self, BufferKey},
    pmio::{self, inb_p, outb, outb_p, port_read_words, port_write_words},
    println,
    segment::uaccess,
    sync::KernelCell,
    trap,
};

/// Register the hard disk block device and install its interrupt gate.
pub fn init() {
    block::register_driver(HARD_DISK_MAJOR, BlockDriverOps { kick });
    trap::set_intr_gate(0x2E, interrupt::hd_interrupt);

    // Keep the cascade IRQ enabled on the master PIC and unmask IRQ14 on the slave PIC.
    outb_p(inb_p(0x21) & !0x04, 0x21);
    outb(inb_p(0xA1) & !0x40, 0xA1);
}

/// Initialize hard disk geometry from the BIOS drive table.
pub fn setup_from_bios(drive_info_addr: *const u8) -> Result<()> {
    /// One BIOS hard disk geometry entry occupies 16 bytes.
    const BIOS_DRIVE_INFO_STRIDE: usize = 16;
    /// CMOS register containing the installed AT hard disk types.
    const CMOS_DISK_TYPE_REGISTER: u8 = 0x12;
    /// Offset of the 0x55AA boot signature in an MBR sector.
    const MBR_SIGNATURE_OFFSET: usize = 510;

    if ATA_DRIVER.exclusive(|driver| driver.setup_completed) {
        return Err(Errno::BUSY);
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

    ATA_DRIVER.exclusive(|driver| {
        if driver.setup_completed {
            return Err(Errno::BUSY);
        }
        driver.disks.drives = drives;
        Ok(())
    })?;

    // Stage 2: read each MBR and fill the four primary partition slots.
    for drive_index in 0..drive_count {
        let dev = DevNum::new(
            HARD_DISK_MAJOR,
            (drive_index * PARTITION_SLOTS_PER_DRIVE) as u8,
        );
        let Some(handle) = buffer::read(BufferKey::new(dev, 0)) else {
            println!("Unable to read partition table of drive {}", drive_index);
            return Err(Errno::IO);
        };
        let mut sector = [0; block::SECTOR_SIZE];
        handle.read_bytes(0, &mut sector);
        if sector[MBR_SIGNATURE_OFFSET..][..2] != [0x55, 0xAA] {
            println!("Bad partition table on drive {}", drive_index);
            return Err(Errno::IO);
        }
        let partitions = core::array::from_fn(|i| DrivePartition::from_mbr_entry(&sector, i));
        drop(handle);

        ATA_DRIVER.exclusive(|driver| {
            let drive = driver.disks.drives[drive_index]
                .as_mut()
                .ok_or(Errno::NODEV)?;
            drive.primary_partitions = partitions;
            Ok::<(), Errno>(())
        })?;
    }

    ATA_DRIVER.exclusive(|driver| driver.setup_completed = true);

    if drive_count != 0 {
        info!(
            "Partition table{} ok.",
            if drive_count > 1 { "s" } else { "" }
        );
    }

    Ok(())
}

static ATA_DRIVER: KernelCell<AtaDriver> = KernelCell::new(AtaDriver::new());

/// Block major reserved for ATA hard disks.
const HARD_DISK_MAJOR: u8 = 3;
/// Supports at most two ATA hard disks.
const MAX_DRIVE_COUNT: usize = 2;
/// Each drive exposes up to four primary partitions.
const PRIMARY_PARTITION_COUNT: usize = 4;
/// Exposes one whole-disk slot plus four partition slots per drive.
const PARTITION_SLOTS_PER_DRIVE: usize = PRIMARY_PARTITION_COUNT + 1;
/// One ATA sector is transferred as 256 16-bit words.
const SECTOR_WORD_COUNT: usize = block::SECTOR_SIZE / 2;
/// Maximum per-request error count before the request is failed.
const MAX_REQUEST_ERRORS: u32 = 7;

/// Legacy CHS geometry reported for one ATA drive.
#[derive(Clone)]
struct DriveGeometry {
    /// Number of heads visible to the controller.
    head_count: u16,
    /// Number of sectors per track.
    sectors_per_track: u16,
    /// Number of cylinders.
    cylinder_count: u16,
    /// Write-precompensation cylinder programmed into the controller.
    write_precompensation: u16,
    /// Control byte written to the ATA control register.
    control: u8,
}

/// One addressable partition range on a drive.
struct DrivePartition {
    /// First 512-byte sector belonging to this partition.
    start_sector: u32,
    /// Number of addressable 512-byte sectors in this partition.
    sector_count: u32,
}

/// Static metadata for one drive slot.
struct DriveDescriptor {
    /// Geometry information for the detected drive.
    geometry: DriveGeometry,
    /// Whole-disk range for the drive.
    whole_disk: DrivePartition,
    /// Four primary partition slots.
    primary_partitions: [Option<DrivePartition>; PRIMARY_PARTITION_COUNT],
}

/// Partition and geometry table for all supported ATA drives.
struct DiskTable {
    /// Static descriptors for both ATA drive slots.
    drives: [Option<DriveDescriptor>; MAX_DRIVE_COUNT],
}

/// Recovery action requested before normal I/O may resume.
enum RecoveryState {
    /// No recovery step is pending.
    Ready,
    /// The controller must be reset before retrying the request.
    NeedReset,
    /// The drive must be recalibrated before retrying the request.
    NeedRecalibrate,
}

/// Interrupt continuation expected for the current controller operation.
enum InterruptPhase {
    /// No hard disk interrupt is currently expected.
    Idle,
    /// A read completion interrupt is expected.
    Reading,
    /// A write completion interrupt is expected.
    Writing,
    /// A controller specify command completion interrupt is expected.
    Specifying,
    /// A recalibration completion interrupt is expected.
    Recalibrating,
}

/// Shared ATA driver state.
struct AtaDriver {
    /// Discovered disk geometry and primary partition ranges.
    disks: DiskTable,
    /// Indicates whether drive geometry has already been initialized.
    setup_completed: bool,
    /// Recovery step that must run before the next request retry.
    recovery: RecoveryState,
    /// Interrupt continuation currently expected from the controller.
    phase: InterruptPhase,
}

/// Snapshot of the current block request needed to issue one ATA command.
struct AtaRequest {
    /// Target block operation.
    ty: BlockRequestType,
    /// Target device minor number.
    minor: u8,
    /// Next request-relative sector to transfer.
    next_sector: u32,
    /// Number of sectors still waiting to be transferred.
    remaining_sectors: u32,
    /// Memory address for the next sector transfer.
    data_addr: NonNull<u8>,
}

/// Start or resume processing the current hard disk request.
fn kick() {
    /// Maximum DRQ wait loop for the first write sector.
    const WRITE_DATA_READY_RETRIES: usize = 3_000;

    loop {
        let Some(request) = current_request_snapshot() else {
            return;
        };

        let Ok((geometry, task_file, interrupt_phase)) = translate_request(&request) else {
            if block::complete_current(HARD_DISK_MAJOR, Err(Errno::NODEV))
                == QueueAfterComplete::Idle
            {
                return;
            }
            continue;
        };

        match take_recovery_state() {
            RecoveryState::NeedReset => {
                reset_drive(task_file.drive_index);
                return;
            }
            RecoveryState::NeedRecalibrate => {
                let task_file = AtaTaskFile {
                    drive_index: task_file.drive_index,
                    sector_count: geometry.sectors_per_track as u8,
                    sector: 0,
                    head: 0,
                    cylinder: 0,
                    command: ControllerCommand::Restore,
                };
                controller::issue_command(geometry, task_file, InterruptPhase::Recalibrating);
                return;
            }
            RecoveryState::Ready => {}
        }

        controller::issue_command(geometry, task_file, interrupt_phase);

        match request.ty {
            BlockRequestType::Read => return,
            BlockRequestType::Write => {
                if controller::wait_for_status(WRITE_DATA_READY_RETRIES, |status| {
                    status.contains(StatusFlags::DATA_REQUEST)
                })
                .is_none()
                {
                    if mark_request_error() {
                        continue;
                    }
                    return;
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

/// Rust-side interrupt continuation for IRQ14.
fn on_interrupt() {
    let phase =
        ATA_DRIVER.exclusive(|driver| core::mem::replace(&mut driver.phase, InterruptPhase::Idle));

    match phase {
        InterruptPhase::Idle => handle_unexpected_interrupt(),
        InterruptPhase::Reading => continue_read(),
        InterruptPhase::Writing => continue_write(),
        InterruptPhase::Specifying | InterruptPhase::Recalibrating => continue_recovery(),
    }
}

/// Handle a spurious or late ATA interrupt with no pending continuation.
fn handle_unexpected_interrupt() {
    println!("Unexpected hard disk interrupt");
}

/// Continue a pending ATA read command.
fn continue_read() {
    if !command_succeeded() {
        if mark_request_error() {
            kick();
        }
        return;
    }

    let Some(buffer) = block::with_current_request(HARD_DISK_MAJOR, |request| {
        request.map(|request| request.buffer_ptr::<u16>())
    }) else {
        handle_unexpected_interrupt();
        return;
    };

    port_read_words(controller::DATA_PORT, buffer.as_ptr(), SECTOR_WORD_COUNT);

    let has_more = block::with_current_request(HARD_DISK_MAJOR, |request| {
        let Some(mut request) = request else {
            return false;
        };
        request.clear_errors();
        request.advance_one_sector();
        request.progress().remaining_sectors() != 0
    });

    if has_more {
        ATA_DRIVER.exclusive(|driver| {
            driver.phase = InterruptPhase::Reading;
        });
        return;
    }

    complete_current_and_maybe_kick(Ok(()));
}

/// Continue a pending ATA write command.
fn continue_write() {
    if !command_succeeded() {
        if mark_request_error() {
            kick();
        }
        return;
    }

    let next_buffer = block::with_current_request(HARD_DISK_MAJOR, |request| {
        let mut request = request?;
        request.clear_errors();
        request.advance_one_sector();
        (request.progress().remaining_sectors() != 0).then(|| request.buffer_ptr::<u16>())
    });

    let Some(next_buffer) = next_buffer else {
        complete_current_and_maybe_kick(Ok(()));
        return;
    };

    ATA_DRIVER.exclusive(|driver| {
        driver.phase = InterruptPhase::Writing;
    });
    port_write_words(
        controller::DATA_PORT,
        next_buffer.as_ptr(),
        SECTOR_WORD_COUNT,
    );
}

/// Continue a pending ATA specify or recalibration command.
fn continue_recovery() {
    if !command_succeeded() && !mark_request_error() {
        return;
    }
    kick();
}

/// Reset one drive and reprogram its geometry into the controller.
fn reset_drive(drive_index: usize) {
    let geometry = ATA_DRIVER.exclusive(|driver| {
        driver.disks.drives[drive_index]
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

    controller::issue_command(geometry, task_file, InterruptPhase::Specifying);
}

/// Translate the current block request cursor into ATA task-file parameters.
fn translate_request(request: &AtaRequest) -> Result<(DriveGeometry, AtaTaskFile, InterruptPhase)> {
    ATA_DRIVER.exclusive(|driver| {
        let drive_index = usize::from(request.minor) / PARTITION_SLOTS_PER_DRIVE;
        let partition_index = usize::from(request.minor) % PARTITION_SLOTS_PER_DRIVE;
        let drive = driver
            .disks
            .drives
            .get(drive_index)
            .and_then(Option::as_ref)
            .ok_or(Errno::NODEV)?;
        let partition = match partition_index {
            0 => Some(&drive.whole_disk),
            slot => drive
                .primary_partitions
                .get(slot - 1)
                .and_then(Option::as_ref),
        }
        .ok_or(Errno::NODEV)?;

        let request_end = request
            .next_sector
            .checked_add(request.remaining_sectors)
            .ok_or(Errno::IO)?;
        if request_end > partition.sector_count {
            return Err(Errno::NODEV);
        }

        let absolute_sector = partition
            .start_sector
            .checked_add(request.next_sector)
            .ok_or(Errno::IO)?;
        let sectors_per_track = u32::from(drive.geometry.sectors_per_track);
        let head_count = u32::from(drive.geometry.head_count);
        if sectors_per_track == 0 || head_count == 0 {
            return Err(Errno::NODEV);
        }

        let sector = (absolute_sector % sectors_per_track) + 1;
        let track = absolute_sector / sectors_per_track;
        let head = track % head_count;
        let cylinder = track / head_count;

        let (command, phase) = match request.ty {
            BlockRequestType::Read => (ControllerCommand::Read, InterruptPhase::Reading),
            BlockRequestType::Write => (ControllerCommand::Write, InterruptPhase::Writing),
        };

        Ok((
            drive.geometry.clone(),
            AtaTaskFile {
                drive_index,
                sector_count: request.remaining_sectors as u8,
                sector: sector as u8,
                head: head as u8,
                cylinder: cylinder as u16,
                command,
            },
            phase,
        ))
    })
}

/// Copy the current block request fields needed outside the block-manager lock.
fn current_request_snapshot() -> Option<AtaRequest> {
    block::with_current_request(HARD_DISK_MAJOR, |request| {
        request.map(|request| AtaRequest {
            ty: request.io().ty(),
            minor: request.io().dev().minor(),
            next_sector: request.progress().next_sector(),
            remaining_sectors: request.progress().remaining_sectors(),
            data_addr: request.progress().next_data_addr(),
        })
    })
}

/// Consume the current recovery request, advancing the reset/recalibrate flow.
fn take_recovery_state() -> RecoveryState {
    ATA_DRIVER.exclusive(|driver| {
        match core::mem::replace(&mut driver.recovery, RecoveryState::Ready) {
            RecoveryState::NeedReset => {
                driver.recovery = RecoveryState::NeedRecalibrate;
                RecoveryState::NeedReset
            }
            state => state,
        }
    })
}

/// Increase the current request error count and report whether work remains.
fn mark_request_error() -> bool {
    let action = block::with_current_request(HARD_DISK_MAJOR, |request| {
        request
            .map(|mut request| request.record_error(MAX_REQUEST_ERRORS))
            .unwrap_or(RequestErrorAction::Retry)
    });

    match action {
        RequestErrorAction::Retry => true,
        RequestErrorAction::Reset => {
            ATA_DRIVER.exclusive(|driver| {
                driver.recovery = RecoveryState::NeedReset;
            });
            true
        }
        RequestErrorAction::Fail => {
            block::complete_current(HARD_DISK_MAJOR, Err(Errno::IO)) == QueueAfterComplete::More
        }
    }
}

/// Complete the current request and immediately kick the next queued request.
fn complete_current_and_maybe_kick(result: Result<()>) {
    if block::complete_current(HARD_DISK_MAJOR, result) == QueueAfterComplete::More {
        kick();
    }
}

impl AtaDriver {
    /// Create an uninitialized ATA driver state.
    const fn new() -> Self {
        Self {
            disks: DiskTable::new(),
            setup_completed: false,
            recovery: RecoveryState::Ready,
            phase: InterruptPhase::Idle,
        }
    }
}

impl DiskTable {
    /// Create an empty disk table with no detected drives.
    const fn new() -> Self {
        Self {
            drives: [const { None }; MAX_DRIVE_COUNT],
        }
    }
}

impl DriveGeometry {
    /// Parse drive geometry from a BIOS drive-info table entry in user memory.
    ///
    /// Returns `None` when the entry describes no drive. The BIOS table is laid
    /// out as 16-byte entries:
    ///
    /// ```text
    /// +0  u16 cylinders
    /// +2  u8  heads
    /// +5  u16 write precompensation
    /// +8  u8  control
    /// +14 u8  sectors per track
    /// ```
    unsafe fn from_bios_entry(entry_addr: *const u8) -> Option<Self> {
        // SAFETY: caller guarantees `entry_addr` points to a valid BIOS drive-info entry.
        let geo = unsafe {
            Self {
                cylinder_count: uaccess::read_u16(entry_addr.cast::<u16>()),
                head_count: u16::from(uaccess::read_u8(entry_addr.add(2))),
                write_precompensation: uaccess::read_u16(entry_addr.add(5).cast::<u16>()),
                control: uaccess::read_u8(entry_addr.add(8)),
                sectors_per_track: u16::from(uaccess::read_u8(entry_addr.add(14))),
            }
        };
        (geo.cylinder_count != 0).then_some(geo)
    }

    /// Total addressable sectors for this CHS geometry.
    fn total_sectors(&self) -> u32 {
        [self.head_count, self.sectors_per_track, self.cylinder_count]
            .into_iter()
            .fold(1u32, |acc, v| acc.saturating_mul(u32::from(v)))
    }
}

impl DrivePartition {
    /// Parse one partition entry from an MBR sector.
    ///
    /// Empty partition entries have a zero sector count and are represented as
    /// `None`. Only the LBA start/count fields are needed by this driver.
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
