//! GDT task descriptor operations.

use super::task_struct::TaskStateSegment;
use crate::segment::Descriptor;

unsafe extern "C" {
    static mut gdt: [u64; 256];
}

/// First TSS entry index in the GDT.
pub const FIRST_TSS_ENTRY: u16 = 4;

/// First LDT descriptor entry index in the GDT.
pub const FIRST_LDT_ENTRY: u16 = 5;

/// Writes a TSS descriptor for the task at `task_slot` into the GDT.
#[inline]
pub fn set_tss_desc(task_slot: u16, tss_addr: u32) {
    let desc = Descriptor::tss(tss_addr, core::mem::size_of::<TaskStateSegment>() as u32);
    unsafe {
        core::ptr::write_volatile(&mut gdt[tss_gdt_index(task_slot)], desc.as_u64());
    }
}

/// Writes an LDT descriptor for the task at `task_slot` into the GDT.
#[inline]
pub fn set_ldt_desc(task_slot: u16, ldt_addr: u32) {
    // 3 entries (null + cs + ds), 8 bytes each, limit = 24 - 1 = 23
    let desc = Descriptor::ldt(ldt_addr, 3 * 8 - 1);
    unsafe {
        core::ptr::write_volatile(&mut gdt[ldt_gdt_index(task_slot)], desc.as_u64());
    }
}

/// Clears both TSS and LDT descriptors for the task at `task_slot`.
#[inline]
pub fn clear_task_descs(task_slot: u16) {
    let null = Descriptor::null().as_u64();
    unsafe {
        core::ptr::write_volatile(&mut gdt[tss_gdt_index(task_slot)], null);
        core::ptr::write_volatile(&mut gdt[ldt_gdt_index(task_slot)], null);
    }
}

const fn tss_gdt_index(task_slot: u16) -> usize {
    (FIRST_TSS_ENTRY + task_slot * 2) as usize
}

const fn ldt_gdt_index(task_slot: u16) -> usize {
    (FIRST_LDT_ENTRY + task_slot * 2) as usize
}
