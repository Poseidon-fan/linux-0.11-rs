//! Synchronization primitives for kernel use.
//!
//! This module provides interior mutability and task-blocking mutexes for a
//! single-core, non-preemptive kernel environment.
//!
//! # Safety
//!
//! [`KernelCell`] is safe to use because:
//! - Single-core CPU: no parallel execution
//! - Non-preemptive kernel: kernel code won't be preempted by scheduler
//! - Runtime borrow checking: `RefCell` panics on conflicting borrows
//!
//! # Interrupt protection
//!
//! If data is accessed by both normal kernel paths and interrupt handlers,
//! use [`KernelCell::exclusive`] to enter a nested IRQ-masked critical
//! section. Code running in contexts that already
//! guarantee IRQ exclusion can use [`KernelCell::exclusive_unchecked`].

use core::arch::naked_asm;

use crate::segment::selectors::{USER_CS, USER_DS};

mod busy_lock;
mod cell;
mod irq;
mod mutex;

pub use busy_lock::BusyLock;
pub use cell::KernelCell;
pub(crate) use cell::assert_can_schedule;
pub(crate) use irq::IrqSaveGuard;
#[allow(unused_imports)]
pub use mutex::{Mutex, MutexGuard};

/// Switch from kernel mode (ring 0) to user mode (ring 3).
///
/// This function constructs a fake interrupt return frame on the stack
/// and uses `iret` to transition to ring 3. After the transition,
/// all data segment registers (DS, ES, FS, GS) are set to the user data segment.
///
/// # How it works
///
/// 1. Save the current stack pointer (ESP)
/// 2. Push a fake interrupt frame: SS, ESP, EFLAGS, CS, EIP
/// 3. Force IF=1 in the saved EFLAGS image for user-mode execution
/// 4. Execute `iret` which pops the frame and switches to ring 3
/// 5. Continue execution at the return address, now in user mode
/// 6. Set all data segment registers to user data segment
///
/// # Safety
///
/// - Must be called from ring 0 (kernel mode)
/// - The current task's LDT must be properly configured with user code/data segments
/// - After this call, the code continues in ring 3 with limited privileges
#[naked]
pub extern "C" fn move_to_user_mode() {
    unsafe {
        naked_asm!(
            // Save current stack pointer
            "movl %esp, %eax",

            // Build iret frame (from high to low address):
            "pushl ${user_ds}",      // SS  = user data segment (0x17)
            "pushl %eax",            // ESP = current stack pointer
            "pushfl",                // EFLAGS
            "orl $0x200, (%esp)",    // Force IF=1 in the saved EFLAGS image
            "pushl ${user_cs}",      // CS  = user code segment (0x0f)
            "pushl $2f",             // EIP = address of label "2"

            // Perform the privilege level switch
            "iret",

            // --- Now executing in user mode (ring 3) ---
            "2:",
            "movl ${user_ds}, %eax",
            "movw %ax, %ds",
            "movw %ax, %es",
            "movw %ax, %fs",
            "movw %ax, %gs",
            "ret",

            user_cs = const USER_CS.as_u16(),
            user_ds = const USER_DS.as_u16(),
            options(att_syntax),
        );
    }
}
