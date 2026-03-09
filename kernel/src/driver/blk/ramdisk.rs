//! RAM disk block driver.
//!
//! The RAM disk reserves one contiguous physical memory range during early
//! boot and exposes it as block major 1, minor 1. Requests are completed by
//! directly copying bytes between the queued buffer and that reserved memory.

use core::{
    ptr::{self, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
};

use super::{BlockRequest, BlockRequestType, SECTOR_SIZE};

/// Block major used by the RAM disk driver.
const RAMDISK_MAJOR: usize = 1;
/// Only minor 1 is valid for the RAM disk device.
const RAMDISK_MINOR: u8 = 1;
/// Fixed RAM disk capacity in bytes.
const RAMDISK_SIZE_BYTES: usize = 512 * 1024;

/// Start address of the reserved RAM disk range.
///
/// The address is written exactly once during early boot before the request
/// handler is registered, then read by the request path for address
/// translation.
static RAMDISK_START: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());

/// Reserve RAM for the RAM disk and register the request handler.
pub fn init(main_memory_start: u32) -> usize {
    let start = NonNull::new(main_memory_start as *mut u8).expect("ramdisk start must be non-null");

    unsafe {
        // Early boot keeps this reserved physical range identity mapped, so it
        // can be cleared through a raw kernel pointer here.
        ptr::write_bytes(start.as_ptr(), 0, RAMDISK_SIZE_BYTES);
    }

    RAMDISK_START
        .compare_exchange(
            ptr::null_mut(),
            start.as_ptr(),
            Ordering::Release,
            Ordering::Relaxed,
        )
        .expect("ramdisk initialized twice");

    super::register_device(RAMDISK_MAJOR, handle_request, None, None);
    RAMDISK_SIZE_BYTES
}

/// Process queued RAM disk requests until the device queue becomes empty.
fn handle_request() {
    loop {
        let Some(request_ptr) = super::BLOCK_MANAGER.exclusive(|manager| {
            let request_slot =
                manager.devices[RAMDISK_MAJOR].and_then(|device| device.current_request)?;
            manager.requests[request_slot].as_mut().map(NonNull::from)
        }) else {
            return;
        };

        // The request slot remains stable until `complete_current_request`
        // removes the queue head after the copy completes.
        let request = unsafe { request_ptr.as_ref() };
        if request.dev.minor() != RAMDISK_MINOR {
            super::complete_current_request(RAMDISK_MAJOR, false);
            continue;
        }

        let Some((ramdisk_addr, byte_len)) = request_bytes(request) else {
            super::complete_current_request(RAMDISK_MAJOR, false);
            continue;
        };

        unsafe {
            match request.ty {
                BlockRequestType::Write => {
                    ptr::copy_nonoverlapping(request.data_addr.as_ptr(), ramdisk_addr, byte_len);
                }
                BlockRequestType::Read => {
                    ptr::copy_nonoverlapping(ramdisk_addr, request.data_addr.as_ptr(), byte_len);
                }
            }
        }

        super::complete_current_request(RAMDISK_MAJOR, true);
    }
}

/// Return the reserved RAM disk base address after initialization.
fn ramdisk_start() -> NonNull<u8> {
    NonNull::new(RAMDISK_START.load(Ordering::Acquire))
        .expect("ramdisk request handler called before initialization")
}

/// Translate one block request into the RAM disk byte window.
fn request_bytes(request: &BlockRequest) -> Option<(*mut u8, usize)> {
    let first_sector = request.first_sector as usize;
    let sector_count = request.sector_count as usize;
    let byte_offset = first_sector.checked_mul(SECTOR_SIZE)?;
    let byte_len = sector_count.checked_mul(SECTOR_SIZE)?;
    let byte_end = byte_offset.checked_add(byte_len)?;
    if byte_end > RAMDISK_SIZE_BYTES {
        return None;
    }

    let start = ramdisk_start();

    // The request range has been validated against the reserved RAM disk
    // capacity, so this pointer arithmetic stays inside the backing area.
    let ramdisk_addr = unsafe { start.as_ptr().add(byte_offset) };
    Some((ramdisk_addr, byte_len))
}
