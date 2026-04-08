//! x86 segmentation: descriptors, selectors, and privilege transitions.
//!
//! This module manages the segment-based isolation model used by the kernel.
//! Each task is assigned a 64 MB linear address window through its LDT, and
//! hardware task switching is driven by per-task TSS/LDT descriptors in the GDT.
//!
//! - [`Descriptor`] — 64-bit GDT/LDT segment descriptor encoding.
//! - [`SegmentSelector`] — 16-bit selector with index, TI, and RPL fields.
//! - [`uaccess`] — kernel ↔ user data transfer via the FS segment register.

mod descriptor;
mod selector;
pub mod uaccess;

use core::arch::{asm, naked_asm};

pub use descriptor::Descriptor;
pub use selector::{
    KERNEL_CS, KERNEL_DS, SegmentSelector, USER_CS, USER_DS, ldt_selector, tss_selector,
};

/// Loads the Task Register with a TSS selector (`ltr` instruction).
#[inline]
pub fn ltr(selector: SegmentSelector) {
    unsafe {
        asm!("ltr {0:x}", in(reg) selector.as_u16(), options(nomem, nostack, att_syntax));
    }
}

/// Loads the LDT Register with an LDT descriptor selector (`lldt` instruction).
#[inline]
pub fn lldt(selector: SegmentSelector) {
    unsafe {
        asm!("lldt {0:x}", in(reg) selector.as_u16(), options(nomem, nostack, att_syntax));
    }
}

/// Switches from kernel mode (Ring 0) to user mode (Ring 3) via `iret`.
///
/// Builds a fake interrupt return frame on the stack, then executes `iret` to
/// drop privilege. After the transition all data segment registers are set to
/// the user data segment.
#[naked]
pub extern "C" fn move_to_user_mode() {
    unsafe {
        naked_asm!(
            "movl %esp, %eax",
            "pushl ${user_ds}",
            "pushl %eax",
            "pushfl",
            "orl $0x200, (%esp)",
            "pushl ${user_cs}",
            "pushl $2f",
            "iret",
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
