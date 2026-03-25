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
use core::{mem::size_of, ptr::NonNull};

use hashbrown::HashMap;
use intrusive_collections::{LinkedList, LinkedListLink, intrusive_adapter};
use lazy_static::lazy_static;

use crate::{
    driver::{DevNum, blk},
    fs::BLOCK_SIZE,
    mm::frame::LOW_MEM,
    sync::{BusyLock, KernelCell, Mutex},
    task::wait_queue::WaitQueue,
};

lazy_static! {
    /// Global singleton manager for the buffer-cache metadata graph.
    pub static ref BUFFER_MANAGER: Mutex<BufferManager> =
        Mutex::new(BufferManager::empty());
}

/// Wait queue for tasks blocked in [`acquire_block`].
static BUFFER_WAIT_QUEUE: WaitQueue = WaitQueue::new();

/// Initialize global buffer metadata by scanning `[LOW_MEM, buffer_memory_end)`.
pub fn init(buffer_memory_end: u32) {
    BUFFER_MANAGER.lock().init(buffer_memory_end);
}

/// Acquire one cache entry for `key`, reusing an existing binding when present.
pub fn acquire_block(key: BufferKey) -> Arc<BufferHandle> {
    loop {
        if let Some(handle) = try_acquire_cached(key) {
            return handle;
        }

        if let Some(handle) = try_acquire_victim(key) {
            return handle;
        }
    }
}

/// Release one logical reference obtained from [`acquire_block`].
pub fn release_block(handle: Arc<BufferHandle>) {
    handle.io_lock.wait();
    handle.dec_ref();
    WaitQueue::wake_up(&BUFFER_WAIT_QUEUE);
}

/// Release multiple logical references obtained from [`acquire_block`].
pub fn release_blocks(handles: impl IntoIterator<Item = Arc<BufferHandle>>) {
    for handle in handles {
        release_block(handle);
    }
}

pub fn read_block(key: BufferKey) -> Option<Arc<BufferHandle>> {
    let handle = acquire_block(key);

    if handle.is_uptodate() {
        return Some(handle);
    }
    blk::submit_request(blk::BlockRequestType::Read, false, Arc::clone(&handle));
    handle.io_lock.wait();
    if handle.is_uptodate() {
        return Some(handle);
    }

    release_block(handle);
    None
}

/// Unique key for one cached filesystem block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferKey {
    /// Device number (`major:minor` encoded).
    pub dev: DevNum,
    /// Filesystem block number on the device.
    pub block_nr: u32,
}

/// Mutable state protected by [`KernelCell`] inside each buffer handle.
struct BufferMeta {
    /// Current `(dev, block)` binding. `None` means not indexed yet.
    key: Option<BufferKey>,
    /// Logical user count for this cache entry.
    ref_count: u16,
    /// Dirty flag: data differs from on-disk copy.
    dirty: bool,
    /// Up-to-date flag: data is known valid.
    uptodate: bool,
}

/// Metadata object for one block-sized cache entry.
pub struct BufferHandle {
    /// Intrusive link node used by [`BufferList`].
    pub buffers_link: LinkedListLink,
    /// Start address of one `BLOCK_SIZE` data block.
    pub data: NonNull<u8>,
    /// Sleepable ownerless lock for in-flight buffer I/O.
    pub io_lock: BusyLock,
    /// Mutable metadata for cache state and binding.
    meta: KernelCell<BufferMeta>,
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

impl BufferMeta {
    /// Construct an empty state for a newly created buffer handle.
    pub const fn empty() -> Self {
        Self {
            key: None,
            ref_count: 0,
            dirty: false,
            uptodate: false,
        }
    }
}

impl BufferHandle {
    /// Build a handle that points to one already-reserved block address.
    fn new(data: NonNull<u8>) -> Self {
        Self {
            buffers_link: LinkedListLink::new(),
            data,
            meta: KernelCell::new(BufferMeta::empty()),
            io_lock: BusyLock::new(),
        }
    }

