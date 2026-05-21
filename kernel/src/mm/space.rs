//! Process memory space management.
//!
//! Each process owns a [`MemorySpace`] that tracks:
//! - Up to 16 page tables (one per 4MB block in the process's 64MB slot)
//! - Shared data page frames (COW references obtained via [`frame::share`])
//!
//! Dropping a `MemorySpace` automatically releases all owned page tables
//! and data frames (decrementing reference counts), and clears the
//! corresponding page directory entries in the shared page directory.
//!
//! # TLB consistency
//!
//! Public [`MemorySpace`] methods that install or change user-visible page
//! table entries flush the TLB before returning.  Internal helpers that only
//! write PTE slots do not; batching code (for example [`cow_copy`]) flushes
//! once after a sequence of updates.

use core::ptr;

use hashbrown::HashMap;

use super::{
    ENTRIES_PER_TABLE, PageDirectoryEntry, PageEntry, PageFlags, PageTable, PageTableEntry,
    address::{LinPageNum, PhysAddr, PhysPageNum},
    frame::{self, LOW_MEM, PAGE_SIZE, PhysFrame},
};
use crate::error::{Errno, Result};

/// Linear address space size per task slot (64MB).
pub const TASK_LINEAR_SIZE: u32 = (PDES_PER_PROCESS * ENTRIES_PER_TABLE * PAGE_SIZE) as u32;

/// A process's virtual memory space.
///
/// # Ownership model
///
/// - `page_tables[i]` owns the page table frame for the i-th 4MB block.
/// - `data_frames` owns shared references to physical data pages (obtained
///   via [`frame::share`] during COW copy).  Each [`PhysFrame`] in this map
///   represents one reference-counted stake; dropping it decrements
///   `mem_map`.
/// - `pde_base` records the starting PDE index in the shared page directory
///   (`task_index * 16`), used by `Drop` to clear the entries.
pub struct MemorySpace {
    page_tables: [Option<PageTable>; PDES_PER_PROCESS],
    data_frames: HashMap<LinPageNum, PhysFrame>,
    // Starting index in the shared page directory for this process.
    // For process n, this is `n * 16`.
    pde_base: usize,
}

/// Number of PTEs to copy when forking from task 0 (640KB).
const TASK0_PAGE_COUNT: usize = 0xA0000 / PAGE_SIZE;

/// Number of page directory entries per process (64MB / 4MB = 16).
const PDES_PER_PROCESS: usize = 16;

impl MemorySpace {
    /// Create an empty memory space for the given task slot.
    ///
    /// No page tables or data frames are allocated; the caller is
    /// responsible for populating them (e.g. via [`cow_copy`]).
    pub fn new(task_index: usize) -> Self {
        Self {
            page_tables: [const { None }; PDES_PER_PROCESS],
            data_frames: HashMap::new(),
            pde_base: task_index * PDES_PER_PROCESS,
        }
    }

    /// Read the PTE for a linear page.
    ///
    /// Returns `None` when the page is outside this memory space range or
    /// when the corresponding PDE is not present.
    pub fn get_pte(&self, page: LinPageNum) -> Option<PageTableEntry> {
        self.find_pte(page).map(|ptr| {
            // SAFETY: `find_pte` only returns a pointer into a present page
            // table for this address space.
            unsafe { *ptr }
        })
    }

    /// Convert an absolute linear page to a process-local `(pde_offset, pte_index)`.
    ///
    /// Returns `None` if the page is outside this process's address range.
    pub fn to_local(&self, page: LinPageNum) -> Option<(usize, usize)> {
        let pde_offset = page.pde_index().checked_sub(self.pde_base)?;
        if pde_offset >= PDES_PER_PROCESS {
            return None;
        }
        Some((pde_offset, page.pte_index()))
    }

    /// Ensure `lin_page` is mapped to a present, writable, user page.
    ///
    /// When `frame` is `Some`, ownership of the pre-allocated frame is
    /// transferred into this memory space.  The target page must not already be
    /// present; otherwise [`Errno::EXIST`] is returned.
    ///
    /// When `frame` is `None`, a zeroed frame is allocated on demand.  If the
    /// page is already present, this is a no-op.
    pub fn map_page(&mut self, lin_page: LinPageNum, frame: Option<PhysFrame>) -> Result<()> {
        self.ensure_page_table(lin_page.pde_index())?;
        let already_present = self.get_pte(lin_page).is_some_and(|pte| pte.is_present());
        if frame.is_none() && already_present {
            return Ok(());
        }
        if frame.is_some() && already_present {
            return Err(Errno::EXIST);
        }
        let frame = frame.map_or_else(|| frame::alloc().ok_or(Errno::NOMEM), Ok)?;
        self.set_pte(lin_page, PageTableEntry::new(frame.ppn, PageFlags::USER_RW));
        self.data_frames.insert(lin_page, frame);
        super::invalidate_tlb();
        Ok(())
    }

