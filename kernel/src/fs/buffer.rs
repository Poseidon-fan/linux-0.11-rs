//! Buffer cache metadata structures.
//!
//! Memory layout model:
//!
//! ```text
//! LOW_MEM (scan start)
//!   |
//!   v
//! +-----------+-----------+-----------+-----------+ ... +-----------+
//! | block #0  | block #1  | block #2  | block #3  |     | block #N  |
//! +-----------+-----------+-----------+-----------+ ... +-----------+
//! ^           ^           ^
//! |           |           |
//! data ptr0   data ptr1   data ptr2
//!
//! Each block is BLOCK_SIZE bytes and each BufferHandle points to one block.
//! ```

use alloc::sync::Arc;
use core::ptr::NonNull;
use log::warn;

use hashbrown::HashMap;
use intrusive_collections::{LinkedList, LinkedListLink, intrusive_adapter};
use lazy_static::lazy_static;

use crate::{
    driver::DevNum,
    fs::BLOCK_SIZE,
    mm::frame::LOW_MEM,
    sync::{self, KernelCell},
    task::wait_queue::WaitQueue,
};

/// Unique key for one cached filesystem block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferKey {
    /// Device number (`major:minor` encoded).
    pub dev: DevNum,
    /// Filesystem block number on the device.
    pub block_nr: u32,
}

/// Mutable state protected by [`KernelCell`] inside each buffer handle.
pub struct BufferState {
    /// Current `(dev, block)` binding. `None` means not indexed yet.
    pub key: Option<BufferKey>,
    /// Logical user count, equivalent to Linux 0.11 `b_count` semantics.
    pub ref_count: u16,
    /// Dirty flag: data differs from on-disk copy.
    pub dirty: bool,
    /// Up-to-date flag: data is known valid.
    pub uptodate: bool,
    /// I/O lock flag: data is currently under block-device I/O.
    pub io_locked: bool,
}

/// Metadata object for one block-sized cache entry.
pub struct BufferHandle {
    /// Intrusive link node used by [`BufferList`].
    pub buffers_link: LinkedListLink,
    /// Start address of one `BLOCK_SIZE` data block.
    pub data: NonNull<u8>,
    /// Mutable status flags and logical refcount.
    pub state: KernelCell<BufferState>,
    /// Wait queue used by tasks waiting for this buffer's I/O lock.
    pub wait_queue: WaitQueue,
}

intrusive_adapter!(
    /// Adapter for storing `Arc<BufferHandle>` nodes in an intrusive linked list.
    pub BufferAdapter = Arc<BufferHandle>: BufferHandle { buffers_link => LinkedListLink }
);

/// Intrusive list wrapper for all buffer handles.
///
/// This wrapper intentionally hides raw cursor operations and keeps all
/// list-related `unsafe` in one place.
pub struct BufferList {
    list: LinkedList<BufferAdapter>,
}

/// Global manager of buffer handles and key index.
pub struct BufferManager {
    /// Replacement-order list that permanently keeps all handles.
    pub buffers: BufferList,
    /// `(dev, block)` lookup index for bound handles.
    pub buffer_index: HashMap<BufferKey, Arc<BufferHandle>>,
}

/// Initialize global buffer metadata by scanning `[LOW_MEM, buffer_memory_end)`.
pub fn init(buffer_memory_end: u32) {
    BUFFER_MANAGER.exclusive(|manager| {
        manager.init(buffer_memory_end);
    });
}

lazy_static! {
    /// Global singleton manager for the buffer-cache metadata graph.
    pub static ref BUFFER_MANAGER: KernelCell<BufferManager> =
        KernelCell::new(BufferManager::empty());
}

impl BufferState {
    /// Construct an empty state for a newly created buffer handle.
    pub const fn empty() -> Self {
        Self {
            key: None,
            ref_count: 0,
            dirty: false,
            uptodate: false,
            io_locked: false,
        }
    }
}

impl BufferHandle {
    /// Build a handle that points to one already-reserved block address.
    fn new(data: NonNull<u8>) -> Self {
        Self {
            buffers_link: LinkedListLink::new(),
            data,
            state: KernelCell::new(BufferState::empty()),
            wait_queue: WaitQueue::new(),
        }
    }

    /// Wait until this buffer is no longer under I/O lock.
    ///
    /// Design note:
    /// This path may sleep and cross a scheduling point, so it must not hold
    /// a `KernelCell` mutable borrow across `schedule()`. We therefore:
    /// 1. mask interrupts,
    /// 2. check lock flag and sleep if needed,
    /// 3. re-check after wakeup in a loop,
    /// 4. restore interrupt state to enabled on exit.
    pub fn wait(&self) {
        assert!(sync::current_irq_depth() == 0);
        sync::cli();
        unsafe {
            while self.state.exclusive_unchecked(|state| state.io_locked) {
                WaitQueue::sleep_on(&self.wait_queue);
            }
        }
        sync::sti();
    }

