//! Blocking mutex built on top of [`BusyLock`] and [`KernelCell`].
//!
//! The underlying sleep/wakeup mechanics live in [`BusyLock`], while this
//! layer adds owner tracking and RAII access to a protected value.

use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    sync::{BusyLock, KernelCell, cell::assert_can_schedule},
    task,
};

/// A task-blocking mutex for single-core kernel code.
///
/// # Design
///
/// - The sleepable lock state lives in [`BusyLock`].
/// - Owner tracking is serialized by [`KernelCell`].
/// - The protected value itself lives in [`UnsafeCell`] and is only accessed
///   while the mutex is held.
///
/// # Constraints
///
/// - This mutex is intended for task context after `task::init()`.
/// - Acquisition may sleep, so callers must not hold a `KernelCell::exclusive`
///   critical section when calling [`lock`](Self::lock).
/// - Interrupt handlers must not acquire this mutex.
pub struct Mutex<T> {
    lock: BusyLock,
    owner_slot: KernelCell<Option<usize>>,
    value: UnsafeCell<T>,
}

// SAFETY: `Mutex` serializes mutable access to `value`. Sharing the
// mutex between tasks is sound when `T` can be transferred across task
// boundaries.
unsafe impl<T: Send> Sync for Mutex<T> {}
// SAFETY: The mutex contains no task-local resources; ownership of `T`
// may move with the mutex as long as `T: Send`.
unsafe impl<T: Send> Send for Mutex<T> {}

/// RAII guard returned by [`Mutex::lock`].
#[must_use = "holding the guard keeps the mutex locked; dropping it unlocks"]
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
    _marker: PhantomData<&'a mut T>,
    _not_send: PhantomData<*mut ()>,
}

impl<T> Mutex<T> {
    /// Create one unlocked mutex that protects `value`.
    pub const fn new(value: T) -> Self {
        Self {
            lock: BusyLock::new(),
            owner_slot: KernelCell::new(None),
            value: UnsafeCell::new(value),
        }
    }

    /// Acquire the mutex, sleeping until it becomes available.
    ///
    /// # Panics
    ///
    /// Panics if called while already inside `KernelCell::exclusive`, because
    /// contended acquisition may sleep and reschedule.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        assert_can_schedule("Mutex::lock");

        let current_slot = task::current_slot();
        self.owner_slot.exclusive(|owner_slot| {
            if *owner_slot == Some(current_slot) {
                panic!("Mutex::lock recursive acquisition");
            }
        });
        self.lock.acquire();
        self.owner_slot.exclusive(|owner_slot| {
            assert!(owner_slot.is_none(), "Mutex::lock owner slot already set");
            *owner_slot = Some(current_slot);
        });
        MutexGuard {
            mutex: self,
            _marker: PhantomData,
            _not_send: PhantomData,
        }
    }

    /// Try to acquire the mutex without sleeping.
    ///
    /// Returns `None` if another task currently holds the mutex.
    ///
    /// # Panics
    ///
    /// Panics on recursive acquisition by the same task.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        let current_slot = task::current_slot();
        let owner_slot = self.owner_slot.exclusive(|owner_slot| *owner_slot);
        if owner_slot == Some(current_slot) {
            panic!("Mutex::try_lock recursive acquisition");
        }
        if !self.lock.try_acquire() {
            return None;
        }
        self.owner_slot.exclusive(|owner_slot| {
            assert!(
                owner_slot.is_none(),
                "Mutex::try_lock owner slot already set"
            );
            *owner_slot = Some(current_slot);
        });
        Some(MutexGuard {
            mutex: self,
            _marker: PhantomData,
            _not_send: PhantomData,
        })
    }

    /// Execute `f` while holding the mutex.
    ///
    /// This convenience wrapper matches the closure-heavy style already used
    /// throughout the kernel and keeps the guard lifetime short by default.
    pub fn with_lock<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut guard = self.lock();
        f(&mut *guard)
    }

    /// Return whether the mutex is currently held by any task.
    pub fn is_locked(&self) -> bool {
        self.lock.is_locked()
    }

    /// Release the mutex and wake one waiting task.
    fn unlock(&self) {
        let current_slot = task::current_slot();
        self.owner_slot.exclusive(|owner_slot| {
            assert_eq!(
                *owner_slot,
                Some(current_slot),
                "Mutex::unlock by non-owner"
            );
            *owner_slot = None;
        });
        self.lock.release();
    }
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The guard represents exclusive ownership of the mutex, so no
        // other mutable or shared access to `value` may exist.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: Same argument as `Deref`; the guard is the unique accessor.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}
