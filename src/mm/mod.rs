pub mod address;
pub mod frame;
mod heap;
mod page;
pub(crate) mod page_fault;
pub mod space;

use crate::{mm::page::PageEntry, task};

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
pub fn ensure_user_area_writable(addr: u32, size: usize) {
    task::current_task().pcb.inner.exclusive(|inner| {
        let base = inner.ldt.data_segment().base();
        if let Some(ms) = inner.memory_space.as_mut() {
            let first_page_offset = addr & (frame::PAGE_SIZE - 1);
            let mut size = size as u32 + first_page_offset;
            let mut linear_addr = (addr & !(frame::PAGE_SIZE - 1)).wrapping_add(base);

            while size > 0 {
                let lin_page = address::LinAddr(linear_addr).floor();
                if let Some(pte) = ms.find_pte(lin_page) {
                    if pte.is_present() && !pte.flags().contains(page::PageFlags::WRITABLE) {
                        ms.ensure_page_writable(lin_page);
                    }
                }
                size = size.saturating_sub(frame::PAGE_SIZE);
                linear_addr += frame::PAGE_SIZE;
            }
        }
    });
}
