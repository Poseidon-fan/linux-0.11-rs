//! Block buffer cache.
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
//! Each block is `BLOCK_SIZE` bytes and each cache slot points to one block.
//! ```

use alloc::{sync::Arc, vec::Vec};
use core::ptr::NonNull;

use hashbrown::HashMap;
use intrusive_collections::{LinkedList, LinkedListLink, intrusive_adapter};
use lazy_static::lazy_static;

use crate::{
    driver::{
        DevNum,
        blk::{self, BlockRequestType},
    },
    fs::BLOCK_SIZE,
    mm::frame::LOW_MEM,
    sync::{BusyLock, KernelCell, Mutex},
    task::WaitQueue,
};

/// Initialize global buffer metadata by scanning `[LOW_MEM, buffer_memory_end)`.
pub fn init(buffer_memory_end: u32) {
    BUFFER_MANAGER.lock().init(buffer_memory_end);
}

/// Pin one cache entry for `key`, reusing an existing binding when present.
///
/// This does not read from disk. Callers that need existing block contents
/// should use [`read`] instead.
pub fn get(key: BufferKey) -> BufferHandle {
    BufferHandle {
        slot: acquire_slot(key),
    }
}

/// Read one block from the cache, submitting disk I/O when the cached copy is invalid.
pub fn read(key: BufferKey) -> Option<BufferHandle> {
    let block = get(key);

    if block.slot.is_up_to_date() {
        return Some(block);
    }
    blk::submit_request(BlockRequestType::Read, false, Arc::clone(&block.slot));
    block.slot.wait_io();
    if block.slot.is_up_to_date() {
        return Some(block);
    }

    None
}

/// Write back every cached buffer that is dirty and whose binding key
/// satisfies `predicate`.
///
/// The matching slot set is snapshotted while the manager lock is held and
/// then flushed sequentially, waiting for each block request to complete
/// before submitting the next one. Buffers that become dirty after the
/// snapshot is taken are not flushed by this call.
pub fn sync_dirty(predicate: impl Fn(&BufferKey) -> bool) {
    let slots: Vec<Arc<BufferSlot>> = BUFFER_MANAGER
        .lock()
        .buffer_index
        .values()
        .filter(|slot| slot.is_dirty() && slot.key().is_some_and(|key| predicate(&key)))
        .map(Arc::clone)
        .collect();

    for slot in slots {
        blk::submit_request(BlockRequestType::Write, false, Arc::clone(&slot));
        slot.wait_io();
    }
}

/// Unique key for one cached filesystem block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferKey {
    dev: DevNum,
    block_number: u32,
}

/// RAII reference to one cached block.
///
/// Dropping the guard releases the logical cache reference. Methods on this
/// type are the intended filesystem-facing interface to block data.
#[must_use = "dropping the guard releases the cached block"]
pub struct BufferHandle {
    slot: Arc<BufferSlot>,
}

/// Metadata object for one block-sized cache entry.
pub struct BufferSlot {
    /// Intrusive link node used by [`BufferList`].
    list_link: LinkedListLink,
    /// Start address of one `BLOCK_SIZE` data block.
    data: NonNull<u8>,
    /// Sleepable ownerless lock for in-flight buffer I/O.
    io_lock: BusyLock,
    /// Mutable metadata for cache state and binding.
    meta: KernelCell<BufferMeta>,
}

/// Mutable state protected by [`KernelCell`] inside each buffer slot.
struct BufferMeta {
    /// Current `(dev, block)` binding. `None` means not indexed yet.
    key: Option<BufferKey>,
    /// Logical user count for this cache entry.
    ref_count: u16,
    /// Dirty flag: data differs from on-disk copy.
    dirty: bool,
    /// Up-to-date flag: data is known valid.
    up_to_date: bool,
}

intrusive_adapter!(
    /// Adapter for storing `Arc<BufferSlot>` nodes in an intrusive linked list.
    BufferAdapter = Arc<BufferSlot>: BufferSlot { list_link => LinkedListLink }
);

/// Intrusive list wrapper for all buffer slots.
///
/// This wrapper intentionally hides raw cursor operations and keeps all
/// list-related `unsafe` in one place.
struct BufferList {
    list: LinkedList<BufferAdapter>,
}

