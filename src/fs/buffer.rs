//! Buffer cache metadata and global manager.
//!
//! This module keeps the Linux 0.11 style memory layout:
//!
//! ```text
//! Low address
//! +------------------------------+
//! | BufferHead[0]                |
//! | BufferHead[1]                |  <- metadata grows upward
//! | ...                          |
//! +------------------------------+ <- stop when next head would cross data
//! |          free gap            |
//! +------------------------------+
//! | data block N-1 (1KB)         |
//! | data block N-2 (1KB)         |  <- data blocks grow downward
//! | ...                          |
//! +------------------------------+
//! High address
//! ```
//!
//! For compatibility with the current heap placement, the low-memory fallback
//! upper bound is `0x70000` instead of the historical `0xA0000`.

use core::{
    mem::{align_of, size_of},
    ptr::{self, null_mut},
};

use hashbrown::HashMap;
use lazy_static::lazy_static;

use crate::{
    driver::DevNum,
    fs::{BLOCK_SIZE, BlockNr},
    println,
    sync::KernelCell,
    task::wait_queue::WaitQueue,
};

unsafe extern "C" {
    fn ekernel();
}

/// Address of 1 MiB boundary.
const ONE_MB: usize = 0x100000;
/// Low-memory fallback top used for data blocks in this project.
///
/// This address is also treated as the hard ceiling of metadata placement to
/// avoid overlapping the kernel heap region `[0x70000, 0x90000)`.
const LOW_MEM_BUFFER_TOP: usize = 0x70000;

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
pub struct BufferHead {
    /// Cached block identity. `None` means this entry is currently free.
    pub key: Option<BufferKey>,
    /// Pointer to the 1KB payload block.
    pub b_data: *mut u8,
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
            b_data: null_mut(),
            uptodate: false,
            dirty: false,
            ref_count: 0,
            locked: false,
            wait: WaitQueue::new(),
            free_prev: self_idx,
            free_next: self_idx,
        }
    }
}

/// Global buffer cache manager.
pub struct BufferManager {
    pool_base: usize,
    pool_len: usize,
    free_head: Option<BufferHeadIdx>,
    index: HashMap<BufferKey, BufferHeadIdx>,
    buffer_wait: WaitQueue,
}

impl BufferManager {
    /// Create an empty manager that still needs `init`.
    fn new() -> Self {
        Self {
            pool_base: 0,
            pool_len: 0,
            free_head: None,
            index: HashMap::new(),
            buffer_wait: WaitQueue::new(),
        }
    }

    /// Find a cached block by key.
    fn lookup(&self, key: BufferKey) -> Option<BufferHeadIdx> {
        self.index.get(&key).copied()
    }

    /// Insert a key-to-buffer mapping.
    fn insert_mapping(&mut self, key: BufferKey, idx: BufferHeadIdx) {
        self.index.insert(key, idx);
    }

    /// Remove and return a key-to-buffer mapping.
    fn remove_mapping(&mut self, key: BufferKey) -> Option<BufferHeadIdx> {
        self.index.remove(&key)
    }

    /// Return immutable access to one buffer head by pool index.
    fn head(&self, idx: BufferHeadIdx) -> &BufferHead {
        assert!(self.pool_len > 0, "buffer manager is not initialized");
        assert!(
            idx.0 < self.pool_len,
            "buffer index out of range: {} >= {}",
            idx.0,
            self.pool_len
        );
        let byte_offset = idx
            .0
            .checked_mul(size_of::<BufferHead>())
            .expect("buffer head offset overflow");
        let addr = self
            .pool_base
            .checked_add(byte_offset)
            .expect("buffer head address overflow");
        // SAFETY:
        // - `pool_base` points to a valid initialized metadata region.
        // - `idx` bounds are checked above.
        // - Returned reference is tied to `&self`.
        unsafe { &*(addr as *const BufferHead) }
    }

