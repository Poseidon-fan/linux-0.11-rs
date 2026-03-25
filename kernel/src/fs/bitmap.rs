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
}

impl<const N: usize> Bitmap<N> {
    /// Create an empty bitmap whose bit 0 maps to `bit0_id`.
    pub fn new(bit0_id: u32) -> Self {
        Self {
            buffers: array::from_fn(|_| None),
            bit0_id,
            bit_count: 0,
        }
    }

    /// Load bitmap buffers and set the active bit count in one step.
    ///
    /// Supplying more buffers than `N` or a `bit_count` that exceeds the
    /// total capacity of those buffers panics immediately, so the bitmap is
    /// never left in a partially-initialised state.
    pub fn load(&mut self, buffers: impl IntoIterator<Item = Arc<BufferHandle>>, bit_count: usize) {
        assert!(
            bit_count <= self.capacity(),
            "bitmap bit count exceeds cached block capacity"
        );
        self.bit_count = bit_count;
        for (slot, buf) in buffers.into_iter().enumerate() {
            assert!(slot < N, "more buffers supplied than bitmap slot capacity");
            self.buffers[slot] = Some(buf);
        }
    }

    /// Allocate the first free logical identifier in ascending bitmap order.
    pub fn alloc(&self) -> Option<u32> {
        for bit in 0..self.bit_count {
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
        let block_slot = bit / BLOCK_BITS;
        let word_index = (bit % BLOCK_BITS) / WORD_BITS;
        let inner_bit = bit % WORD_BITS;
        let mask = 1u64 << inner_bit;
        let buf = self.buffers[block_slot]
            .as_ref()
            .expect("bitmap buffers must be loaded before free");
        buf.write(|bitmap: &mut BitmapBlock| bitmap[word_index] &= !mask);
    }

    const fn capacity(&self) -> usize {
        N * BLOCK_BITS
    }
}