/// Global manager of buffer slots and key index.
struct BufferManager {
    /// Replacement-order list that permanently keeps all slots.
    buffers: BufferList,
    /// `(dev, block)` lookup index for bound slots.
    buffer_index: HashMap<BufferKey, Arc<BufferSlot>>,
}

lazy_static! {
    /// Global singleton manager for the buffer-cache metadata graph.
    static ref BUFFER_MANAGER: Mutex<BufferManager> =
        Mutex::new(BufferManager::empty());
}

/// Wait queue for tasks blocked while pinning a cache entry.
static BUFFER_WAIT_QUEUE: WaitQueue = WaitQueue::new();

fn acquire_slot(key: BufferKey) -> Arc<BufferSlot> {
    loop {
        if let Some(slot) = try_acquire_cached(key) {
            return slot;
        }

        if let Some(slot) = try_acquire_victim(key) {
            return slot;
        }
    }
}

fn release_slot(slot: &BufferSlot) {
    slot.wait_io();
    slot.dec_ref();
    BUFFER_WAIT_QUEUE.wake();
}

fn try_acquire_cached(key: BufferKey) -> Option<Arc<BufferSlot>> {
    let slot = BUFFER_MANAGER.lock().pin_buffer(key)?;
    slot.wait_io();

    if slot.key_matches(key) {
        return Some(slot);
    }

    slot.dec_ref();

    None
}

fn try_acquire_victim(key: BufferKey) -> Option<Arc<BufferSlot>> {
    let Some(slot) = BUFFER_MANAGER.lock().buffers.find_reclaim_candidate() else {
        BUFFER_WAIT_QUEUE.sleep();
        return None;
    };

    slot.wait_io();
    if slot.ref_count() != 0 {
        // Another task claimed this victim while we were sleeping.
        // Restart from the outer acquire loop so a newly cached target
        // block is observed before we scan for another victim.
        return None;
    }

    flush_dirty_victim(&slot);

    if BUFFER_MANAGER.lock().try_rebind_buffer(key, slot.clone()) {
        return Some(slot);
    }

    None
}

fn flush_dirty_victim(slot: &Arc<BufferSlot>) {
    if !slot.is_dirty() {
        return;
    }
    blk::submit_request(BlockRequestType::Write, false, Arc::clone(slot));
    slot.wait_io();
}

impl Drop for BufferHandle {
    fn drop(&mut self) {
        release_slot(&self.slot);
    }
}

impl BufferKey {
    /// Create a cache key for block `block_number` on device `dev`.
    pub fn new(dev: DevNum, block_number: u32) -> Self {
        Self { dev, block_number }
    }

    /// Device number (`major:minor` encoded).
    pub fn dev(self) -> DevNum {
        self.dev
    }

    /// Filesystem block number on the device.
    pub fn block_number(self) -> u32 {
        self.block_number
    }
}

impl BufferMeta {
    /// Construct an empty state for a newly created buffer slot.
    const fn empty() -> Self {
        Self {
            key: None,
            ref_count: 0,
            dirty: false,
            up_to_date: false,
        }
    }
}

impl BufferHandle {
    /// Read one typed view from the start of this buffer block.
    pub fn read<T, R>(&self, reader: impl FnOnce(&T) -> R) -> R {
        assert!(
            core::mem::size_of::<T>() <= BLOCK_SIZE,
            "typed buffer view must fit within one block"
        );

        // SAFETY: buffer data blocks are block-aligned during initialization,
        // and callers only request types that fit within one initialized block.
        let view = unsafe { &*self.slot.data.as_ptr().cast::<T>() };
        reader(view)
    }

    /// Mutate one typed view from the start of this buffer block and mark it dirty.
    pub fn modify<T, R>(&self, writer: impl FnOnce(&mut T) -> R) -> R {
        assert!(
            core::mem::size_of::<T>() <= BLOCK_SIZE,
            "typed buffer view must fit within one block"
        );

        // SAFETY: the kernel is single-core and this method does not
        // reschedule. Filesystem code structures mutable access through short
        // closures, so the mutable typed view cannot outlive this call.
        let result = unsafe { writer(&mut *self.slot.data.as_ptr().cast::<T>()) };
        self.slot.set_dirty(true);
        result
    }

