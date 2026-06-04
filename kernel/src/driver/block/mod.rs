//! Block device request layer.
//!
//! The request layer owns the shared request pool and the per-major elevator
//! queues. Device drivers register a single `kick` entry point and interact
//! with the current request through [`CurrentRequest`], so hardware-specific
//! code never needs to know how request slots are linked together.
//!
//! Request queue shape:
//!
//! ```text
//! BlockManager
//! +---------------------+
//! | request pool [32]   |<-----+
//! +---------------------+      |
//! | device queues [7]   |---- current -> next -> next
//! +---------------------+
//!
//! Each queue head is the in-flight request. New requests are spliced after
//! that head using a seek-ordered elevator rule.
//! ```
//!
//! - [`hd`] — ATA hard disk driver (interrupt-driven, up to 2 drives).

pub mod hd;

use alloc::sync::Arc;
use core::ptr::NonNull;

use log::warn;

use crate::{
    driver::DevNum,
    error::{Errno, Result},
    fs::{BLOCK_SIZE, buffer::BufferSlot},
    sync::{IrqSaveGuard, KernelCell},
    task::WaitQueue,
};

/// All block requests are addressed in 512-byte sectors.
pub const SECTOR_SIZE: usize = 512;

/// Register one block-device major with the shared request queue.
pub fn register_driver(major: u8, ops: BlockDriverOps) {
    unsafe {
        BLOCK_MANAGER.exclusive_unchecked(|manager| {
            manager.register_driver(major, ops);
        });
    }
}

/// Submit one buffer-cache I/O request to the block layer.
pub fn submit_request(ty: BlockRequestType, prefetch: bool, buffer: Arc<BufferSlot>) {
    let Some(key) = buffer.key() else {
        warn!("Buffer key not set");
        return;
    };

    let major = key.dev().major();
    if !is_registered_major(major) {
        warn!("Device not available");
        return;
    }

    // Read-ahead/write-ahead requests are optional. If the target buffer is
    // already busy, dropping the request preserves the non-blocking behavior.
    if prefetch && buffer.is_io_locked() {
        return;
    }

    buffer.acquire_io();
    if ty == BlockRequestType::Read && buffer.is_up_to_date()
        || ty == BlockRequestType::Write && !buffer.is_dirty()
    {
        buffer.release_io();
        return;
    }

    let request_id = loop {
        let irq = IrqSaveGuard::enter();
        let candidate = BLOCK_MANAGER.exclusive(|manager| manager.pool.find_free_slot(ty));
        match candidate {
            Some(id) => break id,
            None if prefetch => {
                drop(irq);
                buffer.release_io();
                return;
            }
            None => DEVICE_WAIT_QUEUE.sleep(),
        }
    };

    let start = BLOCK_MANAGER.exclusive(|manager| {
        let io = BlockRequestIo {
            dev: key.dev(),
            ty,
            first_sector: key.block_number() * BUFFER_BLOCK_SECTOR_COUNT,
            sector_count: BUFFER_BLOCK_SECTOR_COUNT,
            data_addr: buffer.data_addr(),
        };
        manager.pool.insert(request_id, BlockRequest {
            progress: RequestProgress {
                next_sector: io.first_sector,
                remaining_sectors: io.sector_count,
                next_data_addr: io.data_addr,
            },
            io,
            error_count: 0,
            payload: RequestPayload::BufferCache(buffer),
            next_request: None,
        });
        manager.add_request(major, request_id)
    });

    if let Some(ops) = start {
        (ops.kick)();
    }
}

