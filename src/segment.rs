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
        asm!("ltr {0:x}", in(reg) selector.as_u16(), options(nomem, nostack, att_syntax));
    }
}

/// Load LDT Register with LDT descriptor selector.
///
/// This tells the CPU where to find the current task's LDT.
#[inline]
pub fn lldt(selector: SegmentSelector) {
    unsafe {
        asm!("lldt {0:x}", in(reg) selector.as_u16(), options(nomem, nostack, att_syntax));
    }
}

/// Get the current Task Register value.
#[inline]
pub fn str() -> SegmentSelector {
    let selector: u16;
    unsafe {
        asm!("str {0:x}", out(reg) selector, options(nomem, nostack, att_syntax));
    }
    SegmentSelector(selector)
}

// ============================================================================
// FS/DS segment access
// ============================================================================
//
// These functions read/write memory through the FS or DS segment selector.
// Typically used for kernel-to-user data copy when FS is set to user segment.

/// Read a byte from the address through the FS segment.
///
/// # Safety
///
/// `addr` must be a valid pointer within the current FS segment.
#[inline]
pub fn get_fs_byte(addr: *const u8) -> u8 {
    let v: u8;
    unsafe {
        asm!(
            "movb %fs:({}), {}",
            in(reg) addr as u32,
            out(reg_byte) v,
            options(nomem, nostack, att_syntax)
        );
    }
    v
}

/// Read a word (16-bit) from the address through the FS segment.
///
/// # Safety
///
/// `addr` must be a valid pointer within the current FS segment.
#[inline]
pub fn get_fs_word(addr: *const u16) -> u16 {
    let v: u16;
    unsafe {
        asm!(
            "movw %fs:({}), {1:x}",
            in(reg) addr as u32,
            out(reg) v,
            options(nomem, nostack, att_syntax)
        );
    }
    v
}

/// Read a long (32-bit) from the address through the FS segment.
///
/// # Safety
///
/// `addr` must be a valid pointer within the current FS segment.
#[inline]
pub fn get_fs_long(addr: *const u32) -> u32 {
    let v: u32;
    unsafe {
        asm!(
            "movl %fs:({}), {}",
            in(reg) addr as u32,
            out(reg) v,
            options(nomem, nostack, att_syntax)
        );
    }
    v
}

/// Write a byte to the address through the FS segment.
///
/// # Safety
///
/// `addr` must be a valid pointer within the current FS segment.
#[inline]
pub fn put_fs_byte(val: u8, addr: *mut u8) {
    unsafe {
        asm!(
            "movb {}, %fs:({})",
            in(reg_byte) val,
            in(reg) addr as u32,
            options(nomem, nostack, att_syntax)
        );
    }
}

/// Write a word (16-bit) to the address through the FS segment.
///
/// # Safety
///
/// `addr` must be a valid pointer within the current FS segment.
#[inline]
pub fn put_fs_word(val: u16, addr: *mut u16) {
    unsafe {
        asm!(
            "movw {0:x}, %fs:({1})",
            in(reg) val,
            in(reg) addr as u32,
            options(nomem, nostack, att_syntax)
        );
    }
}

/// Write a long (32-bit) to the address through the FS segment.
///
/// # Safety
///
/// `addr` must be a valid pointer within the current FS segment.
#[inline]
pub fn put_fs_long(val: u32, addr: *mut u32) {
    unsafe {
        asm!(
            "movl {}, %fs:({})",
            in(reg) val,
            in(reg) addr as u32,
            options(nomem, nostack, att_syntax)
        );
    }
}

/// Get the current FS segment selector value.
#[inline]
pub fn get_fs() -> u16 {
    let v: u16;
    unsafe {
        asm!("mov %fs, %ax", out("ax") v, options(nomem, nostack, att_syntax));
    }
    v
}

/// Get the current DS segment selector value.
#[inline]
pub fn get_ds() -> u16 {
    let v: u16;
    unsafe {
        asm!("mov %ds, %ax", out("ax") v, options(nomem, nostack, att_syntax));
    }
    v
}

/// Set the FS segment selector.
#[inline]
pub fn set_fs(val: u16) {
    unsafe {
        asm!("mov {0:x}, %fs", in(reg) val, options(nomem, nostack, att_syntax));
    }
}

/// First TSS entry index in GDT.
pub const FIRST_TSS_ENTRY: u16 = 4;

/// First LDT descriptor entry index in GDT.
pub const FIRST_LDT_ENTRY: u16 = 5;

// ============================================================================
// Descriptor operations
// ============================================================================

unsafe extern "C" {
    /// GDT defined in head.s
    static mut gdt: [u64; 256];
}

