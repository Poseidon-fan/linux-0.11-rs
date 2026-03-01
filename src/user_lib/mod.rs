//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

mod syscall;

pub use syscall::*;

/// Run a focused `handle_no_page` test through `sys_test`.
fn test_handle_no_page() {
    test(100).expect("sys_test failed before no_page test");
    let ok = test1().expect("handle_no_page self-test syscall failed");
    test(ok as i32).expect("sys_test failed for no_page self-test result");
    assert_eq!(ok, 1, "handle_no_page self-test failed");
}

pub fn init() -> ! {
    test_handle_no_page();
    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
