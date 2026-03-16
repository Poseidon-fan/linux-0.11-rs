//! Interior mutability wrapper for shared kernel state.

use core::{
    cell::RefCell,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::sync::{EFLAGS_IF, read_eflags_and_cli};

use super::{cli, sti};

/// Low 31 bits store nested `exclusive` depth.
const IRQ_DEPTH_MASK: u32 = 0x7fff_ffff;
/// High bit stores IF value captured at outermost entry.
const IRQ_SAVED_IF_BIT: u32 = 1 << 31;

/// Packed IRQ nesting state for `KernelCell::exclusive`.
static SYNC_IRQ_STATE: AtomicU32 = AtomicU32::new(0);

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

/// Return the current nested IRQ-masking depth for `KernelCell::exclusive`.
#[inline]
pub fn current_irq_depth() -> u32 {
    SYNC_IRQ_STATE.load(Ordering::Relaxed) & IRQ_DEPTH_MASK
}

/// RAII guard for per-task IRQ nesting in `KernelCell::exclusive`.
struct TaskIrqGuard;

impl TaskIrqGuard {
    #[inline]
    pub fn enter() -> Self {
        let outer_flags = (SYNC_IRQ_STATE.load(Ordering::Relaxed) & IRQ_DEPTH_MASK == 0)
            .then(read_eflags_and_cli);

        let packed = SYNC_IRQ_STATE.load(Ordering::Relaxed);
        let depth = packed & IRQ_DEPTH_MASK;
        let next_depth = depth + 1;
        let saved_if_bit = if depth == 0 {
            let flags = outer_flags.expect("KernelCell::exclusive missing outer IRQ snapshot");
            if (flags & EFLAGS_IF) != 0 {
                IRQ_SAVED_IF_BIT
            } else {
                0
            }
        } else {
            packed & IRQ_SAVED_IF_BIT
        };

        SYNC_IRQ_STATE.store(saved_if_bit | next_depth, Ordering::Relaxed);
        Self
    }
}

impl Drop for TaskIrqGuard {
    fn drop(&mut self) {
        let packed = SYNC_IRQ_STATE.load(Ordering::Relaxed);
        let depth = packed & IRQ_DEPTH_MASK;
        assert!(
            depth > 0,
            "KernelCell::exclusive depth underflow at guard drop"
        );

        let next_depth = depth - 1;
        let saved_if_enabled = (packed & IRQ_SAVED_IF_BIT) != 0;
        let next_packed = if next_depth == 0 {
            0
        } else {
            (packed & IRQ_SAVED_IF_BIT) | next_depth
        };
        SYNC_IRQ_STATE.store(next_packed, Ordering::Relaxed);

        if next_depth == 0 {
            if saved_if_enabled {
                sti();
            } else {
                cli();
            }
        }
    }
}

impl<T> KernelCell<T> {
    /// Create a new kernel cell containing `value`.
    pub const fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    /// Execute a closure with exclusive mutable access.
    ///
    /// This API is the normal entry for kernel shared state:
    /// - On first entry for the current task, it records IF and disables IRQs.
    /// - Nested calls only increase per-task depth.
    /// - On final exit, it restores IF state for this task.
    ///
    /// # Panics
    ///
    /// In debug builds, this panics if current-task tracking is not initialized.
    /// Such early-boot paths should use [`exclusive_unchecked`](Self::exclusive_unchecked).
    #[inline]
    pub fn exclusive<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        #[cfg(debug_assertions)]
        {
            use crate::task;

            let _ = task::current_task();
        }
        let _guard = TaskIrqGuard::enter();
        // SAFETY: `exclusive` enforces the interrupt-side exclusion contract.
        unsafe { self.exclusive_unchecked(f) }
    }

    /// Execute a closure with exclusive mutable access without IRQ management.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that no re-entrant access can happen while
    /// this closure runs. Typical valid sites:
    /// - Before `task::init()`, where current-task tracking is not initialized.
    /// - Interrupt-gate handlers where hardware has already masked IRQs.
    #[inline]
    pub unsafe fn exclusive_unchecked<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        f(&mut *self.inner.borrow_mut())
    }
}
