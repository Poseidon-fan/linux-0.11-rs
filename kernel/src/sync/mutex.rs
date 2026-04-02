use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    sync::{BusyLock, KernelCell, cell::assert_can_schedule},
    task,
};

/// Owner-tracked sleeping mutex.
///
/// Built on [`BusyLock`] for sleeping and [`KernelCell`] for owner tracking.
/// Detects recursive locking and unlock-by-non-owner.
///
/// [`lock`](Self::lock) may sleep, so it must not be called while holding
/// a [`KernelCell`] borrow.
pub struct Mutex<T> {
    lock: BusyLock,
    owner_slot: KernelCell<Option<usize>>,
    value: UnsafeCell<T>,
}

#[must_use = "dropping the guard unlocks the mutex"]
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
    _marker: PhantomData<&'a mut T>,
    _not_send: PhantomData<*mut ()>,
}

// SAFETY: All access to `value` is serialized by the lock.
unsafe impl<T: Send> Sync for Mutex<T> {}
unsafe impl<T: Send> Send for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            lock: BusyLock::new(),
            owner_slot: KernelCell::new(None),
            value: UnsafeCell::new(value),
        }
    }

    /// Acquires the mutex, sleeping until it becomes available.
    ///
    /// # Panics
    ///
    /// Panics on recursive locking or if called inside a [`KernelCell`] borrow.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        assert_can_schedule("Mutex::lock");

        let current_slot = task::current_slot();
        self.owner_slot.exclusive(|slot| {
            assert_ne!(
                *slot,
                Some(current_slot),
                "Mutex::lock recursive acquisition"
            );
        });
        self.lock.acquire();
        self.owner_slot.exclusive(|slot| {
            assert!(slot.is_none(), "Mutex::lock owner slot already set");
            *slot = Some(current_slot);
        });
        MutexGuard {
            mutex: self,
            _marker: PhantomData,
            _not_send: PhantomData,
        }
    }

    fn unlock(&self) {
        let current_slot = task::current_slot();
        self.owner_slot.exclusive(|slot| {
            assert_eq!(*slot, Some(current_slot), "Mutex::unlock by non-owner");
            *slot = None;
        });
        self.lock.release();
    }
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the guard holds exclusive access.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds exclusive access.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}
