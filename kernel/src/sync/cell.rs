use core::cell::RefCell;
#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicU32, Ordering};

use crate::sync::irq::IrqSaveGuard;

/// IRQ-safe interior mutability wrapper for `static` kernel data.
///
/// Wraps a [`RefCell`] and gates every mutable access behind an IRQ-masked
/// critical section, making it safe for `static` items on a single-core kernel.
///
/// In debug builds a global borrow-depth counter detects accidental
/// scheduling while a borrow is held (see [`assert_can_schedule`]).
///
/// # Panics
///
/// Panics on reentrant mutable borrows, indicating a kernel bug.
pub struct KernelCell<T> {
    inner: RefCell<T>,
}

// SAFETY: Single-core kernel; IRQ masking prevents concurrent access.
unsafe impl<T> Sync for KernelCell<T> {}

impl<T> KernelCell<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    /// Runs `f` with exclusive access while IRQs are masked.
    #[inline]
    pub fn exclusive<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let _irq = IrqSaveGuard::enter();
        let _borrow = BorrowGuard::enter();
        f(&mut *self.inner.borrow_mut())
    }

    /// Runs `f` with exclusive access **without** masking IRQs.
    ///
    /// # Safety
    ///
    /// The caller must ensure no reentrant access can occur (e.g. early boot
    /// or inside an interrupt handler that already holds IRQs masked).
    #[inline]
    pub unsafe fn exclusive_unchecked<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let _borrow = BorrowGuard::enter();
        f(&mut *self.inner.borrow_mut())
    }
}

/// Panics if a [`KernelCell`] borrow is currently held (debug builds only).
///
/// If a task switch occurs while a borrow is active, the switched-to task
/// may enter the same critical section and attempt a second mutable borrow,
/// violating Rust's aliasing rules and causing a `RefCell` panic.
#[inline]
pub fn assert_can_schedule(context: &str) {
    #[cfg(debug_assertions)]
    {
        assert_eq!(
            BORROW_DEPTH.load(Ordering::Relaxed),
            0,
            "{context} must not reschedule while holding a KernelCell borrow"
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = context;
}

#[cfg(debug_assertions)]
static BORROW_DEPTH: AtomicU32 = AtomicU32::new(0);

struct BorrowGuard;

impl BorrowGuard {
    #[inline]
    fn enter() -> Self {
        #[cfg(debug_assertions)]
        BORROW_DEPTH.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for BorrowGuard {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            let prev = BORROW_DEPTH.fetch_sub(1, Ordering::Relaxed);
            assert!(prev > 0, "KernelCell borrow depth underflow");
        }
    }
}
