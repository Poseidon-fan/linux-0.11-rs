use core::array;

use alloc::sync::Arc;

use crate::fs::buffer::BufferHandle;

pub struct Bitmap<const N: usize> {
    buffers: [Option<Arc<BufferHandle>>; N],
    base_id: u32,
    valid_bits: usize,
}

impl<const N: usize> Bitmap<N> {
    pub fn new(base_id: u32) -> Self {
        Self {
            buffers: array::from_fn(|_| None),
            base_id,
            valid_bits: 0,
        }
    }
}
