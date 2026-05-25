//! Shared utility code for user-space binaries.
//!
//! Each entry under `src/bin/*.rs` is a standalone program. This library
//! crate collects code that several of them want to reuse — currently just
//! [`cli`], a tiny argument parser modelled after `lexopt` with a
//! struct-defining macro on top.

#![no_std]

extern crate alloc;

pub mod cli;