    /// Copy bytes from block offset `off` into `dst`.
    pub fn read_bytes(&self, off: usize, dst: &mut [u8]) {
        let end = off + dst.len();
        assert!(end <= BLOCK_SIZE);
        self.read(|block: &[u8; BLOCK_SIZE]| {
            dst.copy_from_slice(&block[off..end]);
        });
    }

    /// Copy `src` into this block at offset `off` and mark it dirty.
    pub fn write_bytes(&self, off: usize, src: &[u8]) {
        let end = off + src.len();
        assert!(end <= BLOCK_SIZE);
        self.modify(|block: &mut [u8; BLOCK_SIZE]| {
            block[off..end].copy_from_slice(src);
        });
        if off == 0 && src.len() == BLOCK_SIZE {
            self.slot.set_up_to_date(true);
        }
    }

    /// Zero the whole block and mark the cached contents valid and dirty.
    pub fn fill_zero(&self) {
        // SAFETY: the slot points to one initialized cache block.
        unsafe { core::ptr::write_bytes(self.slot.data.as_ptr(), 0, BLOCK_SIZE) };
        self.slot.set_up_to_date(true);
        self.slot.set_dirty(true);
    }
}

impl BufferSlot {
    /// Build a slot that points to one already-reserved block address.
    fn new(data: NonNull<u8>) -> Self {
        Self {
            list_link: LinkedListLink::new(),
            data,
            meta: KernelCell::new(BufferMeta::empty()),
            io_lock: BusyLock::new(),
        }
    }

    /// Return the current binding key, if any.
    pub fn key(&self) -> Option<BufferKey> {
        self.meta.exclusive(|meta| meta.key)
    }

    /// Set or clear the current binding key.
    fn set_key(&self, key: Option<BufferKey>) {
        self.meta.exclusive(|meta| meta.key = key);
    }

