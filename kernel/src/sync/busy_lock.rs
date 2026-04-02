use crate::{
    sync::{KernelCell, cell::assert_can_schedule, irq::IrqSaveGuard},
    task::wait_queue::WaitQueue,
};

/// Ownerless sleepable busy-bit lock.
///
/// Models the classic Linux 0.11 lock pattern:
/// a single busy flag guarded by a wait queue. Unlike [`super::Mutex`], one
/// task may acquire and a different task may release.
pub struct BusyLock {
    locked: KernelCell<bool>,
    wait_queue: WaitQueue,
}

impl BusyLock {
    pub const fn new() -> Self {
        Self {
            locked: KernelCell::new(false),
            wait_queue: WaitQueue::new(),
        }
    }

    /// Sleeps until the lock is released, without acquiring it.
    pub fn wait(&self) {
        assert_can_schedule("BusyLock::wait");
        let _irq = IrqSaveGuard::enter();
        unsafe {
            while self.locked.exclusive_unchecked(|l| *l) {
                WaitQueue::sleep_on(&self.wait_queue);
            }
        }
    }

    /// Acquires the lock, sleeping until it becomes available.
    pub fn acquire(&self) {
        assert_can_schedule("BusyLock::acquire");
        let _irq = IrqSaveGuard::enter();
        unsafe {
            while self.locked.exclusive_unchecked(|l| *l) {
                WaitQueue::sleep_on(&self.wait_queue);
            }
            self.locked.exclusive_unchecked(|l| *l = true);
        }
    }

    /// Releases the lock and wakes one waiter.
    pub fn release(&self) {
        self.locked.exclusive(|l| {
            assert!(*l, "BusyLock::release on unlocked lock");
            *l = false;
        });
        WaitQueue::wake_up(&self.wait_queue);
    }

    /// Returns `true` if the lock is currently held.
    pub fn is_locked(&self) -> bool {
        self.locked.exclusive(|l| *l)
    }
}
