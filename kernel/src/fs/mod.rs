//! Filesystem subsystem.

pub mod buffer;
mod layout;

/// Filesystem logical block size in bytes.
pub const BLOCK_SIZE: usize = 1024;
