use alloc::sync::Arc;
use core::array;
use core::mem::MaybeUninit;
use core::ptr::{addr_of_mut, write_bytes};
use core::sync::atomic::{AtomicU32, Ordering};

use lazy_static::lazy_static;

use crate::{
    mm::space::{MemorySpace, TASK_LINEAR_SIZE},
    segment::{
        self,
        selectors::{self, KERNEL_DS, USER_CS, USER_DS},
    },
    sync::KernelCell,
    syscall::SyscallContext,
    task::{self, task_struct::*},
};

pub const TASK_NUM: usize = 64;

unsafe extern "C" {
    static pg_dir: u8;
}

/// Statically allocated memory for task 0 (idle process).
///
/// Located in kernel memory below LOW_MEM (2MB in current layout), so the frame allocator
/// won't try to free it when the Task is dropped.
static mut INIT_TASK_PAGE: MaybeUninit<TaskPage> = MaybeUninit::uninit();

pub struct TaskManager {
    pub tasks: [Option<Arc<Task>>; TASK_NUM],
    pub last_pid: AtomicU32,
}

impl TaskManager {
    pub fn fork(&mut self, ctx: &SyscallContext) -> Result<u32, ()> {
        // 1. Find a free slot in task array.
        let (slot, pid) = self.find_empty_process().ok_or(())?;

        // 2. Allocate a new task page.
        let mut new_task = Task::new().ok_or(())?;

        // 3. Snapshot parent state and create child memory space by COW.
        let parent_slot = task::current_slot();
        let parent = self.tasks[parent_slot]
            .as_ref()
            .expect("fork: current task missing");
        let parent_pid = parent.pcb.pid;
        let (
            parent_priority,
            parent_pgrp,
            parent_session,
            parent_identity,
            parent_tty,
            parent_file_system,
            parent_ldt,
            parent_signal_info,
            child_memory_space,
        ) = parent.pcb.inner.exclusive(|parent_inner| {
            let old_base = parent_inner.ldt.data_segment().base();
            let code_base = parent_inner.ldt.code_segment().base();
            assert_eq!(old_base, code_base, "separate I&D not supported");

            let code_limit = parent_inner.ldt.code_segment().byte_limit();
            let data_limit = parent_inner.ldt.data_segment().byte_limit();
            assert!(
                data_limit >= code_limit,
                "bad data_limit: data < code (0x{:x} < 0x{:x})",
                data_limit,
                code_limit
            );

            let child_memory_space = parent_inner
                .memory_space
                .as_ref()
                .expect("parent memory space is none, unexpected error")
                .cow_copy(slot, data_limit)
                .map_err(|_| ())?;

            Ok::<_, ()>((
                parent_inner.sched.priority,
                parent_inner.relation.pgrp,
                parent_inner.relation.session,
                parent_inner.identity,
                parent_inner.tty,
                parent_inner.fs.clone(),
                parent_inner.ldt.clone(),
                parent_inner.signal_info.clone(),
                child_memory_space,
            ))
        })?;

        // 4. Initialize PCB fields.
        new_task.pcb = TaskControlBlock::new(slot, pid, TaskControlBlockInner {
            sched: TaskSchedInfo {
                state: TaskState::Uninterruptible,
                counter: parent_priority,
                priority: parent_priority,
            },
            relation: TaskRelationInfo {
                father: parent_pid,
                pgrp: parent_pgrp,
                session: parent_session,
                leader: 0,
            },
            identity: parent_identity,
            acct: TaskAcctInfo {
                utime: 0,
                stime: 0,
                cutime: 0,
                cstime: 0,
            },
            memory_space: None, // empty, set below after LDT base is adjusted
            exit_code: 0,
            tty: parent_tty,
            fs: parent_file_system,
            ldt: parent_ldt,
            tss: TaskStateSegment {
                back_link: 0,
                esp0: new_task.stack_top(),
                ss0: KERNEL_DS.as_u32(),
                esp1: 0,
                ss1: 0,
                esp2: 0,
                ss2: 0,
                cr3: unsafe { &pg_dir as *const u8 as u32 },
                eip: ctx.eip,
                eflags: ctx.eflags,
                eax: 0, // child returns 0 from fork
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
                ldt: selectors::ldt_selector(slot as u16).as_u32(),
                trace_bitmap: 0x8000_0000,
                i387: I387Struct::empty(),
            },
            signal_info: TaskSignalInfo {
                signal: 0,
                blocked: parent_signal_info.blocked,
                sigaction: parent_signal_info.sigaction.clone(),
                alarm: 0,
            },
        });

        // 5. Set child LDT base, install COW memory, and prepare descriptor addresses.
        let new_base = slot as u32 * TASK_LINEAR_SIZE;
        let (tss_addr, ldt_addr) = new_task.pcb.inner.exclusive(|child_inner| {
            child_inner.ldt.set_base(new_base);
            child_inner.memory_space = Some(child_memory_space);
            child_inner.sched.state = TaskState::Running;
            (
                &child_inner.tss as *const TaskStateSegment as u32,
                child_inner.ldt.as_ptr(),
            )
        });

        // 6. Install TSS and LDT descriptors in GDT for the new task.
        segment::set_tss_desc(slot as u16, tss_addr);
        segment::set_ldt_desc(slot as u16, ldt_addr);

        // 7. Insert into task table as runnable.
        self.tasks[slot] = Some(Arc::new(new_task));

        Ok(pid)
    }

