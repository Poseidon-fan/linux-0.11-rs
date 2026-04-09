//! Page-fault handlers for demand paging, page sharing, and COW.
//!
//! - [`handle_no_page`] — not-present fault: share, demand-load, or zero-fill.
//! - [`handle_wp_page`] — write-protect fault: COW copy.

use alloc::sync::Arc;
use core::ptr;

use crate::{
    fs::{
        BLOCK_SIZE,
        buffer::{self, BufferKey},
        minix::Inode,
    },
    mm::{
        PageEntry,
        address::{LinAddr, LinPageNum, PhysAddr},
        frame::{self, PAGE_SIZE, PhysFrame},
    },
    println,
    signal::SIGSEGV,
    task::{self, TASK_MANAGER},
};

/// What kind of page needs to be supplied for a not-present fault.
enum FaultKind {
    /// Fault within the executable's data segment — try sharing, then loading.
    Executable {
        inode: Arc<Inode>,
        addr_offset: u32,
        end_data: u32,
        local_offset: Option<(usize, usize)>,
    },
    /// Fault outside the data segment, or no executable — zero-fill.
    Anonymous,
}

/// Handle a not-present page fault (`P=0`).
///
/// Resolution order:
/// 1. **Share** — reuse a page from another process with the same executable.
/// 2. **Load** — read the page from the executable file on disk.
/// 3. **Zero** — allocate a fresh zero page (stack, heap, BSS).
pub fn handle_no_page(_error_code: u32, address: u32) {
    let fault_page = LinAddr::from(address).floor();

    let resolved = match classify(address, fault_page) {
        FaultKind::Executable {
            ref inode,
            addr_offset,
            end_data,
            local_offset,
        } => {
            try_share_page(inode, local_offset)
                || try_load_page(inode, fault_page, addr_offset, end_data)
        }
        FaultKind::Anonymous => false,
    } || task::with_current(|inner| {
        inner
            .memory_space
            .as_mut()
            .and_then(|ms| ms.map_zero_page(fault_page).ok())
            .is_some()
    });

    if !resolved {
        if task::current_slot() == 0 {
            panic!("handle_no_page(task0): map failed address={:#x}", address);
        }
        oom();
    }
}

/// Handle a write-protect page fault on a present page (`P=1, W=1`).
///
/// Performs copy-on-write: if the page has a single reference, clears the
/// write-protect bit; otherwise allocates a new frame and copies.
pub fn handle_wp_page(address: u32) {
    let fault_page = LinAddr::from(address).floor();
    let result = task::with_current(|inner| {
        inner
            .memory_space
            .as_mut()
            .expect("handle_wp_page: no memory space")
            .ensure_page_writable(fault_page)
    });
    if result.is_err() {
        oom();
    }
}

/// Terminate the current process due to out-of-memory.
///
/// Must NOT be called from inside a `with_current` or `TASK_MANAGER.exclusive`
/// closure — `do_exit` acquires those locks internally.
pub(super) fn oom() -> ! {
    println!("out of memory");
    task::do_exit(SIGSEGV as i32)
}

/// Determine what kind of page the fault requires.
fn classify(address: u32, fault_page: LinPageNum) -> FaultKind {
    task::with_current(|inner| {
        let base = inner.ldt.data_segment().base();
        let addr_offset = (address & !0xFFF).wrapping_sub(base);
        let end_data = inner.mem_layout.end_data;

        match inner.fs.executable_inode.clone() {
            Some(inode) if addr_offset < end_data => {
                let local_offset = inner
                    .memory_space
                    .as_ref()
                    .and_then(|ms| ms.to_local(fault_page));
                FaultKind::Executable {
                    inode,
                    addr_offset,
                    end_data,
                    local_offset,
                }
            }
            _ => FaultKind::Anonymous,
        }
    })
}

/// Scan the task table for a process running the same executable that
/// already has this page loaded, and share it via COW.
fn try_share_page(exe_inode: &Arc<Inode>, local_offset: Option<(usize, usize)>) -> bool {
    let Some((local_pde, local_pte)) = local_offset else {
        return false;
    };
    let current_slot = task::current_slot();

    // Three-level KernelCell nesting: TASK_MANAGER -> source pcb.inner ->
    // current pcb.inner.  Safe on single-core: each is a distinct RefCell
    // and IRQs are masked by the outermost exclusive().
    TASK_MANAGER.exclusive(|tm| {
        tm.tasks
            .iter()
            .enumerate()
            .filter(|&(slot, _)| slot != current_slot)
            .filter_map(|(_, task)| task.as_ref())
            .any(|task| {
                task.pcb.inner.exclusive(|source| {
                    source
                        .fs
                        .executable_inode
                        .as_ref()
                        .is_some_and(|exe| Arc::ptr_eq(exe, exe_inode))
                        && task::with_current(|current| {
                            let (Some(src_ms), Some(dst_ms)) =
                                (source.memory_space.as_mut(), current.memory_space.as_mut())
                            else {
                                return false;
                            };
                            dst_ms
                                .try_share_from(src_ms, local_pde, local_pte)
                                .unwrap_or(false)
                        })
                })
            })
    })
}

/// Load a page from the executable on disk and map it.
fn try_load_page(inode: &Inode, fault_page: LinPageNum, addr_offset: u32, end_data: u32) -> bool {
    let Some(frame) = load_exe_page(inode, addr_offset, end_data) else {
        return false;
    };
    task::with_current(|inner| {
        inner
            .memory_space
            .as_mut()
            .map(|ms| {
                // Another handler may have mapped this page during disk I/O.
                ms.get_pte(fault_page).is_some_and(|pte| pte.is_present())
                    || ms.map_page(fault_page, frame).is_ok()
            })
            .unwrap_or(false)
    })
}

/// Read one page of executable data into a new physical frame.
///
/// Reads 4 filesystem blocks starting at `address_offset` within the data
/// segment (block 0 is the a.out header, so data starts at block 1).
/// Bytes beyond `end_data` are zeroed (BSS region).
fn load_exe_page(inode: &Inode, address_offset: u32, end_data: u32) -> Option<PhysFrame> {
    let frame = frame::alloc()?;
    let page_phys: PhysAddr = frame.ppn.into();

    let blocks_per_page = PAGE_SIZE / BLOCK_SIZE;
    let first_block = 1 + (address_offset as usize / BLOCK_SIZE);

    for i in 0..blocks_per_page {
        let dst = page_phys.byte_add(i * BLOCK_SIZE);
        let block_id = inode.map_block_id(first_block + i, false).unwrap_or(0);

        let buf = if block_id != 0 {
            buffer::read_block(BufferKey {
                dev: inode.id.device,
                block_nr: block_id,
            })
        } else {
            None
        };

        match buf {
            Some(bh) => {
                unsafe { ptr::copy_nonoverlapping(bh.data.as_ptr(), dst, BLOCK_SIZE) };
                buffer::release_block(bh);
            }
            None => unsafe { ptr::write_bytes(dst, 0, BLOCK_SIZE) },
        }
    }

    // Zero BSS: bytes beyond end_data within this page.
    let page_end = address_offset + PAGE_SIZE as u32;
    if page_end > end_data && address_offset < end_data {
        let bss_start = (end_data - address_offset) as usize;
        unsafe { ptr::write_bytes(page_phys.byte_add(bss_start), 0, PAGE_SIZE - bss_start) };
    }

    Some(frame)
}
