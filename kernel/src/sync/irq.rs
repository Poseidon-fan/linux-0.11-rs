//! IRQ state helpers shared by synchronization primitives.
//!
//! External kernel code should prefer [`IrqSaveGuard`] over issuing raw
//! `cli`/`sti` instructions. The guard tracks nested IRQ-masked regions in
//! the current task and restores the original IF state on final drop.

use core::arch::asm;

use crate::task::current::{cur_irq_state, set_cur_irq_state};

/// EFLAGS bit for IF (Interrupt Flag).
const EFLAGS_IF: u32 = 1 << 9;

/// Save current IF and disable interrupts.
#[inline]
fn irq_save_and_disable() -> bool {
    let flags: u32;
    unsafe {
        asm!("pushfl", "popl {0}", "cli", out(reg) flags, options(att_syntax));
    }
    (flags & EFLAGS_IF) != 0
}

/// Restore IF to the previously saved state.
#[inline]
fn irq_restore(saved_if_enabled: bool) {
    if saved_if_enabled {
        sti();
    } else {
        cli();
    }
}

/// RAII guard for nested IRQ-masked regions.
pub struct IrqSaveGuard;

impl IrqSaveGuard {
    /// Enter one IRQ-masked region, recording IF on outermost entry only.
    #[inline]
    pub fn enter() -> Self {
        let (_, depth) = cur_irq_state();
        let outer_if_enabled = (depth == 0).then(irq_save_and_disable);
        let next_depth = depth + 1;

        set_cur_irq_state(outer_if_enabled, Some(next_depth));
        Self
    }
}

impl Drop for IrqSaveGuard {
    fn drop(&mut self) {
        let (saved_if_enabled, depth) = cur_irq_state();
        assert!(depth > 0, "IrqSaveGuard depth underflow at guard drop");

        let next_depth = depth - 1;
        if next_depth == 0 {
            set_cur_irq_state(Some(false), Some(0));
        } else {
            set_cur_irq_state(None, Some(next_depth));
        }

        if next_depth == 0 {
            irq_restore(saved_if_enabled);
        }
    }
}

/// Disable interrupts by clearing IF in EFLAGS.
#[inline]
fn cli() {
    unsafe {
        asm!("cli", options(att_syntax));
    }
}

/// Enable interrupts by setting IF in EFLAGS.
#[inline]
fn sti() {
    unsafe {
        asm!("sti", options(att_syntax));
    }
}