/// Borrow the current request for one major device queue.
pub fn with_current_request<R>(major: u8, f: impl FnOnce(Option<CurrentRequest<'_>>) -> R) -> R {
    BLOCK_MANAGER.exclusive(|manager| {
        let current = manager
            .current_request_mut(major)
            .map(|request| CurrentRequest { request });
        f(current)
    })
}

/// Complete and remove the current request for one block-device major.
pub fn complete_current(major: u8, result: Result<()>) -> QueueAfterComplete {
    let Ok((request, has_more)) =
        BLOCK_MANAGER.exclusive(|manager| manager.take_current_request(major))
    else {
        return QueueAfterComplete::Idle;
    };
    let success = result.is_ok();
    let BlockRequest { io, payload, .. } = request;

    match payload {
        RequestPayload::BufferCache(buffer) => {
            buffer.set_up_to_date(success);
            buffer.release_io();
        }
        RequestPayload::Paging(wait_queue) => wait_queue.wake(),
    }

    if !success {
        warn!(
            "block I/O error: dev {:04x}, sector {}",
            io.dev.0, io.first_sector
        );
    }

    DEVICE_WAIT_QUEUE.wake();
    if has_more {
        QueueAfterComplete::More
    } else {
        QueueAfterComplete::Idle
    }
}

/// Static block-driver callbacks registered for one major number.
#[derive(Clone, Copy)]
pub struct BlockDriverOps {
    /// Starts or resumes processing the current request for this driver.
    pub kick: fn(),
}

/// Outcome returned after removing the current request from a device queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueAfterComplete {
    /// The device queue is empty.
    Idle,
    /// Another request is ready and the driver should kick the queue again.
    More,
}

/// Read or write operation requested by the buffer cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockRequestType {
    /// Read sectors from the block device into memory.
    Read = 0,
    /// Write sectors from memory to the block device.
    Write = 1,
}

/// Immutable I/O parameters for a block request.
#[derive(Clone)]
pub struct BlockRequestIo {
    /// Target device number.
    dev: DevNum,
    /// Read or write operation.
    ty: BlockRequestType,
    /// First 512-byte sector to transfer.
    first_sector: u32,
    /// Total number of sectors to transfer.
    sector_count: u32,
    /// Memory address of the first sector's data.
    data_addr: NonNull<u8>,
}

/// Mutable progress cursor for the current block request.
#[derive(Clone)]
pub struct RequestProgress {
    /// Next 512-byte sector to transfer.
    next_sector: u32,
    /// Number of sectors still waiting to be transferred.
    remaining_sectors: u32,
    /// Memory address for the next sector transfer.
    next_data_addr: NonNull<u8>,
}

/// Action selected after increasing the current request's error counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestErrorAction {
    /// The request may be retried without controller recovery.
    Retry,
    /// The request may be retried after device-specific reset/recalibration.
    Reset,
    /// The request exceeded the maximum error count and should fail.
    Fail,
}

/// Controlled mutable view over the current request for one block driver.
pub struct CurrentRequest<'a> {
    /// Borrowed in-flight request slot.
    request: &'a mut BlockRequest,
}

static BLOCK_MANAGER: KernelCell<BlockManager> = KernelCell::new(BlockManager::new());
static DEVICE_WAIT_QUEUE: WaitQueue = WaitQueue::new();

const REQUEST_POOL_CAPACITY: usize = 32;
const BLOCK_DEVICE_SLOT_COUNT: usize = 7;
const BUFFER_BLOCK_SECTOR_COUNT: u32 = (BLOCK_SIZE / SECTOR_SIZE) as u32;

/// Index of one occupied or free slot in the fixed request pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RequestId(usize);

/// Completion target attached to one block request.
enum RequestPayload {
    /// Request originated from buffer-cache metadata.
    BufferCache(Arc<BufferSlot>),
    /// Request originated from paging path and waits on its own queue.
    #[expect(
        dead_code,
        reason = "paging I/O is reserved until swap support is wired"
    )]
    Paging(WaitQueue),
}

/// Request slot stored in the shared pool.
struct BlockRequest {
    /// Immutable I/O parameters.
    io: BlockRequestIo,
    /// Mutable transfer progress cursor.
    progress: RequestProgress,
    /// Number of failed attempts recorded for this request.
    error_count: u32,
    /// Completion target waited on or notified once the request finishes.
    payload: RequestPayload,
    /// Next request in the per-major queue, if any.
    next_request: Option<RequestId>,
}

/// Per-major queue head and driver entry point.
#[derive(Clone, Copy)]
struct DeviceQueue {
    /// Driver callbacks registered for this major.
    ops: BlockDriverOps,
    /// In-flight request at the head of the queue, if any.
    current_request: Option<RequestId>,
}