/// x86 Segment Descriptor (64-bit).
///
/// A unified type for all segment descriptors stored in the GDT or LDT.
/// Different descriptor kinds (TSS, LDT, user code/data) share the same
/// binary layout and differ only in the access byte and flags nibble.
///
/// # Byte layout
///
/// ```text
/// Byte:  7        6        5        4        3        2        1        0
///     ┌────────┬────────┬────────┬────────┬────────┬────────┬────────┬────────┐
///     │Base    │Flags:4 │Access  │Base    │  Base [15:0]    │   Limit [15:0]  │
///     │[31:24] │Lim:4   │  Byte  │[23:16] │                 │                 │
///     └────────┴────────┴────────┴────────┴────────┴────────┴────────┴────────┘
///
///   Access byte (byte 5):  P(1) DPL(2) S(1) Type(4)
///   Flags nibble (byte 6 high):  G(1) D/B(1) L(1) AVL(1)
/// ```
///
/// # Descriptor kinds
///
/// | Kind            | Where  | Access | Flags | S bit |
/// |-----------------|--------|--------|-------|-------|
/// | TSS descriptor  | GDT    | 0x89   | 0x0   | 0     |
/// | LDT descriptor  | GDT    | 0x82   | 0x0   | 0     |
/// | User code seg   | LDT    | 0xFA   | 0xC   | 1     |
/// | User data seg   | LDT    | 0xF2   | 0xC   | 1     |
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Descriptor(u64);

impl Descriptor {
    /// Create an empty (null) descriptor.
    pub const fn null() -> Self {
        Self(0)
    }

    /// Build a segment descriptor from base, limit, access byte, and flags nibble.
    ///
    /// # Arguments
    /// - `base`:   32-bit segment base address
    /// - `limit`:  20-bit segment limit
    /// - `access`: Access byte (P, DPL, S, Type)
    /// - `flags`:  High nibble of byte 6 (G, D/B, L, AVL)
    pub const fn new(base: u32, limit: u32, access: u8, flags: u8) -> Self {
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
                | ((flags as u64 & 0xF) << 52)
                | (base_high << 56),
        )
    }

    /// Create a TSS (Task State Segment) descriptor.
    ///
    /// Access byte = 0x89: Present, DPL=0, System, Type=9 (Available 32-bit TSS).
    /// Flags = 0 (byte granularity).
    pub const fn tss(base: u32, limit: u32) -> Self {
        Self::new(base, limit, 0x89, 0x0)
    }

    /// Create an LDT (Local Descriptor Table) descriptor.
    ///
    /// Access byte = 0x82: Present, DPL=0, System, Type=2 (LDT).
    /// Flags = 0 (byte granularity).
    pub const fn ldt(base: u32, limit: u32) -> Self {
        Self::new(base, limit, 0x82, 0x0)
    }

    /// Create a user code segment descriptor.
    ///
    /// Access = 0xFA: Present, DPL=3, non-system, Code, Execute/Read.
    /// Flags  = 0xC:  4KB granularity, 32-bit operand size.
    pub const fn user_code(base: u32, limit: u32) -> Self {
        Self::new(base, limit, 0xFA, 0xC)
    }

    /// Create a user data segment descriptor.
    ///
    /// Access = 0xF2: Present, DPL=3, non-system, Data, Read/Write.
    /// Flags  = 0xC:  4KB granularity, 32-bit operand size.
    pub const fn user_data(base: u32, limit: u32) -> Self {
        Self::new(base, limit, 0xF2, 0xC)
    }

    /// Extract the 32-bit base address from this descriptor.
    ///
    /// Reassembles the base from the three fields scattered across the
    /// descriptor (see byte layout diagram above).
    pub const fn base(self) -> u32 {
        let lo = ((self.0 >> 16) & 0xFFFF) as u32; // byte[2..3]
        let mid = ((self.0 >> 32) & 0xFF) as u32; // byte[4]
        let hi = ((self.0 >> 56) & 0xFF) as u32; // byte[7]
        lo | (mid << 16) | (hi << 24)
    }

    /// Extract the 20-bit segment limit from this descriptor.
    ///
    /// Returns the raw limit value (before granularity scaling).
    const fn limit(self) -> u32 {
        let lo = (self.0 & 0xFFFF) as u32; // byte[0..1]
        let hi = ((self.0 >> 48) & 0xF) as u32; // byte[6] low nibble
        lo | (hi << 16)
    }

    /// Get the byte-granular segment limit (maximum valid offset).
    ///
    /// If the G (Granularity) bit is set (byte 6, bit 7), the raw 20-bit
    /// limit is scaled by 4KB: `(raw_limit << 12) | 0xFFF`.
    /// Otherwise the raw limit is returned as-is.
    ///
    /// Note: this returns the *maximum offset*, not the segment *size*.
    /// To get the size, add 1 to the result.
    pub const fn byte_limit(self) -> u32 {
        let raw = self.limit();
        // G bit is bit 55 of the descriptor (byte 6, bit 7).
        let g_bit = ((self.0 >> 55) & 1) as u32;
        if g_bit == 1 { (raw << 12) | 0xFFF } else { raw }
    }

    /// Return a new descriptor with the base address changed.
    ///
    /// All other fields (limit, access, flags) are preserved.
    pub const fn with_base(self, base: u32) -> Self {
        let base_low = (base & 0xFFFF) as u64;
        let base_mid = ((base >> 16) & 0xFF) as u64;
        let base_high = ((base >> 24) & 0xFF) as u64;

        // Clear old base bits, then set new ones.
        let cleared = self.0
            & !(0xFFFF << 16)       // clear base[15:0]  in byte[2..3]
            & !(0xFF << 32)         // clear base[23:16] in byte[4]
            & !(0xFF << 56); // clear base[31:24] in byte[7]

        Self(cleared | (base_low << 16) | (base_mid << 32) | (base_high << 56))
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
