//! ATA hard disk controller register and command definitions.
//!
//! This module keeps the low-level constants used by the original Linux 0.11
//! hard disk driver, but exposes them with Rust-style names and types.

use core::hint::spin_loop;

use bitflags::bitflags;
use log::warn;

use crate::pmio::{inb, inb_p, outb, outb_p};

use super::{DriveGeometry, HARD_DISK_MANAGER, InterruptPhase};

/// Primary ATA task-file data register.
pub const DATA_PORT: u16 = 0x1F0;
/// Primary ATA error register when read.
pub const ERROR_PORT: u16 = 0x1F1;
/// Primary ATA write-precompensation register when written.
pub const PRECOMP_PORT: u16 = ERROR_PORT;
/// Primary ATA sector-count register.
pub const SECTOR_COUNT_PORT: u16 = 0x1F2;
/// Primary ATA sector-number register.
pub const SECTOR_NUMBER_PORT: u16 = 0x1F3;
/// Primary ATA cylinder-low register.
pub const CYLINDER_LOW_PORT: u16 = 0x1F4;
/// Primary ATA cylinder-high register.
pub const CYLINDER_HIGH_PORT: u16 = 0x1F5;
/// Primary ATA drive/head selector register.
pub const DRIVE_HEAD_PORT: u16 = 0x1F6;
/// Primary ATA status register when read.
pub const STATUS_PORT: u16 = 0x1F7;
/// Primary ATA command register when written.
pub const COMMAND_PORT: u16 = STATUS_PORT;
/// Primary ATA device-control register.
pub const CONTROL_PORT: u16 = 0x3F6;

/// Drive/head register base pattern used by the original driver.
///
/// Bit layout:
///
/// ```text
///  7 6 5 4 3 2 1 0
/// +-----+-+-------+
/// | 101 |D| Head  |
/// +-----+-+-------+
/// ```
const DRIVE_HEAD_BASE: u8 = 0xA0;

/// Software reset bit written to the control register.
const CONTROL_RESET_BIT: u8 = 1 << 2;

/// Low control bits restored after a controller reset.
const CONTROL_CONFIGURATION_MASK: u8 = 0x0F;

bitflags! {
    /// ATA controller status bits returned by [`STATUS_PORT`].
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct StatusFlags: u8 {
        /// The error register contains command failure details.
        const ERROR = 1 << 0;
        /// The controller requests a PIO data transfer.
        const DATA_REQUEST = 1 << 3;
        /// The selected drive completed its seek operation.
        const SEEK_COMPLETE = 1 << 4;
        /// The drive reported a write fault.
        const WRITE_FAULT = 1 << 5;
        /// The selected drive is ready to accept commands.
        const READY = 1 << 6;
        /// The controller is busy executing a command.
        const BUSY = 1 << 7;
    }
}

/// ATA commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ControllerCommand {
    /// Recalibrate the selected drive to cylinder 0.
    Restore = 0x10,
    /// Read one or more sectors using PIO.
    Read = 0x20,
    /// Write one or more sectors using PIO.
    Write = 0x30,
    /// Program the controller with the drive geometry.
    Specify = 0x91,
}

/// ATA task-file register values for one controller command.
pub struct AtaTaskFile {
    /// Target drive index, `0` or `1`.
    pub drive_index: usize,
    /// Number of sectors to transfer.
    pub sector_count: u8,
    /// One-based sector number inside the current track.
    pub sector: u8,
    /// Head number inside the current cylinder.
    pub head: u8,
    /// Cylinder number.
    pub cylinder: u16,
    /// Command opcode written to the controller.
    pub command: ControllerCommand,
}

/// Poll the ATA status register until the provided predicate accepts it.
pub fn wait_for_status(retries: usize, ready: impl Fn(StatusFlags) -> bool) -> Option<StatusFlags> {
    (0..retries).find_map(|_| {
        let status = StatusFlags::from_bits_truncate(inb_p(STATUS_PORT));
        ready(status).then_some(status)
    })
}

/// Check whether the most recently completed ATA command succeeded.
pub fn command_succeeded() -> bool {
    let status = StatusFlags::from_bits_truncate(inb_p(STATUS_PORT));
    let expected = StatusFlags::READY | StatusFlags::SEEK_COMPLETE;
    let observed = status
        & (StatusFlags::BUSY
            | StatusFlags::READY
            | StatusFlags::WRITE_FAULT
            | StatusFlags::SEEK_COMPLETE
            | StatusFlags::ERROR);

    if observed == expected {
        return true;
    }

    if status.contains(StatusFlags::ERROR) {
        let _ = inb(ERROR_PORT);
    }

    false
}

/// Reset the ATA controller and restore its normal control byte.
pub fn reset_controller(control: u8) {
    const RESET_DELAY_ITERATIONS: usize = 100;
    const RESET_READY_RETRIES: usize = 10_000;
    const RESET_EXPECTED_ERROR_STATUS: u8 = 0x01;

    outb(CONTROL_RESET_BIT, CONTROL_PORT);
    for _ in 0..RESET_DELAY_ITERATIONS {
        spin_loop();
    }
    outb(control & CONTROL_CONFIGURATION_MASK, CONTROL_PORT);

    // Wait until the controller is ready to accept commands.
    if !{
        let _ = wait_for_status(RESET_READY_RETRIES, |status| {
            !status.contains(StatusFlags::BUSY) && status.contains(StatusFlags::READY)
        });

        let status = StatusFlags::from_bits_truncate(inb(STATUS_PORT));
        let expected = StatusFlags::READY | StatusFlags::SEEK_COMPLETE;
        let observed =
            status & (StatusFlags::BUSY | StatusFlags::READY | StatusFlags::SEEK_COMPLETE);

        observed == expected
    } {
        warn!("HD controller still busy after reset");
    }

    let error_status = inb(ERROR_PORT);
    if error_status != RESET_EXPECTED_ERROR_STATUS {
        warn!("HD controller reset failed: {:02x}", error_status);
    }
}

/// Program the ATA task-file registers and issue one controller command.
pub fn issue_command(
    geometry: DriveGeometry,
    task_file: AtaTaskFile,
    interrupt_phase: InterruptPhase,
) {
    const COMMAND_READY_RETRIES: usize = 100_000;

    if task_file.drive_index > 1 || task_file.head > 0x0F {
        panic!("Trying to issue ATA command with invalid drive/head");
    }

    if wait_for_status(COMMAND_READY_RETRIES, |status| {
        !status.contains(StatusFlags::BUSY)
    })
    .is_none()
    {
        panic!("HD controller not ready");
    }

    HARD_DISK_MANAGER.exclusive(|manager| {
        manager.interrupt_phase = interrupt_phase;
    });

    outb_p(geometry.control, CONTROL_PORT);
    outb_p((geometry.write_precompensation >> 2) as u8, PRECOMP_PORT);
    outb_p(task_file.sector_count, SECTOR_COUNT_PORT);
    outb_p(task_file.sector, SECTOR_NUMBER_PORT);
    outb_p(task_file.cylinder as u8, CYLINDER_LOW_PORT);
    outb_p((task_file.cylinder >> 8) as u8, CYLINDER_HIGH_PORT);
    outb_p(
        DRIVE_HEAD_BASE | ((task_file.drive_index as u8) << 4) | task_file.head,
        DRIVE_HEAD_PORT,
    );
    outb(task_file.command as u8, COMMAND_PORT);
}
