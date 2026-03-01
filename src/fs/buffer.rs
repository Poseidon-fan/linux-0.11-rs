use crate::{
    driver::DevNum,
    fs::{BLOCK_SIZE, BlockNr},
    task::wait_queue::WaitQueue,
};

/// Buffer-cache lookup key.
///
/// The same block number on different devices refers to different cached
/// entries, so both fields are required for uniqueness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferKey {
    pub dev: DevNum,
    pub block_nr: BlockNr,
}

impl BufferKey {
    /// Build a cache key from device number and logical block number.
    #[inline]
    pub const fn new(dev: DevNum, block_nr: BlockNr) -> Self {
        Self { dev, block_nr }
    }
}

/// Stable index of a `BufferHead` entry inside the global buffer pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferHeadIdx(pub usize);

/// In-memory cache entry metadata.
///
/// Queue links are represented as indices:
///
/// +------------------------+
/// | hash_prev / hash_next  | -> hash bucket chain
/// +------------------------+
/// | free_prev / free_next  | -> global free-list ring
/// +------------------------+
pub struct BufferHead {
    /// Cached block identity. `None` means this entry is currently free.
    pub key: Option<BufferKey>,
    /// Cached block payload.
    pub data: [u8; BLOCK_SIZE],
    /// Data validity flag set after successful read completion.
    pub uptodate: bool,
    /// Dirty flag set when buffer data is newer than on-disk data.
    pub dirty: bool,
    /// Active user count.
    pub ref_count: u16,
    /// I/O lock flag.
    pub locked: bool,
    /// Wait queue used by tasks sleeping on the lock.
    pub wait: WaitQueue,
    /// Previous node in hash chain.
    pub hash_prev: Option<BufferHeadIdx>,
    /// Next node in hash chain.
    pub hash_next: Option<BufferHeadIdx>,
    /// Previous node in free-list ring.
    pub free_prev: BufferHeadIdx,
    /// Next node in free-list ring.
    pub free_next: BufferHeadIdx,
}

impl BufferHead {
    /// Create an unused buffer entry in a single-node free-list ring.
    pub const fn new_empty(self_idx: BufferHeadIdx) -> Self {
        Self {
            key: None,
            data: [0; BLOCK_SIZE],
            uptodate: false,
            dirty: false,
            ref_count: 0,
            locked: false,
            wait: WaitQueue::new(),
            hash_prev: None,
            hash_next: None,
            free_prev: self_idx,
            free_next: self_idx,
        }
    }

    /// Return true when the entry is not attached to any block key.
    #[inline]
    pub const fn is_free(&self) -> bool {
        self.key.is_none()
    }
}