    /// Return the current binding key, if any.
    pub(crate) fn key(&self) -> Option<BufferKey> {
        self.meta.exclusive(|meta| meta.key)
    }

    /// Set or clear the current binding key.
    fn set_key(&self, key: Option<BufferKey>) {
        self.meta.exclusive(|meta| meta.key = key);
    }

    /// Return whether the handle is currently bound to `key`.
    fn key_matches(&self, key: BufferKey) -> bool {
        self.meta.exclusive(|meta| meta.key == Some(key))
    }

    /// Increment the logical reference count.
    fn inc_ref(&self) {
        self.meta.exclusive(|meta| meta.ref_count += 1);
    }

    /// Decrement the logical reference count.
    fn dec_ref(&self) {
        self.meta.exclusive(|meta| {
            if meta.ref_count == 0 {
                panic!("Trying to free free buffer");
            }
            meta.ref_count -= 1;
        });
    }

    /// Return the current logical reference count.
    fn ref_count(&self) -> u16 {
        self.meta.exclusive(|meta| meta.ref_count)
    }

    /// Mark the buffer dirty or clean.
    pub(crate) fn set_dirty(&self, dirty: bool) {
        self.meta.exclusive(|meta| meta.dirty = dirty);
    }

    /// Return whether the buffer is dirty.
    pub(crate) fn is_dirty(&self) -> bool {
        self.meta.exclusive(|meta| meta.dirty)
    }

    /// Mark the buffer up-to-date or invalid.
    pub(crate) fn set_uptodate(&self, uptodate: bool) {
        self.meta.exclusive(|meta| meta.uptodate = uptodate);
    }

    /// Return whether the buffer contents are valid.
    pub(crate) fn is_uptodate(&self) -> bool {
        self.meta.exclusive(|meta| meta.uptodate)
    }

    /// Interpret the block start as one `T` reference.
    pub(crate) fn as_ref<T>(&self) -> &T {
        assert!(
            size_of::<T>() <= BLOCK_SIZE,
            "typed buffer view must fit within one block"
        );

        unsafe { &*self.data.as_ptr().cast::<T>() }
    }

    /// Interpret the block start as one mutable `T` reference.
    pub(crate) fn as_mut<T>(&mut self) -> &mut T {
        assert!(
            size_of::<T>() <= BLOCK_SIZE,
            "typed buffer view must fit within one block"
        );

        unsafe { &mut *self.data.as_ptr().cast::<T>() }
    }

    /// Read one typed view from the start of this buffer block.
    pub(crate) fn read<T, R>(&self, reader: impl FnOnce(&T) -> R) -> R {
        reader(self.as_ref::<T>())
    }

    /// Mutate one typed view from the start of this buffer block and mark it dirty.
    pub(crate) fn write<T, R>(&self, writer: impl FnOnce(&mut T) -> R) -> R {
        let result = unsafe { writer(&mut *self.data.as_ptr().cast::<T>()) };
        self.set_dirty(true);
        result
    }

    /// Reset metadata for a newly rebound cache entry.
    fn reset_after_rebind(&self) {
        self.meta.exclusive(|meta| {
            meta.ref_count = 1;
            meta.dirty = false;
            meta.uptodate = false;
        });
    }

