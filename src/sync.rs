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
//! # Interrupt protection
//!
//! If data is accessed by both normal kernel paths and interrupt handlers,
//! use [`KernelCell::exclusive`] to enter a per-task IRQ-nested critical
//! section. Code running before `task::init()` or in contexts that already
//! guarantee IRQ exclusion can use [`KernelCell::exclusive_unchecked`].

use core::{
    arch::{asm, naked_asm},
    cell::RefCell,
    sync::atomic::{AtomicU32, Ordering},
};
use lazy_static::lazy_static;

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

/// EFLAGS bit for IF (Interrupt Flag).
const EFLAGS_IF: u32 = 1 << 9;
/// Low 31 bits store nested `exclusive` depth.
const IRQ_DEPTH_MASK: u32 = 0x7fff_ffff;
/// High bit stores IF value captured at outermost entry.
const IRQ_SAVED_IF_BIT: u32 = 1 << 31;

lazy_static! {
    /// Packed IRQ nesting state for `KernelCell::exclusive`.
    static ref SYNC_IRQ_STATE: AtomicU32 = AtomicU32::new(0);
}

#[inline]
pub fn current_irq_depth() -> u32 {
    SYNC_IRQ_STATE.load(Ordering::Relaxed) & IRQ_DEPTH_MASK
}

/// Save current EFLAGS and disable interrupts.
#[inline]
fn read_eflags_and_cli() -> u32 {
    let flags: u32;
    unsafe {
        asm!("pushfl", "popl {0}", "cli", out(reg) flags, options(att_syntax));
    }
    flags
}

/// RAII guard for per-task IRQ nesting in [`KernelCell::exclusive`].
pub struct TaskIrqGuard;

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
