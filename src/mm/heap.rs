//! Heap allocator for the kernel.

use buddy_system_allocator::LockedHeap;

// Kernel claims 0 ~ 1M memory space, starting from address 0x0.
// However, 0xA0000 - 0xFFFFF (640KB - 1MB) is reserved for VGA/ROM,
// and some info data is stored on 0x90000(although it'll be useless after initialization).
// We place the kernel heap at the top of 0x90000, in case of any conflicts.
const HEAP_END: usize = 0x90000;
// 128KB for kernel heap.
const HEAP_SIZE: usize = 128 * 1024;

unsafe extern "C" {
    // This symbol is defined in linker.ld, pointing to the end address of the kernel.
    fn ekernel();
}

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::empty();

#[alloc_error_handler]
fn alloc_error_handler(layout: core::alloc::Layout) -> ! {
    panic!("heap allocation error: {:?}", layout);
}

pub(super) fn init() {
    // Ensure the kernel is not overlapping with the heap.
    assert!((ekernel as usize) < HEAP_END - HEAP_SIZE);

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
}
