//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

#![allow(unused)]
#![cfg_attr(feature = "alloc", feature(alloc_error_handler))]
#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
pub mod allocator;
pub mod console;
#[cfg(feature = "alloc")]
pub mod env;
#[cfg(feature = "runtime")]
pub mod rt;
pub mod syscall;

#[cfg(feature = "runtime")]
pub use user_lib_macros::main;

/// Terminate the current process with the given 8-bit exit status.
///
/// The underlying syscall is exposed as [`syscall::process::exit`] and follows
/// the uniform syscall wrapper convention by returning `Result<u32, Errno>`.
/// This convenience wrapper provides the process-level diverging contract.
#[inline(always)]
pub fn exit(status: u32) -> ! {
    let _ = syscall::process::exit(status);
    loop {
        core::hint::spin_loop();
    }
}
