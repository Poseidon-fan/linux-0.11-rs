//! GDT task descriptor operations.

use crate::segment::Descriptor;

unsafe extern "C" {
    static mut gdt: [u64; 256];
}

/// First TSS entry index in the GDT.
pub const FIRST_TSS_ENTRY: u16 = 4;

/// First LDT descriptor entry index in the GDT.
pub const FIRST_LDT_ENTRY: u16 = 5;

/// TSS structure size (104 bytes in Linux 0.11).
const TSS_SIZE: u32 = 104;

/// Writes a TSS descriptor for task `n` into the GDT.
#[inline]
pub fn set_tss_desc(n: u16, tss_addr: u32) {
    let desc = Descriptor::tss(tss_addr, TSS_SIZE);
    unsafe {
        core::ptr::write_volatile(&mut gdt[tss_index(n)], desc.as_u64());
    }
}

/// Writes an LDT descriptor for task `n` into the GDT.
#[inline]
pub fn set_ldt_desc(n: u16, ldt_addr: u32) {
    // 3 entries (null + cs + ds), 8 bytes each, limit = 24 - 1 = 23
    let desc = Descriptor::ldt(ldt_addr, 3 * 8 - 1);
    unsafe {
        core::ptr::write_volatile(&mut gdt[ldt_index(n)], desc.as_u64());
    }
}

/// Clears both TSS and LDT descriptors for task `n`.
#[inline]
pub fn clear_task_descs(n: u16) {
    let null = Descriptor::null().as_u64();
    unsafe {
        core::ptr::write_volatile(&mut gdt[tss_index(n)], null);
        core::ptr::write_volatile(&mut gdt[ldt_index(n)], null);
    }
}

const fn tss_index(n: u16) -> usize {
    (FIRST_TSS_ENTRY + n * 2) as usize
}

const fn ldt_index(n: u16) -> usize {
    (FIRST_LDT_ENTRY + n * 2) as usize
}
