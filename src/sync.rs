//! Synchronization primitives for kernel use.
//!
//! This module provides interior mutability for static variables in a
//! single-core, non-preemptive kernel environment.
//!
//! # Safety
//!
//! [`KernelCell`] is safe to use because:
//! - Single-core CPU: no parallel execution
//! - Non-preemptive kernel: kernel code won't be preempted by scheduler
//! - Runtime borrow checking: `RefCell` panics on conflicting borrows
//!
//! # Note
//!
//! If data is accessed by both syscall handlers and interrupt handlers,
//! you must manually use [`cli`]/[`sti`] to protect critical sections.

use core::{
    arch::{asm, naked_asm},
    cell::{Ref, RefCell, RefMut},
};

use crate::segment::selectors::{USER_CS, USER_DS};

/// Enables interrupts by setting the IF (Interrupt Flag) in EFLAGS.
#[inline]
pub fn sti() {
    unsafe {
        asm!("sti", options(att_syntax));
    }
}

/// Disables interrupts by clearing the IF (Interrupt Flag) in EFLAGS.
#[inline]
pub fn cli() {
    unsafe {
        asm!("cli", options(att_syntax));
    }
}

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
/// 3. Execute `iret` which pops the frame and switches to ring 3
/// 4. Continue execution at the return address, now in user mode
/// 5. Set all data segment registers to user data segment
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

/// Interior mutability wrapper for static variables in kernel context.
///
/// # Example
///
/// ```ignore
/// static DATA: KernelCell<u32> = KernelCell::new(0);
///
/// fn increment() {
///     *DATA.borrow_mut() += 1;
/// }
/// ```
///
/// # Panics
///
/// Panics if a borrow conflict occurs (e.g., nested mutable borrows).
/// This indicates a bug in the kernel code.
#[derive(Clone)]
pub struct KernelCell<T> {
    inner: RefCell<T>,
}

// SAFETY: Single-core, non-preemptive kernel ensures only one execution
// flow accesses the cell at a time. RefCell's runtime checks catch bugs.
unsafe impl<T> Sync for KernelCell<T> {}

impl<T> KernelCell<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    #[inline]
    pub fn borrow(&self) -> Ref<'_, T> {
        self.inner.borrow()
    }

    #[inline]
    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        self.inner.borrow_mut()
    }

    /// Executes a closure with mutable access to the inner value.
    #[inline]
    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        f(&mut *self.borrow_mut())
    }
}
