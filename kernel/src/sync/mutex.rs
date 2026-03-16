//! Blocking mutex built on top of [`KernelCell`] and [`WaitQueue`].
//!
//! The mutex metadata is protected by short IRQ-masked critical sections,
//! while contended callers sleep on a wait queue until another task unlocks.

use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    sync::{self, EFLAGS_IF, KernelCell, read_eflags_and_cli},
    task::{self, wait_queue::WaitQueue},
};

/// Internal metadata for one blocking mutex.
struct MutexState {
    /// Whether the mutex is currently held by some task.
    locked: bool,
    /// Task-table slot of the current owner.
    owner_slot: Option<usize>,
}

/// A task-blocking mutex for single-core kernel code.
///
/// # Design
///
/// - Lock metadata is serialized by [`KernelCell`].
/// - Contended callers sleep on [`WaitQueue`] instead of spinning.
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
    state: KernelCell<MutexState>,
    value: UnsafeCell<T>,
    wait_queue: WaitQueue,
}

// SAFETY: `Mutex` serializes mutable access to `value`. Sharing the
// mutex between tasks is sound when `T` can be transferred across task
// boundaries.
unsafe impl<T: Send> Sync for Mutex<T> {}

/// RAII guard returned by [`Mutex::lock`].
#[must_use = "holding the guard keeps the mutex locked; dropping it unlocks"]
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
    _marker: PhantomData<&'a mut T>,
}

impl<T> Mutex<T> {
    /// Create one unlocked mutex that protects `value`.
    pub const fn new(value: T) -> Self {
        Self {
            state: KernelCell::new(MutexState {
                locked: false,
                owner_slot: None,
            }),
            value: UnsafeCell::new(value),
            wait_queue: WaitQueue::new(),
        }
    }

    /// Acquire the mutex, sleeping until it becomes available.
    ///
    /// # Panics
    ///
    /// Panics if called while already inside `KernelCell::exclusive`, because
    /// contended acquisition may sleep and reschedule.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        assert!(
            sync::current_irq_depth() == 0,
            "Mutex::lock must not run inside KernelCell::exclusive"
        );

        let current_slot = task::current_slot();
        let saved_if_enabled = (read_eflags_and_cli() & EFLAGS_IF) != 0;

        // Keep interrupts masked across the check-sleep loop so unlock cannot
        // race between "saw locked" and "queued ourselves to sleep".
        loop {
            let acquired = unsafe {
                self.state.exclusive_unchecked(|state| {
                    if state.owner_slot == Some(current_slot) {
                        panic!("Mutex::lock recursive acquisition");
                    }

                    if state.locked {
                        false
                    } else {
                        state.locked = true;
                        state.owner_slot = Some(current_slot);
                        true
                    }
                })
            };

            if acquired {
                if saved_if_enabled {
                    sync::sti();
                } else {
                    sync::cli();
                }
                return MutexGuard {
                    mutex: self,
                    _marker: PhantomData,
                };
            }

            WaitQueue::sleep_on(&self.wait_queue);
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
        let acquired = if sync::current_irq_depth() == 0 {
            self.state.exclusive(|state| {
                if state.owner_slot == Some(current_slot) {
                    panic!("Mutex::try_lock recursive acquisition");
                }

                if state.locked {
                    false
                } else {
                    state.locked = true;
                    state.owner_slot = Some(current_slot);
                    true
                }
            })
        } else {
            unsafe {
                self.state.exclusive_unchecked(|state| {
                    if state.owner_slot == Some(current_slot) {
                        panic!("Mutex::try_lock recursive acquisition");
                    }

                    if state.locked {
                        false
                    } else {
                        state.locked = true;
                        state.owner_slot = Some(current_slot);
                        true
                    }
                })
            }
        };

        acquired.then_some(MutexGuard {
            mutex: self,
            _marker: PhantomData,
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
        if sync::current_irq_depth() == 0 {
            self.state.exclusive(|state| state.locked)
        } else {
            unsafe { self.state.exclusive_unchecked(|state| state.locked) }
        }
    }

    /// Release the mutex and wake one waiting task.
    fn unlock(&self) {
        let current_slot = task::current_slot();
        self.state.exclusive(|state| {
            assert!(state.locked, "Mutex::unlock unlocked mutex");
            assert_eq!(
                state.owner_slot,
                Some(current_slot),
                "Mutex::unlock by non-owner"
            );
            state.locked = false;
            state.owner_slot = None;
        });
        WaitQueue::wake_up(&self.wait_queue);
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
