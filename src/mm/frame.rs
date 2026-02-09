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
pub const LOW_MEM: u32 = 0x100000;

const PAGING_MEMORY: u32 = 15 * 1024 * 1024;
const PAGING_PAGES: u32 = PAGING_MEMORY >> 12;
const UNPAGED_PAGES: u32 = LOW_MEM >> 12;

pub fn init(start_mem: u32, end_mem: u32) {
    FRAME_ALLOCATOR.with_mut(|a| a.init(start_mem, end_mem));

    #[cfg(debug_assertions)]
    frame_test();
}

/// Allocate a fresh physical page frame (zeroed).
///
/// Returns `None` if no free frames remain.  The page's reference count
/// in `mem_map` is set to 1.
pub fn alloc() -> Option<PhysFrame> {
    FRAME_ALLOCATOR.with_mut(|allocator| allocator.alloc())
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
    FRAME_ALLOCATOR.with_mut(|allocator| allocator.share(ppn))
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

struct FrameAllocator {
    mem_map: [u8; PAGING_PAGES as usize],
}

impl Drop for PhysFrame {
    fn drop(&mut self) {
        FRAME_ALLOCATOR.with_mut(|allocator| allocator.dealloc(self.ppn));
    }
}

impl FrameAllocator {
    fn init(&mut self, start_mem: u32, end_mem: u32) {
        const USED: u8 = 100;
        self.mem_map.fill(USED);
        let start_no = (PhysAddr::from(start_mem).floor().0 - UNPAGED_PAGES) as usize;
        let end_no = (PhysAddr::from(end_mem).floor().0 - UNPAGED_PAGES) as usize;
        self.mem_map[start_no..end_no].fill(0);
    }

    fn alloc(&mut self) -> Option<PhysFrame> {
        self.mem_map.iter().rposition(|&x| x == 0).map(|i| {
            let page_addr = PhysAddr::from(LOW_MEM + ((i as u32) << 12));
            unsafe { ptr::write_bytes(page_addr.0 as *mut u8, 0, PAGE_SIZE as usize) };
            self.mem_map[i] = 1;
            PhysFrame {
                ppn: page_addr.into(),
            }
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
        self.mem_map[(ppn.0 - UNPAGED_PAGES) as usize] -= 1;
    }

    /// Increment the reference count for an existing page and return a
    /// new [`PhysFrame`] handle to the same physical page.
    fn share(&mut self, ppn: PhysPageNum) -> PhysFrame {
        let idx = (ppn.0 - UNPAGED_PAGES) as usize;
        assert!(self.mem_map[idx] > 0, "Sharing a free page (ppn {})", ppn.0);
        self.mem_map[idx] += 1;
        PhysFrame { ppn }
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
});

#[allow(unused)]
pub fn frame_test() {
    // Test 1: Allocate a frame
    let frame1 = alloc().expect("Failed to allocate frame 1");
    let ppn1 = frame1.ppn.0;

    // Test 2: Allocate another frame, should have lower ppn (alloc from high to low)
    let frame2 = alloc().expect("Failed to allocate frame 2");
    let ppn2 = frame2.ppn.0;
    assert!(ppn2 < ppn1, "Frame 2 should have lower ppn than frame 1");

    // Test 3: Drop frame1, then allocate again, should reuse ppn1
    drop(frame1);
    let frame3 = alloc().expect("Failed to allocate frame 3");
    let ppn3 = frame3.ppn.0;
    assert_eq!(ppn3, ppn1, "Frame 3 should reuse ppn of dropped frame 1");

    println!("[frame_test] Frame allocator test passed!");
}
