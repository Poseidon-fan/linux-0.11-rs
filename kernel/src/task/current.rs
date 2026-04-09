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

use super::task_struct::Task;

/// Raw pointer to the current task object.
///
/// # Safety
///
/// The pointer must always come from `Arc::as_ptr` and stay valid while
/// running code can call [`current_task`].
static CURRENT_TASK: AtomicPtr<Task> = AtomicPtr::new(null_mut());
/// Bit 7 stores the IF value captured at the outermost IRQ-masked entry.
const IRQ_SAVED_IF_BIT: u8 = 1 << 7;
/// Low 7 bits store nested IRQ-masked depth.
const IRQ_DEPTH_MASK: u8 = 0x7f;

/// Return the current task as a strong `Arc`.
///
/// # Panics
///
/// Panics if called before `task::init()` initializes current-task tracking.
pub fn current_task() -> Arc<Task> {
    try_current_task().expect("current_task called before task::init initialized current task")
}

/// Return the current task as a strong `Arc` when task tracking is initialized.
pub fn try_current_task() -> Option<Arc<Task>> {
    let ptr = CURRENT_TASK.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }

    Some(unsafe {
        // SAFETY:
        // - `ptr` comes from `Arc::as_ptr` in init/set path.
        // - The task table keeps a long-lived strong reference.
        Arc::increment_strong_count(ptr.cast_const());
        Arc::from_raw(ptr.cast_const())
    })
}

/// Return the current task's slot index.
///
/// Reads the raw pointer directly to avoid Arc refcount overhead.
#[inline]
pub fn current_slot() -> usize {
    let ptr = CURRENT_TASK.load(Ordering::Acquire);
    assert!(!ptr.is_null(), "current_slot called before task::init");
    unsafe { (*ptr).pcb.slot }
}

/// Return the current task's process ID.
///
/// Reads the raw pointer directly to avoid Arc refcount overhead.
#[inline]
pub fn current_pid() -> u32 {
    let ptr = CURRENT_TASK.load(Ordering::Acquire);
    assert!(!ptr.is_null(), "current_pid called before task::init");
    unsafe { (*ptr).pcb.pid }
}

/// Return the current task's slot index when task tracking is initialized.
#[inline]
pub fn try_current_slot() -> Option<usize> {
    let ptr = CURRENT_TASK.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }

    Some(unsafe {
        // SAFETY:
        // - `ptr` is the stable `Arc::as_ptr` stored in `CURRENT_TASK`.
        // - The currently running task stays alive while it is scheduled.
        (*ptr).pcb.slot
    })
}

/// Store `task` as the current-task pointer.
///
/// Used both at boot (task 0 init) and before each hardware task switch.
pub fn set_current_task(task: &Arc<Task>) {
    let ptr = Arc::as_ptr(task).cast_mut();
    CURRENT_TASK.store(ptr, Ordering::Release);
}

/// Return the packed IRQ state of the currently running task.
pub fn current_irq_state() -> (bool, u8) {
    let ptr = CURRENT_TASK.load(Ordering::Acquire);
    let packed = unsafe {
        // SAFETY:
        // - `ptr` is the same stable `Arc::as_ptr` published in `CURRENT_TASK`.
        // - The current task remains alive while it is scheduled.
        // - `irq_state` lives outside `KernelCell`, so accessing it here does
        //   not recurse back into synchronization primitives.
        (*ptr).pcb.irq_state.load(Ordering::Relaxed)
    };
    ((packed & IRQ_SAVED_IF_BIT) != 0, packed & IRQ_DEPTH_MASK)
}

/// Update selected fields of the current task's packed IRQ state.
///
/// Passing `None` keeps the previous value for that field.
pub fn set_current_irq_state(saved_if_enabled: Option<bool>, depth: Option<u8>) {
    let ptr = CURRENT_TASK.load(Ordering::Acquire);
    unsafe {
        // SAFETY:
        // - `ptr` is the same stable `Arc::as_ptr` published in `CURRENT_TASK`.
        // - The current task remains alive while it is scheduled.
        // - `irq_state` lives outside `KernelCell`, so accessing it here does
        //   not recurse back into synchronization primitives.
        let old_packed = (*ptr).pcb.irq_state.load(Ordering::Relaxed);
        let next_saved_if_enabled =
            saved_if_enabled.unwrap_or((old_packed & IRQ_SAVED_IF_BIT) != 0);
        let next_depth = depth.unwrap_or(old_packed & IRQ_DEPTH_MASK) & IRQ_DEPTH_MASK;
        let packed = ((next_saved_if_enabled as u8) << 7) | next_depth;
        (*ptr).pcb.irq_state.store(packed, Ordering::Relaxed);
    }
}
