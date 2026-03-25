//! Filesystem allocation bitmap helpers.
//!
//! Each bitmap consists of up to `N` cached blocks. Bit 0 maps to `bit0_id`,
//! and allocation scans those bits in ascending order.

use core::array;

use alloc::sync::Arc;

use crate::fs::{BLOCK_SIZE, buffer::BufferHandle, layout::BitmapBlock};

const BLOCK_BITS: usize = BLOCK_SIZE * 8;
const WORD_BITS: usize = u64::BITS as usize;

/// Filesystem allocation bitmap cached in a fixed number of block slots.
pub struct Bitmap<const N: usize> {
    buffers: [Option<Arc<BufferHandle>>; N],
    bit0_id: u32,
    bit_count: usize,
    first_allocatable_bit: usize,
}

impl<const N: usize> Bitmap<N> {
    /// Build a bitmap from its backing blocks.
    ///
    /// `bit0_id` is the logical identifier that bit index 0 maps to.
    /// `first_allocatable_bit` is the first bitmap bit that allocation may
    /// return, leaving lower bits permanently reserved.
    /// `bit_count` must not exceed the total bit capacity of the supplied
    /// buffers, and the number of buffers must not exceed `N`; both
    /// conditions are checked eagerly so the bitmap is never partially
    /// initialised.
    pub fn new(
        bit0_id: u32,
        first_allocatable_bit: usize,
        buffers: impl IntoIterator<Item = Arc<BufferHandle>>,
        bit_count: usize,
    ) -> Self {
        let mut slots: [Option<Arc<BufferHandle>>; N] = array::from_fn(|_| None);
        let mut loaded = 0usize;
        for (slot, buf) in buffers.into_iter().enumerate() {
            assert!(slot < N, "more buffers supplied than bitmap slot capacity");
            slots[slot] = Some(buf);
            loaded = slot + 1;
        }
        assert!(
            bit_count <= loaded * BLOCK_BITS,
            "bitmap bit count exceeds capacity of supplied buffers"
        );
        assert!(
            first_allocatable_bit <= bit_count,
            "first allocatable bit must stay within bitmap range"
        );
        Self {
            buffers: slots,
            bit0_id,
            bit_count,
            first_allocatable_bit,
        }
    }

    /// Allocate the first free logical identifier in ascending bitmap order.
    pub fn alloc(&self) -> Option<u32> {
        for bit in self.first_allocatable_bit..self.bit_count {
            let (block_slot, word_index, inner_bit) = {
                let mut bit = bit;
                let block_slot = bit / BLOCK_BITS;
                bit %= BLOCK_BITS;
                (block_slot, bit / WORD_BITS, bit % WORD_BITS)
            };
            let mask = 1u64 << inner_bit;
            let buf = self.buffers[block_slot]
                .as_ref()
                .expect("bitmap buffers must be loaded before allocation");

            if buf.read(|bitmap: &BitmapBlock| bitmap[word_index] & mask != 0) {
                continue;
            }

            buf.write(|bitmap: &mut BitmapBlock| bitmap[word_index] |= mask);
            return Some(self.bit0_id + bit as u32);
        }

        None
    }

    /// Release a previously allocated logical identifier by clearing its bit.
    pub fn dealloc(&self, id: u32) {
        let bit = (id - self.bit0_id) as usize;
        assert!(bit < self.bit_count, "bitmap id out of range");
        assert!(
            bit >= self.first_allocatable_bit,
            "cannot free reserved bitmap bit"
        );
        let block_slot = bit / BLOCK_BITS;
        let word_index = (bit % BLOCK_BITS) / WORD_BITS;
        let inner_bit = bit % WORD_BITS;
        let mask = 1u64 << inner_bit;
        let buf = self.buffers[block_slot]
            .as_ref()
            .expect("bitmap buffers must be loaded before free");
        buf.write(|bitmap: &mut BitmapBlock| bitmap[word_index] &= !mask);
    }
}
