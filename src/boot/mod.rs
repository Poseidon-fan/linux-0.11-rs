//! Boot-time initialization structures.
//!
//! This module contains data structures used during kernel boot,
//! referenced by head.s before Rust code takes over.

use core::ptr::addr_of_mut;

use crate::mm::PAGE_SIZE;

/// Initial kernel stack used by head.s during boot.
///
/// Sized as PAGE_SIZE / 4 = 1024 entries of u32 (one full page).
static mut USER_STACK: [u32; (PAGE_SIZE >> 2) as usize] = [0; (PAGE_SIZE >> 2) as usize];

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
    sp: unsafe {
        addr_of_mut!(USER_STACK)
            .cast::<u32>()
            .add((PAGE_SIZE >> 2) as usize)
    },
    ss: 0x10,
};