    /// Acquire this buffer's I/O lock, sleeping if another task holds it.
    ///
    /// Design note:
    /// Lock ownership transition (`false -> true`) is done inside the same
    /// IRQ-masked loop as the lock-state check so no wakeup edge is missed.
    /// No guard is kept across sleep, because sleeping while holding a borrow
    /// would conflict with scheduler requirements.
    pub fn lock(&self) {
        assert!(sync::current_irq_depth() == 0);
        sync::cli();
        unsafe {
            while self.state.exclusive_unchecked(|state| state.io_locked) {
                WaitQueue::sleep_on(&self.wait_queue);
            }
            self.state
                .exclusive_unchecked(|state| state.io_locked = true);
        }
        sync::sti();
    }

    /// Release this buffer's I/O lock and wake one waiter.
    pub fn unlock(&self) {
        self.state.exclusive(|state| {
            (!state.io_locked).then(|| warn!("buffer not locked"));
            state.io_locked = false;
        });
        WaitQueue::wake_up(&self.wait_queue);
    }
}

// SAFETY: This kernel runs on a single core and shared mutable access is
// serialized by `KernelCell` critical sections. `data` is just an address
// descriptor, while intrusive link mutation is also done under manager-level
// serialization.
unsafe impl Send for BufferHandle {}
// SAFETY: Same rationale as `Send`; concurrent mutation is not allowed
// outside the serialized kernel critical-section model.
unsafe impl Sync for BufferHandle {}

impl BufferList {
    /// Create an empty buffer list.
    pub fn new() -> Self {
        Self {
            list: LinkedList::new(BufferAdapter::new()),
        }
    }

    /// Count current list nodes.
    pub fn len(&self) -> usize {
        self.list.iter().count()
    }

    /// Insert one handle at list tail.
    pub fn push_back(&mut self, handle: Arc<BufferHandle>) {
        self.list.push_back(handle);
    }

    /// Remove and return list head.
    pub fn pop_front(&mut self) -> Option<Arc<BufferHandle>> {
        self.list.pop_front()
    }

    /// Remove all nodes from the list.
    pub fn clear(&mut self) {
        while self.pop_front().is_some() {}
    }

    /// Iterate over buffer handles in list order.
    pub fn iter(&self) -> impl Iterator<Item = &BufferHandle> {
        self.list.iter()
    }
}

impl BufferManager {
    /// Construct an empty manager.
    ///
    /// Starts as an empty metadata set (no scanned blocks yet).
    fn empty() -> Self {
        Self {
            buffers: BufferList::new(),
            buffer_index: HashMap::new(),
        }
    }

    /// Initialize handles by scanning `[LOW_MEM, buffer_memory_end)` in
    /// `BLOCK_SIZE` chunks.
    ///
    /// Existing handles and index entries are discarded.
    fn init(&mut self, buffer_memory_end: u32) {
        self.buffer_index.clear();
        self.buffers.clear();
        let region_start = LOW_MEM as usize;
        let clamped_end = buffer_memory_end.max(LOW_MEM) as usize;
        let region_end = (clamped_end / BLOCK_SIZE) * BLOCK_SIZE;
        let buffer_count = (region_end - region_start) / BLOCK_SIZE;

        for index in 0..buffer_count {
            let addr = region_start + index * BLOCK_SIZE;
            let data = NonNull::new(addr as *mut u8)
                .expect("LOW_MEM and scanned block addresses are non-zero");
            self.buffers.push_back(Arc::new(BufferHandle::new(data)));
        }

        #[cfg(debug_assertions)]
        self.assert_basic_invariants();
        crate::println!("buffer len: {}", self.buffer_count());
    }

    /// Return the current number of managed buffers.
    fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Insert a key mapping and update handle state key.
    pub fn index_insert(
        &mut self,
        key: BufferKey,
        handle: Arc<BufferHandle>,
    ) -> Option<Arc<BufferHandle>> {
        handle.state.exclusive(|state| {
            state.key = Some(key);
        });
        let replaced = self.buffer_index.insert(key, handle);
        if let Some(old_handle) = replaced.as_ref() {
            old_handle.state.exclusive(|state| {
                if state.key == Some(key) {
                    state.key = None;
                }
            });
        }
        replaced
    }

    /// Remove a key mapping and clear matching handle state key.
    pub fn index_remove(&mut self, key: BufferKey) -> Option<Arc<BufferHandle>> {
        let removed = self.buffer_index.remove(&key);
        if let Some(handle) = removed.as_ref() {
            handle.state.exclusive(|state| {
                if state.key == Some(key) {
                    state.key = None;
                }
            });
        }
        removed
    }

    /// Validate basic manager invariants in debug builds.
    #[cfg(debug_assertions)]
    fn assert_basic_invariants(&self) {
        for handle in self.buffer_index.values() {
            // SAFETY: Read-only invariant check, expected to run under manager
            // serialization during development and tests.
            let key = unsafe { handle.state.exclusive_unchecked(|state| state.key) };
            debug_assert!(key.is_some(), "indexed buffer must have a key");
        }
    }
}