    /// Return the reclaim penalty used by victim selection.
    fn reclaim_penalty(&self) -> u8 {
        let dirty = self.meta.exclusive(|meta| meta.dirty);
        ((dirty as u8) << 1) | self.io_lock.is_locked() as u8
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

    /// Return the best reclaim candidate in current list order.
    fn find_reclaim_candidate(&self) -> Option<Arc<BufferHandle>> {
        let mut cursor = self.list.front();
        let mut best: Option<(Arc<BufferHandle>, u8)> = None;

        while let Some(handle) = cursor.get() {
            let state = (handle.ref_count(), handle.reclaim_penalty());

            if state.0 == 0 && best.as_ref().is_none_or(|(_, penalty)| state.1 < *penalty) {
                let handle = cursor
                    .clone_pointer()
                    .expect("cursor must point at a live buffer handle");
                best = Some((handle, state.1));
                if state.1 == 0 {
                    break;
                }
            }

            cursor.move_next();
        }

        best.map(|(handle, _)| handle)
    }

    /// Move one buffer handle to the free-list tail.
    fn move_to_back(&mut self, handle: &Arc<BufferHandle>) {
        let ptr = Arc::as_ptr(handle);
        let removed = unsafe {
            self.list
                .cursor_mut_from_ptr(ptr)
                .remove()
                .expect("buffer handle must stay linked in the list")
        };
        debug_assert!(Arc::ptr_eq(&removed, handle));
        self.list.push_back(removed);
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

    /// Pin an existing buffer and increment its logical reference count.
    fn pin_buffer(&mut self, key: BufferKey) -> Option<Arc<BufferHandle>> {
        let handle = Arc::clone(self.buffer_index.get(&key)?);
        handle.inc_ref();
        Some(handle)
    }

    /// Rebind one reclaim candidate to a new key.
    fn try_rebind_buffer(&mut self, key: BufferKey, handle: Arc<BufferHandle>) -> bool {
        if self.buffer_index.contains_key(&key) {
            return false;
        }

        let old_key = handle.key();
        if let Some(old_key) = old_key {
            self.index_remove(old_key);
        }

        handle.reset_after_rebind();

        let replaced = self.index_insert(key, handle.clone());
        debug_assert!(replaced.is_none(), "buffer key must stay unique");
        self.buffers.move_to_back(&handle);
        true
    }

    /// Insert a key mapping and update handle state key.
    pub fn index_insert(
        &mut self,
        key: BufferKey,
        handle: Arc<BufferHandle>,
    ) -> Option<Arc<BufferHandle>> {
        handle.set_key(Some(key));
        let replaced = self.buffer_index.insert(key, handle);
        if let Some(old_handle) = replaced.as_ref() {
            if old_handle.key_matches(key) {
                old_handle.set_key(None);
            }
        }
        replaced
    }

    /// Remove a key mapping and clear matching handle state key.
    pub fn index_remove(&mut self, key: BufferKey) -> Option<Arc<BufferHandle>> {
        let removed = self.buffer_index.remove(&key);
        if let Some(handle) = removed.as_ref() {
            if handle.key_matches(key) {
                handle.set_key(None);
            }
        }
        removed
    }

    /// Validate basic manager invariants in debug builds.
    #[cfg(debug_assertions)]
    fn assert_basic_invariants(&self) {
        for handle in self.buffer_index.values() {
            debug_assert!(handle.key().is_some(), "indexed buffer must have a key");
        }
    }
}

fn try_acquire_cached(key: BufferKey) -> Option<Arc<BufferHandle>> {
    let handle = BUFFER_MANAGER.lock().pin_buffer(key)?;
    handle.io_lock.wait();

    if handle.key_matches(key) {
        return Some(handle);
    }

    handle.dec_ref();

    None
}

fn try_acquire_victim(key: BufferKey) -> Option<Arc<BufferHandle>> {
    let Some(handle) = BUFFER_MANAGER.lock().buffers.find_reclaim_candidate() else {
        WaitQueue::sleep_on(&BUFFER_WAIT_QUEUE);
        return None;
    };

    handle.io_lock.wait();
    if handle.ref_count() != 0 {
        // Another task claimed this victim while we were sleeping.
        // Restart from the outer acquire loop so a newly cached target
        // block is observed before we scan for another victim.
        return None;
    }

    flush_dirty_victim(&handle);

    if BUFFER_MANAGER.lock().try_rebind_buffer(key, handle.clone()) {
        return Some(handle);
    }

    None
}

fn flush_dirty_victim(handle: &Arc<BufferHandle>) {
    while handle.is_dirty() {
        let _dev = handle.key().map(|buffer_key| buffer_key.dev);

        // Dirty victims must be written back before they can be rebound.
        todo!("buffer writeback not implemented");
    }
}
