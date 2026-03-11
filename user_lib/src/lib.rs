//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

#![no_std]

mod syscall;

pub use syscall::*;

/// Boot-time location of the BIOS drive table.
const DRIVE_INFO_ADDR: *const u8 = 0x90080 as *const u8;

pub fn init() -> ! {
    setup(DRIVE_INFO_ADDR).unwrap();

    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
