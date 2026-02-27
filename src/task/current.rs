//! Current task tracking.
//!
//! This module stores the currently running task as a raw pointer to the
//! inner `Task` managed by `Arc<Task>`. The owner strong reference lives in
//! `TaskManager::tasks`; callers get temporary strong references through
//! [`current_task`].

use alloc::sync::Arc;
use core::{
    ptr::null_mut,
    sync::atomic::{AtomicPtr, Ordering},
};

use crate::sync::TaskIrqGuard;

use super::task_struct::Task;

/// Raw pointer to the current task object.
///
/// # Safety
///
/// The pointer must always come from `Arc::as_ptr` and stay valid while
/// running code can call [`current_task`].
static CURRENT_TASK: AtomicPtr<Task> = AtomicPtr::new(null_mut());

/// Return the current task as a strong `Arc`.
///
/// # Panics
///
/// Panics if called before `task::init()` initializes current-task tracking.
pub fn current_task() -> Arc<Task> {
    let _guard = TaskIrqGuard::enter();
    let ptr = CURRENT_TASK.load(Ordering::Acquire);
    assert!(
        !ptr.is_null(),
        "current_task called before task::init initialized current task",
    );

    unsafe {
        // SAFETY:
        // - `ptr` comes from `Arc::as_ptr` in init/set path.
        // - The task table keeps a long-lived strong reference.
        Arc::increment_strong_count(ptr.cast_const());
        Arc::from_raw(ptr.cast_const())
    }
}

/// Return the current task's slot index.
#[inline]
pub fn current_slot() -> usize {
    current_task().pcb.slot
}

/// Initialize current-task tracking with task 0 during boot.
pub(crate) fn init_current_task(task: &Arc<Task>) {
    let ptr = Arc::as_ptr(task).cast_mut();
    CURRENT_TASK.store(ptr, Ordering::Release);
}

/// Update current-task pointer before hardware task switch.
pub(crate) fn set_current_task(task: &Arc<Task>) {
    let ptr = Arc::as_ptr(task).cast_mut();
    CURRENT_TASK.store(ptr, Ordering::Release);
}
