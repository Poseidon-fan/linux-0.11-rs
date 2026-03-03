use core::ptr;

use crate::{
    mm::address::{PhysAddr, PhysPageNum},
    println,
    sync::KernelCell,
};

pub const PAGE_SIZE: u32 = 4096;

/// Physical addresses below LOW_MEM belong to the kernel / BIOS and are
/// identity-mapped.  They are never tracked by the frame allocator's
/// reference-counting `mem_map`, so operations like `share()` and the
/// `Drop` impl on [`PhysFrame`] silently skip them.
pub const LOW_MEM: u32 = 0x200000;

const PAGING_MEMORY: u32 = 14 * 1024 * 1024;
const PAGING_PAGES: u32 = PAGING_MEMORY >> 12;
const HIGH_MEMORY: u32 = LOW_MEM + PAGING_MEMORY;
const UNPAGED_PAGES: u32 = LOW_MEM >> 12;
const BITMAP_WORD_BITS: usize = u32::BITS as usize;
const FREE_BITMAP_WORDS: usize = (PAGING_PAGES as usize).div_ceil(BITMAP_WORD_BITS);

pub fn init(start_mem: u32, end_mem: u32) {
    // Safety: frame allocator bootstrap runs before task::init in a single
    // boot flow, so no re-entrant access can contend here.
    unsafe {
        FRAME_ALLOCATOR.exclusive_unchecked(|a| a.init(start_mem, end_mem));
    }

    #[cfg(debug_assertions)]
    frame_test();
}

/// Allocate a fresh physical page frame (zeroed).
///
/// Returns `None` if no free frames remain.  The page's reference count
/// in `mem_map` is set to 1.
pub fn alloc() -> Option<PhysFrame> {
    FRAME_ALLOCATOR.exclusive(|allocator| allocator.alloc())
}

/// Allocate a contiguous run of fresh physical page frames (zeroed).
///
/// Returns `None` if no free run of `page_count` contiguous frames exists.
/// Every page in the returned run has reference count 1.
pub fn alloc_contiguous(page_count: usize) -> Option<PhysFrameRange> {
    FRAME_ALLOCATOR.exclusive(|allocator| allocator.alloc_contiguous(page_count))
}

/// Create a shared reference to an existing physical page frame.
///
/// Increments the reference count in `mem_map` and returns a new
/// [`PhysFrame`] handle pointing to the same physical page.  When both
/// the original and the shared handle are dropped, each `Drop`
/// decrements the count, so the page is freed only when the last
/// reference is gone.
///
/// # Panics
///
/// Panics if `ppn` refers to a page that is not currently allocated
/// (i.e. `mem_map` entry is 0).
pub fn share(ppn: PhysPageNum) -> PhysFrame {
    FRAME_ALLOCATOR.exclusive(|allocator| allocator.share(ppn))
}

/// Return the current reference count of a physical page frame.
///
/// For frames below [`LOW_MEM`], the allocator does not track reference
/// counts, so this function returns `u8::MAX` as a stable sentinel.
pub fn ref_count(ppn: PhysPageNum) -> u8 {
    FRAME_ALLOCATOR.exclusive(|allocator| allocator.ref_count(ppn))
}

/// An owned handle to a physical page frame.
///
/// Represents one reference-counted ownership stake in a physical page.
/// Dropping a `PhysFrame` decrements the page's reference count in
/// `mem_map`; the underlying memory is only truly freed when the count
/// reaches zero.  Frames below [`LOW_MEM`] are never freed (they belong
/// to the kernel's identity-mapped region).
pub struct PhysFrame {
    pub ppn: PhysPageNum,
}

/// An owned contiguous run of physical page frames.
///
/// This type is useful for kernel objects that require physically contiguous
/// memory, could be seen as batch version of [`PhysFrame`].
pub struct PhysFrameRange {
    pub start_ppn: PhysPageNum,
    pub page_count: usize,
}

struct FrameAllocator {
    mem_map: [u8; PAGING_PAGES as usize],
    /// Bitset cache for free pages (1 = free, 0 = used/refcounted).
    ///
    /// This auxiliary structure avoids scanning the whole `mem_map` for every
    /// allocation and also speeds up contiguous-run checks.
    free_bitmap: [u32; FREE_BITMAP_WORDS],
}

impl Drop for PhysFrame {
    fn drop(&mut self) {
        FRAME_ALLOCATOR.exclusive(|allocator| allocator.dealloc(self.ppn));
    }
}

impl Drop for PhysFrameRange {
    fn drop(&mut self) {
        FRAME_ALLOCATOR
            .exclusive(|allocator| allocator.dealloc_range(self.start_ppn, self.page_count));
    }
}

