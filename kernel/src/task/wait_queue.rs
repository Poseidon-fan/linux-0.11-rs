//! Single-slot wait queue primitives.
//!
//! The queue stores one waiting task in `slot` as a weak reference.
//! Sleep operations replace this slot with the current task and perform
//! handoff wakeups after rescheduling.
//!
//! Synchronization contract:
//! - Task state changes use `with_current` / `pcb.inner.exclusive` for IRQ exclusion.
//! - Queue slot mutation is protected internally by `KernelCell`.

use alloc::sync::{Arc, Weak};

use super::task_struct::{Task, TaskState};
use crate::{
    sync::{KernelCell, assert_can_schedule},
    task,
};

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
    pub fn sleep(&self) {
        assert_can_schedule("WaitQueue::sleep");
        assert_ne!(task::current_slot(), 0, "task[0] trying to sleep");

        let weak_self = Arc::downgrade(&task::current_task());
        let handoff_slot = self.slot.exclusive(|slot| slot.replace(weak_self));
        task::with_current(|inner| inner.sched.state = TaskState::Uninterruptible);

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
    pub fn sleep_interruptible(&self) {
        assert_can_schedule("WaitQueue::sleep_interruptible");
        let current_slot = task::current_slot();
        assert_ne!(current_slot, 0, "task[0] trying to sleep");

        let weak_self = Arc::downgrade(&task::current_task());
        let handoff_slot = self.slot.exclusive(|slot| slot.replace(weak_self));
        task::with_current(|inner| inner.sched.state = TaskState::Interruptible);

        loop {
            task::schedule();

            let replaced =
                self.slot
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
            // Mark ourselves interruptible again and wake the interloper,
            // then schedule once more until our slot settles.
            task::with_current(|inner| inner.sched.state = TaskState::Interruptible);
            task.pcb
                .inner
                .exclusive(|inner| inner.sched.state = TaskState::Running);
        }

        if let Some(task) = handoff_slot {
            Self::wake_task(task);
        }
    }

    /// Wake one waiter, if present.
    pub fn wake(&self) {
        if let Some(task) = self.slot.exclusive(|slot| slot.take()) {
            Self::wake_task(task);
        }
    }

    /// Wake one task by weak reference.
    fn wake_task(task: Weak<Task>) {
        if let Some(task) = task.upgrade() {
            task.pcb
                .inner
                .exclusive(|inner| inner.sched.state = TaskState::Running);
        }
    }
}
