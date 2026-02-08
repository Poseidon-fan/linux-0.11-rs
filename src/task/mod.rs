mod manager;
pub mod task_struct;

use core::arch::asm;

use crate::{
    segment::{self, Descriptor, selectors},
    trap::set_system_gate,
};

pub use manager::{TASK_MANAGER, TASK_NUM};

unsafe extern "C" {
    /// GDT defined in head.s
    static mut gdt: [u64; 256];
    /// Assembly entry point for `int 0x80`, defined in `syscall_entry.s`.
    fn system_call();
}

/// Initialize the scheduler and task system.
///
/// This corresponds to `sched_init()` in the original Linux 0.11 kernel.
/// It performs the following:
/// 1. Set up TSS and LDT descriptors in GDT for task 0
/// 2. Clear GDT entries for other tasks
/// 3. Clear NT flag in EFLAGS
/// 4. Load Task Register and LDT Register
///
/// Timer initialization is not handled now.
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

    // Safety: system_call is the assembly entry point in syscall_entry.s.
    // It must be cast because extern functions declared in `unsafe extern`
    // blocks are typed as `unsafe extern "C" fn()`, while Handler is safe.
    set_system_gate(0x80, unsafe {
        core::mem::transmute::<unsafe extern "C" fn(), extern "C" fn()>(system_call)
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
