//! In-memory block device backed by one reserved physical memory window.
//!
//! Memory layout:
//!
//! ```text
//! main_memory_start
//!      |
//!      v
//! +---------------------------+---------------------------+
//! |      ramdisk storage      |  normal allocatable RAM   |
//! +---------------------------+---------------------------+
//! ^                           ^
//! |                           |
//! rd_start                    rd_start + rd_length
//! ```
//!
//! The ramdisk consumes a fixed memory range before the page-frame allocator
//! starts. Requests are completed synchronously by copying bytes between this
//! reserved region and the buffer-cache block memory.

use core::ptr::{self, NonNull};

use super::BlockRequestType;

/// Block-device major reserved for the ramdisk.
const RAMDISK_MAJOR: usize = 1;
/// The active ramdisk minor matches the original Linux 0.11 layout.
const RAMDISK_MINOR: u8 = 1;
/// Fixed ramdisk capacity reserved during boot.
const RAMDISK_SIZE: usize = 512 * 1024;

/// One immutable snapshot of the reserved ramdisk memory window.
#[derive(Clone, Copy)]
struct RamDisk {
    start: NonNull<u8>,
    length: usize,
}

static mut RAMDISK: Option<RamDisk> = None;

/// Reserve and zero the ramdisk memory window.
///
/// Returns the number of bytes that must be excluded from normal RAM
/// allocation after boot.
pub fn init(mem_start: u32) -> usize {
    let start = NonNull::new(mem_start as *mut u8).expect("ramdisk start must be non-zero");

    unsafe {
        // SAFETY: Boot-time initialization runs before the general allocator
        // and before any ramdisk request can be submitted.
        ptr::write_bytes(start.as_ptr(), 0, RAMDISK_SIZE);
        RAMDISK = Some(RamDisk {
            start,
            length: RAMDISK_SIZE,
        });
    }

    super::register_device(RAMDISK_MAJOR, do_request, None, None);

    RAMDISK_SIZE
}

/// Drain queued requests for the ramdisk device.
fn do_request() {
    loop {
        let Some(request) = super::current_request(RAMDISK_MAJOR) else {
            return;
        };
        let success = transfer(request);
        super::complete_current_request(RAMDISK_MAJOR, success);
    }
}

/// Execute one memory-to-memory block transfer.
fn transfer(request: super::BlockRequestState) -> bool {
    let ramdisk = unsafe {
        // SAFETY: `init()` installs the ramdisk state before registration,
        // so the request path can only run after the storage window exists.
        RAMDISK.expect("ramdisk not initialized")
    };

    if request.dev.minor() != RAMDISK_MINOR {
        return false;
    }

    let Some(offset) = (request.first_sector as usize).checked_mul(super::SECTOR_SIZE) else {
        return false;
    };
    let Some(length) = (request.sector_count as usize).checked_mul(super::SECTOR_SIZE) else {
        return false;
    };
    let Some(end) = offset.checked_add(length) else {
        return false;
    };
    if end > ramdisk.length {
        return false;
    }

    unsafe {
        // SAFETY:
        // - `offset..end` is range-checked against the reserved ramdisk region.
        // - `data_addr` comes from the block-request queue and points to one
        //   locked buffer-cache block for the lifetime of the request.
        // - Source and destination do not overlap because ramdisk storage lives
        //   in a dedicated reserved memory range outside the buffer-cache pool.
        let ramdisk_addr = ramdisk.start.as_ptr().add(offset);
        match request.ty {
            BlockRequestType::Write => {
                ptr::copy_nonoverlapping(request.data_addr.as_ptr(), ramdisk_addr, length);
            }
            BlockRequestType::Read => {
                ptr::copy_nonoverlapping(ramdisk_addr, request.data_addr.as_ptr(), length);
            }
        }
    }

    true
}
