//! Ownerless sleepable busy-bit lock.
//!
//! This primitive mirrors the classic Linux 0.11 pattern used by inode,
//! buffer, and superblock locks:
//! - one busy bit,
//! - one wait queue,
//! - `wait`, `acquire`, and `release` operations.
//!
//! Unlike [`super::Mutex`], a `BusyLock` does not track ownership and allows
//! one context to acquire the lock while another context later releases it.

use crate::task::wait_queue::WaitQueue;

use super::{KernelCell, TaskIrqGuard, assert_can_schedule};

/// Sleepable ownerless busy lock for single-core kernel code.
pub struct BusyLock {
    locked: KernelCell<bool>,
    wait_queue: WaitQueue,
}

impl BusyLock {
    /// Create one unlocked busy lock.
    pub const fn new() -> Self {
        Self {
            locked: KernelCell::new(false),
            wait_queue: WaitQueue::new(),
        }
    }

    /// Wait until the busy bit becomes clear without taking ownership.
    pub fn wait(&self) {
        assert_can_schedule("BusyLock::wait");

        let _irq_guard = TaskIrqGuard::enter();
        unsafe {
            while self.locked.exclusive_unchecked(|locked| *locked) {
                WaitQueue::sleep_on(&self.wait_queue);
            }
        }
    }

    /// Acquire the busy bit, sleeping until it becomes available.
    pub fn acquire(&self) {
        assert_can_schedule("BusyLock::acquire");

        let _irq_guard = TaskIrqGuard::enter();
        unsafe {
            while self.locked.exclusive_unchecked(|locked| *locked) {
                WaitQueue::sleep_on(&self.wait_queue);
            }
            self.locked.exclusive_unchecked(|locked| *locked = true);
        }
    }

    /// Try to acquire the busy bit without sleeping.
    pub fn try_acquire(&self) -> bool {
        self.locked.exclusive(|locked| {
            if *locked {
                false
            } else {
                *locked = true;
                true
            }
        })
    }

    /// Release the busy bit and wake one waiter.
    pub fn release(&self) {
        self.locked.exclusive(|locked| *locked = false);
        WaitQueue::wake_up(&self.wait_queue);
    }

    /// Return whether the busy bit is currently set.
    pub fn is_locked(&self) -> bool {
        self.locked.exclusive(|locked| *locked)
    }
}
