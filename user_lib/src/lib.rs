//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

#![no_std]

mod syscall;

pub use syscall::*;

pub fn init() -> ! {
    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
