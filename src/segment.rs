//! x86 Segmentation utilities.
//!
//! This module provides type-safe abstractions for x86 segment selectors
//! and operations on GDT (Global Descriptor Table) and LDT (Local Descriptor Table).

use core::arch::asm;

/// Segment Selector - 16-bit value for selecting a segment descriptor.
///
/// In x86 protected mode, segment selectors are used to index into the
/// GDT (Global Descriptor Table) or LDT (Local Descriptor Table).
///
/// # Format
///
/// ```text
/// 15                                 3   2   1   0
/// ┌─────────────────────────────────┬───┬───────┐
/// │           Index (13-bit)         │TI │  RPL  │
/// └─────────────────────────────────┴───┴───────┘
/// ```
///
/// - **Index**: Descriptor index in the table (0-8191)
/// - **TI**: Table Indicator (0=GDT, 1=LDT)
/// - **RPL**: Requested Privilege Level (0-3)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct SegmentSelector(u16);

impl SegmentSelector {
    /// Create a new segment selector.
    ///
    /// # Arguments
    /// - `index`: Descriptor index in GDT/LDT (0-8191)
    /// - `ti`: Table Indicator (false=GDT, true=LDT)
    /// - `rpl`: Requested Privilege Level (0-3)
    const fn new(index: u16, ti: bool, rpl: PrivilegeLevel) -> Self {
        Self((index << 3) | ((ti as u16) << 2) | (rpl as u16))
    }

    /// Create selector for GDT entry at given index with specified privilege.
    pub const fn gdt(index: u16, rpl: PrivilegeLevel) -> Self {
        Self::new(index, false, rpl)
    }

    /// Create selector for LDT entry at given index with specified privilege.
    pub const fn ldt(index: u16, rpl: PrivilegeLevel) -> Self {
        Self::new(index, true, rpl)
    }

    /// Get the raw 16-bit selector value.
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Get the raw value as u32 (for TSS fields that use 32-bit storage).
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }

    /// Get the descriptor index.
    pub const fn index(self) -> u16 {
        self.0 >> 3
    }

    /// Check if this selector refers to LDT (vs GDT).
    pub const fn is_ldt(self) -> bool {
        (self.0 & 0b100) != 0
    }

    /// Get the RPL (Requested Privilege Level).
    pub const fn rpl(self) -> PrivilegeLevel {
        match self.0 & 0b11 {
            0 => PrivilegeLevel::Ring0,
            1 => PrivilegeLevel::Ring1,
            2 => PrivilegeLevel::Ring2,
            _ => PrivilegeLevel::Ring3,
        }
    }
}

/// CPU Privilege Level (Ring 0-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PrivilegeLevel {
    /// Ring 0 - Kernel mode (highest privilege)
    Ring0 = 0,
    /// Ring 1 - Unused
    Ring1 = 1,
    /// Ring 2 - Unused
    Ring2 = 2,
    /// Ring 3 - User mode (lowest privilege)
    Ring3 = 3,
}

/// Well-known segment selectors used in the kernel.
///
/// # GDT Layout
///
/// ```text
/// Index  Entry          Selector
/// ─────────────────────────────────
///   0    NULL           0x00
///   1    Kernel CS      0x08
///   2    Kernel DS      0x10
///   3    Syscall        0x18
///   4    TSS0           0x20
///   5    LDT0           0x28
///   6    TSS1           0x30
///   7    LDT1           0x38
///   ...  ...            ...
/// ```
pub mod selectors {
    use super::*;

    /// Kernel code segment (GDT[1], Ring 0) = 0x08
    pub const KERNEL_CS: SegmentSelector = SegmentSelector::gdt(1, PrivilegeLevel::Ring0);

    /// Kernel data segment (GDT[2], Ring 0) = 0x10
    pub const KERNEL_DS: SegmentSelector = SegmentSelector::gdt(2, PrivilegeLevel::Ring0);

    /// User code segment (LDT[1], Ring 3) = 0x0f
    pub const USER_CS: SegmentSelector = SegmentSelector::ldt(1, PrivilegeLevel::Ring3);

    /// User data segment (LDT[2], Ring 3) = 0x17
    pub const USER_DS: SegmentSelector = SegmentSelector::ldt(2, PrivilegeLevel::Ring3);

    /// Get TSS selector for task n.
    ///
    /// Each task occupies 2 GDT entries (TSS and LDT descriptor).
    /// TSS for task n is at GDT[4 + n*2].
    pub const fn tss_selector(n: u16) -> SegmentSelector {
        SegmentSelector::gdt(super::FIRST_TSS_ENTRY + n * 2, PrivilegeLevel::Ring0)
    }

    /// Get LDT descriptor selector for task n (stored in GDT).
    ///
    /// LDT descriptor for task n is at GDT[5 + n*2].
    pub const fn ldt_selector(n: u16) -> SegmentSelector {
        SegmentSelector::gdt(super::FIRST_LDT_ENTRY + n * 2, PrivilegeLevel::Ring0)
    }
}

/// Load Task Register with TSS selector.
///
/// This tells the CPU where to find the current task's TSS.
#[inline]
pub fn ltr(selector: SegmentSelector) {
    unsafe {
        asm!("ltr {0:x}", in(reg) selector.as_u16(), options(nomem, nostack));
    }
}

