//! This module defines Memory Address structures.

use super::frame::PAGE_SHIFT;

const PAGE_OFFSET_MASK: u32 = (1u32 << PAGE_SHIFT) - 1;

/// Linear address after segment translation.
///
/// In i386 architecture: `Logical Address --[Segmentation]--> Linear Address --[Paging]--> Physical Address`
///
/// In our kernel, process `n` is assigned linear address space `[n * 64MB, (n + 1) * 64MB)`.
/// This means each process has a 64MB linear address space, and the kernel can support
/// up to 64 processes (64 * 64MB = 4GB total linear address space).
///
/// # Linear Address Structure (32-bit, 4KB pages)
///
/// ```text
///  31                22 21                12 11                 0
/// +--------------------+--------------------+--------------------+
/// | Page Directory Idx |   Page Table Idx   |    Page Offset     |
/// +--------------------+--------------------+--------------------+
///        10 bits             10 bits              12 bits
/// ```
///
/// - **Page Directory Index**: Selects one of 1024 entries in the page directory
/// - **Page Table Index**: Selects one of 1024 entries in the selected page table
/// - **Page Offset**: Byte offset within the 4KB page
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct LinAddr(pub u32);

/// Physical address.
///
/// The physical address is composed of two parts:
/// - **High 20 bits**: Physical Page Number (PPN) from the Page Table Entry (PTE),
///   obtained after segment translation and page table lookup.
/// - **Low 12 bits**: Page Offset, copied directly from the original linear address.
///
/// # Physical Address Composition
///
/// ```text
///  31                                    12 11                 0
/// +----------------------------------------+--------------------+
/// |    Physical Page Number (from PTE)     |    Page Offset     |
/// +----------------------------------------+--------------------+
///              20 bits                           12 bits
///         (from page table)              (from linear address)
/// ```
///
/// # Translation Process
///
/// ```text
///    31      22 21      12 11        0
///   ┌─────────┬──────────┬───────────┐
///   │ Dir Idx │ Tbl Idx  │  Offset   │ Linear Address
///   └────┬────┴────┬─────┴─────┬─────┘
///        │         │           │
///        ▼         ▼           │
/// CR3─►[Page Dir]─►[Page Tbl]──┼──►[Page Frame]
///        │              │      │         │
///        └───►PDE───────┘      │         ▼
///              └────►PTE───────┴───►Physical Addr
///                    (PPN)        (PPN:20|Offset:12)
/// ```
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PhysAddr(pub u32);

/// Physical page number (PPN).
///
/// Physical address is 32 bits, and memory is divided into 4KB pages.
/// Therefore, the high 20 bits represent the physical page number,
/// and the low 12 bits represent the offset within the page.
///
/// ```text
///  31                 12 11          0
/// +---------------------+-------------+
/// │ Physical Page Number│ Page Offset │
/// +---------------------+-------------+
///        20 bits            12 bits
/// ```
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PhysPageNum(pub u32);

/// Linear page number (LPN).
///
/// Similar to [`PhysPageNum`], but for linear address.
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct LinPageNum(pub u32);

impl From<u32> for LinAddr {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<u32> for PhysAddr {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<u32> for PhysPageNum {
    fn from(value: u32) -> Self {
        const PPN_WIDTH: u32 = 20;
        Self(value & ((1 << PPN_WIDTH) - 1))
    }
}

impl From<u32> for LinPageNum {
    fn from(value: u32) -> Self {
        const LPN_WIDTH: u32 = 20;
        Self(value & ((1 << LPN_WIDTH) - 1))
    }
}

impl From<PhysAddr> for PhysPageNum {
    fn from(value: PhysAddr) -> Self {
        assert_eq!(value.page_offset(), 0);
        value.floor()
    }
}

impl From<LinAddr> for LinPageNum {
    fn from(value: LinAddr) -> Self {
        value.floor()
    }
}

impl From<LinPageNum> for LinAddr {
    fn from(value: LinPageNum) -> Self {
        value.addr()
    }
}

impl From<PhysPageNum> for PhysAddr {
    fn from(value: PhysPageNum) -> Self {
        value.addr()
    }
}

impl LinAddr {
    /// Return this linear address as raw `u32`.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Round this linear address down to the nearest page boundary.
    pub fn align_down(self) -> Self {
        Self(self.0 & !PAGE_OFFSET_MASK)
    }

    /// Return the linear page number containing this address.
    pub fn floor(self) -> LinPageNum {
        LinPageNum(self.0 >> PAGE_SHIFT)
    }

    /// Return the byte offset within the 4KB page.
    pub fn page_offset(self) -> u32 {
        self.0 & PAGE_OFFSET_MASK
    }

    /// Return the page-directory index (top 10 bits).
    pub fn pde_index(self) -> usize {
        ((self.0 >> 22) & 0x3FF) as usize
    }

    /// Return the page-table index (middle 10 bits).
    pub fn pte_index(self) -> usize {
        ((self.0 >> 12) & 0x3FF) as usize
    }
}

impl PhysAddr {
    /// Return this physical address as raw `u32`.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    pub fn floor(&self) -> PhysPageNum {
        PhysPageNum(self.0 >> PAGE_SHIFT)
    }

    pub fn page_offset(&self) -> u32 {
        self.0 & PAGE_OFFSET_MASK
    }

    /// View this physical address as a typed const pointer.
    pub fn as_ptr<T>(self) -> *const T {
        self.0 as *const T
    }

    /// View this physical address as a typed mut pointer.
    pub fn as_mut_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }
}

impl PhysPageNum {
    /// Return this physical page number as raw `u32`.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Convert this page number to its page-aligned physical address.
    pub fn addr(self) -> PhysAddr {
        PhysAddr(self.0 << PAGE_SHIFT)
    }
}

impl LinPageNum {
    /// Return this linear page number as raw `u32`.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Build a linear page number from `(pde_index, pte_index)`.
    pub fn from_indices(pde_index: usize, pte_index: usize) -> Self {
        debug_assert!(pde_index < 1024);
        debug_assert!(pte_index < 1024);
        Self(((pde_index as u32) << 10) | pte_index as u32)
    }

    /// Return the global page-directory index of this linear page.
    pub fn pde_index(self) -> usize {
        (self.0 >> 10) as usize
    }

    /// Return the page-table index within the selected page directory entry.
    pub fn pte_index(self) -> usize {
        (self.0 & 0x3FF) as usize
    }

    /// Convert this linear page number to the page-aligned linear address.
    pub fn addr(self) -> LinAddr {
        LinAddr(self.0 << PAGE_SHIFT)
    }
}
