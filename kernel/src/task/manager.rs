use alloc::sync::Arc;
use core::{
    array,
    mem::MaybeUninit,
    ptr::{addr_of_mut, write_bytes},
    sync::atomic::{AtomicU32, Ordering},
};

use lazy_static::lazy_static;

use super::task_struct::*;
use crate::{
    mm::space::MemorySpace,
    segment::{KERNEL_DS, USER_CS, USER_DS},
    sync::KernelCell,
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
    /// Select the best runnable task.
    ///
    /// Returns:
    /// - `Some(next)` if caller should perform a hardware switch.
    /// - `None` if current task remains unchanged.
    pub fn select_next_task(&self) -> Option<Arc<Task>> {
        let current_slot = super::current_slot();
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
    pub fn find_empty_process(&self) -> Option<(usize, u32)> {
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
                mem_layout: TaskMemoryLayout::default(),
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
                    ldt: crate::segment::ldt_selector(0).as_u32(),
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