/// Load LDT Register with LDT descriptor selector.
///
/// This tells the CPU where to find the current task's LDT.
#[inline]
pub fn lldt(selector: SegmentSelector) {
    unsafe {
        asm!("lldt {0:x}", in(reg) selector.as_u16(), options(nomem, nostack));
    }
}

/// Get the current Task Register value.
#[inline]
pub fn str() -> SegmentSelector {
    let selector: u16;
    unsafe {
        asm!("str {0:x}", out(reg) selector, options(nomem, nostack));
    }
    SegmentSelector(selector)
}

/// First TSS entry index in GDT.
pub const FIRST_TSS_ENTRY: u16 = 4;

/// First LDT descriptor entry index in GDT.
pub const FIRST_LDT_ENTRY: u16 = 5;

/// Get the current task number from Task Register.
///
/// Calculates which task is current based on the TSS selector in TR.
#[inline]
pub fn current_task_nr() -> u16 {
    let selector = str();
    (selector.index() - FIRST_TSS_ENTRY) >> 1
}

// ============================================================================
// Descriptor operations
// ============================================================================

unsafe extern "C" {
    /// GDT defined in head.s
    static mut gdt: [u64; 256];
}

/// x86 System Segment Descriptor (64-bit).
///
/// # Format
///
/// ```text
/// 63       56 55   52 51   48 47       40 39       32
/// ┌──────────┬───────┬───────┬──────────┬──────────┐
/// │ Base     │ Flags │ Limit │ Access   │ Base     │
/// │ [31:24]  │ G D 0 │[19:16]│ P DPL S T│ [23:16]  │
/// └──────────┴───────┴───────┴──────────┴──────────┘
/// 31                  16 15                       0
/// ┌─────────────────────┬─────────────────────────┐
/// │   Base [15:0]       │     Limit [15:0]        │
/// └─────────────────────┴─────────────────────────┘
/// ```
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Descriptor(u64);

impl Descriptor {
    /// Create an empty (null) descriptor.
    pub const fn null() -> Self {
        Self(0)
    }

    /// Create a system segment descriptor (TSS or LDT).
    ///
    /// # Arguments
    /// - `base`: Linear address of the segment
    /// - `limit`: Segment limit (size - 1), up to 20 bits
    /// - `access`: Access byte (type, DPL, present bit, etc.)
    const fn system_segment(base: u32, limit: u32, access: u8) -> Self {
        let limit_low = (limit & 0xFFFF) as u64;
        let limit_high = ((limit >> 16) & 0xF) as u64;
        let base_low = (base & 0xFFFF) as u64;
        let base_mid = ((base >> 16) & 0xFF) as u64;
        let base_high = ((base >> 24) & 0xFF) as u64;

        Self(
            limit_low
                | (base_low << 16)
                | (base_mid << 32)
                | ((access as u64) << 40)
                | (limit_high << 48)
                // flags = 0 (byte granularity, 16-bit for system segments)
                | (base_high << 56),
        )
    }

    /// Create a TSS (Task State Segment) descriptor.
    ///
    /// Access byte = 0x89: Present, DPL=0, System, Type=9 (Available 32-bit TSS)
    pub const fn tss(base: u32, limit: u32) -> Self {
        Self::system_segment(base, limit, 0x89)
    }

    /// Create an LDT (Local Descriptor Table) descriptor.
    ///
    /// Access byte = 0x82: Present, DPL=0, System, Type=2 (LDT)
    pub const fn ldt(base: u32, limit: u32) -> Self {
        Self::system_segment(base, limit, 0x82)
    }

    /// Get the raw 64-bit value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Size of TSS structure in Linux 0.11 (104 bytes).
/// Used as the limit for TSS descriptors.
pub const TSS_SIZE: u32 = 104;

/// Set TSS descriptor for task n in GDT.
///
/// Writes a TSS descriptor at GDT[4 + n*2] pointing to the given TSS address.
///
/// # Safety
///
/// - `tss_addr` must point to a valid TSS structure
/// - Must not be called concurrently with other GDT modifications
#[inline]
pub fn set_tss_desc(n: u16, tss_addr: u32) {
    let index = (FIRST_TSS_ENTRY + n * 2) as usize;
    let desc = Descriptor::tss(tss_addr, TSS_SIZE);
    unsafe {
        core::ptr::write_volatile(&mut gdt[index], desc.as_u64());
    }
}

/// Set LDT descriptor for task n in GDT.
///
/// Writes an LDT descriptor at GDT[5 + n*2] pointing to the given LDT address.
///
/// # Safety
///
/// - `ldt_addr` must point to a valid LDT structure
/// - Must not be called concurrently with other GDT modifications
///
/// # Note
///
/// This implementation uses the correct limit (23) for LDT with 3 entries.
/// The original Linux 0.11 has a bug where `_set_tssldt_desc` hardcodes
/// limit=104 for both TSS and LDT, which is incorrect for LDT but harmless.
#[inline]
pub fn set_ldt_desc(n: u16, ldt_addr: u32) {
    let index = (FIRST_LDT_ENTRY + n * 2) as usize;
    // LDT has 3 entries (null, cs, ds), each 8 bytes = 24 bytes, limit = 24 - 1 = 23 = 0x17
    let desc = Descriptor::ldt(ldt_addr, 3 * 8 - 1);
    unsafe {
        core::ptr::write_volatile(&mut gdt[index], desc.as_u64());
    }
}
