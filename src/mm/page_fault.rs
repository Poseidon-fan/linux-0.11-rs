//! Page-fault handlers for demand paging and COW paths.
//!
//! These handlers are called from the trap layer with the minimal fault
//! context needed by memory management logic.

use crate::{mm::address::LinAddr, sync, task::TASK_MANAGER};

/// Handle a not-present page fault (`P=0` in the CPU error code).
///
/// # Arguments
/// - `error_code`: Raw i386 page-fault error code.
/// - `address`: Faulting linear address from CR2.
pub fn handle_no_page(error_code: u32, address: u32) {
    let _ = (error_code, address);
    crate::println!("no page fault on address: {:x}", address);
    todo!()
}

/// Handle a write-protect page fault on a present page (`P=1, W=1`).
///
/// # Arguments
/// - `address`: Faulting linear address from CR2.
pub fn handle_wp_page(address: u32) {
    let fault_addr = LinAddr::from(address);
    sync::cli();
    TASK_MANAGER.with_mut(|manager| {
        let task = manager.current();
        let mut inner = task.pcb.inner.borrow_mut();
        inner
            .memory_space
            .as_mut()
            .expect("handle_wp_page: current task has no memory space")
            .handle_wp_fault(fault_addr)
    });
    sync::sti();
}
