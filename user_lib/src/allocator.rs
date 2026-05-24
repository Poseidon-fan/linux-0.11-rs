//! User-space heap allocator backed by the `brk` system call.
//!
//! The allocator implements Rust's [`GlobalAlloc`] interface. It asks the
//! kernel to extend the process break in page-sized chunks, then serves
//! allocations from an address-ordered free list.

use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    mem::{align_of, size_of},
    ptr,
};

use crate::{println, syscall::process};

/// Heap growth granularity. The kernel maps anonymous pages lazily on fault.
const PAGE_SIZE: usize = 4096;

/// Header stored immediately before each allocation's returned pointer.
#[repr(C)]
struct AllocationHeader {
    /// Start address of the whole allocated block, including front padding.
    block_start: usize,
    /// Size of the whole allocated block in bytes.
    block_size: usize,
}

/// Header stored at the start of each free block.
#[repr(C)]
struct FreeBlock {
    /// Size of this free block in bytes, including this header.
    size: usize,
    /// Next free block in ascending address order.
    next: *mut FreeBlock,
}

/// A single-core interior-mutable wrapper for the process heap.
struct LockedAllocator {
    inner: UnsafeCell<BrkAllocator>,
}

unsafe impl Sync for LockedAllocator {}

impl LockedAllocator {
    /// Creates an empty allocator that will lazily query `brk` on first use.
    const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(BrkAllocator::new()),
        }
    }

    /// Runs `f` with exclusive access to the heap state.
    fn with<R>(&self, f: impl FnOnce(&mut BrkAllocator) -> R) -> R {
        // SAFETY: user programs are currently single-threaded. The allocator is
        // only re-entered by normal control flow, and allocation routines avoid
        // allocating while they hold this mutable reference.
        f(unsafe { &mut *self.inner.get() })
    }
}

unsafe impl GlobalAlloc for LockedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.with(|allocator| allocator.alloc(layout))
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.with(|allocator| allocator.dealloc(ptr, layout));
    }
}

/// Process heap allocator state.
struct BrkAllocator {
    free_list: *mut FreeBlock,
    heap_end: usize,
    initialized: bool,
}

impl BrkAllocator {
    /// Creates an allocator with no known heap range.
    const fn new() -> Self {
        Self {
            free_list: ptr::null_mut(),
            heap_end: 0,
            initialized: false,
        }
    }

    /// Allocates memory for `layout`, growing the heap if necessary.
    fn alloc(&mut self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return layout.align() as *mut u8;
        }

