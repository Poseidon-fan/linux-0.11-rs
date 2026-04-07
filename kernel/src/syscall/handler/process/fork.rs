use alloc::sync::Arc;

use linkme::distributed_slice;

#[allow(unused_imports)]
use crate::syscall::SYSCALL_TABLE;
use crate::{
    define_syscall_handler,
    mm::space::TASK_LINEAR_SIZE,
    segment::{self, KERNEL_DS},
    syscall::{EAGAIN, context::SyscallContext},
    task::{self, TASK_MANAGER, task_struct::*},
};

define_syscall_handler!(
    user_lib::NR_FORK = 2,
    fn sys_fork(ctx: &mut SyscallContext) -> Result<u32, u32> {
        unsafe extern "C" {
            static pg_dir: u8;
        }

        // 1. Find a free slot and allocate a new task page.
        let (slot, pid) = TASK_MANAGER
            .exclusive(|manager| manager.find_empty_process())
            .ok_or(EAGAIN)?;
        let mut new_task = Task::new().ok_or(EAGAIN)?;

        // 2. Build child PCB from parent state with COW memory.
        let parent = task::current_task();
        let new_base = slot as u32 * TASK_LINEAR_SIZE;
        let stack_top = new_task.stack_top();
        let cr3 = unsafe { &pg_dir as *const u8 as u32 };

        let child_inner = parent.pcb.inner.exclusive(|p| {
            let data_base = p.ldt.data_segment().base();
            let code_base = p.ldt.code_segment().base();
            assert_eq!(data_base, code_base, "separate I&D not supported");

            let code_limit = p.ldt.code_segment().byte_limit();
            let data_limit = p.ldt.data_segment().byte_limit();
            assert!(
                data_limit >= code_limit,
                "bad data_limit: data < code (0x{:x} < 0x{:x})",
                data_limit,
                code_limit
            );

            let child_memory_space = p
                .memory_space
                .as_ref()
                .expect("parent has no memory space")
                .cow_copy(slot, data_limit)
                .map_err(|_| EAGAIN)?;

            let mut child_ldt = p.ldt.clone();
            child_ldt.set_base(new_base);

            Ok::<_, u32>(TaskControlBlockInner {
                sched: TaskSchedInfo {
                    state: TaskState::Running,
                    counter: p.sched.priority,
                    priority: p.sched.priority,
                },
                relation: TaskRelationInfo {
                    father: parent.pcb.pid,
                    pgrp: p.relation.pgrp,
                    session: p.relation.session,
                    leader: 0,
                },
                identity: p.identity,
                acct: TaskAcctInfo::default(),
                memory_space: Some(child_memory_space),
                mem_layout: p.mem_layout,
                exit_code: 0,
                tty: p.tty,
                fs: p.fs.clone(),
                ldt: child_ldt,
                tss: child_tss(ctx, stack_top, cr3, slot),
                signal_info: TaskSignalInfo {
                    signal: 0,
                    blocked: p.signal_info.blocked,
                    sigaction: p.signal_info.sigaction.clone(),
                    alarm: 0,
                },
            })
        })?;

        new_task.pcb = TaskControlBlock::new(slot, pid, child_inner);

        // 3. Install TSS and LDT descriptors in GDT.
        let (tss_addr, ldt_addr) = new_task.pcb.inner.exclusive(|inner| {
            (
                &inner.tss as *const TaskStateSegment as u32,
                inner.ldt.as_ptr(),
            )
        });
        task::set_tss_desc(slot as u16, tss_addr);
        task::set_ldt_desc(slot as u16, ldt_addr);

        // 4. Insert into task table.
        TASK_MANAGER.exclusive(|manager| {
            manager.tasks[slot] = Some(Arc::new(new_task));
        });

        Ok(pid)
    }
);

/// EAX = 0 so `fork()` returns 0 in the child; all other registers
/// and segment selectors are copied from the parent's syscall context.
fn child_tss(ctx: &SyscallContext, stack_top: u32, cr3: u32, slot: usize) -> TaskStateSegment {
    TaskStateSegment {
        back_link: 0,
        esp0: stack_top,
        ss0: KERNEL_DS.as_u32(),
        esp1: 0,
        ss1: 0,
        esp2: 0,
        ss2: 0,
        cr3,
        eip: ctx.eip,
        eflags: ctx.eflags,
        eax: 0,
        ecx: ctx.ecx,
        edx: ctx.edx,
        ebx: ctx.ebx,
        esp: ctx.user_esp,
        ebp: ctx.ebp,
        esi: ctx.esi,
        edi: ctx.edi,
        es: ctx.es & 0xffff,
        cs: ctx.cs & 0xffff,
        ss: ctx.user_ss & 0xffff,
        ds: ctx.ds & 0xffff,
        fs: ctx.fs & 0xffff,
        gs: ctx.gs & 0xffff,
        ldt: segment::ldt_selector(slot as u16).as_u32(),
        trace_bitmap: 0x8000_0000,
        i387: I387Struct::empty(),
    }
}
