//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

mod syscall;

pub use syscall::*;

extern "C" fn signal_test_handler(signr: i32) {
    test(1000 + signr).unwrap();
}

extern "C" fn signal_test_restorer() -> ! {
    test(1999).unwrap();
    exit().unwrap();

    #[allow(clippy::empty_loop)]
    loop {}
}

pub fn init() -> ! {
    const TEST_SIGNAL: u32 = 10;

    test(100).unwrap();
    test1(
        signal_test_handler as usize as u32,
        signal_test_restorer as usize as u32,
        TEST_SIGNAL,
    )
    .unwrap();

    // Should not be reached if signal delivery path works.
    test(-100).unwrap();
    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