/// Fixed-size request storage shared by all block drivers.
struct RequestPool {
    /// Backing storage for all request slots.
    slots: [Option<BlockRequest>; REQUEST_POOL_CAPACITY],
}

/// Shared block request scheduler and device queue table.
struct BlockManager {
    /// Shared request slot pool.
    pool: RequestPool,
    /// Per-major device queues indexed by major number.
    queues: [Option<DeviceQueue>; BLOCK_DEVICE_SLOT_COUNT],
}

/// Return whether one major number has a registered block driver.
fn is_registered_major(major: u8) -> bool {
    BLOCK_MANAGER.exclusive(|manager| manager.driver_ops(major).is_some())
}

impl BlockRequestIo {
    /// Device number targeted by this request.
    pub fn dev(&self) -> DevNum {
        self.dev
    }

    /// I/O operation requested by the buffer cache.
    pub fn ty(&self) -> BlockRequestType {
        self.ty
    }
}

impl RequestProgress {
    /// Next 512-byte sector to transfer.
    pub fn next_sector(&self) -> u32 {
        self.next_sector
    }

    /// Number of sectors still waiting to be transferred.
    pub fn remaining_sectors(&self) -> u32 {
        self.remaining_sectors
    }

    /// Memory address for the next sector transfer.
    pub fn next_data_addr(&self) -> NonNull<u8> {
        self.next_data_addr
    }
}

impl CurrentRequest<'_> {
    /// Return immutable request parameters.
    pub fn io(&self) -> &BlockRequestIo {
        &self.request.io
    }

    /// Return the current progress cursor.
    pub fn progress(&self) -> &RequestProgress {
        &self.request.progress
    }

    /// Return the memory address for the next sector transfer.
    pub fn buffer_ptr<T>(&self) -> NonNull<T> {
        self.request.progress.next_data_addr.cast()
    }

    /// Advance the request cursor by one 512-byte sector.
    pub fn advance_one_sector(&mut self) {
        assert!(
            self.request.progress.remaining_sectors != 0,
            "cannot advance a completed block request"
        );
        self.request.progress.next_sector += 1;
        self.request.progress.remaining_sectors -= 1;
        self.request.progress.next_data_addr = unsafe {
            NonNull::new_unchecked(
                self.request
                    .progress
                    .next_data_addr
                    .as_ptr()
                    .add(SECTOR_SIZE),
            )
        };
    }

    /// Clear the per-request error counter after a successful transfer.
    pub fn clear_errors(&mut self) {
        self.request.error_count = 0;
    }

    /// Record one failed transfer and classify the next driver action.
    pub fn record_error(&mut self, max_errors: u32) -> RequestErrorAction {
        self.request.error_count += 1;
        if self.request.error_count >= max_errors {
            RequestErrorAction::Fail
        } else if self.request.error_count > max_errors / 2 {
            RequestErrorAction::Reset
        } else {
            RequestErrorAction::Retry
        }
    }
}

impl RequestPool {
    /// Create an empty request pool.
    const fn new() -> Self {
        Self {
            slots: [const { None }; REQUEST_POOL_CAPACITY],
        }
    }

    /// Find one free request slot using read/write reservation policy.
    ///
    /// Read requests can use the whole pool, while write requests are
    /// restricted to the first two thirds so read requests still have room.
    fn find_free_slot(&self, ty: BlockRequestType) -> Option<RequestId> {
        let search_end = match ty {
            BlockRequestType::Read => REQUEST_POOL_CAPACITY,
            BlockRequestType::Write => (REQUEST_POOL_CAPACITY * 2) / 3,
        };

        self.slots[..search_end]
            .iter()
            .rposition(|entry| entry.is_none())
            .map(RequestId)
    }

    /// Store a freshly built request in a known-free slot.
    fn insert(&mut self, id: RequestId, request: BlockRequest) {
        debug_assert!(id.0 < REQUEST_POOL_CAPACITY);
        debug_assert!(self.slots[id.0].is_none());
        self.slots[id.0] = Some(request);
    }

    /// Remove a completed request from the pool.
    fn take(&mut self, id: RequestId) -> BlockRequest {
        self.slots[id.0]
            .take()
            .expect("request slot must contain a request")
    }

