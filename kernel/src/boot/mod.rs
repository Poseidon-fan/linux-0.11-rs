//! Boot-time initialization structures.
//!
//! This module contains data structures used during kernel boot,
//! referenced by head.s before Rust code takes over.

use core::ptr::addr_of_mut;

use crate::{mm::frame::PAGE_SIZE, segment::KERNEL_DS};

/// Initial kernel stack used by head.s during boot.
///
/// 16KB instead of 4KB because Rust debug builds use more stack space.
static mut USER_STACK: [u32; BOOT_STACK_WORDS] = [0; BOOT_STACK_WORDS];

/// Exported `stack_start` symbol holding the boot stack pointer and selector.
/// Referenced by `lss stack_start,%esp` in head.s.
#[unsafe(export_name = "stack_start")]
static mut STACK_START: StackStart = StackStart {
    sp: unsafe { addr_of_mut!(USER_STACK).cast::<u32>().add(BOOT_STACK_WORDS) },
    ss: KERNEL_DS.as_u16(),
};

/// Size in 32-bit words of the initial boot stack (16KB).
const BOOT_STACK_WORDS: usize = (PAGE_SIZE >> 2) * 4;

/// Stack pointer and segment selector for initial kernel stack.
/// Referenced by `lss stack_start,%esp` in head.s.
#[repr(C)]
struct StackStart {
    /// Top-of-stack pointer loaded into `ESP`.
    sp: *mut u32,
    /// Kernel data segment selector (0x10).
    ss: u16,
}
