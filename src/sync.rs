//! Synchronization primitives for kernel use.
//!
//! This module provides interrupt-safe wrappers for shared mutable state.
//! In a single-core kernel without preemptive scheduling, disabling interrupts
//! is sufficient to ensure exclusive access to shared data.
//!
//! # Contents
//!
//! - [`sti`] / [`cli`]: Low-level functions to enable/disable interrupts.
//! - [`IrqSafeCell`]: Interior mutability wrapper with automatic interrupt management.

#![allow(dead_code)]
use core::{
    arch::asm,
    cell::{RefCell, RefMut},
    ops::{Deref, DerefMut},
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

/// A wrapper providing interior mutability with automatic interrupt disabling.
///
/// # Example
///
/// ```ignore
/// static DATA: IrqSafeCell<u32> = unsafe { IrqSafeCell::new(0) };
///
/// // Option 1: Scoped access
/// DATA.exclusive_session(|data| {
///     *data += 1;
/// });
///
/// // Option 2: RAII guard
/// {
///     let mut guard = DATA.exclusive_access();
///     *guard += 1;
/// } // interrupts restored here
/// ```
pub struct IrqSafeCell<T> {
    inner: RefCell<T>,
}

unsafe impl<T> Sync for IrqSafeCell<T> {}

/// RAII guard for [`IrqSafeCell`]. Restores interrupt state on drop.
pub struct IrqSafeRefMut<'a, T> {
    inner: Option<RefMut<'a, T>>,
    irq_was_enabled: bool,
}

impl<T> IrqSafeCell<T> {
    pub const unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    pub fn exclusive_access(&self) -> IrqSafeRefMut<'_, T> {
        let irq_was_enabled = interrupts_enabled();
        irq_was_enabled.then(cli);
        IrqSafeRefMut {
            inner: Some(self.inner.borrow_mut()),
            irq_was_enabled,
        }
    }

    pub fn exclusive_session<F, V>(&self, f: F) -> V
    where
        F: FnOnce(&mut T) -> V,
    {
        let mut inner = self.exclusive_access();
        f(inner.deref_mut())
    }
}

impl<T> Drop for IrqSafeRefMut<'_, T> {
    fn drop(&mut self) {
        self.inner = None;
        self.irq_was_enabled.then(sti);
    }
}

impl<T> Deref for IrqSafeRefMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap().deref()
    }
}

impl<T> DerefMut for IrqSafeRefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap().deref_mut()
    }
}

/// Returns true if interrupts are currently enabled (IF flag is set).
#[inline]
fn interrupts_enabled() -> bool {
    let eflags: u32;
    unsafe {
        asm!("pushfd; pop {}", out(reg) eflags, options(nomem, preserves_flags));
    }
    // IF (Interrupt Flag) is bit 9
    (eflags & (1 << 9)) != 0
}
