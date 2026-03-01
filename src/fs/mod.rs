//! Filesystem subsystem.

pub mod buffer;

/// Filesystem logical block number (`BLOCK_SIZE` unit).
pub type BlockNr = u32;
/// Filesystem logical block size in bytes.
pub const BLOCK_SIZE: usize = 1024;
