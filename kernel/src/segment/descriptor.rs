//! 64-bit segment descriptor for GDT and LDT entries.

/// A 64-bit segment descriptor for the GDT or LDT.
///
/// ```text
/// Byte:  7        6        5        4        3        2        1        0
///     ┌────────┬────────┬────────┬────────┬────────┬────────┬────────┬────────┐
///     │Base    │Flags:4 │Access  │Base    │  Base [15:0]    │   Limit [15:0]  │
///     │[31:24] │Lim:4   │  Byte  │[23:16] │                 │                 │
///     └────────┴────────┴────────┴────────┴────────┴────────┴────────┴────────┘
/// ```
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Descriptor(u64);

impl Descriptor {
    /// Builds a descriptor from `base`, 20-bit `limit`, access byte, and flags nibble.
    const fn new(base: u32, limit: u32, access: u8, flags: u8) -> Self {
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

    /// Creates a null descriptor.
    pub const fn null() -> Self {
        Self(0)
    }

    /// Creates a TSS descriptor (access=0x89, flags=0x0).
    pub const fn tss(base: u32, limit: u32) -> Self {
        Self::new(base, limit, 0x89, 0x0)
    }

    /// Creates an LDT descriptor (access=0x82, flags=0x0).
    pub const fn ldt(base: u32, limit: u32) -> Self {
        Self::new(base, limit, 0x82, 0x0)
    }

    /// Creates a user code segment descriptor (access=0xFA, flags=0xC).
    pub const fn user_code(base: u32, limit: u32) -> Self {
        Self::new(base, limit, 0xFA, 0xC)
    }

    /// Creates a user data segment descriptor (access=0xF2, flags=0xC).
    pub const fn user_data(base: u32, limit: u32) -> Self {
        Self::new(base, limit, 0xF2, 0xC)
    }

    /// Extracts the 32-bit base address.
    pub const fn base(self) -> u32 {
        let lo = ((self.0 >> 16) & 0xFFFF) as u32;
        let mid = ((self.0 >> 32) & 0xFF) as u32;
        let hi = ((self.0 >> 56) & 0xFF) as u32;
        lo | (mid << 16) | (hi << 24)
    }

    /// Returns the byte-granular segment limit.
    ///
    /// If the G bit is set, the raw 20-bit limit is scaled by 4 KiB.
    pub const fn byte_limit(self) -> u32 {
        let lo = (self.0 & 0xFFFF) as u32;
        let hi = ((self.0 >> 48) & 0xF) as u32;
        let raw = lo | (hi << 16);
        let g_bit = ((self.0 >> 55) & 1) as u32;
        if g_bit == 1 { (raw << 12) | 0xFFF } else { raw }
    }

    /// Returns a new descriptor with the base address changed.
    pub const fn with_base(self, base: u32) -> Self {
        let base_low = (base & 0xFFFF) as u64;
        let base_mid = ((base >> 16) & 0xFF) as u64;
        let base_high = ((base >> 24) & 0xFF) as u64;

        let cleared = self.0 & !(0xFFFF << 16) & !(0xFF << 32) & !(0xFF << 56);

        Self(cleared | (base_low << 16) | (base_mid << 32) | (base_high << 56))
    }

    /// Returns a new descriptor with the 20-bit limit changed.
    pub const fn with_limit(self, limit: u32) -> Self {
        let limit_low = (limit & 0xFFFF) as u64;
        let limit_high = ((limit >> 16) & 0xF) as u64;

        let cleared = self.0 & !(0xFFFF_u64) & !(0xF_u64 << 48);

        Self(cleared | limit_low | (limit_high << 48))
    }

    /// Returns the raw 64-bit value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}