    /// Select the best runnable task.
    ///
    /// Returns:
    /// - `Some(next)` if caller should perform a hardware switch.
    /// - `None` if current task remains unchanged.
    pub(super) fn select_next_task(&self) -> Option<Arc<Task>> {
        let current_slot = task::current_slot();
        loop {
            // Pick a runnable non-idle task with the largest counter.
            // For equal counters, prefer the higher slot index.
            let candidate = self
                .tasks
                .iter()
                .enumerate()
                .skip(1)
                .filter_map(|(idx, task)| {
                    let task = task.as_ref()?;
                    task.pcb.inner.exclusive(|inner| {
                        (inner.sched.state == TaskState::Running)
                            .then_some((idx, inner.sched.counter))
                    })
                })
                .max_by_key(|&(idx, counter)| (counter, idx));

            match candidate {
                Some((next, counter)) if counter > 0 => {
                    if current_slot != next {
                        return Some(
                            self.tasks[next]
                                .as_ref()
                                .expect("select_next_task: candidate task missing")
                                .clone(),
                        );
                    }
                    return None;
                }
                None => {
                    if current_slot != 0 {
                        return Some(
                            self.tasks[0]
                                .as_ref()
                                .expect("select_next_task: task0 missing")
                                .clone(),
                        );
                    }
                    return None;
                }
                Some(_) => {
                    self.tasks.iter().skip(1).flatten().for_each(|task| {
                        task.pcb.inner.exclusive(|inner| {
                            inner.sched.counter = (inner.sched.counter >> 1) + inner.sched.priority;
                        });
                    });
                }
            }
        }
    }

    /// Find an unused PID and an empty slot in the task table.
    ///
    /// Increments `last_pid` (wrapping to 1 on overflow) until a PID is
    /// found that no existing task uses, then scans `tasks[1..]` for the
    /// first empty slot.
    ///
    /// Returns `(slot, pid)` on success.
    /// Returns `None` if no empty slot is available.
    fn find_empty_process(&self) -> Option<(usize, u32)> {
        // Step 1: find a unique PID not used by any existing task.
        let pid = 'retry: loop {
            let previous = self
                .last_pid
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
                    Some(last.wrapping_add(1).max(1))
                })
                .expect("fetch_update with unconditional update should never fail");
            let next_pid = previous.wrapping_add(1).max(1);

            for task in self.tasks.iter().flatten() {
                if task.pcb.pid == next_pid {
                    continue 'retry;
                }
            }
            break next_pid;
        };

        // Step 2: find an empty slot in tasks[1..].
        (1..TASK_NUM)
            .find(|&i| self.tasks[i].is_none())
            .map(|slot| (slot, pid))
    }
}