        loop {
            if let Some(ptr) = self.try_alloc(layout) {
                return ptr;
            }
            if !self.grow(layout) {
                return ptr::null_mut();
            }
        }
    }

    /// Returns one allocation to the free list.
    fn dealloc(&mut self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }

        let header_addr = ptr as usize - size_of::<AllocationHeader>();
        let header = header_addr as *const AllocationHeader;
        // SAFETY: `alloc` writes this header immediately before every pointer
        // it returns, and Rust only calls `dealloc` with such pointers.
        let block_start = unsafe { (*header).block_start };
        let block_size = unsafe { (*header).block_size };
        self.insert_free_block(block_start, block_size);
    }

    /// Attempts to allocate from the current free list.
    fn try_alloc(&mut self, layout: Layout) -> Option<*mut u8> {
        let mut prev = ptr::null_mut();
        let mut current = self.free_list;

        while !current.is_null() {
            let placement = unsafe { Placement::new(current as usize, (*current).size, layout) };
            if let Some(placement) = placement {
                return Some(unsafe { self.take_from_block(prev, current, placement) });
            }

            prev = current;
            current = unsafe { (*current).next };
        }

        None
    }

    /// Removes `placement` from `current` and returns the payload pointer.
    unsafe fn take_from_block(
        &mut self,
        prev: *mut FreeBlock,
        current: *mut FreeBlock,
        placement: Placement,
    ) -> *mut u8 {
        let next = unsafe { (*current).next };

        if placement.prefix_size >= min_free_block_size() {
            unsafe {
                (*current).size = placement.prefix_size;
                (*current).next = next;
            }
            if placement.suffix_size >= min_free_block_size() {
                let suffix = placement.suffix_start as *mut FreeBlock;
                unsafe {
                    (*suffix).size = placement.suffix_size;
                    (*suffix).next = next;
                    (*current).next = suffix;
                }
            }
        } else if placement.suffix_size >= min_free_block_size() {
            let suffix = placement.suffix_start as *mut FreeBlock;
            unsafe {
                (*suffix).size = placement.suffix_size;
                (*suffix).next = next;
            }
            self.replace_block(prev, current, suffix);
        } else {
            self.remove_block(prev, current);
        }

        let header = placement.header_start as *mut AllocationHeader;
        unsafe {
            (*header).block_start = placement.block_start;
            (*header).block_size = placement.block_size;
        }

        placement.payload_start as *mut u8
    }

    /// Extends the process break and inserts the new range into the free list.
    fn grow(&mut self, layout: Layout) -> bool {
        let current_break = if self.initialized {
            self.heap_end
        } else {
            match process::brk(0) {
                Ok(addr) => addr as usize,
                Err(_) => return false,
            }
        };

        let needed = layout
            .size()
            .saturating_add(size_of::<AllocationHeader>())
            .saturating_add(layout.align())
            .saturating_add(min_free_block_size());
        let grow_size = align_up(needed.max(PAGE_SIZE), PAGE_SIZE);
        let Some(new_break) = current_break.checked_add(grow_size) else {
            return false;
        };

        let Ok(actual_break) = process::brk(new_break as u32) else {
            return false;
        };
        let actual_break = actual_break as usize;
        if actual_break < new_break {
            return false;
        }

        self.initialized = true;
        self.heap_end = actual_break;

        let start = align_up(current_break, align_of::<FreeBlock>());
        if actual_break <= start {
            return false;
        }

        let size = actual_break - start;
        if size < min_free_block_size() {
            return false;
        }

        self.insert_free_block(start, size);
        true
    }

    /// Inserts a free block and coalesces adjacent neighbors.
    fn insert_free_block(&mut self, start: usize, size: usize) {
        if size < min_free_block_size() {
            return;
        }

        let block = start as *mut FreeBlock;
        unsafe {
            (*block).size = size;
            (*block).next = ptr::null_mut();
        }

        if self.free_list.is_null() || start < self.free_list as usize {
            unsafe {
                (*block).next = self.free_list;
            }
            self.free_list = block;
            self.coalesce_with_next(block);
            return;
        }

        let mut prev = self.free_list;
        unsafe {
            while !(*prev).next.is_null() && ((*prev).next as usize) < start {
                prev = (*prev).next;
            }
            (*block).next = (*prev).next;
            (*prev).next = block;
        }

        self.coalesce_with_next(block);
        self.coalesce_with_next(prev);
    }

    /// Replaces `current` with `replacement` in the linked list.
    fn replace_block(
        &mut self,
        prev: *mut FreeBlock,
        current: *mut FreeBlock,
        replacement: *mut FreeBlock,
    ) {
        if prev.is_null() {
            self.free_list = replacement;
        } else {
            unsafe {
                (*prev).next = replacement;
            }
        }
        let _ = current;
    }

    /// Removes `current` from the linked list.
    fn remove_block(&mut self, prev: *mut FreeBlock, current: *mut FreeBlock) {
        let next = unsafe { (*current).next };
        if prev.is_null() {
            self.free_list = next;
        } else {
            unsafe {
                (*prev).next = next;
            }
        }
    }

    /// Merges `block` with its direct successor when they are adjacent.
    fn coalesce_with_next(&mut self, block: *mut FreeBlock) {
        if block.is_null() {
            return;
        }

        unsafe {
            let next = (*block).next;
            if next.is_null() {
                return;
            }

            let block_end = block as usize + (*block).size;
            if block_end == next as usize {
                (*block).size += (*next).size;
                (*block).next = (*next).next;
            }
        }
    }
}

/// Allocation placement inside a free block.
struct Placement {
    block_start: usize,
    block_size: usize,
    header_start: usize,
    payload_start: usize,
    prefix_size: usize,
    suffix_start: usize,
    suffix_size: usize,
}

impl Placement {
    /// Computes where an allocation can live inside one free block.
    unsafe fn new(block_start: usize, block_size: usize, layout: Layout) -> Option<Self> {
        let block_end = block_start.checked_add(block_size)?;
        let payload_align = layout.align().max(align_of::<AllocationHeader>());
        let payload_start = align_up(
            block_start.checked_add(size_of::<AllocationHeader>())?,
            payload_align,
        );
        let header_start = payload_start.checked_sub(size_of::<AllocationHeader>())?;
        let requested_end = payload_start.checked_add(layout.size())?;

        if requested_end > block_end {
            return None;
        }

        let raw_prefix_size = header_start - block_start;
        let keep_prefix = raw_prefix_size >= min_free_block_size();
        let allocation_start = if keep_prefix {
            header_start
        } else {
            block_start
        };
        let prefix_size = if keep_prefix { raw_prefix_size } else { 0 };

        let raw_suffix_size = block_end - requested_end;
        let keep_suffix = raw_suffix_size >= min_free_block_size();
        let allocation_end = if keep_suffix {
            requested_end
        } else {
            block_end
        };
        let suffix_start = allocation_end;
        let suffix_size = if keep_suffix { raw_suffix_size } else { 0 };

        Some(Self {
            block_start: allocation_start,
            block_size: allocation_end - allocation_start,
            header_start,
            payload_start,
            prefix_size,
            suffix_start,
            suffix_size,
        })
    }
}

/// Returns the smallest free block that can be represented in the free list.
#[inline]
const fn min_free_block_size() -> usize {
    size_of::<FreeBlock>()
}

/// Aligns `addr` upward to `align`, which must be a power of two.
#[inline]
const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

#[global_allocator]
static USER_ALLOCATOR: LockedAllocator = LockedAllocator::new();

/// Handles allocation failure for user programs.
#[alloc_error_handler]
fn alloc_error_handler(layout: Layout) -> ! {
    println!(
        "allocation error: size={} align={}",
        layout.size(),
        layout.align()
    );
    crate::exit(101)
}
