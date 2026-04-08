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
//! All public methods that modify page table entries flush the TLB before
//! returning.  Callers never need to invalidate the TLB themselves.

use core::ptr;

use hashbrown::HashMap;

use super::{
    ENTRIES_PER_TABLE, PageDirectoryEntry, PageEntry, PageFlags, PageTable, PageTableEntry,
    address::{LinPageNum, PhysAddr, PhysPageNum},
    frame::{self, LOW_MEM, PAGE_SIZE, PhysFrame},
};
use crate::syscall::ENOMEM;

/// Number of page directory entries per process (64MB / 4MB = 16).
const PDES_PER_PROCESS: usize = 16;

/// Linear address space size per task slot (64MB).
pub const TASK_LINEAR_SIZE: u32 = (PDES_PER_PROCESS * ENTRIES_PER_TABLE * PAGE_SIZE) as u32;

/// Number of PTEs to copy when forking from task 0 (640KB).
const TASK0_NR_PAGES: usize = 0xA0000 / PAGE_SIZE;

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
///   (`task_nr * 16`), used by `Drop` to clear the entries.
pub struct MemorySpace {
    page_tables: [Option<PageTable>; PDES_PER_PROCESS],
    data_frames: HashMap<LinPageNum, PhysFrame>,
    /// Starting index in the shared page directory for this process.
    /// For process n, this is `n * 16`.
    pde_base: usize,
}

impl MemorySpace {
    /// Create an empty memory space for the given task slot.
    ///
    /// No page tables or data frames are allocated; the caller is
    /// responsible for populating them (e.g. via [`cow_copy`]).
    pub fn new(task_nr: usize) -> Self {
        Self {
            page_tables: [const { None }; PDES_PER_PROCESS],
            data_frames: HashMap::new(),
            pde_base: task_nr * PDES_PER_PROCESS,
        }
    }

    /// Starting PDE index for this process in the shared page directory.
    pub fn pde_base(&self) -> usize {
        self.pde_base
    }

    /// Find the mutable PTE for a linear page.
    ///
    /// Returns `None` when the page is outside this memory space range or
    /// when the corresponding PDE is not present.
    pub fn find_pte(&mut self, page: LinPageNum) -> Option<&mut PageTableEntry> {
        let pde_index = page.pde_index();
        if !(self.pde_base..self.pde_base + PDES_PER_PROCESS).contains(&pde_index) {
            return None;
        }

        let pde = super::read_pde(pde_index);
        if !pde.is_present() {
            return None;
        }

        let page_table_phys: PhysAddr = pde.ppn().into();
        let page_table =
            unsafe { &mut *page_table_phys.as_mut_ptr::<[PageTableEntry; ENTRIES_PER_TABLE]>() };
        Some(&mut page_table[page.pte_index()])
    }

    /// Translate a global PDE index into the per-process 0..16 slot index.
    #[inline]
    fn local_pde_index(&self, pde_index: usize) -> Option<usize> {
        (self.pde_base..self.pde_base + PDES_PER_PROCESS)
            .contains(&pde_index)
            .then_some(pde_index - self.pde_base)
    }

    /// Allocate a page table for `pde_index` if one does not already exist.
    fn ensure_page_table(&mut self, pde_index: usize) -> Result<(), u32> {
        if super::read_pde(pde_index).is_present() {
            return Ok(());
        }
        let local = self.local_pde_index(pde_index).ok_or(ENOMEM)?;
        let page_table = PageTable::new().ok_or(ENOMEM)?;
        super::write_pde(
            pde_index,
            PageDirectoryEntry::user_page_table(page_table.phys_addr()),
        );
        self.page_tables[local] = Some(page_table);
        Ok(())
    }

    /// Map a pre-allocated physical frame at the given linear page address.
    ///
    /// Ownership of `frame` is transferred into this memory space.  On
    /// failure the frame is dropped (freed) and `ENOMEM` is returned.
    pub fn map_page(&mut self, lin_page: LinPageNum, frame: PhysFrame) -> Result<(), u32> {
        self.ensure_page_table(lin_page.pde_index())?;
        let Some(pte) = self.find_pte(lin_page) else {
            return Err(ENOMEM);
        };
        let ppn = frame.ppn;
        *pte = PageTableEntry::new(ppn, PageFlags::USER_RW);
        self.data_frames.insert(lin_page, frame);
        super::invalidate_tlb();
        Ok(())
    }

    /// Ensure `fault_page` is mapped to a present, writable, user page.
    ///
    /// This is the anonymous-demand-paging path for not-present page faults:
    /// - Allocate a page table when the PDE is missing.
    /// - Allocate a zeroed data frame.
    /// - Install a user/writable/present PTE.
    pub fn map_zero_page(&mut self, fault_page: LinPageNum) -> Result<(), u32> {
        self.ensure_page_table(fault_page.pde_index())?;

        let Some(pte) = self.find_pte(fault_page) else {
            return Err(ENOMEM);
        };
        if pte.is_present() {
            return Ok(());
        }

        let frame = frame::alloc().ok_or(ENOMEM)?;
        let ppn = frame.ppn;
        *pte = PageTableEntry::new(ppn, PageFlags::USER_RW);
        self.data_frames.insert(fault_page, frame);
        super::invalidate_tlb();
        Ok(())
    }

