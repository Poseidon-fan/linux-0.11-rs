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

#![allow(dead_code)]
use core::{
    arch::asm,
    cell::{Ref, RefCell, RefMut},
};

/// Enables interrupts by setting the IF (Interrupt Flag) in EFLAGS.
#[inline]
pub fn sti() {
    unsafe {
        asm!("sti");
    }
}

/// Disables interrupts by clearing the IF (Interrupt Flag) in EFLAGS.
#[inline]
pub fn cli() {
    unsafe {
        asm!("cli");
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
