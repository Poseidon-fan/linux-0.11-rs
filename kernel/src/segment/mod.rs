//! x86 segmentation utilities.

mod descriptor;
mod selector;
pub mod uaccess;

pub use descriptor::Descriptor;
pub use selector::{KERNEL_DS, SegmentSelector, USER_CS, USER_DS, ldt_selector, tss_selector};

use core::arch::asm;

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
