//! IRQ state helpers shared by synchronization primitives.
//!
//! External kernel code should prefer [`IrqSaveGuard`] over issuing raw
//! `cli`/`sti` instructions. The guard only tracks nested IRQ-masked regions
//! and restores the original IF state on final drop.

use core::{
    arch::asm,
    sync::atomic::{AtomicU32, Ordering},
};

/// EFLAGS bit for IF (Interrupt Flag).
const EFLAGS_IF: u32 = 1 << 9;
/// Low 31 bits store nested IRQ-masked depth.
const IRQ_DEPTH_MASK: u32 = 0x7fff_ffff;
/// High bit stores IF value captured at outermost entry.
const IRQ_SAVED_IF_BIT: u32 = 1 << 31;

/// Packed per-task IRQ nesting state shared by synchronization primitives.
static SYNC_IRQ_STATE: AtomicU32 = AtomicU32::new(0);

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
        let outer_if_enabled = ((SYNC_IRQ_STATE.load(Ordering::Relaxed) & IRQ_DEPTH_MASK) == 0)
            .then(irq_save_and_disable);

        let packed = SYNC_IRQ_STATE.load(Ordering::Relaxed);
        let depth = packed & IRQ_DEPTH_MASK;
        let next_depth = depth + 1;
        let saved_if_bit = if depth == 0 {
            let if_enabled =
                outer_if_enabled.expect("IrqSaveGuard::enter missing outer IRQ snapshot");
            if if_enabled { IRQ_SAVED_IF_BIT } else { 0 }
        } else {
            packed & IRQ_SAVED_IF_BIT
        };

        SYNC_IRQ_STATE.store(saved_if_bit | next_depth, Ordering::Relaxed);
        Self
    }
}

impl Drop for IrqSaveGuard {
    fn drop(&mut self) {
        let packed = SYNC_IRQ_STATE.load(Ordering::Relaxed);
        let depth = packed & IRQ_DEPTH_MASK;
        assert!(depth > 0, "IrqSaveGuard depth underflow at guard drop");

        let next_depth = depth - 1;
        let saved_if_enabled = (packed & IRQ_SAVED_IF_BIT) != 0;
        let next_packed = if next_depth == 0 {
            0
        } else {
            (packed & IRQ_SAVED_IF_BIT) | next_depth
        };
        SYNC_IRQ_STATE.store(next_packed, Ordering::Relaxed);

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
