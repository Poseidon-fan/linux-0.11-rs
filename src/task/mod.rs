pub mod current;
mod manager;
pub mod task_struct;
mod timer;
pub mod wait_queue;

use core::{arch::asm, mem};

use crate::{
    pmio::{inb_p, outb, outb_p},
    segment::{self, Descriptor, selectors},
    task::task_struct::TaskState,
    trap::{set_intr_gate, set_system_gate},
};

use self::current::{init_current_task, set_current_task};

pub use current::{current_slot, current_task};
pub use manager::{TASK_MANAGER, TASK_NUM};

unsafe extern "C" {
    /// GDT defined in head.s
    static mut gdt: [u64; 256];
    /// Assembly entry point for `int 0x80`, defined in `syscall_entry.s`.
    fn system_call();
}

pub const HZ: u32 = 100;
const LATCH: u16 = (1193180 / HZ) as u16;

/// Select and switch to the next runnable task.
///
/// If no better task exists, this function returns without switching.
#[inline]
pub fn schedule() {
    /// 32-bit far pointer used by `ljmp m16:32`.
    #[repr(C, packed)]
    struct FarPointer {
        offset: u32,
        selector: u16,
    }

    let Some(next) = TASK_MANAGER.exclusive(|manager| manager.select_next_task()) else {
        return;
    };

    let next_slot = next.pcb.slot;
    debug_assert!(next_slot < TASK_NUM);
    // Publish software-visible current task before hardware task switch.
    set_current_task(&next);
    let target = FarPointer {
        // For hardware task switching, only the selector is used.
        offset: 0,
        selector: selectors::tss_selector(next_slot as u16).as_u16(),
    };

    unsafe {
        asm!(
            "ljmp *({ptr})",
            ptr = in(reg) (&target as *const FarPointer),
            options(att_syntax),
        );
    }
}

/// Terminate the current task and switch to another runnable task.
///
/// Interrupt handling:
/// - The critical section is wrapped by `KernelCell::exclusive`, preventing
///   IRQ re-entry while task state is mutably borrowed.
pub fn do_exit(code: i32) -> ! {
    TASK_MANAGER.exclusive(|manager| {
        let current = current_task();
        let current_slot = current.pcb.slot;
        assert_ne!(current_slot, 0, "task[0] cannot exit");
        let current_pid = current.pcb.pid;

        // Re-parent all direct children to task 1.
        manager
            .tasks
            .iter()
            .enumerate()
            .filter(|(slot, _)| *slot != current_slot)
            .filter_map(|(_, task)| task.as_ref())
            .for_each(|task| unsafe {
                task.pcb.inner.exclusive_unchecked(|inner| {
                    if inner.relation.father == current_pid {
                        inner.relation.father = 1;
                    }
                });
            });

        unsafe {
            current.pcb.inner.exclusive_unchecked(|current_inner| {
                // Setting `memory_space` to `None` releases owned page tables and data frames.
                current_inner.memory_space = None;
                current_inner.sched.state = TaskState::Zombie;
                current_inner.exit_code = code;
            });
        }
    });
    schedule();

    panic!("do_exit returned unexpectedly");
}

/// Initialize the scheduler and task system.
pub fn init() {
    // Access TASK_MANAGER before TR is initialized.
    // Safety: boot path is single-flow and no IRQ-driven re-entry can contend here.
    let (task0, tss_addr, ldt_addr) = unsafe {
        TASK_MANAGER.exclusive_unchecked(|manager| {
            let task0 = manager.tasks[0]
                .as_ref()
                .expect("task 0 should exist")
                .clone();
            let (tss_addr, ldt_addr) = task0.pcb.inner.exclusive_unchecked(|task0_inner| {
                (
                    &task0_inner.tss as *const _ as u32,
                    &task0_inner.ldt as *const _ as u32,
                )
            });
            (task0, tss_addr, ldt_addr)
        })
    };
    init_current_task(&task0);

    // Set TSS and LDT descriptors for task 0 in GDT
    segment::set_tss_desc(0, tss_addr);
    segment::set_ldt_desc(0, ldt_addr);

    // Clear GDT entries for tasks 1 to TASK_NUM-1
    // Each task uses 2 GDT entries (TSS and LDT descriptor)
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