    /// Return mutable access to one buffer head by pool index.
    fn head_mut(&mut self, idx: BufferHeadIdx) -> &mut BufferHead {
        assert!(self.pool_len > 0, "buffer manager is not initialized");
        assert!(
            idx.0 < self.pool_len,
            "buffer index out of range: {} >= {}",
            idx.0,
            self.pool_len
        );
        let byte_offset = idx
            .0
            .checked_mul(size_of::<BufferHead>())
            .expect("buffer head offset overflow");
        let addr = self
            .pool_base
            .checked_add(byte_offset)
            .expect("buffer head address overflow");
        // SAFETY:
        // - `pool_base` points to a valid initialized metadata region.
        // - `idx` bounds are checked above.
        // - Caller holds `&mut self`, so no aliasing mutable references.
        unsafe { &mut *(addr as *mut BufferHead) }
    }

    /// Remove one buffer from the intrusive free-list ring.
    fn remove_from_free_list(&mut self, idx: BufferHeadIdx) {
        let Some(head_idx) = self.free_head else {
            return;
        };

        let (prev, next) = {
            let bh = self.head(idx);
            (bh.free_prev, bh.free_next)
        };

        if prev == idx && next == idx {
            self.free_head = None;
            return;
        }

        self.head_mut(prev).free_next = next;
        self.head_mut(next).free_prev = prev;
        if head_idx == idx {
            self.free_head = Some(next);
        }
        self.head_mut(idx).free_prev = idx;
        self.head_mut(idx).free_next = idx;
    }

    /// Insert one buffer at the tail of the intrusive free-list ring.
    fn insert_into_free_list_tail(&mut self, idx: BufferHeadIdx) {
        match self.free_head {
            None => {
                self.head_mut(idx).free_prev = idx;
                self.head_mut(idx).free_next = idx;
                self.free_head = Some(idx);
            }
            Some(head_idx) => {
                let tail_idx = self.head(head_idx).free_prev;
                self.head_mut(idx).free_prev = tail_idx;
                self.head_mut(idx).free_next = head_idx;
                self.head_mut(tail_idx).free_next = idx;
                self.head_mut(head_idx).free_prev = idx;
            }
        }
    }

    /// Walk exactly one full cycle of free-list nodes from `start`.
    fn iter_free_list_once<F>(&self, start: BufferHeadIdx, mut f: F)
    where
        F: FnMut(BufferHeadIdx),
    {
        let mut cur = start;
        loop {
            f(cur);
            cur = self.head(cur).free_next;
            if cur == start {
                break;
            }
        }
    }

    /// Initialize the cache pool using the classic metadata/data "meeting" layout.
    fn init_impl(&mut self, buffer_memory_end: u32) {
        if self.pool_len > 0 || self.free_head.is_some() {
            panic!("buffer manager already initialized");
        }

        let head_size = size_of::<BufferHead>();
        let start_buffer =
            (ekernel as usize + align_of::<BufferHead>() - 1) & !(align_of::<BufferHead>() - 1);
        let mut head_addr = start_buffer;
        let mut data_top = if buffer_memory_end as usize == ONE_MB {
            LOW_MEM_BUFFER_TOP
        } else {
            buffer_memory_end as usize
        };

        let mut count = 0usize;
        core::iter::from_fn(|| {
            let data_addr = data_top.checked_sub(BLOCK_SIZE)?;
            let next_head_addr = head_addr
                .checked_add(head_size)
                .expect("buffer head address overflow");

            (next_head_addr <= LOW_MEM_BUFFER_TOP && data_addr >= next_head_addr).then(|| {
                let idx = BufferHeadIdx(count);
                let write_head_addr = head_addr;
                count += 1;
                head_addr = next_head_addr;
                data_top = if data_addr == ONE_MB {
                    LOW_MEM_BUFFER_TOP
                } else {
                    data_addr
                };
                (idx, write_head_addr, data_addr)
            })
        })
        .for_each(|(idx, write_head_addr, data_addr)| {
            // SAFETY:
            // - The iterator only yields non-overlapping metadata/data placements.
            // - Each metadata slot is initialized at most once.
            unsafe {
                let head_ptr = write_head_addr as *mut BufferHead;
                ptr::write(head_ptr, BufferHead::new_empty(idx));
                (*head_ptr).b_data = data_addr as *mut u8;
            }
        });

        if count == 0 {
            panic!(
                "buffer init produced zero entries: start=0x{:x}, end=0x{:x}, head_size={}",
                start_buffer,
                buffer_memory_end as usize,
                size_of::<BufferHead>()
            );
        }

        self.pool_base = start_buffer;
        self.pool_len = count;
        self.free_head = Some(BufferHeadIdx(0));
        self.index = HashMap::with_capacity(count);
        self.buffer_wait = WaitQueue::new();

        (0..count).for_each(|i| {
            let prev = if i == 0 { count - 1 } else { i - 1 };
            let next = if i + 1 == count { 0 } else { i + 1 };
            let bh = self.head_mut(BufferHeadIdx(i));
            bh.free_prev = BufferHeadIdx(prev);
            bh.free_next = BufferHeadIdx(next);
        });

        #[cfg(debug_assertions)]
        self.validate_invariants();

        println!(
            "[buffer_init] pool_base=0x{:x}, pool_len={}, free_head={:?}",
            self.pool_base, self.pool_len, self.free_head
        );
    }