    /// Ensure the linear page mapping becomes writable.
    ///
    /// - If the old page is uniquely referenced (`ref_count == 1`), just clear write-protect.
    /// - Otherwise allocate a new page, copy old content, and remap this PTE to the new page.
    ///
    /// # Panics
    ///
    /// Panics if `lin_page` has no present page-table entry.
    pub fn ensure_page_writable(&mut self, lin_page: LinPageNum) -> Result<()> {
        let pte = self
            .get_pte(lin_page)
            .expect("ensure_page_writable: PTE not found");
        let old_phys_addr = pte.phys_addr();
        let old_ppn: PhysPageNum = old_phys_addr.into();
        if old_phys_addr.as_u32() >= LOW_MEM && frame::ref_count(old_ppn) == 1 {
            self.set_pte(
                lin_page,
                PageTableEntry::new(old_ppn, pte.flags().union(PageFlags::WRITABLE)),
            );
            super::invalidate_tlb();
            return Ok(());
        }
        let new_frame = frame::alloc().ok_or(Errno::NOMEM)?;
        let new_ppn = new_frame.ppn;
        self.set_pte(lin_page, PageTableEntry::new(new_ppn, PageFlags::USER_RW));
        self.data_frames.insert(lin_page, new_frame);
        super::invalidate_tlb();

        let src_addr: PhysAddr = old_ppn.into();
        let dst_addr: PhysAddr = new_ppn.into();
        // SAFETY: Both addresses point to valid page frames.  The destination
        // was freshly allocated, so it cannot overlap the source frame.
        unsafe {
            ptr::copy_nonoverlapping(
                src_addr.as_ptr::<u8>(),
                dst_addr.as_mut_ptr::<u8>(),
                PAGE_SIZE,
            );
        }
        Ok(())
    }

    /// Try to share a local page from `source_space` into this memory space.
    ///
    /// `local_pde_offset` and `pte_index` identify the same virtual page
    /// position within both processes' address spaces, even though each
    /// process has a different linear PDE base.
    ///
    /// Returns `Ok(true)` when the page was successfully shared (both PTEs
    /// are write-protected and the frame's reference count is incremented).
    ///
    /// Returns `Ok(false)` when the source page is not eligible for sharing
    /// (not present, dirty, below LOW_MEM, or target already mapped).
    ///
    /// Returns `Err(Errno::NOMEM)` when a page table allocation fails.
    pub fn try_share_local_page_from(
        &mut self,
        source_space: &mut MemorySpace,
        (local_pde_offset, pte_index): (usize, usize),
    ) -> Result<bool> {
        let source_page =
            LinPageNum::from_indices(source_space.pde_base + local_pde_offset, pte_index);
        let target_page = LinPageNum::from_indices(self.pde_base + local_pde_offset, pte_index);

        let Some(source_pte) = source_space.get_pte(source_page) else {
            return Ok(false);
        };
        if !source_pte.is_present() || source_pte.flags().contains(PageFlags::DIRTY) {
            return Ok(false);
        }
        let phys_addr = source_pte.phys_addr();
        if phys_addr.as_u32() < LOW_MEM {
            return Ok(false);
        }

        self.ensure_page_table(target_page.pde_index())?;
        if self
            .get_pte(target_page)
            .is_some_and(|pte| pte.is_present())
        {
            return Ok(false);
        }

        // Write-protect both source and target PTEs for COW semantics.
        let cow_pte = source_pte.without_writable();
        source_space.set_pte(source_page, cow_pte);
        self.set_pte(target_page, cow_pte);

        self.data_frames
            .insert(target_page, frame::share(phys_addr.into()));
        super::invalidate_tlb();
        Ok(true)
    }

