//! Single-slot wait queue primitives.
//!
//! The queue stores one waiting task in `slot` as a weak reference.
//! Sleep operations replace this slot with the current task and perform
//! handoff wakeups after rescheduling.
//!
//! Synchronization contract:
//! - Task state changes use `pcb.inner.exclusive` for IRQ exclusion.
//! - Queue slot mutation is protected internally by `KernelCell`.

use alloc::sync::{Arc, Weak};

use crate::{
    sync::{KernelCell, assert_can_schedule},
    task,
};

use super::task_struct::{Task, TaskState, TaskState::Running};

/// Single-slot wait queue.
pub struct WaitQueue {
    slot: KernelCell<Option<Weak<Task>>>,
}

impl WaitQueue {
    /// Create an empty wait queue.
    pub const fn new() -> Self {
        Self {
            slot: KernelCell::new(None),
        }
    }

    /// Put current task into uninterruptible sleep.
    pub fn sleep_on(wait_queue: &WaitQueue) {
        assert_can_schedule("WaitQueue::sleep_on");
        let current = task::current_task();
        assert_ne!(current.pcb.slot, 0, "task[0] trying to sleep");

        let handoff_slot = wait_queue
            .slot
            .exclusive(|slot| slot.replace(Arc::downgrade(&current)));
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
    pub fn interruptible_sleep_on(wait_queue: &WaitQueue) {
        assert_can_schedule("WaitQueue::interruptible_sleep_on");
        let current = task::current_task();
        let current_slot = current.pcb.slot;
        assert_ne!(current_slot, 0, "task[0] trying to sleep");

        let handoff_slot = wait_queue
            .slot
            .exclusive(|slot| slot.replace(Arc::downgrade(&current)));
        current
            .pcb
            .inner
            .exclusive(|inner| inner.sched.state = TaskState::Interruptible);

        loop {
            task::schedule();

            let replaced =
                wait_queue
                    .slot
                    .exclusive(|slot| match slot.as_ref().and_then(Weak::upgrade) {
                        Some(task) if task.pcb.slot != current_slot => Some(task),
                        _ => {
                            *slot = None;
                            None
                        }
                    });

            let Some(task) = replaced else {
                break;
            };

            // Another task replaced our queue slot while we slept.
            // Wake that task first, then mark ourselves interruptible
            // again and schedule once more until our slot settles.
            current.pcb.inner.exclusive(|current_inner| {
                current_inner.sched.state = TaskState::Interruptible;
            });
            task.pcb
                .inner
                .exclusive(|task_inner| task_inner.sched.state = Running);
        }

        if let Some(task) = handoff_slot {
            Self::wake_task(task);
        }
    }

    /// Wake one waiter, if present.
    pub fn wake_up(wait_queue: &WaitQueue) {
        if let Some(task) = wait_queue.slot.exclusive(|slot| slot.take()) {
            Self::wake_task(task);
        }
    }

    /// Return whether the queue currently has a live waiter.
    pub fn has_waiter(&self) -> bool {
        self.slot
            .exclusive(|slot| slot.as_ref().and_then(Weak::upgrade).is_some())
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
