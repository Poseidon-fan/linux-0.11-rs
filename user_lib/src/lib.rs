//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

#![allow(unused)]
#![cfg_attr(feature = "alloc", feature(alloc_error_handler))]
#![no_std]

extern crate alloc;

#[cfg(feature = "alloc")]
pub mod allocator;
pub mod env;
pub mod ffi;
pub mod fs;
pub mod io;
#[macro_use]
mod macros;
pub mod path;
pub mod process;
#[cfg(feature = "runtime")]
pub mod rt;
pub mod syscall;
pub mod time;

#[cfg(feature = "runtime")]
pub use user_lib_macros::main;
