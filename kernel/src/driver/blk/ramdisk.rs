//! RAM disk block driver.
//!
//! The RAM disk reserves one contiguous physical memory range during early
//! boot and exposes it as block major 1, minor 1. Requests are completed by
//! directly copying bytes between the queued buffer and that reserved memory.

use core::mem::MaybeUninit;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use crate::driver::blk::{BLOCK_MANAGER, BlockDeviceDriver, BlockRequest, RequestCmd, SECTOR_SIZE};

/// Block major used by the RAM disk driver.
const RAMDISK_MAJOR: usize = 1;
/// Only minor 1 is valid for the original RAM disk device.
const RAMDISK_MINOR: u8 = 1;
/// Fixed RAM disk capacity in bytes.
const RAMDISK_SIZE_BYTES: usize = 512 * 1024;

/// Storage for the single boot-time RAM disk driver instance.
static mut RAMDISK_DRIVER: MaybeUninit<Ramdisk> = MaybeUninit::uninit();

/// Synchronous RAM disk driver backed by one fixed byte range.
struct Ramdisk {
    /// Start address of the reserved RAM disk range.
    start: NonNull<u8>,
    /// Length in bytes of the reserved RAM disk range.
    length: usize,
}

// SAFETY: The driver stores one fixed boot-time RAM range. The pointer is used
// as an address token and all request dispatch remains serialized by the block
// layer in this single-core kernel model.
unsafe impl Sync for Ramdisk {}

/// Reserve RAM for the RAM disk and register the block driver.
pub fn init(main_memory_start: u32) -> u32 {
    let start = NonNull::new(main_memory_start as *mut u8).expect("ramdisk start must be non-null");

    unsafe {
        addr_of_mut!(RAMDISK_DRIVER).write(MaybeUninit::new(Ramdisk {
            start,
            length: RAMDISK_SIZE_BYTES,
        }));

        // The boot-time memory map is identity-mapped, so the physical start
        // address can be cleared through a raw kernel pointer here.
        ptr::write_bytes(start.as_ptr(), 0, RAMDISK_SIZE_BYTES);

        let driver = &*addr_of!(RAMDISK_DRIVER).cast::<Ramdisk>();
        BLOCK_MANAGER.exclusive_unchecked(|manager| {
            manager.register_block_driver(RAMDISK_MAJOR, driver);
        });
    }

    RAMDISK_SIZE_BYTES as u32
}

impl BlockDeviceDriver for Ramdisk {
    fn process_pending_requests(&self) {
        loop {
            let Some(request_ptr) =
                BLOCK_MANAGER.exclusive(|manager| manager.current_request(RAMDISK_MAJOR))
            else {
                return;
            };

            // The block layer keeps request slots in a fixed array. The current
            // request stays valid until `complete_current_request` removes it.
            let request = unsafe { request_ptr.as_ref() };
            if request.dev.minor() != RAMDISK_MINOR {
                super::complete_current_request(RAMDISK_MAJOR, false);
                continue;
            }

            let Some((rd_addr, byte_len)) = self.request_bytes(request) else {
                super::complete_current_request(RAMDISK_MAJOR, false);
                continue;
            };

            unsafe {
                match request.cmd {
                    RequestCmd::Write => {
                        ptr::copy_nonoverlapping(request.data_addr.as_ptr(), rd_addr, byte_len);
                    }
                    RequestCmd::Read => {
                        ptr::copy_nonoverlapping(rd_addr, request.data_addr.as_ptr(), byte_len);
                    }
                }
            }

            super::complete_current_request(RAMDISK_MAJOR, true);
        }
    }
}

impl Ramdisk {
    /// Translate one block request into this driver's byte window.
    fn request_bytes(&self, request: &BlockRequest) -> Option<(*mut u8, usize)> {
        let first_sector = request.first_sector as usize;
        let sector_count = request.sector_count as usize;
        let byte_offset = first_sector.checked_mul(SECTOR_SIZE)?;
        let byte_len = sector_count.checked_mul(SECTOR_SIZE)?;
        let byte_end = byte_offset.checked_add(byte_len)?;
        if byte_end > self.length {
            return None;
        }

        // The request has already been range-checked against the reserved RAM
        // disk length, so this pointer arithmetic stays inside the allocation.
        let rd_addr = unsafe { self.start.as_ptr().add(byte_offset) };
        Some((rd_addr, byte_len))
    }
}
