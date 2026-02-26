//! Single-slot wait queue primitives.
//!
//! The queue stores one waiting task in `slot` as a weak reference.
//! Sleep operations replace this slot with the current task and perform
//! handoff wakeups after rescheduling.
//!
//! Synchronization contract:
//! - Task state changes use `pcb.inner.exclusive` for IRQ exclusion.
//! - `queue` itself is protected by the caller through `&mut WaitQueue`.

use alloc::sync::{Arc, Weak};

use crate::task;

use super::task_struct::{Task, TaskState, TaskState::Running};

/// Single-slot wait queue.
pub struct WaitQueue {
    slot: Option<Weak<Task>>,
}

impl WaitQueue {
    /// Create an empty wait queue.
    pub const fn new() -> Self {
        Self { slot: None }
    }

    /// Put current task into uninterruptible sleep.
    pub fn sleep_on(queue: &mut WaitQueue) {
        let current = task::current_task();
        assert_ne!(current.pcb.slot, 0, "task[0] trying to sleep");

        let handoff_slot = queue.slot.replace(Arc::downgrade(&current));
        current
            .pcb
            .inner
            .exclusive(|inner| inner.sched.state = TaskState::Uninterruptible);

        task::schedule();

        // Resume the previous waiter captured before we slept.
        // Each sleeper wakes the one that used to be in the queue slot.
        if let Some(task) = handoff_slot {
            Self::wake_task(task);
        }
    }

    /// Put current task into interruptible sleep.
    ///
    /// If another task replaces this queue slot while we are sleeping,
    /// wake that task and retry until the slot settles.
    pub fn interruptible_sleep_on(queue: &mut WaitQueue) {
        let current = task::current_task();
        let current_slot = current.pcb.slot;
        assert_ne!(current_slot, 0, "task[0] trying to sleep");

        let handoff_slot = queue.slot.replace(Arc::downgrade(&current));
        current
            .pcb
            .inner
            .exclusive(|inner| inner.sched.state = TaskState::Interruptible);

        loop {
            task::schedule();

            match queue.slot.as_ref().and_then(Weak::upgrade) {
                Some(task) if task.pcb.slot != current_slot => {
                    // Another task replaced our queue slot while we slept.
                    // Wake that task first, then mark ourselves interruptible
                    // again and schedule once more until our slot settles.
                    current.pcb.inner.exclusive(|current_inner| {
                        current_inner.sched.state = TaskState::Interruptible;
                        task.pcb
                            .inner
                            .exclusive(|task_inner| task_inner.sched.state = Running);
                    });
                }
                _ => {
                    queue.slot = None;
                    break;
                }
            }
        }

        if let Some(task) = handoff_slot {
            Self::wake_task(task);
        }
    }

    /// Wake one waiter, if present.
    pub fn wake_up(queue: &mut WaitQueue) {
        if let Some(task) = queue.slot.take() {
            Self::wake_task(task);
        }
    }

    /// Wake one task by weak reference.
    fn wake_task(task: Weak<Task>) {
        if let Some(task) = task.upgrade() {
            task.pcb
                .inner
                .exclusive(|inner| inner.sched.state = Running);
        }
    }
}
