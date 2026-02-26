//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

mod syscall;

pub use syscall::*;

static mut WAITPID_STATUS: u32 = 0;

pub fn waitpid_demo() {
    const WNOHANG: u32 = 1;

    let child_pid = fork().expect("waitpid_demo: fork failed");
    if child_pid == 0 {
        test(20).unwrap();
        exit().unwrap();
        unreachable!("waitpid_demo: child returned after exit");
    }

    let waited = loop {
        match waitpid(
            child_pid as i32,
            core::ptr::addr_of_mut!(WAITPID_STATUS),
            WNOHANG,
        ) {
            Ok(0) => core::hint::spin_loop(),
            Ok(pid) => break pid,
            Err(errno) => panic!("waitpid_demo: waitpid failed, errno={}", errno),
        }
    };
    // 30: parent observed a finished child via waitpid.
    test(30).unwrap();
    // Print the pid returned by waitpid for direct inspection.
    test(waited as i32).unwrap();
    // 31 means returned pid matches expected child pid; -31 means mismatch.
    test(if waited == child_pid { 31 } else { -31 }).unwrap();
}

pub fn init() -> ! {
    waitpid_demo();

    exit().unwrap();

    // Should not be reached.
    test(-1).unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
