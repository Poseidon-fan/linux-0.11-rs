//! Page-fault handlers for demand paging, page sharing, and COW.
//!
//! - [`handle_no_page`] — not-present fault: share, demand-load, or zero-fill.
//! - [`handle_wp_page`] — write-protect fault: COW copy.

use alloc::sync::Arc;
use core::ptr;

use crate::{
    error::Errno,
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
    task::{self, TASK_MANAGER},
};

/// Handle a write-protect page fault on a present page (`P=1, W=1`).
///
/// Performs copy-on-write: if the page has a single reference, clears the
/// write-protect bit; otherwise allocates a new frame and copies.
pub fn handle_wp_page(address: u32) {
    let fault_page = LinAddr::from(address).floor();
    task::with_current(|inner| {
        inner
            .memory_space
            .as_mut()
            .expect("handle_wp_page: no memory space")
            .ensure_page_writable(fault_page)
    })
    .unwrap_or_else(|_| super::oom());
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
        } => match try_share_page(inode, local_offset) {
            Ok(true) => true,
            Ok(false) => try_load_page(inode, fault_page, addr_offset, end_data),
            Err(_) => false,
        },
        FaultKind::Anonymous => map_zero_page(fault_page),
    };

    if !resolved {
        if task::current_slot() == 0 {
            panic!(
                "handle_no_page(task0): could not resolve page fault address={:#x}",
                address
            );
        }
        super::oom();
    }
}

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
///
/// Returns `Err` when allocating a page table for the current task fails
/// (same outcome as the historical `oom()` path in the reference flow).
fn try_share_page(
    exe_inode: &Arc<Inode>,
    local_offset: Option<(usize, usize)>,
) -> Result<bool, Errno> {
    let Some(local_offset) = local_offset else {
        return Ok(false);
    };
    let current_slot = task::current_slot();

    // Three-level KernelCell nesting: TASK_MANAGER -> source pcb.inner ->
    // current pcb.inner.  Safe on single-core: each is a distinct RefCell
    // and IRQs are masked by the outermost exclusive().
    TASK_MANAGER.exclusive(|tm| {
        for (slot, task_opt) in tm.tasks.iter().enumerate() {
            if slot == current_slot {
                continue;
            }
            let Some(task) = task_opt.as_ref() else {
                continue;
            };
            let shared = task.pcb.inner.exclusive(|source| {
                if !source
                    .fs
                    .executable_inode
                    .as_ref()
                    .is_some_and(|exe| Arc::ptr_eq(exe, exe_inode))
                {
                    return Ok(false);
                }
                task::with_current(|current| {
                    let (Some(src_ms), Some(dst_ms)) =
                        (source.memory_space.as_mut(), current.memory_space.as_mut())
                    else {
                        return Ok(false);
                    };
                    dst_ms.try_share_local_page_from(src_ms, local_offset)
                })
            })?;
            if shared {
                return Ok(true);
            }
        }
        Ok(false)
    })
}

/// Load a page from the executable on disk and map it.
fn try_load_page(inode: &Inode, fault_page: LinPageNum, addr_offset: u32, end_data: u32) -> bool {
    let Some(frame) = load_exe_page(inode, addr_offset, end_data) else {
        return false;
    };
    task::with_current(|inner| {
        let Some(ms) = inner.memory_space.as_mut() else {
            return false;
        };
        if ms.get_pte(fault_page).is_some_and(|pte| pte.is_present()) {
            return true;
        }
        ms.map_page(fault_page, Some(frame)).is_ok()
    })
}

fn map_zero_page(fault_page: LinPageNum) -> bool {
    task::with_current(|inner| {
        inner
            .memory_space
            .as_mut()
            .and_then(|ms| ms.map_page(fault_page, None).ok())
            .is_some()
    })
}

/// Read one page of executable data into a new physical frame.
///
/// Reads 4 filesystem blocks starting at `address_offset` within the data
/// segment (block 0 is the a.out header, so data starts at block 1).
/// Bytes beyond `end_data` are zeroed (BSS region).
///
/// Unmapped logical blocks (`Ok(0)` from [`Inode::map_block_id`]) are treated
/// as zero-filled, matching a sparse executable image.
fn load_exe_page(inode: &Inode, address_offset: u32, end_data: u32) -> Option<PhysFrame> {
    let frame = frame::alloc()?;
    let page_addr: PhysAddr = frame.ppn.into();

    let blocks_per_page = PAGE_SIZE / BLOCK_SIZE;
    let first_block = 1 + (address_offset as usize / BLOCK_SIZE);

    for i in 0..blocks_per_page {
        let dst = page_addr.byte_add(i * BLOCK_SIZE);
        let block_id = inode.map_block_id(first_block + i, false).ok()?;

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
                // SAFETY: `dst` points inside the freshly allocated page, and
                // the buffer block contains exactly `BLOCK_SIZE` initialized bytes.
                unsafe { ptr::copy_nonoverlapping(bh.data.as_ptr(), dst, BLOCK_SIZE) };
                buffer::release_block(bh);
            }
            None => {
                // SAFETY: `dst` points to one block-sized range inside the
                // freshly allocated page.
                unsafe { ptr::write_bytes(dst, 0, BLOCK_SIZE) };
            }
        }
    }

    let page_end = address_offset + PAGE_SIZE as u32;
    if page_end > end_data && address_offset < end_data {
        let bss_start = (end_data - address_offset) as usize;
        // SAFETY: `bss_start` is within this page because callers only request
        // executable data pages whose start offset is below `end_data`.
        unsafe { ptr::write_bytes(page_addr.byte_add(bss_start), 0, PAGE_SIZE - bss_start) };
    }

    Some(frame)
}
