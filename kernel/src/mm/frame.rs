//! Reference-counted physical frame allocator (`mem_map` + free bitmap).

use core::ptr;

use super::address::{PhysAddr, PhysPageNum};
use crate::sync::KernelCell;

pub const PAGE_SHIFT: u32 = 12;
pub const PAGE_SIZE: usize = 1usize << PAGE_SHIFT;

/// Physical addresses below LOW_MEM belong to the kernel / BIOS and are
/// identity-mapped.  They are never tracked by the frame allocator's
/// reference-counting `mem_map`, so operations like `share()` and the
/// `Drop` impl on [`PhysFrame`] silently skip them.
pub const LOW_MEM: u32 = 0x200000;

const PAGING_MEMORY: u32 = 14 * 1024 * 1024;
const PAGING_PAGES: usize = (PAGING_MEMORY as usize) >> PAGE_SHIFT;
const HIGH_MEMORY: u32 = LOW_MEM + PAGING_MEMORY;
const UNPAGED_PAGES: u32 = LOW_MEM >> PAGE_SHIFT;
const BITMAP_WORD_BITS: usize = u32::BITS as usize;
const FREE_BITMAP_WORDS: usize = PAGING_PAGES.div_ceil(BITMAP_WORD_BITS);

pub fn init(start_mem: u32, end_mem: u32) {
    // Safety: frame allocator bootstrap runs before task::init in a single
    // boot flow, so no re-entrant access can contend here.
    unsafe {
        FRAME_ALLOCATOR.exclusive_unchecked(|a| a.init(start_mem, end_mem));
    }
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
    mem_map: [u8; PAGING_PAGES],
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
    pub fn phys_addr(&self) -> PhysAddr {
        self.start_ppn.addr()
    }
}

impl FrameAllocator {
    #[inline]
    fn page_addr_from_idx(idx: usize) -> PhysAddr {
        debug_assert!(idx < PAGING_PAGES);
        let idx_u32 = idx as u32;
        PhysAddr::from(LOW_MEM + (idx_u32 << PAGE_SHIFT))
    }

    #[inline]
    fn idx_for_ppn(ppn: PhysPageNum) -> usize {
        (ppn.0 - UNPAGED_PAGES) as usize
    }

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
        let page_addr = Self::page_addr_from_idx(idx);
        unsafe { ptr::write_bytes(page_addr.as_mut_ptr::<u8>(), 0, PAGE_SIZE) };
        self.mem_map[idx] = 1;
        self.mark_used(idx);
        Some(PhysFrame {
            ppn: page_addr.into(),
        })
    }

    fn alloc_contiguous(&mut self, page_count: usize) -> Option<PhysFrameRange> {
        if page_count == 0 || page_count > PAGING_PAGES {
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
            let page_addr = Self::page_addr_from_idx(idx);
            unsafe { ptr::write_bytes(page_addr.as_mut_ptr::<u8>(), 0, PAGE_SIZE) };
            self.mem_map[idx] = 1;
            self.mark_used(idx);
        }

        let start_addr = Self::page_addr_from_idx(start_idx);
        Some(PhysFrameRange {
            start_ppn: start_addr.into(),
            page_count,
        })
    }

    fn dealloc(&mut self, ppn: PhysPageNum) {
        if ppn.0 < UNPAGED_PAGES {
            return;
        }
        let idx = Self::idx_for_ppn(ppn);
        assert!(
            self.mem_map[idx] > 0,
            "Frame {} is not referenced, but dealloc is called",
            ppn.0
        );
        self.mem_map[idx] -= 1;
        if self.mem_map[idx] == 0 {
            self.mark_free(idx);
        }
    }

    fn dealloc_range(&mut self, start_ppn: PhysPageNum, page_count: usize) {
        for i in 0..page_count {
            let delta = i as u32;
            self.dealloc(PhysPageNum(start_ppn.0 + delta));
        }
    }

    /// Increment the reference count for an existing page and return a
    /// new [`PhysFrame`] handle to the same physical page.
    fn share(&mut self, ppn: PhysPageNum) -> PhysFrame {
        let idx = Self::idx_for_ppn(ppn);
        assert!(self.mem_map[idx] > 0, "Sharing a free page (ppn {})", ppn.0);
        self.mem_map[idx] += 1;
        PhysFrame { ppn }
    }

    /// Get a page's current reference count from `mem_map`.
    fn ref_count(&self, ppn: PhysPageNum) -> u8 {
        if ppn.0 < UNPAGED_PAGES {
            return u8::MAX;
        }
        self.mem_map[Self::idx_for_ppn(ppn)]
    }

    #[inline]
    fn mark_free(&mut self, idx: usize) {
        debug_assert!(idx < PAGING_PAGES);
        let word = idx / BITMAP_WORD_BITS;
        let bit = idx % BITMAP_WORD_BITS;
        self.free_bitmap[word] |= 1u32 << bit;
    }

    #[inline]
    fn mark_used(&mut self, idx: usize) {
        debug_assert!(idx < PAGING_PAGES);
        let word = idx / BITMAP_WORD_BITS;
        let bit = idx % BITMAP_WORD_BITS;
        self.free_bitmap[word] &= !(1u32 << bit);
    }

    #[inline]
    fn is_free(&self, idx: usize) -> bool {
        debug_assert!(idx < PAGING_PAGES);
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
        (0..PAGING_PAGES)
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
            let valid_bits = PAGING_PAGES - word_idx * BITMAP_WORD_BITS;
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
    mem_map: [0; PAGING_PAGES],
    free_bitmap: [0; FREE_BITMAP_WORDS],
});
