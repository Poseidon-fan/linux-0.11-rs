mod manager;
pub mod task_struct;
mod timer;

use core::{arch::asm, mem};

use crate::{
    pmio::{inb_p, outb, outb_p},
    segment::{self, Descriptor, selectors},
    trap::{set_intr_gate, set_system_gate},
};

pub use manager::{TASK_MANAGER, TASK_NUM};

unsafe extern "C" {
    /// GDT defined in head.s
    static mut gdt: [u64; 256];
    /// Assembly entry point for `int 0x80`, defined in `syscall_entry.s`.
    fn system_call();
}

pub const HZ: u32 = 100;
const LATCH: u16 = (1193180 / HZ) as u16;

/// Initialize the scheduler and task system.
pub fn init() {
    // Access the TASK_MANAGER to trigger lazy initialization of task 0
    let manager = TASK_MANAGER.borrow();
    let task0 = manager.tasks[0].as_ref().expect("task 0 should exist");

    // Get addresses of task 0's TSS and LDT
    let task0_inner = task0.pcb.inner.borrow();
    let tss_addr = &task0_inner.tss as *const _ as u32;
    let ldt_addr = &task0_inner.ldt as *const _ as u32;
    drop(task0_inner);
    drop(manager);

    // Set TSS and LDT descriptors for task 0 in GDT
    segment::set_tss_desc(0, tss_addr);
    segment::set_ldt_desc(0, ldt_addr);

    // Clear GDT entries for tasks 1 to TASK_NUM-1
    // Each task uses 2 GDT entries (TSS and LDT descriptor)
    clear_gdt_entries_for_tasks();

    // Clear NT (Nested Task) flag in EFLAGS to prevent issues with task switching
    unsafe {
        asm!(
            "pushfl",
            "andl $0xffffbfff, (%esp)",
            "popfl",
            options(preserves_flags, att_syntax)
        );
    }

    // Load Task Register with task 0's TSS selector
    segment::ltr(selectors::tss_selector(0));

    // Load LDT Register with task 0's LDT selector
    segment::lldt(selectors::ldt_selector(0));

    outb_p(0x36, 0x43);
    outb_p((LATCH & 0xff) as u8, 0x40);
    outb_p((LATCH >> 8) as u8, 0x40);
    set_intr_gate(0x20, timer::timer_interrupt);
    outb(inb_p(0x21) & !0x01, 0x21);

    set_system_gate(0x80, unsafe {
        mem::transmute::<unsafe extern "C" fn(), extern "C" fn()>(system_call)
    });
}

/// Clear GDT entries for tasks 1 to TASK_NUM-1.
///
/// Each task has 2 GDT entries (TSS descriptor and LDT descriptor).
/// We set them all to null descriptors.
fn clear_gdt_entries_for_tasks() {
    for i in 1..TASK_NUM {
        let n = i as u16;
        // TSS entry index: FIRST_TSS_ENTRY + n * 2
        // LDT entry index: FIRST_LDT_ENTRY + n * 2
        let tss_index = (segment::FIRST_TSS_ENTRY + n * 2) as usize;
        let ldt_index = (segment::FIRST_LDT_ENTRY + n * 2) as usize;

        // Clear TSS and LDT descriptors
        unsafe {
            core::ptr::write_volatile(&mut gdt[tss_index], Descriptor::null().as_u64());
            core::ptr::write_volatile(&mut gdt[ldt_index], Descriptor::null().as_u64());
        }
    }
}
