pub mod process;
pub mod todo;

use linkme::distributed_slice;

use crate::{mm::address::LinAddr, syscall::context::SyscallContext, task};

#[distributed_slice]
pub static SYSCALL_TABLE: [fn(&SyscallContext) -> Result<u32, u32>];

/// Probe offset (inside each task's 64MB slot) for no-page self-test.
const TEST_NO_PAGE_OFFSET: u32 = 0x0010_0000;

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
        crate::println!("test value: {}", value as i32);
        Ok(0)
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
