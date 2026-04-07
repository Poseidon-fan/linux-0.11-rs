//! Kernel synchronization primitives.
//!
//! The kernel runs on a single core but is preemptible and supports nested
//! interrupts, so shared state requires explicit protection.
//!
//! - [`KernelCell`] — IRQ-masked interior mutability for `static` data.
//! - [`BusyLock`] — ownerless sleepable busy-bit lock.
//! - [`Mutex`] — owner-tracked sleeping mutex with deadlock detection.

mod busy_lock;
mod cell;
mod irq;
mod mutex;

pub use busy_lock::BusyLock;
pub use cell::{KernelCell, assert_can_schedule};
pub use irq::IrqSaveGuard;
pub use mutex::Mutex;