    /// Borrow one occupied request slot.
    fn request(&self, id: RequestId) -> &BlockRequest {
        self.slots[id.0]
            .as_ref()
            .expect("request slot must contain a request")
    }

    /// Mutably borrow one occupied request slot.
    fn request_mut(&mut self, id: RequestId) -> &mut BlockRequest {
        self.slots[id.0]
            .as_mut()
            .expect("request slot must contain a request")
    }
}

impl BlockManager {
    /// Create an empty block manager with no registered drivers.
    const fn new() -> Self {
        Self {
            pool: RequestPool::new(),
            queues: [const { None }; BLOCK_DEVICE_SLOT_COUNT],
        }
    }

    /// Register one block-device major and its driver callbacks.
    fn register_driver(&mut self, major: u8, ops: BlockDriverOps) {
        let major_index = usize::from(major);
        debug_assert!(major_index < BLOCK_DEVICE_SLOT_COUNT);
        debug_assert!(self.queues[major_index].is_none());
        self.queues[major_index] = Some(DeviceQueue {
            ops,
            current_request: None,
        });
    }

    /// Return the registered callbacks for a major number.
    fn driver_ops(&self, major: u8) -> Option<BlockDriverOps> {
        self.queues
            .get(usize::from(major))
            .copied()
            .flatten()
            .map(|q| q.ops)
    }

    /// Mutably borrow the current request for one major queue.
    fn current_request_mut(&mut self, major: u8) -> Option<&mut BlockRequest> {
        let request_id = self
            .queues
            .get(usize::from(major))?
            .as_ref()?
            .current_request?;
        Some(self.pool.request_mut(request_id))
    }

    /// Remove and return the current request for one major queue.
    fn take_current_request(&mut self, major: u8) -> Result<(BlockRequest, bool)> {
        let queue = self
            .queues
            .get_mut(usize::from(major))
            .and_then(Option::as_mut)
            .ok_or(Errno::NODEV)?;
        let current_id = queue.current_request.ok_or(Errno::IO)?;
        let request = self.pool.take(current_id);
        queue.current_request = request.next_request;
        Ok((request, queue.current_request.is_some()))
    }

    /// Compare requests using the seek-ordered elevator key.
    fn request_in_order(left: &BlockRequest, right: &BlockRequest) -> bool {
        (left.io.ty as u8, left.io.dev.0, left.progress.next_sector)
            < (
                right.io.ty as u8,
                right.io.dev.0,
                right.progress.next_sector,
            )
    }

    /// Insert a request into one per-major queue.
    fn add_request(&mut self, major: u8, request_id: RequestId) -> Option<BlockDriverOps> {
        debug_assert_eq!(
            self.pool.request(request_id).io.dev.major(),
            major,
            "request major must match target device queue"
        );

        if let RequestPayload::BufferCache(buffer) = &self.pool.request(request_id).payload {
            buffer.set_dirty(false);
        }

        self.pool.request_mut(request_id).next_request = None;

        let queue = self.queues[usize::from(major)]
            .as_mut()
            .expect("block device not found");
        let Some(mut current_id) = queue.current_request else {
            queue.current_request = Some(request_id);
            return Some(queue.ops);
        };

        // Keep the current head as the in-flight request and splice the new
        // node into the singly linked request chain using the seek-ordered
        // elevator rule. The list therefore stays ordered with at most one
        // wrap point after the current head.
        loop {
            let Some(next_id) = self.pool.request(current_id).next_request else {
                self.pool.request_mut(current_id).next_request = Some(request_id);
                return None;
            };
            let current_request = self.pool.request(current_id);
            let new_request = self.pool.request(request_id);
            let next_request = self.pool.request(next_id);
            let current_before_new = Self::request_in_order(current_request, new_request);
            let current_before_next = Self::request_in_order(current_request, next_request);
            let new_before_next = Self::request_in_order(new_request, next_request);

            if (current_before_new || !current_before_next) && new_before_next {
                self.pool.request_mut(request_id).next_request = Some(next_id);
                self.pool.request_mut(current_id).next_request = Some(request_id);
                return None;
            }

            current_id = next_id;
        }
    }
}
