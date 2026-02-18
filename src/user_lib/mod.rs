//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

mod syscall;

pub use syscall::*;

pub fn init() -> ! {
    test(false).unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