    /// Ensure the faulting page mapping becomes writable.
    ///
    /// - If the old page is uniquely referenced (`ref_count == 1`), just clear write-protect.
    /// - Otherwise allocate a new page, copy old content, and remap this PTE to the new page.
    pub fn ensure_page_writable(&mut self, fault_page: LinPageNum) -> Result<(), u32> {
        let pte = self
            .find_pte(fault_page)
            .expect("ensure_page_writable: PTE not found");
        let old_phys_addr = pte.phys_addr();
        let old_ppn = old_phys_addr.into();
        if old_phys_addr.as_u32() >= LOW_MEM && frame::ref_count(old_ppn) == 1 {
            let new_flag = pte.flags().union(PageFlags::WRITABLE);
            *pte = PageTableEntry::new(old_ppn, new_flag);
            super::invalidate_tlb();
            return Ok(());
        }
        let new_frame = frame::alloc().ok_or(ENOMEM)?;
        let new_ppn = new_frame.ppn;
        *pte = PageTableEntry::new(new_ppn, PageFlags::USER_RW);
        self.data_frames.insert(fault_page, new_frame);
        super::invalidate_tlb();
        copy_page(old_ppn, new_ppn);
        Ok(())
    }

    /// Try to share a page from `source_space` at `source_page`.
    ///
    /// Returns `Ok(true)` when the page was successfully shared (both PTEs
    /// are write-protected and the frame's reference count is incremented).
    ///
    /// Returns `Ok(false)` when the source page is not eligible for sharing
    /// (not present, dirty, below LOW_MEM, or target already mapped).
    ///
    /// Returns `Err(ENOMEM)` when a page table allocation fails.
    pub fn try_share_from(
        &mut self,
        source_space: &mut MemorySpace,
        source_page: LinPageNum,
        target_page: LinPageNum,
    ) -> Result<bool, u32> {
        let Some(source_pte) = source_space.find_pte(source_page) else {
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

        let Some(target_pte) = self.find_pte(target_page) else {
            return Ok(false);
        };

        if target_pte.is_present() {
            return Ok(false);
        }

        // Write-protect the source page for COW semantics.
        let cow_pte = source_pte.without_writable();
        *source_pte = cow_pte;

        // Map the same physical page into the target, also write-protected.
        *target_pte = cow_pte;

        let shared_frame = frame::share(phys_addr.into());
        self.data_frames.insert(target_page, shared_frame);

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
    /// When forking from task 0, only the first [`TASK0_NR_PAGES`] PTEs
    /// (640KB) are copied.  Pages below LOW_MEM are shared without
    /// reference counting (they are kernel/BIOS memory that is never freed).
    ///
    /// # Arguments
    ///
    /// - `child_nr`: task slot number for the child process
    /// - `data_limit`: byte-granular data segment limit (from LDT), used
    ///   to compute how many PDEs (4MB blocks) need to be copied.
    ///
    /// # Returns
    ///
    /// A new `MemorySpace` for the child on success, or `Err(ENOMEM)` if a
    /// page table frame could not be allocated.  On failure, any partially
    /// built state is cleaned up automatically when the returned
    /// `MemorySpace` is dropped.
    pub fn cow_copy(&mut self, child_nr: usize, data_limit: u32) -> Result<MemorySpace, u32> {
        let parent_pde_start = self.pde_base;
        let child_pde_start = child_nr * PDES_PER_PROCESS;
        let is_task0 = parent_pde_start == 0;

        let nr_pdes = (data_limit as usize)
            .div_ceil(ENTRIES_PER_TABLE * PAGE_SIZE)
            .min(PDES_PER_PROCESS);

        let mut child = MemorySpace::new(child_nr);

        for i in 0..nr_pdes {
            let parent_pde = super::read_pde(parent_pde_start + i);
            if !parent_pde.is_present() {
                continue;
            }

            debug_assert!(
                !super::read_pde(child_pde_start + i).is_present(),
                "cow_copy: child PDE {} already present",
                child_pde_start + i
            );

            let mut child_pt = PageTable::new().ok_or(ENOMEM)?;

            let parent_ptes = unsafe {
                &mut *(parent_pde
                    .phys_addr()
                    .as_mut_ptr::<[PageTableEntry; ENTRIES_PER_TABLE]>())
            };

            let nr_entries = if is_task0 {
                TASK0_NR_PAGES
            } else {
                ENTRIES_PER_TABLE
            };

            let child_ptes = child_pt.as_pte_array_mut();
            for (j, (parent_pte, child_pte)) in parent_ptes[..nr_entries]
                .iter_mut()
                .zip(&mut child_ptes[..nr_entries])
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

/// Copy one 4KB physical page.
///
/// Both `src_page` and `dst_page` must be valid allocated frames
/// (physical addresses well above zero).
fn copy_page(src_page: PhysPageNum, dst_page: PhysPageNum) {
    let src: PhysAddr = src_page.into();
    let dst: PhysAddr = dst_page.into();
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr::<u8>(), dst.as_mut_ptr::<u8>(), PAGE_SIZE);
    }
}
