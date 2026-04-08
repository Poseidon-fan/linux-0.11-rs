//! RAII interrupt-flag save/restore guard.

use core::arch::asm;

use crate::task::{cur_irq_state, set_cur_irq_state};

/// RAII guard that masks interrupts for the duration of its lifetime.
///
/// Supports nesting: each task carries an `irq_state: AtomicU8` packed as
/// `[IF:1 | depth:7]`. Only the outermost guard (depth 0→1) executes `cli`
/// and saves IF; only the last drop (1→0) restores it. Inner guards merely
/// bump the counter.
///
/// `irq_state` is `AtomicU8` rather than [`KernelCell`](super::KernelCell)
/// to avoid circularity — `KernelCell::exclusive` itself creates a guard.
pub struct IrqSaveGuard;

impl IrqSaveGuard {
    /// Enters an IRQ-masked region, saving IF on the outermost call.
    #[inline]
    pub fn enter() -> Self {
        let (_, depth) = cur_irq_state();
        let outer_if_enabled = (depth == 0).then(save_and_disable);
        set_cur_irq_state(outer_if_enabled, Some(depth + 1));
        Self
    }
}

impl Drop for IrqSaveGuard {
    fn drop(&mut self) {
        let (saved_if, depth) = cur_irq_state();
        assert!(depth > 0, "IrqSaveGuard depth underflow");

        let next = depth - 1;
        if next == 0 {
            set_cur_irq_state(Some(false), Some(0));
            restore(saved_if);
        } else {
            set_cur_irq_state(None, Some(next));
        }
    }
}

const EFLAGS_IF: u32 = 1 << 9;

/// Saves the current IF state and executes `cli`.
#[inline]
fn save_and_disable() -> bool {
    let flags: u32;
    unsafe {
        asm!("pushfl", "popl {0}", "cli", out(reg) flags, options(att_syntax));
    }
    (flags & EFLAGS_IF) != 0
}

/// Restores IF to `if_was_enabled`.
#[inline]
fn restore(if_was_enabled: bool) {
    if if_was_enabled {
        unsafe { asm!("sti", options(att_syntax)) };
    } else {
        unsafe { asm!("cli", options(att_syntax)) };
    }
}
