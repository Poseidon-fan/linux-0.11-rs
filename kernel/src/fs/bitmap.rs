//! Filesystem allocation bitmap helpers.
//!
//! Each bitmap consists of up to `N` cached blocks. Bit 0 maps to `bit0_id`,
//! and allocation scans those bits in ascending order. Bit 0 is always marked
//! as occupied during construction so it is never handed out.

use core::array;

use alloc::sync::Arc;

use crate::fs::{BLOCK_SIZE, buffer::BufferHandle, layout::BitmapBlock};

const BLOCK_BITS: usize = BLOCK_SIZE * 8;
const WORD_BITS: usize = u64::BITS as usize;
const WORDS_PER_BLOCK: usize = BLOCK_BITS / WORD_BITS;

/// Filesystem allocation bitmap cached in a fixed number of block slots.
pub struct Bitmap<const N: usize> {
    buffers: [Option<Arc<BufferHandle>>; N],
    bit0_id: u32,
    bit_count: usize,
}

impl<const N: usize> Bitmap<N> {
    /// Build a bitmap from its backing blocks and mark bit 0 as permanently
    /// occupied.
    ///
    /// `bit0_id` is the logical identifier that bit index 0 maps to.
    /// `bit_count` must not exceed the total bit capacity of the supplied
    /// buffers, and the number of buffers must not exceed `N`; both
    /// conditions are checked eagerly so the bitmap is never partially
    /// initialised.
    pub fn new(
        bit0_id: u32,
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

        slots[0]
            .as_ref()
            .expect("bitmap must have at least one buffer")
            .write(|bitmap: &mut BitmapBlock| bitmap[0] |= 1);

        Self {
            buffers: slots,
            bit0_id,
            bit_count,
        }
    }

    /// Allocate the first free logical identifier in ascending bitmap order.
    pub fn alloc(&self) -> Option<u32> {
        for bit in 0..self.bit_count {
            let block_slot = bit / BLOCK_BITS;
            let word_index = (bit % BLOCK_BITS) / WORD_BITS;
            let inner_bit = bit % WORD_BITS;
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

    /// Count the number of free (zero) bits in the bitmap.
    pub fn count_free(&self) -> usize {
        let full_words = self.bit_count / WORD_BITS;
        let remaining = self.bit_count % WORD_BITS;
        let mut used = 0usize;

        for i in 0..full_words {
            let word = self.buffers[i / WORDS_PER_BLOCK]
                .as_ref()
                .unwrap()
                .read(|bitmap: &BitmapBlock| bitmap[i % WORDS_PER_BLOCK]);
            used += word.count_ones() as usize;
        }

        if remaining > 0 {
            let word = self.buffers[full_words / WORDS_PER_BLOCK]
                .as_ref()
                .unwrap()
                .read(|bitmap: &BitmapBlock| bitmap[full_words % WORDS_PER_BLOCK]);
            used += (word & ((1u64 << remaining) - 1)).count_ones() as usize;
        }

        self.bit_count - used
    }
}
