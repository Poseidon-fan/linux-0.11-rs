//! In-memory bitmap helpers used by inode and zone allocation.

use crate::layout::BLOCK_SIZE;

/// The number of addressable bits stored in one bitmap block.
pub const BITS_PER_BLOCK: usize = BLOCK_SIZE * 8;

/// One loaded bitmap made of full 1 KiB blocks.
#[derive(Clone, Debug)]
pub struct Bitmap {
    start_index: u32,
    bit_count: usize,
    blocks: Vec<[u8; BLOCK_SIZE]>,
}

impl Bitmap {
    /// Build one zero-filled bitmap with the required number of blocks.
    pub fn empty(start_index: u32, bit_count: usize) -> Self {
        let block_count = bit_count.max(1).div_ceil(BITS_PER_BLOCK);
        Self {
            start_index,
            bit_count,
            blocks: vec![[0_u8; BLOCK_SIZE]; block_count],
        }
    }

    /// Build one bitmap from already loaded blocks.
    pub fn from_blocks(start_index: u32, bit_count: usize, blocks: Vec<[u8; BLOCK_SIZE]>) -> Self {
        Self {
            start_index,
            bit_count,
            blocks,
        }
    }

    /// Return the first represented numeric value.
    pub fn start_index(&self) -> u32 {
        self.start_index
    }

    /// Return the number of represented bits.
    pub fn bit_count(&self) -> usize {
        self.bit_count
    }

    /// Return the number of loaded bitmap blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Return the loaded bitmap blocks.
    pub fn blocks(&self) -> &[[u8; BLOCK_SIZE]] {
        &self.blocks
    }

    /// Test whether the provided value is marked in the bitmap.
    pub fn is_set(&self, value: u32) -> bool {
        let Some(relative) = value.checked_sub(self.start_index) else {
            return false;
        };
        let relative = relative as usize;
        if relative >= self.bit_count {
            return false;
        }

        let (byte_index, bit_mask) = Self::bit_position(relative);
        let block = byte_index / BLOCK_SIZE;
        let byte = byte_index % BLOCK_SIZE;
        self.blocks[block][byte] & bit_mask != 0
    }

    /// Mark the provided value as allocated.
    pub fn set(&mut self, value: u32) {
        let relative = (value - self.start_index) as usize;
        let (byte_index, bit_mask) = Self::bit_position(relative);
        let block = byte_index / BLOCK_SIZE;
        let byte = byte_index % BLOCK_SIZE;
        self.blocks[block][byte] |= bit_mask;
    }

    /// Mark the provided value as free.
    pub fn clear(&mut self, value: u32) {
        let relative = (value - self.start_index) as usize;
        let (byte_index, bit_mask) = Self::bit_position(relative);
        let block = byte_index / BLOCK_SIZE;
        let byte = byte_index % BLOCK_SIZE;
        self.blocks[block][byte] &= !bit_mask;
    }

    /// Allocate and return the first free represented value.
    pub fn alloc(&mut self) -> Option<u32> {
        for relative in 0..self.bit_count {
            let value = self.start_index + relative as u32;
            if !self.is_set(value) {
                self.set(value);
                return Some(value);
            }
        }

        None
    }

    /// Return the count of free represented values.
    pub fn count_free(&self) -> usize {
        (0..self.bit_count)
            .map(|relative| self.start_index + relative as u32)
            .filter(|value| !self.is_set(*value))
            .count()
    }

    /// Return the byte position and bit mask for one relative index.
    fn bit_position(relative: usize) -> (usize, u8) {
        let byte_index = relative / 8;
        let bit_mask = 1_u8 << (relative % 8);
        (byte_index, bit_mask)
    }
}

#[cfg(test)]
mod tests {
    //! Bitmap-focused unit tests.

    use super::*;

    /// Confirm allocation walks the bitmap from the first free value.
    #[test]
    fn allocation_returns_the_first_free_value() {
        let mut bitmap = Bitmap::empty(5, 4);
        bitmap.set(5);
        bitmap.set(6);
        assert_eq!(bitmap.alloc(), Some(7));
        assert_eq!(bitmap.alloc(), Some(8));
        assert_eq!(bitmap.alloc(), None);
    }
}
