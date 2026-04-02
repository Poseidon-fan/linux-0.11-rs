//! Segment selector types and well-known kernel selectors.

use crate::task::{FIRST_LDT_ENTRY, FIRST_TSS_ENTRY};

/// A 16-bit segment selector for indexing into the GDT or LDT.
///
/// ```text
/// 15                                3   2   1   0
/// ┌─────────────────────────────────┬───┬───────┐
/// │           Index (13-bit)        │TI │  RPL  │
/// └─────────────────────────────────┴───┴───────┘
/// ```
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct SegmentSelector(u16);

impl SegmentSelector {
    /// Creates a GDT selector with the given index and privilege level.
    const fn gdt(index: u16, rpl: u16) -> Self {
        Self((index << 3) | (rpl & 0x3))
    }

    /// Creates an LDT selector with the given index and privilege level.
    const fn ldt(index: u16, rpl: u16) -> Self {
        Self((index << 3) | (1 << 2) | (rpl & 0x3))
    }

    /// Returns the raw 16-bit value.
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Returns the raw value as `u32` (for TSS fields).
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }
}

/// Kernel code segment (GDT index 1, Ring 0) = `0x08`.
pub const KERNEL_CS: SegmentSelector = SegmentSelector::gdt(1, 0);

/// Kernel data segment (GDT index 2, Ring 0) = `0x10`.
pub const KERNEL_DS: SegmentSelector = SegmentSelector::gdt(2, 0);

/// User code segment (LDT index 1, Ring 3) = `0x0f`.
pub const USER_CS: SegmentSelector = SegmentSelector::ldt(1, 3);

/// User data segment (LDT index 2, Ring 3) = `0x17`.
pub const USER_DS: SegmentSelector = SegmentSelector::ldt(2, 3);

/// Returns the TSS selector for task `n` (GDT index `4 + n*2`).
pub const fn tss_selector(n: u16) -> SegmentSelector {
    SegmentSelector::gdt(FIRST_TSS_ENTRY + n * 2, 0)
}

/// Returns the LDT descriptor selector for task `n` (GDT index `5 + n*2`).
pub const fn ldt_selector(n: u16) -> SegmentSelector {
    SegmentSelector::gdt(FIRST_LDT_ENTRY + n * 2, 0)
}
