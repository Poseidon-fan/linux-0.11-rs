//! Interior mutability wrapper for shared kernel state.

use core::cell::RefCell;
#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicU32, Ordering};

use super::TaskIrqGuard;

#[cfg(debug_assertions)]
static KERNEL_CELL_BORROW_DEPTH: AtomicU32 = AtomicU32::new(0);

/// Debug-only tracker for active `KernelCell` borrows.
///
/// The borrow itself is scoped by `RefCell`, but this extra depth counter lets
/// us assert that no task switch happens while any `KernelCell` closure is
/// still running.
struct KernelCellBorrowGuard;

impl KernelCellBorrowGuard {
    #[inline]
    fn enter() -> Self {
        #[cfg(debug_assertions)]
        {
            KERNEL_CELL_BORROW_DEPTH.fetch_add(1, Ordering::Relaxed);
        }
        Self
    }
}

impl Drop for KernelCellBorrowGuard {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            let previous = KERNEL_CELL_BORROW_DEPTH.fetch_sub(1, Ordering::Relaxed);
            assert!(previous > 0, "KernelCell borrow depth underflow");
        }
    }
}

/// Assert that the current code path may safely reschedule.
///
/// In debug builds this checks that no `KernelCell` borrow is currently active,
/// which would otherwise cross a scheduling point. Release builds compile this
/// check away.
#[inline]
pub(crate) fn assert_can_schedule(context: &str) {
    #[cfg(debug_assertions)]
    {
        assert_eq!(
            KERNEL_CELL_BORROW_DEPTH.load(Ordering::Relaxed),
            0,
            "{context} must not reschedule while holding a KernelCell borrow"
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = context;
}

/// Interior mutability wrapper for static variables in kernel context.
///
/// # Example
///
/// ```ignore
/// static DATA: KernelCell<u32> = KernelCell::new(0);
///
/// fn increment() {
///     DATA.exclusive(|value| *value += 1);
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
    /// Create a new kernel cell containing `value`.
    pub const fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    /// Execute a closure with exclusive mutable access.
    ///
    /// This is the normal entry for shared kernel state. It disables IRQs for
    /// the duration of the closure and, in debug builds, tracks that a
    /// `KernelCell` borrow is active so scheduling assertions can catch bugs.
    #[inline]
    pub fn exclusive<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let _irq_guard = TaskIrqGuard::enter();
        let _borrow_guard = KernelCellBorrowGuard::enter();
        self.with_borrow(f)
    }

    /// Execute a closure with exclusive mutable access without IRQ management.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that no re-entrant access can happen while
    /// this closure runs. Typical valid sites:
    /// - Early boot code that already runs without IRQ-driven re-entry.
    /// - Interrupt-gate handlers where hardware has already masked IRQs.
    #[inline]
    pub unsafe fn exclusive_unchecked<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let _borrow_guard = KernelCellBorrowGuard::enter();
        self.with_borrow(f)
    }

    #[inline]
    fn with_borrow<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        f(&mut *self.inner.borrow_mut())
    }
}
