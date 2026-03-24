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
    blocks: [Option<Arc<BufferHandle>>; N],
    bit0_id: u32,
    bit_count: usize,
}

impl<const N: usize> Bitmap<N> {
    /// Create an empty bitmap whose bit 0 maps to `bit0_id`.
    pub fn new(bit0_id: u32) -> Self {
        Self {
            blocks: array::from_fn(|_| None),
            bit0_id,
            bit_count: 0,
        }
    }

    /// Set the active number of bits visible through this bitmap.
    pub fn set_bit_count(&mut self, bit_count: usize) {
        assert!(
            bit_count <= self.capacity(),
            "bitmap bit count exceeds cached block capacity"
        );
        self.bit_count = bit_count;
    }

    /// Attach one cached bitmap block to the given slot.
    pub fn attach_block(&mut self, slot: usize, block: Arc<BufferHandle>) {
        assert!(
            slot < N,
            "bitmap block slot must stay within the fixed capacity"
        );
        self.blocks[slot] = Some(block);
    }

    /// Allocate the first free logical identifier in ascending bitmap order.
    pub fn alloc(&self) -> Option<u32> {
        for bit in 0..self.bit_count {
            let (block_slot, word_index, inner_bit) = decomposition(bit);
            let mask = 1u64 << inner_bit;
            let block = self.blocks[block_slot]
                .as_ref()
                .expect("bitmap blocks must be attached before allocation");

            if block.read(|bitmap: &BitmapBlock| bitmap[word_index] & mask != 0) {
                continue;
            }

            block.write(|bitmap: &mut BitmapBlock| bitmap[word_index] |= mask);
            return Some(self.bit0_id + bit as u32);
        }

        None
    }

    const fn capacity(&self) -> usize {
        N * BLOCK_BITS
    }
}

fn decomposition(mut bit: usize) -> (usize, usize, usize) {
    let block_slot = bit / BLOCK_BITS;
    bit %= BLOCK_BITS;
    (block_slot, bit / WORD_BITS, bit % WORD_BITS)
}
