//! Page table and page directory operations for i386 two-level paging.
//!
//! The kernel uses a single shared page directory at physical address 0.
//! Each process occupies 16 consecutive PDEs (64MB / 4MB per PDE = 16).
//!
//! This module provides:
//!
//! - Type-safe [`PageTableEntry`] and [`PageDirectoryEntry`] wrappers
//! - [`PageTable`]: an owned page table backed by a physical frame
//! - Page directory read/write helpers ([`read_pde`], [`write_pde`])
//! - TLB invalidation ([`invalidate_tlb`])

use core::arch::asm;

use bitflags::bitflags;

use crate::mm::{
    address::PhysPageNum,
    frame::{self, PhysFrame},
};

bitflags! {
    /// Page table / directory entry flags (low 12 bits of a PTE/PDE).
    pub struct PageFlags: u32 {
        const PRESENT       = 1 << 0;
        const WRITABLE      = 1 << 1;
        const USER          = 1 << 2;
        const WRITE_THROUGH = 1 << 3;
        const CACHE_DISABLE = 1 << 4;
        const ACCESSED      = 1 << 5;
        const DIRTY         = 1 << 6;
        const HUGE_PAGE     = 1 << 7;
        const GLOBAL        = 1 << 8;
    }
}

/// Number of entries in a page table (and in the page directory).
pub const ENTRIES_PER_TABLE: usize = 1024;

// ============================================================================
// Page Entry types
// ============================================================================

/// Common interface for page table entries and page directory entries.
///
/// Both PTE and PDE share the same 32-bit layout:
///
/// ```text
///  31                          12 11          0
/// +------------------------------+------------+
/// |   Physical Page Number (20)  | Flags (12) |
/// +------------------------------+------------+
/// ```
pub trait PageEntry: From<u32> + Into<u32> + Copy {
    /// Create an entry from a physical page number and flags.
    fn new(ppn: PhysPageNum, flags: PageFlags) -> Self {
        ((ppn.0 << 12) | flags.bits()).into()
    }

    /// Check whether the PRESENT flag is set.
    fn is_present(self) -> bool {
        let raw: u32 = self.into();
        raw & PageFlags::PRESENT.bits() != 0
    }

    /// Extract the physical page number.
    fn ppn(self) -> PhysPageNum {
        let raw: u32 = self.into();
        PhysPageNum(raw >> 12)
    }

    /// Extract the physical address (page-aligned, i.e. PPN << 12).
    fn phys_addr(self) -> u32 {
        let raw: u32 = self.into();
        raw & 0xFFFFF000
    }

    /// Extract the flags.
    fn flags(self) -> PageFlags {
        let raw: u32 = self.into();
        PageFlags::from_bits_truncate(raw)
    }

    /// Return a copy with the WRITABLE flag cleared.
    fn without_writable(self) -> Self {
        let raw: u32 = self.into();
        (raw & !PageFlags::WRITABLE.bits()).into()
    }
}

/// Page Table Entry (PTE) — maps a 4KB page.
#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct PageTableEntry(u32);

/// Page Directory Entry (PDE) — points to a page table.
#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct PageDirectoryEntry(u32);

impl PageDirectoryEntry {
    /// Create a PDE pointing to a page table with User + Writable + Present flags.
    pub fn user_page_table(page_table_addr: u32) -> Self {
        let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER;
        Self(page_table_addr | flags.bits())
    }

    /// Create a null (non-present) PDE.
    pub const fn empty() -> Self {
        Self(0)
    }
}

impl From<u32> for PageTableEntry {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<PageTableEntry> for u32 {
    fn from(val: PageTableEntry) -> Self {
        val.0
    }
}

impl From<u32> for PageDirectoryEntry {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<PageDirectoryEntry> for u32 {
    fn from(val: PageDirectoryEntry) -> Self {
        val.0
    }
}

impl PageEntry for PageTableEntry {}
impl PageEntry for PageDirectoryEntry {}

// ============================================================================
// Page Table
// ============================================================================

/// An owned page table backed by a physical frame (4KB = 1024 × 4-byte PTEs).
///
/// When dropped, the underlying [`PhysFrame`] is released (reference count
/// decremented).  The caller is responsible for clearing the corresponding
/// PDE in the page directory *before* dropping this struct.
pub struct PageTable {
    frame: PhysFrame,
}

impl PageTable {
    /// Allocate a new, zeroed page table.
    ///
    /// Returns `None` if no physical frame is available.
    pub fn new() -> Option<Self> {
        Some(Self {
            frame: frame::alloc()?,
        })
    }

    /// Physical address of this page table (for writing into a PDE).
    pub fn phys_addr(&self) -> u32 {
        self.frame.ppn.0 << 12
    }

    /// Interpret the underlying frame as an array of 1024 page table entries.
    pub fn as_pte_array(&self) -> &[PageTableEntry; ENTRIES_PER_TABLE] {
        unsafe { &*(self.phys_addr() as *const [PageTableEntry; ENTRIES_PER_TABLE]) }
    }

    /// Mutable version of [`as_pte_array`](Self::as_pte_array).
    pub fn as_pte_array_mut(&mut self) -> &mut [PageTableEntry; ENTRIES_PER_TABLE] {
        unsafe { &mut *(self.phys_addr() as *mut [PageTableEntry; ENTRIES_PER_TABLE]) }
    }
}

// ============================================================================
// Page Directory helpers
// ============================================================================
//
// The page directory lives at physical address 0 (identity-mapped by head.s).
// We access it via raw volatile pointers.

/// Read a page directory entry by index (0..1023).
#[inline]
pub fn read_pde(index: usize) -> PageDirectoryEntry {
    debug_assert!(index < ENTRIES_PER_TABLE);
    let ptr = (index * 4) as *const u32;
    let raw = unsafe { core::ptr::read_volatile(ptr) };
    PageDirectoryEntry::from(raw)
}

/// Write a page directory entry by index (0..1023).
#[inline]
pub fn write_pde(index: usize, pde: PageDirectoryEntry) {
    debug_assert!(index < ENTRIES_PER_TABLE);
    let ptr = (index * 4) as *mut u32;
    unsafe { core::ptr::write_volatile(ptr, pde.into()) }
}

/// Flush the entire TLB by reloading CR3 with 0.
#[inline]
pub fn invalidate_tlb() {
    unsafe {
        asm!("movl %eax, %cr3", in("eax") 0u32, options(att_syntax));
    }
}
