//! Boot-time initialization structures.
//!
//! This module contains data structures used during kernel boot,
//! referenced by head.s before Rust code takes over.

use core::ptr::addr_of_mut;

use crate::mm::frame::PAGE_SIZE;

/// Initial kernel stack used by head.s during boot.
///
/// The original Linux 0.11 used only 4KB (one page). Here we use 8KB because
/// Rust debug builds use more stack space, so 4KB can overflow.
const BOOT_STACK_WORDS: usize = (PAGE_SIZE >> 2) as usize * 2;
static mut USER_STACK: [u32; BOOT_STACK_WORDS] = [0; BOOT_STACK_WORDS];

/// Stack pointer and segment selector for initial kernel stack.
/// Referenced by `lss stack_start,%esp` in head.s.
#[repr(C)]
struct StackStart {
    sp: *mut u32,
    /// Kernel data segment selector (0x10).
    ss: u16,
}

#[unsafe(export_name = "stack_start")]
static mut STACK_START: StackStart = StackStart {
    sp: unsafe { addr_of_mut!(USER_STACK).cast::<u32>().add(BOOT_STACK_WORDS) },
    ss: 0x10,
};
