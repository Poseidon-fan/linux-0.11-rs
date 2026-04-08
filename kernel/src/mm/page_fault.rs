//! Page-fault handlers for demand paging, page sharing, and COW paths.
//!
//! These handlers are called from the trap layer with the minimal fault
//! context needed by memory management logic.
//!
//! The not-present handler (`handle_no_page`) implements three resolution
//! paths in priority order:
//!
//! 1. **Page sharing** — if another process runs the same executable,
//!    share its already-loaded page (write-protected for COW).
//! 2. **Demand loading** — read the page from the executable file on disk.
//! 3. **Anonymous zero page** — allocate a fresh zeroed page (stack, heap,
//!    pure BSS regions, or processes with no executable).

use alloc::sync::Arc;

use crate::{
    fs::minix::Inode,
    mm::address::{LinAddr, LinPageNum},
    signal::SIGSEGV,
    task::{self, TASK_MANAGER},
};

/// Handle a not-present page fault (`P=0` in the CPU error code).
pub fn handle_no_page(_error_code: u32, address: u32) {
    let fault_page = LinAddr::from(address).floor();

    let (exe_inode, addr_offset, end_data, current_pde_base) = task::with_current(|inner| {
        let base = inner.ldt.data_segment().base();
        let pde_base = inner
            .memory_space
            .as_ref()
            .map(|ms| ms.pde_base())
            .unwrap_or(0);
        (
            inner.fs.executable_inode.clone(),
            (address & !0xFFF).wrapping_sub(base),
            inner.mem_layout.end_data,
            pde_base,
        )
    });

    if let Some(ref inode) = exe_inode {
        if addr_offset < end_data {
            if try_share_page(fault_page, inode, current_pde_base) {
                return;
            }

            // Take the MemorySpace out so the borrow is released before
            // `map_demand_page` does disk I/O (which may sleep/schedule).
            let mut space = task::current_task()
                .pcb
                .inner
                .exclusive(|inner| inner.memory_space.take());
            let loaded = space
                .as_mut()
                .map(|ms| ms.map_demand_page(fault_page, inode, addr_offset, end_data))
                .unwrap_or(false);
            task::current_task()
                .pcb
                .inner
                .exclusive(|inner| inner.memory_space = space);
            if loaded {
                return;
            }
        }
    }

    let mapped = task::with_current(|inner| {
        inner
            .memory_space
            .as_mut()
            .and_then(|ms| ms.map_zero_page(fault_page).ok())
            .is_some()
    });

    if mapped {
        return;
    }

    if task::current_slot() == 0 {
        panic!("handle_no_page(task0): map failed address={:#x}", address);
    }

    task::do_exit((1u32 << (SIGSEGV - 1)) as i32);
}

/// Scan the task table for a process that runs the same executable and
/// has the target page already loaded, then try to share that page.
fn try_share_page(fault_page: LinPageNum, exe_inode: &Arc<Inode>, current_pde_base: usize) -> bool {
    let current_slot = task::current_slot();
    let pte_index = fault_page.pte_index();
    let local_pde_index = fault_page.pde_index() - current_pde_base;

    TASK_MANAGER.exclusive(|tm| {
        for (slot, task) in tm.tasks.iter().enumerate() {
            let Some(task) = task else { continue };
            if slot == current_slot {
                continue;
            }

            // Single lock acquisition per candidate: check executable match
            // and extract pde_base together.
            let candidate = task.pcb.inner.exclusive(|inner| {
                let same_exe = inner
                    .fs
                    .executable_inode
                    .as_ref()
                    .map(|exe| Arc::ptr_eq(exe, exe_inode))
                    .unwrap_or(false);
                if !same_exe {
                    return None;
                }
                inner.memory_space.as_ref().map(|ms| ms.pde_base())
            });

            let Some(source_pde_base) = candidate else {
                continue;
            };

            let source_page =
                LinPageNum::from_indices(source_pde_base + local_pde_index, pte_index);

            let shared = task.pcb.inner.exclusive(|source_inner| {
                task::with_current(|current_inner| {
                    let source_ms = match source_inner.memory_space.as_mut() {
                        Some(ms) => ms,
                        None => return false,
                    };
                    let target_ms = match current_inner.memory_space.as_mut() {
                        Some(ms) => ms,
                        None => return false,
                    };
                    target_ms.try_share_from(source_ms, source_page, fault_page)
                })
            });

            if shared {
                return true;
            }
        }
        false
    })
}

/// Handle a write-protect page fault on a present page (`P=1, W=1`).
pub fn handle_wp_page(address: u32) {
    let fault_page = LinAddr::from(address).floor();
    task::with_current(|inner| {
        inner
            .memory_space
            .as_mut()
            .expect("handle_wp_page: current task has no memory space")
            .ensure_page_writable(fault_page)
    });
}
