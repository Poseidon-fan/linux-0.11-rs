//! Kernel-wide error types.
//!
//! Re-exports [`Errno`] and all `E*` constants from `user_lib`, and defines
//! the [`Result`] alias used throughout the kernel.

pub use user_lib::syscall::errno::*;

/// Standard kernel [`Result`] with [`Errno`] as the error type.
pub type Result<T> = core::result::Result<T, Errno>;
