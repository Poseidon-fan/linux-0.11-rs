//! Single-slot wait queue primitives.
//!
//! The queue stores one waiting task slot in `slot`.
//! Sleep operations replace this slot with the current task and perform
//! handoff wakeups after rescheduling.
//!
//! Synchronization contract:
//! - These APIs do not disable interrupts internally.
//! - Callers are responsible for protecting call sites where interrupt
//!   re-entry could contend on `TASK_MANAGER`.

use super::{
    TASK_MANAGER, switch_to,
    task_struct::{TaskState, TaskState::Running},
};

/// Single-slot wait queue.
pub struct WaitQueue {
    slot: Option<usize>,
}

impl WaitQueue {
    /// Create an empty wait queue.
    pub const fn new() -> Self {
        Self { slot: None }
    }

    /// Put current task into uninterruptible sleep.
    pub fn sleep_on(queue: &mut WaitQueue) {
        let (next_slot, handoff_slot) = TASK_MANAGER.with_mut(|manager| {
            let current_slot = manager.current;
            assert_ne!(current_slot, 0, "task[0] trying to sleep");

            // `queue.slot` is a single shared entry.
            // Replacing it returns the previous waiter, which we keep in a
            // stack-local `handoff_slot` and wake after we run again.
            // This stack-local handoff is what forms the wakeup chain.
            let handoff_slot = queue.slot.replace(current_slot);
            if let Some(task) = manager.tasks.get(current_slot).and_then(Option::as_ref) {
                task.pcb.inner.borrow_mut().sched.state = TaskState::Uninterruptible;
            }

            (manager.schedule(), handoff_slot)
        });

        if let Some(next) = next_slot {
            switch_to(next);
        }

        // Resume the previous waiter captured before we slept.
        // Each sleeper wakes the one that used to be in the queue slot.
        if let Some(slot) = handoff_slot {
            Self::wake_slot(slot);
        }
    }

    /// Put current task into interruptible sleep.
    ///
    /// If another task replaces this queue slot while we are sleeping,
    /// wake that task and retry until the slot settles.
    pub fn interruptible_sleep_on(queue: &mut WaitQueue) {
        let (current_slot, handoff_slot, mut next_slot) = TASK_MANAGER.with_mut(|manager| {
            let current_slot = manager.current;
            assert_ne!(current_slot, 0, "task[0] trying to sleep");

            // Same handoff capture as `sleep_on`, but this path may retry.
            let handoff_slot = queue.slot.replace(current_slot);
            if let Some(task) = manager.tasks.get(current_slot).and_then(Option::as_ref) {
                task.pcb.inner.borrow_mut().sched.state = TaskState::Interruptible;
            }

            (current_slot, handoff_slot, manager.schedule())
        });

        loop {
            if let Some(next) = next_slot {
                switch_to(next);
            }

            match queue.slot {
                Some(slot) if slot != current_slot => {
                    // Another task replaced our queue slot while we slept.
                    // Wake that task first, then mark ourselves interruptible
                    // again and schedule once more until our slot settles.
                    next_slot = TASK_MANAGER.with_mut(|manager| {
                        if let Some(task) = manager.tasks.get(slot).and_then(Option::as_ref) {
                            task.pcb.inner.borrow_mut().sched.state = Running;
                        }
                        if let Some(task) = manager.tasks.get(current_slot).and_then(Option::as_ref)
                        {
                            task.pcb.inner.borrow_mut().sched.state = TaskState::Interruptible;
                        }
                        manager.schedule()
                    });
                }
                _ => {
                    queue.slot = None;
                    break;
                }
            }
        }

        if let Some(slot) = handoff_slot {
            Self::wake_slot(slot);
        }
    }

    /// Wake one waiter, if present.
    pub fn wake_up(queue: &mut WaitQueue) {
        if let Some(slot) = queue.slot.take() {
            Self::wake_slot(slot);
        }
    }

    /// Wake one task by slot index.
    fn wake_slot(slot: usize) {
        TASK_MANAGER.with_mut(|manager| {
            if let Some(task) = manager.tasks.get(slot).and_then(Option::as_ref) {
                task.pcb.inner.borrow_mut().sched.state = Running;
            }
        });
    }
}
