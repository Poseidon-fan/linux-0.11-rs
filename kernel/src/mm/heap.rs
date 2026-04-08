//! Kernel heap allocator.
//!
//! This module provides dynamic memory allocation for the kernel using a buddy system allocator.
//!
//! # Memory Layout
//!
//! ```text
//!   0x00000000 ┌───────────────────────────┐
//!              │ Kernel image + static data│
//!   0x00090000 ├───────────────────────────┤
//!              │ Boot params scratch area  │
//!   0x000A0000 ├───────────────────────────┤
//!              │ VGA / ROM (reserved)      │
//!   0x00100000 ├───────────────────────────┤ ← HEAP_START
//!              │ Kernel heap               │   (1 MB)
//!   0x00200000 ├───────────────────────────┤ ← HEAP_END / LOW_MEM
//!              │ Frame-managed memory ...  │
//!              └───────────────────────────┘
//! ```

use buddy_system_allocator::LockedHeap;

/// Start address of the kernel heap (inclusive).
pub const HEAP_START: usize = 0x100000;
/// End address of the kernel heap (exclusive).
pub const HEAP_END: usize = 0x200000;
/// Size of the kernel heap in bytes (1 MB).
pub const HEAP_SIZE: usize = HEAP_END - HEAP_START;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::empty();

#[alloc_error_handler]
fn alloc_error_handler(layout: core::alloc::Layout) -> ! {
    panic!("heap allocation error: {:?}", layout);
}

/// Initializes the kernel heap allocator.
pub fn init() {
    unsafe {
        HEAP_ALLOCATOR.lock().init(HEAP_START, HEAP_SIZE);
    }
}
