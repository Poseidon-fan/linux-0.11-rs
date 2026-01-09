//! This module defines Memory Address structures.

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
//  31                22 21                12 11                 0
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
pub struct LinearAddr(pub u32);
