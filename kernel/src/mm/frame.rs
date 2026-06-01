//! Physical frame handles backed by a reference-counted allocator.

use core::ptr;

use super::address::{PhysAddr, PhysPageNum};
use crate::sync::KernelCell;

/// Number of bits in a page offset.
pub const PAGE_SHIFT: u32 = 12;

/// Size of a physical page frame in bytes.
pub const PAGE_SIZE: usize = 1usize << PAGE_SHIFT;

/// Physical addresses below LOW_MEM belong to the kernel / BIOS and are
/// identity-mapped.  They are never tracked by the frame allocator's
/// reference-counting `mem_map`, so operations like `share()` and the
/// `Drop` impl on [`PhysFrame`] silently skip them.
pub const LOW_MEM: u32 = 0x200000;

/// Initialize the physical frame allocator.
///
/// Frames in `[start_mem, end_mem)` become available for allocation; all other
/// tracked frames start out reserved.
///
/// # Panics
///
/// Panics if `start_mem` is below [`LOW_MEM`], if `start_mem > end_mem`, or if
/// `end_mem` exceeds the allocator's supported physical memory range.
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
    /// Physical page number owned by this handle.
    pub ppn: PhysPageNum,
}

/// An owned contiguous run of physical page frames.
///
/// Used by kernel objects that require physically contiguous memory.
pub struct PhysFrameRange {
    /// Physical page number of the first frame in the run.
    pub start_ppn: PhysPageNum,

    /// Number of contiguous frames in the run.
    pub page_count: usize,
}

/// Frame allocator instance.
///
/// Using `static` instead of `lazy_static!` ensures the mem_map array
/// is placed directly in .bss section at compile time, avoiding stack
/// allocation during initialization (which would cause stack overflow
/// in debug builds with the 4KB kernel stack).
static FRAME_ALLOCATOR: KernelCell<FrameAllocator> = KernelCell::new(FrameAllocator {
    mem_map: [0; PAGING_PAGES],
});

const PAGING_MEMORY: u32 = 14 * 1024 * 1024;
const PAGING_PAGES: usize = (PAGING_MEMORY as usize) >> PAGE_SHIFT;
const HIGH_MEMORY: u32 = LOW_MEM + PAGING_MEMORY;
const UNPAGED_PAGES: u32 = LOW_MEM >> PAGE_SHIFT;

struct FrameAllocator {
    mem_map: [u8; PAGING_PAGES],
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
    fn page_addr_from_index(frame_index: usize) -> PhysAddr {
        debug_assert!(frame_index < PAGING_PAGES);
        let offset = (frame_index as u32) << PAGE_SHIFT;
        PhysAddr::from(LOW_MEM + offset)
    }

    #[inline]
    fn index_for_ppn(ppn: PhysPageNum) -> usize {
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
        let start_no = (PhysAddr::from(start_mem).floor().0 - UNPAGED_PAGES) as usize;
        let end_no = (PhysAddr::from(end_mem).floor().0 - UNPAGED_PAGES) as usize;
        self.mem_map[start_no..end_no].fill(0);
    }

    fn alloc(&mut self) -> Option<PhysFrame> {
        let frame_index = self.mem_map.iter().rposition(|&count| count == 0)?;
        let page_addr = Self::page_addr_from_index(frame_index);
        // Safety: tracked frames are part of the kernel's writable physical
        // memory window, and this allocator has exclusive access here.
        unsafe {
            ptr::write_bytes(page_addr.as_mut_ptr::<u8>(), 0, PAGE_SIZE);
        }
        self.mem_map[frame_index] = 1;
        Some(PhysFrame {
            ppn: page_addr.into(),
        })
    }

    fn alloc_contiguous(&mut self, page_count: usize) -> Option<PhysFrameRange> {
        if page_count == 0 {
            return None;
        }

        let start_index = self
            .mem_map
            .windows(page_count)
            .rposition(|run| run.iter().all(|&count| count == 0))?;
        let start_addr = Self::page_addr_from_index(start_index);
        for frame_index in start_index..start_index + page_count {
            let page_addr = Self::page_addr_from_index(frame_index);
            // Safety: tracked frames are part of the kernel's writable physical
            // memory window, and this allocator has exclusive access here.
            unsafe {
                ptr::write_bytes(page_addr.as_mut_ptr::<u8>(), 0, PAGE_SIZE);
            }
            self.mem_map[frame_index] = 1;
        }
        Some(PhysFrameRange {
            start_ppn: start_addr.into(),
            page_count,
        })
    }

    fn dealloc(&mut self, ppn: PhysPageNum) {
        if ppn.0 < UNPAGED_PAGES {
            return;
        }
        let frame_index = Self::index_for_ppn(ppn);
        assert!(
            self.mem_map[frame_index] > 0,
            "Frame {} is not referenced, but dealloc is called",
            ppn.0
        );
        self.mem_map[frame_index] -= 1;
    }

    fn dealloc_range(&mut self, start_ppn: PhysPageNum, page_count: usize) {
        for ppn in start_ppn.0..start_ppn.0 + page_count as u32 {
            self.dealloc(PhysPageNum(ppn));
        }
    }

    fn share(&mut self, ppn: PhysPageNum) -> PhysFrame {
        let frame_index = Self::index_for_ppn(ppn);
        assert!(
            self.mem_map[frame_index] > 0,
            "Sharing a free page (ppn {})",
            ppn.0
        );
        self.mem_map[frame_index] += 1;
        PhysFrame { ppn }
    }

    fn ref_count(&self, ppn: PhysPageNum) -> u8 {
        if ppn.0 < UNPAGED_PAGES {
            return u8::MAX;
        }
        self.mem_map[Self::index_for_ppn(ppn)]
    }
}