    /// Validate structural invariants in debug builds.
    #[cfg(debug_assertions)]
    fn validate_invariants(&self) {
        let head_base = self.pool_base;
        let head_bytes = self
            .pool_len
            .checked_mul(size_of::<BufferHead>())
            .expect("buffer metadata byte-size overflow");
        let head_end = head_base
            .checked_add(head_bytes)
            .expect("buffer metadata range overflow");
        assert!(
            head_end <= LOW_MEM_BUFFER_TOP,
            "buffer metadata crosses heap boundary: head_end=0x{:x}, heap_start=0x{:x}",
            head_end,
            LOW_MEM_BUFFER_TOP
        );

        let start = self
            .free_head
            .expect("initialized buffer manager must have free_head");
        let mut visited = 0usize;
        self.iter_free_list_once(start, |idx| {
            let bh = self.head(idx);
            assert!(!bh.b_data.is_null(), "buffer {} has null b_data", idx.0);
            let data_begin = bh.b_data as usize;
            let data_end = data_begin.checked_add(BLOCK_SIZE).unwrap_or_else(|| {
                panic!(
                    "buffer {} data range overflow: data_begin=0x{:x}, block_size={}",
                    idx.0, data_begin, BLOCK_SIZE
                )
            });
            assert!(
                data_begin >= head_end || data_end <= head_base,
                "buffer {} data overlaps metadata: data=[0x{:x},0x{:x}) metadata=[0x{:x},0x{:x})",
                idx.0,
                data_begin,
                data_end,
                head_base,
                head_end
            );
            let next = bh.free_next;
            assert_eq!(
                self.head(next).free_prev,
                idx,
                "free-list link mismatch at {}",
                idx.0
            );
            visited += 1;
        });
        assert_eq!(
            visited, self.pool_len,
            "free-list size mismatch: visited={} pool_len={}",
            visited, self.pool_len
        );

        self.index.iter().for_each(|(key, idx)| {
            assert_eq!(
                self.head(*idx).key,
                Some(*key),
                "index and buffer key diverged at {}",
                idx.0
            );
        });
    }
}

lazy_static! {
    /// Global singleton buffer manager.
    pub static ref BUFFER_MANAGER: KernelCell<BufferManager> =
        KernelCell::new(BufferManager::new());
}

/// Initialize the global buffer manager once during boot.
pub fn init(buffer_memory_end: u32) {
    // Safety: boot initialization is single-flow before IRQ-driven concurrent
    // access to buffer cache state exists.
    unsafe {
        BUFFER_MANAGER.exclusive_unchecked(|manager| manager.init_impl(buffer_memory_end));
    }
}