    /// Create a COW (Copy-on-Write) copy of this memory space for fork.
    ///
    /// For each 4MB block in the parent's linear address range:
    /// 1. Read the parent's PDE; skip if not present.
    /// 2. Allocate a new page table for the child.
    /// 3. Copy each PTE with the WRITABLE bit cleared (COW).
    /// 4. For pages >= LOW_MEM, also clear WRITABLE in the parent's PTE
    ///    and call [`frame::share`] to create a tracked reference in the
    ///    child's `data_frames`.
    /// 5. Install the child's PDE in the shared page directory.
    ///
    /// # Special case: task 0 (`pde_base == 0`)
    ///
    /// When forking from task 0, only the first [`TASK0_PAGE_COUNT`] PTEs
    /// (640KB) are copied.  Pages below LOW_MEM are shared without
    /// reference counting (they are kernel/BIOS memory that is never freed).
    ///
    /// # Arguments
    ///
    /// - `child_index`: task slot index for the child process
    /// - `data_limit`: byte-granular data segment limit (from LDT), used
    ///   to compute how many PDEs (4MB blocks) need to be copied.
    ///
    /// # Returns
    ///
    /// A new `MemorySpace` for the child on success, or `Err(Errno::NOMEM)` if a
    /// page table frame could not be allocated.  On failure, any partially
    /// built state is cleaned up automatically when the returned
    /// `MemorySpace` is dropped.
    pub fn cow_copy(&mut self, child_index: usize, data_limit: u32) -> Result<MemorySpace> {
        let parent_pde_start = self.pde_base;
        let child_pde_start = child_index * PDES_PER_PROCESS;
        let is_task0 = parent_pde_start == 0;

        let pde_count = (data_limit as usize)
            .div_ceil(ENTRIES_PER_TABLE * PAGE_SIZE)
            .min(PDES_PER_PROCESS);

        let mut child = MemorySpace::new(child_index);

        for i in 0..pde_count {
            let parent_pde = super::read_pde(parent_pde_start + i);
            if !parent_pde.is_present() {
                continue;
            }

            debug_assert!(
                !super::read_pde(child_pde_start + i).is_present(),
                "cow_copy: child PDE {} already present",
                child_pde_start + i
            );

            let mut child_pt = PageTable::new().ok_or(Errno::NOMEM)?;

            // SAFETY: The parent PDE is present, so it points to a valid page
            // table frame.  For task 0 the page tables are set up by head.s
            // (not tracked in self.page_tables), so we must go through the PDE.
            let parent_ptes = unsafe {
                &mut *parent_pde
                    .phys_addr()
                    .as_mut_ptr::<[PageTableEntry; ENTRIES_PER_TABLE]>()
            };

            let entry_count = if is_task0 {
                TASK0_PAGE_COUNT
            } else {
                ENTRIES_PER_TABLE
            };

            let child_ptes = child_pt.as_pte_array_mut();
            for (j, (parent_pte, child_pte)) in parent_ptes[..entry_count]
                .iter_mut()
                .zip(&mut child_ptes[..entry_count])
                .enumerate()
                .filter(|(_, (p, _))| p.is_present())
            {
                let cow_pte = parent_pte.without_writable();
                *child_pte = cow_pte;

                let parent_ppn = parent_pte.ppn();
                let parent_phys: PhysAddr = parent_ppn.into();
                if parent_phys.as_u32() >= LOW_MEM {
                    *parent_pte = cow_pte;
                    let parent_lin_page = LinPageNum::from_indices(parent_pde_start + i, j);
                    debug_assert!(
                        self.data_frames.contains_key(&parent_lin_page),
                        "cow_copy: parent missing frame handle for lin_page={} phys={:#x} pde_base={}",
                        parent_lin_page.as_u32(),
                        parent_phys.as_u32(),
                        self.pde_base
                    );
                    let lin_page = LinPageNum::from_indices(child_pde_start + i, j);
                    child.data_frames.insert(lin_page, frame::share(parent_ppn));
                }
            }

            super::write_pde(
                child_pde_start + i,
                PageDirectoryEntry::user_page_table(child_pt.phys_addr()),
            );
            child.page_tables[i] = Some(child_pt);
        }

        super::invalidate_tlb();
        Ok(child)
    }

    /// Locate the raw pointer to the PTE for a linear page.
    ///
    /// Returns `None` when the page is outside this memory space range or
    /// when the corresponding PDE is not present.
    fn find_pte(&self, page: LinPageNum) -> Option<*mut PageTableEntry> {
        let pde_index = page.pde_index();
        if !(self.pde_base..self.pde_base + PDES_PER_PROCESS).contains(&pde_index) {
            return None;
        }
        let pde = super::read_pde(pde_index);
        if !pde.is_present() {
            return None;
        }
        // SAFETY: The PDE is present, so it points to a valid page table frame
        // allocated by `ensure_page_table` (or by head.s for task 0).
        Some(unsafe {
            pde.ppn()
                .addr()
                .as_mut_ptr::<PageTableEntry>()
                .add(page.pte_index())
        })
    }

    /// Write a PTE for a linear page.
    ///
    /// The caller must ensure the PDE already exists (e.g. via
    /// `ensure_page_table`).  Does **not** flush the TLB.
    fn set_pte(&mut self, page: LinPageNum, pte: PageTableEntry) {
        let ptr = self
            .find_pte(page)
            .expect("set_pte: PDE not present for target page");
        // SAFETY: `find_pte` returned a valid mutable pointer to the target
        // page table entry.
        unsafe { *ptr = pte };
    }

    /// Allocate a page table for `pde_index` if one does not already exist.
    fn ensure_page_table(&mut self, pde_index: usize) -> Result<()> {
        if super::read_pde(pde_index).is_present() {
            return Ok(());
        }
        let local = pde_index
            .checked_sub(self.pde_base)
            .filter(|&i| i < PDES_PER_PROCESS)
            .ok_or(Errno::NOMEM)?;
        let page_table = PageTable::new().ok_or(Errno::NOMEM)?;
        super::write_pde(
            pde_index,
            PageDirectoryEntry::user_page_table(page_table.phys_addr()),
        );
        self.page_tables[local] = Some(page_table);
        Ok(())
    }
}

impl Drop for MemorySpace {
    fn drop(&mut self) {
        let has_page_tables = self.page_tables.iter().any(|pt| pt.is_some());
        if !has_page_tables {
            return;
        }

        assert!(
            self.pde_base != 0,
            "Trying to free kernel memory space (task 0)"
        );

        for i in 0..PDES_PER_PROCESS {
            if self.page_tables[i].is_some() {
                super::write_pde(self.pde_base + i, PageDirectoryEntry::empty());
            }
        }
        super::invalidate_tlb();
    }
}