impl PhysFrameRange {
    /// Physical address of the first page in this run.
    pub fn phys_addr(&self) -> u32 {
        self.start_ppn.0 << 12
    }
}

impl FrameAllocator {
    fn init(&mut self, start_mem: u32, end_mem: u32) {
        const USED: u8 = 100;
        assert!(
            start_mem >= LOW_MEM,
            "frame init start_mem {:#x} is below LOW_MEM {:#x}",
            start_mem,
            LOW_MEM
        );
        assert!(
            start_mem <= end_mem,
            "frame init start_mem {:#x} must be <= end_mem {:#x}",
            start_mem,
            end_mem
        );
        assert!(
            end_mem <= HIGH_MEMORY,
            "frame init end_mem {:#x} exceeds high memory limit {:#x}",
            end_mem,
            HIGH_MEMORY
        );

        self.mem_map.fill(USED);
        self.free_bitmap.fill(0);
        let start_no = (PhysAddr::from(start_mem).floor().0 - UNPAGED_PAGES) as usize;
        let end_no = (PhysAddr::from(end_mem).floor().0 - UNPAGED_PAGES) as usize;
        self.mem_map[start_no..end_no].fill(0);
        for idx in start_no..end_no {
            self.mark_free(idx);
        }
    }

    fn alloc(&mut self) -> Option<PhysFrame> {
        let idx = self.find_free_page_from_high()?;
        let page_addr = PhysAddr::from(LOW_MEM + ((idx as u32) << 12));
        unsafe { ptr::write_bytes(page_addr.0 as *mut u8, 0, PAGE_SIZE as usize) };
        self.mem_map[idx] = 1;
        self.mark_used(idx);
        Some(PhysFrame {
            ppn: page_addr.into(),
        })
    }

    fn alloc_contiguous(&mut self, page_count: usize) -> Option<PhysFrameRange> {
        if page_count == 0 || page_count > PAGING_PAGES as usize {
            return None;
        }

        if page_count == 1 {
            let frame = self.alloc()?;
            return Some(PhysFrameRange {
                start_ppn: frame.ppn,
                page_count: 1,
            });
        }

        let start_idx = self.find_free_run_from_high(page_count)?;
        for idx in start_idx..start_idx + page_count {
            let page_addr = PhysAddr::from(LOW_MEM + ((idx as u32) << 12));
            unsafe { ptr::write_bytes(page_addr.0 as *mut u8, 0, PAGE_SIZE as usize) };
            self.mem_map[idx] = 1;
            self.mark_used(idx);
        }

        let start_addr = PhysAddr::from(LOW_MEM + ((start_idx as u32) << 12));
        Some(PhysFrameRange {
            start_ppn: start_addr.into(),
            page_count,
        })
    }

    fn dealloc(&mut self, ppn: PhysPageNum) {
        if ppn.0 < UNPAGED_PAGES {
            return;
        }
        assert!(
            self.mem_map[(ppn.0 - UNPAGED_PAGES) as usize] > 0,
            "Frame {} is not referenced, but dealloc is called",
            ppn.0
        );
        let idx = (ppn.0 - UNPAGED_PAGES) as usize;
        self.mem_map[idx] -= 1;
        if self.mem_map[idx] == 0 {
            self.mark_free(idx);
        }
    }

    fn dealloc_range(&mut self, start_ppn: PhysPageNum, page_count: usize) {
        for i in 0..page_count {
            self.dealloc(PhysPageNum(start_ppn.0 + i as u32));
        }
    }

    /// Increment the reference count for an existing page and return a
    /// new [`PhysFrame`] handle to the same physical page.
    fn share(&mut self, ppn: PhysPageNum) -> PhysFrame {
        let idx = (ppn.0 - UNPAGED_PAGES) as usize;
        assert!(self.mem_map[idx] > 0, "Sharing a free page (ppn {})", ppn.0);
        self.mem_map[idx] += 1;
        PhysFrame { ppn }
    }

    /// Get a page's current reference count from `mem_map`.
    fn ref_count(&self, ppn: PhysPageNum) -> u8 {
        if ppn.0 < UNPAGED_PAGES {
            return u8::MAX;
        }
        self.mem_map[(ppn.0 - UNPAGED_PAGES) as usize]
    }

    #[inline]
    fn mark_free(&mut self, idx: usize) {
        debug_assert!(idx < PAGING_PAGES as usize);
        let word = idx / BITMAP_WORD_BITS;
        let bit = idx % BITMAP_WORD_BITS;
        self.free_bitmap[word] |= 1u32 << bit;
    }

