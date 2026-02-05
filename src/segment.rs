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
