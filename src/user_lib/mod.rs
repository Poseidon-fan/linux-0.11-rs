//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

mod syscall;

pub use syscall::*;

extern "C" fn sigchld_test_handler(signr: i32) {
    test(2000 + signr).unwrap();
}

extern "C" fn sigchld_test_restorer() -> ! {
    test(2999).unwrap();
    exit().unwrap();

    #[allow(clippy::empty_loop)]
    loop {}
}

pub fn init() -> ! {
    const SIGCHLD: u32 = 17;

    test(100).unwrap();
    test2(
        sigchld_test_handler as usize as u32,
        sigchld_test_restorer as usize as u32,
        SIGCHLD,
    )
    .unwrap();

    let pid = fork().unwrap();
    if pid == 0 {
        test(300).unwrap();
        exit().unwrap();

        #[allow(clippy::empty_loop)]
        loop {}
    }

    let mut status = 0u32;
    waitpid(pid as i32, &mut status as *mut u32, 0).unwrap();

    // Should not be reached if SIGCHLD delivery path works.
    test(-100).unwrap();
    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