lazy_static! {
    pub static ref TASK_MANAGER: KernelCell<TaskManager> = unsafe {
        // Initialize the static memory for task 0.
        let init_task_ptr = addr_of_mut!(INIT_TASK_PAGE).cast::<TaskPage>();
        let init_task_addr = init_task_ptr as u32;
        let task_page_size_u32 = TASK_PAGE_SIZE as u32;

        // Zero the whole task page.
        write_bytes(init_task_ptr.cast::<u8>(), 0, TASK_PAGE_SIZE);

        // Then initialize only the PCB.
        addr_of_mut!((*init_task_ptr).pcb).write(TaskControlBlock::new(
            0,
            0, // pid = 0
            TaskControlBlockInner {
                sched: TaskSchedInfo {
                    state: TaskState::Running,
                    counter: 15,
                    priority: 15,
                },
                relation: TaskRelationInfo {
                    father: u32::MAX,
                    pgrp: 0,
                    session: 0,
                    leader: 0,
                },
                identity: TaskIdentityInfo {
                    uid: 0,
                    euid: 0,
                    suid: 0,
                    gid: 0,
                    egid: 0,
                    sgid: 0,
                },
                acct: TaskAcctInfo {
                    utime: 0,
                    stime: 0,
                    cutime: 0,
                    cstime: 0,
                },
                memory_space: Some(MemorySpace::new(0)), // task 0
                exit_code: 0,
                tty: -1,
                fs: TaskFileSystemContext {
                    umask: 0,
                    root_directory: None,
                    current_directory: None,
                    executable_inode: None,
                    close_on_exec: 0,
                    open_files: array::from_fn(|_| None),
                },
                ldt: LocalDescriptorTable::new(0, 0x9f),
                signal_info: TaskSignalInfo {
                    signal: 0,
                    blocked: 0,
                    sigaction: array::from_fn(|_| SigAction {
                        sa_handler: 0,
                        sa_mask: 0,
                        sa_flags: 0,
                        sa_restorer: 0,
                    }),
                    alarm: 0,
                },
                tss: TaskStateSegment {
                    back_link: 0,
                    esp0: init_task_addr + task_page_size_u32,
                    ss0: KERNEL_DS.as_u32(),
                    esp1: 0,
                    ss1: 0,
                    esp2: 0,
                    ss2: 0,
                    cr3: &pg_dir as *const u8 as u32,
                    eip: 0,
                    eflags: 0,
                    eax: 0,
                    ecx: 0,
                    edx: 0,
                    ebx: 0,
                    esp: 0,
                    ebp: 0,
                    esi: 0,
                    edi: 0,
                    es: USER_DS.as_u32(),
                    cs: USER_CS.as_u32(),
                    ss: USER_DS.as_u32(),
                    ds: USER_DS.as_u32(),
                    fs: USER_DS.as_u32(),
                    gs: USER_DS.as_u32(),
                    ldt: selectors::ldt_selector(0).as_u32(),
                    trace_bitmap: 0x8000_0000,
                    i387: I387Struct::empty(),
                },
            },
        ));

        // Create Task from the static address.
        let task0 = Task::from_static_addr(init_task_addr);

        // Initialize task array with task 0.
        let mut tasks: [Option<Arc<Task>>; TASK_NUM] = [const { None }; TASK_NUM];
        tasks[0] = Some(Arc::new(task0));

        KernelCell::new(TaskManager {
            tasks,
            last_pid: AtomicU32::new(0),
        })
    };
}