    /// Return whether the slot is currently bound to `key`.
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
                panic!("releasing buffer slot with zero reference count");
            }
            meta.ref_count -= 1;
        });
    }

    /// Return the current logical reference count.
    fn ref_count(&self) -> u16 {
        self.meta.exclusive(|meta| meta.ref_count)
    }

    /// Mark the buffer dirty or clean.
    pub fn set_dirty(&self, dirty: bool) {
        self.meta.exclusive(|meta| meta.dirty = dirty);
    }

    /// Return whether the buffer is dirty.
    pub fn is_dirty(&self) -> bool {
        self.meta.exclusive(|meta| meta.dirty)
    }

    /// Mark the buffer up-to-date or invalid.
    pub fn set_up_to_date(&self, up_to_date: bool) {
        self.meta.exclusive(|meta| meta.up_to_date = up_to_date);
    }

    /// Return whether the buffer contents are valid.
    pub fn is_up_to_date(&self) -> bool {
        self.meta.exclusive(|meta| meta.up_to_date)
    }

    /// Sleep until any in-flight I/O for this buffer completes.
    pub fn wait_io(&self) {
        self.io_lock.wait();
    }

    /// Acquire the ownerless I/O lock for a block request.
    pub fn acquire_io(&self) {
        self.io_lock.acquire();
    }

    /// Release the ownerless I/O lock after request completion.
    pub fn release_io(&self) {
        self.io_lock.release();
    }

    /// Return whether this buffer currently has in-flight I/O.
    pub fn is_io_locked(&self) -> bool {
        self.io_lock.is_locked()
    }

    /// Return the raw data address used by the block request layer.
    pub fn data_addr(&self) -> NonNull<u8> {
        self.data
    }

    /// Reset metadata for a newly rebound cache entry.
    fn reset_after_rebind(&self) {
        self.meta.exclusive(|meta| {
            meta.ref_count = 1;
            meta.dirty = false;
            meta.up_to_date = false;
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
unsafe impl Send for BufferSlot {}
// SAFETY: Same rationale as `Send`; concurrent mutation is not allowed
// outside the serialized kernel critical-section model.
unsafe impl Sync for BufferSlot {}

impl BufferList {
    /// Create an empty buffer list.
    fn new() -> Self {
        Self {
            list: LinkedList::new(BufferAdapter::new()),
        }
    }

    /// Count current list nodes.
    ///
    /// This is O(n) — it walks the intrusive list. Callers should treat this
    /// as a diagnostic helper rather than a hot-path accessor.
    fn count(&self) -> usize {
        self.list.iter().count()
    }

    /// Insert one slot at list tail.
    fn push_back(&mut self, slot: Arc<BufferSlot>) {
        self.list.push_back(slot);
    }

    /// Remove and return list head.
    fn pop_front(&mut self) -> Option<Arc<BufferSlot>> {
        self.list.pop_front()
    }

    /// Remove all nodes from the list.
    fn clear(&mut self) {
        while self.pop_front().is_some() {}
    }

    /// Return the best reclaim candidate in current list order.
    fn find_reclaim_candidate(&self) -> Option<Arc<BufferSlot>> {
        let mut cursor = self.list.front();
        let mut best: Option<(Arc<BufferSlot>, u8)> = None;

        while let Some(slot) = cursor.get() {
            let state = (slot.ref_count(), slot.reclaim_penalty());

            if state.0 == 0 && best.as_ref().is_none_or(|(_, penalty)| state.1 < *penalty) {
                let slot = cursor
                    .clone_pointer()
                    .expect("cursor must point at a live buffer slot");
                best = Some((slot, state.1));
                if state.1 == 0 {
                    break;
                }
            }

            cursor.move_next();
        }

        best.map(|(slot, _)| slot)
    }

    /// Move one buffer slot to the free-list tail.
    fn move_to_back(&mut self, slot: &Arc<BufferSlot>) {
        let ptr = Arc::as_ptr(slot);
        // SAFETY: every slot managed by `BufferManager` is permanently
        // linked in this intrusive list, and list mutation is serialized by
        // the manager mutex.
        let removed = unsafe {
            self.list
                .cursor_mut_from_ptr(ptr)
                .remove()
                .expect("buffer slot must stay linked in the list")
        };
        debug_assert!(Arc::ptr_eq(&removed, slot));
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

    /// Initialize slots by scanning `[LOW_MEM, buffer_memory_end)` in
    /// `BLOCK_SIZE` chunks.
    ///
    /// Existing slots and index entries are discarded.
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
            self.buffers.push_back(Arc::new(BufferSlot::new(data)));
        }

        #[cfg(debug_assertions)]
        self.assert_basic_invariants();
        crate::println!("buffer len: {}", self.buffer_count());
    }

    /// Return the current number of managed buffers.
    fn buffer_count(&self) -> usize {
        self.buffers.count()
    }

    /// Pin an existing buffer and increment its logical reference count.
    fn pin_buffer(&mut self, key: BufferKey) -> Option<Arc<BufferSlot>> {
        let slot = Arc::clone(self.buffer_index.get(&key)?);
        slot.inc_ref();
        Some(slot)
    }

    /// Rebind one reclaim candidate to a new key.
    fn try_rebind_buffer(&mut self, key: BufferKey, slot: Arc<BufferSlot>) -> bool {
        if self.buffer_index.contains_key(&key) {
            return false;
        }

        let old_key = slot.key();
        if let Some(old_key) = old_key {
            self.index_remove(old_key);
        }

        slot.reset_after_rebind();

        let replaced = self.index_insert(key, slot.clone());
        debug_assert!(replaced.is_none(), "buffer key must stay unique");
        self.buffers.move_to_back(&slot);
        true
    }

    /// Insert a key mapping and update slot state key.
    fn index_insert(&mut self, key: BufferKey, slot: Arc<BufferSlot>) -> Option<Arc<BufferSlot>> {
        slot.set_key(Some(key));
        let replaced = self.buffer_index.insert(key, slot);
        if let Some(old_handle) = replaced.as_ref() {
            if old_handle.key_matches(key) {
                old_handle.set_key(None);
            }
        }
        replaced
    }

    /// Remove a key mapping and clear matching slot state key.
    fn index_remove(&mut self, key: BufferKey) -> Option<Arc<BufferSlot>> {
        let removed = self.buffer_index.remove(&key);
        if let Some(slot) = removed.as_ref() {
            if slot.key_matches(key) {
                slot.set_key(None);
            }
        }
        removed
    }

    /// Validate basic manager invariants in debug builds.
    #[cfg(debug_assertions)]
    fn assert_basic_invariants(&self) {
        for slot in self.buffer_index.values() {
            debug_assert!(slot.key().is_some(), "indexed buffer must have a key");
        }
    }
}
