pub mod process;
pub mod todo;

use linkme::distributed_slice;

use crate::{mm::address::LinAddr, syscall::context::SyscallContext, task};

#[distributed_slice]
pub static SYSCALL_TABLE: [fn(&SyscallContext) -> Result<u32, u32>];

/// Probe offset (inside each task's 64MB slot) for no-page self-test.
const TEST_NO_PAGE_OFFSET: u32 = 0x0010_0000;
/// Test command: lock the test buffer.
const TEST_BUFFER_LOCK_CMD: i32 = 0x2001;
/// Test command: wait on the test buffer.
const TEST_BUFFER_WAIT_CMD: i32 = 0x2002;
/// Test command: unlock the test buffer.
const TEST_BUFFER_UNLOCK_CMD: i32 = 0x2003;
/// Test command: query whether the test buffer has waiters.
const TEST_BUFFER_HAS_WAITER_CMD: i32 = 0x2004;
/// Test marker: parent locked test buffer.
const TEST_BUFFER_MARK_PARENT_LOCKED: i32 = 0x2101;
/// Test marker: child is about to wait.
const TEST_BUFFER_MARK_CHILD_WAIT_ENTER: i32 = 0x2102;
/// Test marker: parent observed child waiter.
const TEST_BUFFER_MARK_PARENT_WAITER_SEEN: i32 = 0x2103;
/// Test marker: parent is about to unlock.
const TEST_BUFFER_MARK_PARENT_UNLOCK: i32 = 0x2104;
/// Test marker: child wait returned.
const TEST_BUFFER_MARK_CHILD_WAIT_RETURN: i32 = 0x2105;
/// Test marker: parent finished waitpid.
const TEST_BUFFER_MARK_PARENT_WAITPID_DONE: i32 = 0x2106;

// linkme requires an integer literal in `distributed_slice(..., N)`.
// This helper keeps a named syscall number and the required literal in one place.
#[macro_export]
macro_rules! define_syscall_handler {
    (
        $nr_name:ident = $nr:literal,
        fn $fn_name:ident($ctx:ident : &SyscallContext) -> $ret:ty $body:block
    ) => {
        pub const $nr_name: u32 = $nr;

        #[distributed_slice(SYSCALL_TABLE, $nr)]
        fn $fn_name($ctx: &SyscallContext) -> $ret $body
    };
}

define_syscall_handler!(
    NR_TEST = 74,
    fn sys_test(ctx: &SyscallContext) -> Result<u32, u32> {
        let (value, _, _) = ctx.args();
        let value = value as i32;
        match value {
            TEST_BUFFER_LOCK_CMD => {
                let buffer =
                    crate::fs::buffer::first_buffer_handle().ok_or(crate::syscall::ENODEV)?;
                buffer.lock();
                Ok(1)
            }
            TEST_BUFFER_WAIT_CMD => {
                let buffer =
                    crate::fs::buffer::first_buffer_handle().ok_or(crate::syscall::ENODEV)?;
                buffer.wait();
                Ok(1)
            }
            TEST_BUFFER_UNLOCK_CMD => {
                let buffer =
                    crate::fs::buffer::first_buffer_handle().ok_or(crate::syscall::ENODEV)?;
                buffer.unlock();
                Ok(1)
            }
            TEST_BUFFER_HAS_WAITER_CMD => {
                let buffer =
                    crate::fs::buffer::first_buffer_handle().ok_or(crate::syscall::ENODEV)?;
                Ok(buffer.has_waiter() as u32)
            }
            TEST_BUFFER_MARK_PARENT_LOCKED => {
                crate::println!("[buffer-test] parent locked test buffer");
                Ok(1)
            }
            TEST_BUFFER_MARK_CHILD_WAIT_ENTER => {
                crate::println!("[buffer-test] child entering wait");
                Ok(1)
            }
            TEST_BUFFER_MARK_PARENT_WAITER_SEEN => {
                crate::println!("[buffer-test] parent detected waiter");
                Ok(1)
            }
            TEST_BUFFER_MARK_PARENT_UNLOCK => {
                crate::println!("[buffer-test] parent unlocking test buffer");
                Ok(1)
            }
            TEST_BUFFER_MARK_CHILD_WAIT_RETURN => {
                crate::println!("[buffer-test] child wait returned");
                Ok(1)
            }
            TEST_BUFFER_MARK_PARENT_WAITPID_DONE => {
                crate::println!("[buffer-test] parent waitpid done");
                Ok(1)
            }
            _ => {
                crate::println!("test value: {}", value);
                Ok(0)
            }
        }
    }
);

define_syscall_handler!(
    NR_TEST1 = 75,
    fn sys_test1(_ctx: &SyscallContext) -> Result<u32, u32> {
        // Trigger `handle_no_page` once and verify probe PTE becomes present.
        let linear_addr = task::current_task().pcb.inner.exclusive(|inner| {
            inner
                .ldt
                .data_segment()
                .base()
                .wrapping_add(TEST_NO_PAGE_OFFSET)
        });
        let probe_page = LinAddr::from(linear_addr).floor();

        let before_present = task::current_task().pcb.inner.exclusive(|inner| {
            inner
                .memory_space
                .as_mut()
                .and_then(|space| space.find_pte(probe_page))
                .map(|pte| {
                    let raw: u32 = (*pte).into();
                    (raw & 1) != 0
                })
                .unwrap_or(false)
        });

        if !before_present {
            crate::mm::page_fault::handle_no_page(0, linear_addr);
        }

        let after_present = task::current_task().pcb.inner.exclusive(|inner| {
            inner
                .memory_space
                .as_mut()
                .and_then(|space| space.find_pte(probe_page))
                .map(|pte| {
                    let raw: u32 = (*pte).into();
                    (raw & 1) != 0
                })
                .unwrap_or(false)
        });

        crate::println!(
            "[sys_test] no_page probe_addr={:#x} before_present={} after_present={}",
            linear_addr,
            before_present,
            after_present
        );
        Ok(after_present as u32)
    }
);
