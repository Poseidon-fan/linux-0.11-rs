//! Process management: scheduling, task lifecycle, and timer interrupt.
//!
//! - [`task_struct`] — PCB, TSS, LDT, and the `Task` / `TaskPage` types.
//! - [`manager`] — 64-slot task table and runnable-task selection.
//! - [`current`] — tracks the currently executing task.
//! - [`timer`] — PIT IRQ0 handler, jiffies counter, CPU time accounting.
//! - [`wait_queue`] — single-slot sleep/wake primitives.

mod current;
mod gdt;
mod manager;
pub mod task_struct;
mod timer;
mod wait_queue;

use core::{arch::asm, mem};

pub use current::{cur_irq_state, current_slot, current_task, set_cur_irq_state, try_current_slot};
pub use gdt::{FIRST_LDT_ENTRY, FIRST_TSS_ENTRY, set_ldt_desc, set_tss_desc};
pub use manager::{TASK_MANAGER, TASK_NUM};
pub use task_struct::{TASK_OPEN_FILES_LIMIT, TaskState};
pub use timer::{HZ, jiffies};
pub use wait_queue::WaitQueue;

use crate::{
    pmio::{inb_p, outb, outb_p},
    segment,
    signal::{SIGALRM, SIGCHLD, SIGHUP, SIGKILL, SIGSTOP},
    sync::assert_can_schedule,
    trap::{self, TrapHandler},
};

/// Execute `f` with exclusive access to the current task's PCB inner state.
pub fn with_current<F, R>(f: F) -> R
where F: FnOnce(&mut task_struct::TaskControlBlockInner) -> R {
    current_task().pcb.inner.exclusive(f)
}

/// Returns true if the current process has superuser privileges (euid == 0).
#[inline]
pub fn is_super() -> bool {
    with_current(|inner| inner.identity.euid == 0)
}

unsafe extern "C" {
    /// Assembly entry point for `int 0x80`, defined in `syscall_entry.s`.
    fn system_call();
}

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
    assert_can_schedule("schedule");

    let Some(next) = TASK_MANAGER.exclusive(|manager| {
        const BLOCKABLE: u32 = !((1 << (SIGKILL - 1)) | (1 << (SIGSTOP - 1)));
        let j = timer::jiffies();
        manager
            .tasks
            .iter()
            .filter_map(|t| t.as_ref())
            .for_each(|task| {
                task.pcb.inner.exclusive(|inner| {
                    if inner.signal_info.alarm > 0 && inner.signal_info.alarm < j {
                        inner.signal_info.raise(SIGALRM);
                        inner.signal_info.alarm = 0;
                    }
                    let unblocked =
                        inner.signal_info.signal & !(BLOCKABLE & inner.signal_info.blocked);
                    if unblocked != 0 {
                        inner.sched.wake_if_interruptible();
                    }
                });
            });
        manager.select_next_task()
    }) else {
        return;
    };

    let next_slot = next.pcb.slot;
    debug_assert!(next_slot < TASK_NUM);
    // Publish software-visible current task before hardware task switch.
    current::set_current_task(&next);
    let target = FarPointer {
        // For hardware task switching, only the selector is used.
        offset: 0,
        selector: segment::tss_selector(next_slot as u16).as_u16(),
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

        // Read relation fields and tear down the current process in one access.
        let (father_pid, is_leader, current_session) = unsafe {
            current.pcb.inner.exclusive_unchecked(|inner| {
                let rel = (
                    inner.relation.father,
                    inner.relation.leader,
                    inner.relation.session,
                );
                // Release owned page tables and data frames.
                inner.memory_space = None;
                inner.sched.state = TaskState::Zombie;
                inner.exit_code = code;
                rel
            })
        };

        // Re-parent all direct children to task 1.
        // If session leader, also send SIGHUP to tasks in same session.
        manager
            .tasks
            .iter()
            .enumerate()
            .filter(|&(slot, _)| slot != current_slot)
            .filter_map(|(_, task)| task.as_ref())
            .for_each(|task| unsafe {
                task.pcb.inner.exclusive_unchecked(|inner| {
                    if inner.relation.father == current_pid {
                        inner.relation.father = 1;
                    }
                    if is_leader && inner.relation.session == current_session {
                        inner.signal_info.raise(SIGHUP);
                        inner.sched.wake_if_interruptible();
                    }
                });
            });

        // Notify parent process with SIGCHLD so waiters can observe child exit.
        if let Some(father_task) = manager
            .tasks
            .iter()
            .filter_map(|task| task.as_ref())
            .find(|task| task.pcb.pid == father_pid)
        {
            unsafe {
                father_task.pcb.inner.exclusive_unchecked(|father_inner| {
                    father_inner.signal_info.raise(SIGCHLD);
                    father_inner.sched.wake_if_interruptible();
                });
            }
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
    current::set_current_task(&task0);

    // Set TSS and LDT descriptors for task 0 in GDT
    gdt::set_tss_desc(0, tss_addr);
    gdt::set_ldt_desc(0, ldt_addr);

    // Clear GDT entries for tasks 1 to TASK_NUM-1
    for i in 1..TASK_NUM {
        gdt::clear_task_descs(i as u16);
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
    segment::ltr(segment::tss_selector(0));

    // Load LDT Register with task 0's LDT selector
    segment::lldt(segment::ldt_selector(0));

    const PIT_CMD: u16 = 0x43;
    const PIT_CH0: u16 = 0x40;
    // Channel 0, lobyte/hibyte, mode 3 (square wave generator).
    outb_p(0x36, PIT_CMD);
    outb_p((timer::LATCH & 0xff) as u8, PIT_CH0);
    outb_p((timer::LATCH >> 8) as u8, PIT_CH0);
    trap::set_intr_gate(0x20, timer::timer_interrupt);
    outb(inb_p(0x21) & !0x01, 0x21);

    trap::set_system_gate(0x80, unsafe {
        mem::transmute::<unsafe extern "C" fn(), TrapHandler>(system_call)
    });
}