    #[inline]
    fn mark_used(&mut self, idx: usize) {
        debug_assert!(idx < PAGING_PAGES as usize);
        let word = idx / BITMAP_WORD_BITS;
        let bit = idx % BITMAP_WORD_BITS;
        self.free_bitmap[word] &= !(1u32 << bit);
    }

    #[inline]
    fn is_free(&self, idx: usize) -> bool {
        debug_assert!(idx < PAGING_PAGES as usize);
        let word = idx / BITMAP_WORD_BITS;
        let bit = idx % BITMAP_WORD_BITS;
        (self.free_bitmap[word] >> bit) & 1 == 1
    }

    fn find_free_page_from_high(&self) -> Option<usize> {
        (0..FREE_BITMAP_WORDS).rev().find_map(|word_idx| {
            let word = self.masked_free_word(word_idx);
            (word != 0).then(|| {
                let bit = BITMAP_WORD_BITS - 1 - word.leading_zeros() as usize;
                word_idx * BITMAP_WORD_BITS + bit
            })
        })
    }

    fn find_free_run_from_high(&self, page_count: usize) -> Option<usize> {
        (0..PAGING_PAGES as usize)
            .rev()
            .scan(0usize, |run_len, idx| {
                *run_len = if self.is_free(idx) { *run_len + 1 } else { 0 };
                Some((idx, *run_len))
            })
            .find_map(|(idx, run_len)| (run_len == page_count).then_some(idx))
    }

    /// Return a free-bitmap word with out-of-range tail bits masked off.
    #[inline]
    fn masked_free_word(&self, word_idx: usize) -> u32 {
        let mut word = self.free_bitmap[word_idx];
        if word_idx == FREE_BITMAP_WORDS - 1 {
            let valid_bits = PAGING_PAGES as usize - word_idx * BITMAP_WORD_BITS;
            if valid_bits < BITMAP_WORD_BITS {
                let mask = (1u32 << valid_bits) - 1;
                word &= mask;
            }
        }
        word
    }
}

/// Frame allocator instance.
///
/// Using `static` instead of `lazy_static!` ensures the mem_map array
/// is placed directly in .bss section at compile time, avoiding stack
/// allocation during initialization (which would cause stack overflow
/// in debug builds with the 4KB kernel stack).
static FRAME_ALLOCATOR: KernelCell<FrameAllocator> = KernelCell::new(FrameAllocator {
    mem_map: [0; PAGING_PAGES as usize],
    free_bitmap: [0; FREE_BITMAP_WORDS],
});

#[allow(unused)]
pub fn frame_test() {
    use core::mem::ManuallyDrop;

    // Safety: debug self-test is called from `init()` before task::init.
    // This path must use unchecked access and explicit cleanup.
    unsafe {
        FRAME_ALLOCATOR.exclusive_unchecked(|allocator| {
            // Test 1: Allocate a frame.
            let frame1 = ManuallyDrop::new(allocator.alloc().expect("Failed to allocate frame 1"));
            let ppn1 = frame1.ppn.0;

            // Test 2: Allocate another frame, should have lower ppn (high-to-low allocation).
            let frame2 = ManuallyDrop::new(allocator.alloc().expect("Failed to allocate frame 2"));
            let ppn2 = frame2.ppn.0;
            assert!(ppn2 < ppn1, "Frame 2 should have lower ppn than frame 1");

            // Test 3: Free frame1 manually, then allocate again and expect reuse.
            allocator.dealloc(frame1.ppn);
            let frame3 = ManuallyDrop::new(allocator.alloc().expect("Failed to allocate frame 3"));
            let ppn3 = frame3.ppn.0;
            assert_eq!(ppn3, ppn1, "Frame 3 should reuse ppn of dropped frame 1");

            // Test 4: Allocate and free 2 contiguous frames.
            let run = ManuallyDrop::new(
                allocator
                    .alloc_contiguous(2)
                    .expect("Failed to allocate run"),
            );
            assert_eq!(run.page_count, 2, "Run length should be 2 pages");
            allocator.dealloc_range(run.start_ppn, run.page_count);

            // Test 5: Allocate contiguous frames again (allocator should still work).
            let run2 = ManuallyDrop::new(
                allocator
                    .alloc_contiguous(2)
                    .expect("Failed to re-allocate 2 contiguous frames"),
            );
            assert_eq!(
                run2.page_count, 2,
                "Re-allocated run length should be 2 pages"
            );

            // Cleanup for handles wrapped in `ManuallyDrop`.
            allocator.dealloc(frame2.ppn);
            allocator.dealloc(frame3.ppn);
            allocator.dealloc_range(run2.start_ppn, run2.page_count);
        });
    }

    println!("[frame_test] Frame allocator test passed!");
}
