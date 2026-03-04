//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

use core::hint::spin_loop;

mod syscall;

pub use syscall::*;

/// `sys_test` command: lock the test buffer.
const TEST_BUFFER_LOCK_CMD: i32 = 0x2001;
/// `sys_test` command: wait on the test buffer.
const TEST_BUFFER_WAIT_CMD: i32 = 0x2002;
/// `sys_test` command: unlock the test buffer.
const TEST_BUFFER_UNLOCK_CMD: i32 = 0x2003;
/// `sys_test` command: query whether a waiter is queued on the test buffer.
const TEST_BUFFER_HAS_WAITER_CMD: i32 = 0x2004;
/// `sys_test` marker: parent locked test buffer.
const TEST_BUFFER_MARK_PARENT_LOCKED: i32 = 0x2101;
/// `sys_test` marker: child is about to call wait.
const TEST_BUFFER_MARK_CHILD_WAIT_ENTER: i32 = 0x2102;
/// `sys_test` marker: parent observed queued waiter.
const TEST_BUFFER_MARK_PARENT_WAITER_SEEN: i32 = 0x2103;
/// `sys_test` marker: parent is about to unlock.
const TEST_BUFFER_MARK_PARENT_UNLOCK: i32 = 0x2104;
/// `sys_test` marker: child returned from wait.
const TEST_BUFFER_MARK_CHILD_WAIT_RETURN: i32 = 0x2105;
/// `sys_test` marker: parent finished waitpid.
const TEST_BUFFER_MARK_PARENT_WAITPID_DONE: i32 = 0x2106;

/// Run a focused `handle_no_page` test through `sys_test`.
fn test_handle_no_page() {
    test(100).expect("sys_test failed before no_page test");
    let ok = test1().expect("handle_no_page self-test syscall failed");
    test(ok as i32).expect("sys_test failed for no_page self-test result");
    assert_eq!(ok, 1, "handle_no_page self-test failed");
}

/// Run a wait/wakeup smoke test for buffer lock interfaces.
///
/// Flow:
/// 1. Parent locks one kernel test buffer.
/// 2. Child enters `wait` on that buffer and blocks.
/// 3. Parent polls until a waiter appears, then unlocks and wakes child.
/// 4. Child returns from wait and exits.
fn test_buffer() {
    test(TEST_BUFFER_UNLOCK_CMD).expect("buffer unlock reset failed");
    let lock_ok = test(TEST_BUFFER_LOCK_CMD).expect("buffer lock syscall failed");
    assert_eq!(lock_ok, 1, "buffer lock syscall returned unexpected value");
    test(TEST_BUFFER_MARK_PARENT_LOCKED).expect("buffer marker syscall failed");

    let pid = fork().expect("fork failed in buffer wait test");
    if pid == 0 {
        test(TEST_BUFFER_MARK_CHILD_WAIT_ENTER).expect("buffer marker syscall failed");
        let wait_ok = test(TEST_BUFFER_WAIT_CMD).expect("buffer wait syscall failed in child");
        assert_eq!(wait_ok, 1, "buffer wait syscall returned unexpected value");
        test(TEST_BUFFER_MARK_CHILD_WAIT_RETURN).expect("buffer marker syscall failed");
        exit().expect("child exit failed in buffer wait test");
        #[allow(clippy::empty_loop)]
        loop {}
    }

    const MAX_SPINS: usize = 2_000_000;
    let mut spins = 0usize;
    while test(TEST_BUFFER_HAS_WAITER_CMD).expect("buffer waiter query failed") == 0 {
        spins += 1;
        assert!(
            spins < MAX_SPINS,
            "buffer wait test timed out waiting for child sleep"
        );
        spin_loop();
    }

    test(TEST_BUFFER_MARK_PARENT_WAITER_SEEN).expect("buffer marker syscall failed");
    test(TEST_BUFFER_MARK_PARENT_UNLOCK).expect("buffer marker syscall failed");
    let unlock_ok = test(TEST_BUFFER_UNLOCK_CMD).expect("buffer unlock syscall failed");
    assert_eq!(
        unlock_ok, 1,
        "buffer unlock syscall returned unexpected value"
    );

    let mut status = 0u32;
    let waited_pid = waitpid(pid as i32, &mut status as *mut u32, 0)
        .expect("waitpid failed in buffer wait test");
    assert_eq!(waited_pid, pid, "waitpid returned unexpected child pid");
    test(TEST_BUFFER_MARK_PARENT_WAITPID_DONE).expect("buffer marker syscall failed");

    let waiter_left =
        test(TEST_BUFFER_HAS_WAITER_CMD).expect("buffer waiter query after wake failed");
    assert_eq!(
        waiter_left, 0,
        "buffer waiter queue should be empty after wake"
    );
}

pub fn init() -> ! {
    // test_handle_no_page();
    test_buffer();
    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
