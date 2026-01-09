//! Kernel heap allocator.
//!
//! This module provides dynamic memory allocation for the kernel using a buddy system allocator.
//!
//! # Memory Layout
//!
//! ```text
//!   0x00000 ┌─────────────────┐
//!           │  Kernel Code    │
//!           │  & Data         │
//!   ekernel ├─────────────────┤
//!           │                 │
//!   0x70000 ├─────────────────┤ ← HEAP_END - HEAP_SIZE
//!           │  Kernel Heap    │   (128 KB)
//!   0x90000 ├─────────────────┤ ← HEAP_END
//!           │  BIOS Info      │
//!   0xA0000 ├─────────────────┤
//!           │  VGA / ROM      │   (Reserved, not usable)
//!   0xFFFFF └─────────────────┘
//! ```

use buddy_system_allocator::LockedHeap;

use crate::println;

/// End address of the kernel heap (exclusive).
///
/// Kernel claims 0 ~ 1M memory space, starting from address 0x0.
/// However, 0xA0000 - 0xFFFFF (640KB - 1MB) is reserved for VGA/ROM,
/// and some info data is stored at 0x90000 (useless after initialization).
/// We place the kernel heap right below 0x90000 to avoid conflicts.
const HEAP_END: usize = 0x90000;

/// Size of the kernel heap in bytes (128 KB).
const HEAP_SIZE: usize = 128 * 1024;

unsafe extern "C" {
    /// Linker-defined symbol marking the end of the kernel image.
    fn ekernel();
}

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::empty();

#[alloc_error_handler]
fn alloc_error_handler(layout: core::alloc::Layout) -> ! {
    panic!("heap allocation error: {:?}", layout);
}

/// Initializes the kernel heap allocator.
///
/// # Panics
/// Panics if the kernel image overlaps with the heap region.
pub(super) fn init() {
    assert!(
        (ekernel as usize) < HEAP_END - HEAP_SIZE,
        "kernel overlaps with heap"
    );

    unsafe {
        HEAP_ALLOCATOR.lock().init(HEAP_END - HEAP_SIZE, HEAP_SIZE);
    }

    #[cfg(debug_assertions)]
    heap_test();
}

#[allow(unused)]
fn heap_test() {
    use alloc::vec;
    let mut tmp = vec![0, 1, 2, 3, 4];
    tmp.push(5);
    for (i, &item) in tmp.iter().enumerate() {
        assert_eq!(item, i as u8);
    }
    println!("[heap_test] passed");
}
