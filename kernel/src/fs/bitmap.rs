//! Filesystem allocation bitmap helpers.
//!
//! Each bitmap consists of up to `N` cached blocks. Bit 0 maps to `base_id`,
//! and allocation scans those bits in ascending order. Bit 0 is always marked
//! as occupied during construction so it is never handed out.

use core::array;

use crate::fs::{BLOCK_SIZE, buffer::BufferHandle, layout::BitmapBlock};

/// Filesystem allocation bitmap cached in a fixed number of block slots.
pub struct Bitmap<const N: usize> {
    /// Pinned bitmap blocks in ascending bit order; trailing slots stay `None`.
    buffers: [Option<BufferHandle>; N],
    /// Logical identifier that bit index 0 maps to.
    base_id: u32,
    /// Bits that participate in allocation; any trailing bits in the last block are ignored.
    bit_count: usize,
}

const BITS_PER_BLOCK: usize = BLOCK_SIZE * 8;
const BITS_PER_WORD: usize = u64::BITS as usize;
const WORDS_PER_BLOCK: usize = BITS_PER_BLOCK / BITS_PER_WORD;

impl<const N: usize> Bitmap<N> {
    /// Build a bitmap from its backing blocks and mark bit 0 as permanently
    /// occupied.
    ///
    /// `base_id` is the logical identifier that bit index 0 maps to.
    /// `bit_count` must not exceed the total bit capacity of the supplied
    /// buffers, and the number of buffers must not exceed `N`; both
    /// conditions are checked eagerly so the bitmap is never partially
    /// initialised.
    pub fn new(
        base_id: u32,
        block_buffers: impl IntoIterator<Item = BufferHandle>,
        bit_count: usize,
    ) -> Self {
        let mut buffers: [Option<BufferHandle>; N] = array::from_fn(|_| None);
        let mut loaded = 0usize;
        for (index, buffer) in block_buffers.into_iter().enumerate() {
            assert!(index < N, "more buffers supplied than bitmap slot capacity");
            buffers[index] = Some(buffer);
            loaded = index + 1;
        }
        assert!(
            bit_count <= loaded * BITS_PER_BLOCK,
            "bitmap bit count exceeds capacity of supplied buffers"
        );

        buffers[0]
            .as_ref()
            .expect("bitmap must have at least one buffer")
            .modify(|bitmap: &mut BitmapBlock| bitmap[0] |= 1);

        Self {
            buffers,
            base_id,
            bit_count,
        }
    }

    /// Allocate the first free logical identifier in ascending bitmap order.
    pub fn alloc(&self) -> Option<u32> {
        let total_words = self.bit_count.div_ceil(BITS_PER_WORD);
        for word_index in 0..total_words {
            let free = !self.read_word(word_index);
            if free == 0 {
                continue;
            }
            let bit_in_word = free.trailing_zeros() as usize;
            let bit = word_index * BITS_PER_WORD + bit_in_word;
            if bit >= self.bit_count {
                return None;
            }
            self.modify_word(word_index, |word| *word |= 1u64 << bit_in_word);
            return Some(self.base_id + bit as u32);
        }
        None
    }

    /// Release a previously allocated logical identifier by clearing its bit.
    pub fn dealloc(&self, id: u32) {
        let bit = (id - self.base_id) as usize;
        assert!(bit < self.bit_count, "bitmap id out of range");
        let word_index = bit / BITS_PER_WORD;
        let bit_in_word = bit % BITS_PER_WORD;
        self.modify_word(word_index, |word| *word &= !(1u64 << bit_in_word));
    }

    /// Count the number of free (zero) bits in the bitmap.
    pub fn count_free(&self) -> usize {
        let full_words = self.bit_count / BITS_PER_WORD;
        let remaining = self.bit_count % BITS_PER_WORD;

        let used_in_full: u32 = (0..full_words)
            .map(|word_index| self.read_word(word_index).count_ones())
            .sum();

        let used_in_tail = if remaining > 0 {
            let mask = (1u64 << remaining) - 1;
            (self.read_word(full_words) & mask).count_ones()
        } else {
            0
        };

        self.bit_count - (used_in_full + used_in_tail) as usize
    }

    /// Borrow one loaded buffer slot. Out-of-range or unloaded access
    /// indicates a logic bug because `bit_count` constrains every public
    /// entry point to stay within `0..loaded * BITS_PER_BLOCK`.
    fn buffer(&self, block_slot: usize) -> &BufferHandle {
        self.buffers[block_slot]
            .as_ref()
            .expect("bitmap buffer slot must be loaded before access")
    }

    /// Read one 64-bit word at the given global word index.
    ///
    /// The index addresses the bitmap as a flat sequence of `u64` words
    /// regardless of the underlying block boundary.
    fn read_word(&self, global_word_index: usize) -> u64 {
        let (block_slot, word_in_block) = split_word_index(global_word_index);
        self.buffer(block_slot)
            .read(|bitmap: &BitmapBlock| bitmap[word_in_block])
    }

    /// Mutate one 64-bit word at the given global word index, marking the
    /// underlying buffer dirty.
    fn modify_word(&self, global_word_index: usize, mutate: impl FnOnce(&mut u64)) {
        let (block_slot, word_in_block) = split_word_index(global_word_index);
        self.buffer(block_slot)
            .modify(|bitmap: &mut BitmapBlock| mutate(&mut bitmap[word_in_block]));
    }
}

/// Decompose a global word index into `(block_slot, word_in_block)`.
#[inline]
fn split_word_index(global_word_index: usize) -> (usize, usize) {
    (
        global_word_index / WORDS_PER_BLOCK,
        global_word_index % WORDS_PER_BLOCK,
    )
}
