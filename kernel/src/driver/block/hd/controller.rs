//! ATA hard disk controller register and command definitions.

use core::hint::spin_loop;

use bitflags::bitflags;
use log::warn;

use super::{ATA_DRIVER, DriveGeometry, InterruptPhase};
use crate::pmio::{
    ATA_CONTROL_PORT, ATA_CYLINDER_HIGH_PORT, ATA_CYLINDER_LOW_PORT, ATA_DRIVE_HEAD_PORT,
    ATA_ERROR_PORT, ATA_SECTOR_COUNT_PORT, ATA_SECTOR_NUMBER_PORT, ATA_STATUS_PORT, inb, inb_p,
    outb, outb_p,
};

/// Poll the ATA status register until the provided predicate accepts it.
pub fn wait_for_status(retries: usize, ready: impl Fn(StatusFlags) -> bool) -> Option<StatusFlags> {
    (0..retries).find_map(|_| {
        let status = StatusFlags::from_bits_truncate(inb_p(ATA_STATUS_PORT));
        ready(status).then_some(status)
    })
}

/// Check whether the most recently completed ATA command succeeded.
pub fn command_succeeded() -> bool {
    let status = StatusFlags::from_bits_truncate(inb_p(ATA_STATUS_PORT));
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
        let _ = inb(ATA_ERROR_PORT);
    }

    false
}

/// Reset the ATA controller and restore its normal control byte.
pub fn reset_controller(control: u8) {
    const RESET_DELAY_ITERATIONS: usize = 100;
    const RESET_READY_RETRIES: usize = 10_000;
    const RESET_EXPECTED_ERROR_STATUS: u8 = 0x01;

    outb(CONTROL_RESET_BIT, ATA_CONTROL_PORT);
    for _ in 0..RESET_DELAY_ITERATIONS {
        spin_loop();
    }
    outb(control & CONTROL_CONFIGURATION_MASK, ATA_CONTROL_PORT);

    // Wait until the controller is ready to accept commands.
    if !{
        let _ = wait_for_status(RESET_READY_RETRIES, |status| {
            !status.contains(StatusFlags::BUSY) && status.contains(StatusFlags::READY)
        });

        let status = StatusFlags::from_bits_truncate(inb(ATA_STATUS_PORT));
        let expected = StatusFlags::READY | StatusFlags::SEEK_COMPLETE;
        let observed =
            status & (StatusFlags::BUSY | StatusFlags::READY | StatusFlags::SEEK_COMPLETE);

        observed == expected
    } {
        warn!("HD controller still busy after reset");
    }

    let error_status = inb(ATA_ERROR_PORT);
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

    ATA_DRIVER.exclusive(|driver| {
        driver.phase = interrupt_phase;
    });

    outb_p(geometry.control, ATA_CONTROL_PORT);
    outb_p((geometry.write_precompensation >> 2) as u8, ATA_ERROR_PORT);
    outb_p(task_file.sector_count, ATA_SECTOR_COUNT_PORT);
    outb_p(task_file.sector, ATA_SECTOR_NUMBER_PORT);
    outb_p(task_file.cylinder as u8, ATA_CYLINDER_LOW_PORT);
    outb_p((task_file.cylinder >> 8) as u8, ATA_CYLINDER_HIGH_PORT);
    outb_p(
        DRIVE_HEAD_BASE | ((task_file.drive_index as u8) << 4) | task_file.head,
        ATA_DRIVE_HEAD_PORT,
    );
    outb(task_file.command as u8, ATA_STATUS_PORT);
}

bitflags! {
    /// ATA controller status bits returned by [`ATA_STATUS_PORT`].
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

/// Drive/head register base pattern selecting the LBA/CHS addressing mode.
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
