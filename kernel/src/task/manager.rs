//! Global task table and runnable-task selection.

use alloc::sync::Arc;
use core::{
    mem::MaybeUninit,
    ptr::{addr_of_mut, write_bytes},
    sync::atomic::{AtomicU32, Ordering},
};

use lazy_static::lazy_static;
use user_lib::syscall::signal::Signal;

use super::task_struct::{TASK_PAGE_SIZE, Task, TaskControlBlock, TaskPage, TaskState};
use crate::sync::KernelCell;

lazy_static! {
    /// Global task table and PID allocator for the whole kernel.
    pub static ref TASK_MANAGER: KernelCell<TaskManager> = unsafe {
        // Initialize the static memory for task 0.
        let init_task_ptr = addr_of_mut!(INIT_TASK_PAGE).cast::<TaskPage>();
        let init_task_addr = init_task_ptr as u32;

        // Zero the whole task page.
        write_bytes(init_task_ptr.cast::<u8>(), 0, TASK_PAGE_SIZE);

        // Then initialize only the PCB.
        addr_of_mut!((*init_task_ptr).pcb).write(TaskControlBlock::new_kernel(
            init_task_addr,
            &pg_dir as *const u8 as u32,
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

/// Number of tasks in the task table.
pub const TASK_NUM: usize = 64;

/// The global task table.
pub struct TaskManager {
    /// Per-slot task entries; `tasks[0]` is the idle process.
    pub tasks: [Option<Arc<Task>>; TASK_NUM],
    /// Last allocated PID, used as the starting point for the next search.
    last_pid: AtomicU32,
}

unsafe extern "C" {
    /// Page directory for the kernel, defined in `head.s`.
    static pg_dir: u8;
}

/// Statically allocated memory for task 0 (idle process).
///
/// Located in kernel memory below LOW_MEM (2MB in current layout), so the frame allocator
/// won't try to free it when the Task is dropped.
static mut INIT_TASK_PAGE: MaybeUninit<TaskPage> = MaybeUninit::uninit();

impl TaskManager {
    /// Select the best runnable task.
    ///
    /// Returns:
    /// - `Some(next)` if caller should perform a hardware switch.
    /// - `None` if current task remains unchanged.
    pub fn select_next_task(&self) -> Option<Arc<Task>> {
        // Signal-aware pre-scan: deliver expired SIGALRM and wake
        // interruptible tasks that have a pending unblocked signal.
        // Mirrors the original Linux 0.11 schedule() pre-scan.
        let j = super::jiffies();
        let unblockable = (1u32 << (Signal::Kill as u32 - 1)) | (1u32 << (Signal::Stop as u32 - 1));
        for task in self.tasks.iter().flatten() {
            task.pcb.inner.exclusive(|inner| {
                if inner.signal_info.alarm != 0 && inner.signal_info.alarm < j {
                    inner.signal_info.raise(Signal::Alrm as u32);
                    inner.signal_info.alarm = 0;
                }
                let pending_unblocked =
                    inner.signal_info.signal & (unblockable | !inner.signal_info.blocked);
                if inner.sched.state == TaskState::Interruptible && pending_unblocked != 0 {
                    inner.sched.state = TaskState::Running;
                }
            });
        }

        let current_slot = super::current_slot();
        loop {
            // Pick a runnable non-idle task with the largest counter.
            // For equal counters, prefer the higher slot index.
            let candidate = self
                .tasks
                .iter()
                .enumerate()
                .skip(1)
                .filter_map(|(slot, task)| {
                    let task = task.as_ref()?;
                    task.pcb.inner.exclusive(|inner| {
                        (inner.sched.state == TaskState::Running)
                            .then_some((slot, inner.sched.counter))
                    })
                })
                .max_by_key(|&(slot, counter)| (counter, slot));

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
    pub fn alloc_process(&self) -> Option<(usize, u32)> {
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
