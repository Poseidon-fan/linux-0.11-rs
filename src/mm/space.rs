//! Process memory space management.
//!
//! Each process owns a [`MemorySpace`] that tracks:
//! - Up to 16 page tables (one per 4MB block in the process's 64MB slot)
//! - Shared data page frames (COW references obtained via [`frame::share`])
//!
//! Dropping a `MemorySpace` automatically releases all owned page tables
//! and data frames (decrementing reference counts), and clears the
//! corresponding page directory entries in the shared page directory.

use alloc::collections::btree_map::BTreeMap;

use crate::mm::{
    address::LinPageNum,
    frame::{self, LOW_MEM, PAGE_SIZE, PhysFrame},
    page::{self, ENTRIES_PER_TABLE, PageDirectoryEntry, PageEntry, PageTable, PageTableEntry},
};

/// Number of page directory entries per process (64MB / 4MB = 16).
const PDES_PER_PROCESS: usize = 16;

/// Linear address space size per task slot (64MB).
pub const TASK_LINEAR_SIZE: u32 =
    (PDES_PER_PROCESS as u32) * (ENTRIES_PER_TABLE as u32) * PAGE_SIZE;

/// Number of PTEs to copy when forking from task 0 (640KB / 4KB = 160).
const TASK0_NR_PAGES: usize = 0xA0;

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
    data_frames: BTreeMap<LinPageNum, PhysFrame>,
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
            data_frames: BTreeMap::new(),
            pde_base: task_nr * PDES_PER_PROCESS,
        }
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
    /// A new `MemorySpace` for the child on success, or `Err(())` if a
    /// page table frame could not be allocated.  On failure, any partially
    /// built state is cleaned up automatically when the returned
    /// `MemorySpace` is dropped.
    pub fn cow_copy(&mut self, child_nr: usize, data_limit: u32) -> Result<MemorySpace, ()> {
        let parent_pde_start = self.pde_base;
        let child_pde_start = child_nr * PDES_PER_PROCESS;
        let is_task0 = parent_pde_start == 0;

        // Round data_limit up to 4MB boundary, then convert to PDE count.
        let nr_pdes = ((data_limit as usize + 0x3F_FFFF) >> 22).min(PDES_PER_PROCESS);

        let mut child = MemorySpace::new(child_nr);

        for i in 0..nr_pdes {
            let parent_pde = page::read_pde(parent_pde_start + i);
            if !parent_pde.is_present() {
                continue;
            }

            assert!(
                !page::read_pde(child_pde_start + i).is_present(),
                "cow_copy: child PDE {} already present",
                child_pde_start + i
            );

            let mut child_pt = PageTable::new().ok_or(())?;

            // Interpret the parent's page table as a PTE slice.
            let parent_ptes = unsafe {
                &mut *(parent_pde.phys_addr() as *mut [PageTableEntry; ENTRIES_PER_TABLE])
            };

            // Task 0 special case: only copy first 640KB.
            let nr_entries = if is_task0 {
                TASK0_NR_PAGES
            } else {
                ENTRIES_PER_TABLE
            };

            // Copy present PTEs with COW semantics.
            let child_ptes = child_pt.as_pte_array_mut();
            for (j, (parent_pte, child_pte)) in parent_ptes[..nr_entries]
                .iter_mut()
                .zip(&mut child_ptes[..nr_entries])
                .enumerate()
                .filter(|(_, (p, _))| p.is_present())
            {
                let cow_pte = parent_pte.without_writable();
                *child_pte = cow_pte;

                // Pages >= LOW_MEM participate in COW reference counting.
                let phys = parent_pte.phys_addr();
                if phys >= LOW_MEM {
                    *parent_pte = cow_pte;
                    let lin_page = LinPageNum::from(
                        (child_pde_start + i) as u32 * ENTRIES_PER_TABLE as u32 + j as u32,
                    );
                    child
                        .data_frames
                        .insert(lin_page, frame::share((phys / PAGE_SIZE).into()));
                }
            }

            // Install the child's page table in the page directory.
            page::write_pde(
                child_pde_start + i,
                PageDirectoryEntry::user_page_table(child_pt.phys_addr()),
            );
            child.page_tables[i] = Some(child_pt);
        }

        page::invalidate_tlb();
        Ok(child)
    }
}

impl Drop for MemorySpace {
    fn drop(&mut self) {
        let has_page_tables = self.page_tables.iter().any(|pt| pt.is_some());
        if !has_page_tables {
            return;
        }

        // Never free task 0's kernel page directory entries.
        assert!(
            self.pde_base != 0,
            "Trying to free kernel memory space (task 0)"
        );

        // Clear our PDEs in the shared page directory before the
        // PageTable frames are freed (otherwise the PDEs would dangle).
        for i in 0..PDES_PER_PROCESS {
            if self.page_tables[i].is_some() {
                page::write_pde(self.pde_base + i, PageDirectoryEntry::empty());
            }
        }
        page::invalidate_tlb();

        // `page_tables` and `data_frames` are dropped automatically after
        // this, which decrements the reference counts for all owned frames.
    }
}
