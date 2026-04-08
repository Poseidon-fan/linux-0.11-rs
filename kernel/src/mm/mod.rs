//! Memory management: paging, physical frame allocation, and per-process address spaces.
//!
//! The kernel uses i386 two-level paging with a single shared page directory at
//! physical address 0. Each task occupies a 64 MB linear window (16 PDEs) via
//! its LDT base, and memory is managed through demand paging with COW on fork.
//!
//! - [`address`] — linear / physical address and page number types.
//! - [`frame`] — reference-counted physical frame allocator (`mem_map` bitmap).
//! - [`space`] — per-process [`MemorySpace`](space::MemorySpace) owning page tables and data frames.
//! - [`page`] — page table / directory entry types and the shared page directory accessors.
//! - [`page_fault`] — not-present and write-protect fault handlers.
//! - [`heap`] — kernel heap backed by a buddy-system allocator (1 MB at `0x100000..0x200000`).

pub mod address;
pub mod frame;
mod heap;
mod page;
mod page_fault;
pub mod space;

pub use page::{
    ENTRIES_PER_TABLE, PageDirectoryEntry, PageEntry, PageFlags, PageTable, PageTableEntry,
    invalidate_tlb, read_pde, write_pde,
};
pub use page_fault::{handle_no_page, handle_wp_page};

use crate::task;

unsafe extern "C" {
    fn ekernel();
}

pub fn init(start_mem: u32, end_mem: u32) {
    crate::println!("ekernel: 0x{:x}", ekernel as usize);
    heap::init();
    frame::init(start_mem, end_mem);
}

/// Ensure user address range [addr, addr+size) is writable.
///
/// Touches each present, write-protected (COW) page in the range to trigger
/// copy-on-write before the kernel writes to user memory.  Matches the
/// semantics of Linux 0.11's `verify_area`.
///
/// `addr` is a raw user-space virtual address passed from syscall context.
pub fn ensure_user_area_writable(addr: u32, size: usize) {
    task::with_current(|inner| {
        let base = inner.ldt.data_segment().base();
        if let Some(ms) = inner.memory_space.as_mut() {
            let user_addr = address::LinAddr(addr);
            let first_page_offset = user_addr.page_offset();
            let mut size = (size as u32).saturating_add(first_page_offset);
            let mut linear_addr = user_addr.align_down().0.wrapping_add(base);

            while size > 0 {
                let lin_page = address::LinAddr(linear_addr).floor();
                if let Some(pte) = ms.find_pte(lin_page) {
                    if pte.is_present() && !pte.flags().contains(PageFlags::WRITABLE) {
                        let _ = ms.ensure_page_writable(lin_page);
                    }
                }
                size = size.saturating_sub(frame::PAGE_SIZE as u32);
                linear_addr = linear_addr.wrapping_add(frame::PAGE_SIZE as u32);
            }
        }
    });
}
