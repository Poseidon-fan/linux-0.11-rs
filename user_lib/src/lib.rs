//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

#![allow(unused)]
#![no_std]

pub mod console;
mod syscall;

pub use syscall::*;
