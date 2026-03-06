//! Block-layer request model and global queue state.
//!
//! Request-pool topology:
//!
//! ```text
//! request_pool[0..REQUEST_POOL_CAPACITY)
//! +---------+---------+---------+---------+
//! | slot 0  | slot 1  | slot 2  |   ...   |
//! | req?    | req?    | req?    |         |
//! | next    | next    | next    |         |
//! +---------+---------+---------+---------+
//!
//! devices[device].current_request -> request-pool index chain
//! ```
//!
//! This module implements low-level request submission and queue ordering.

#[cfg(feature = "ramdisk")]
pub mod ramdisk;

use alloc::sync::Arc;
use core::array;
use core::ptr::NonNull;
use log::warn;

use lazy_static::lazy_static;

use crate::{
    driver::DevNum, fs::buffer::BufferHandle, sync::KernelCell, task::wait_queue::WaitQueue,
};

/// Number of reserved major slots in the block-device table (`0..=6`).
///
/// Current block I/O paths are wired for three major classes:
/// memory-backed disk, floppy, and hard disk.
const BLOCK_DEVICE_SLOT_COUNT: usize = 7;
/// Number of fixed request slots in the global request pool.
const REQUEST_POOL_CAPACITY: usize = 32;
/// Hardware sector size used by block requests.
pub(super) const SECTOR_SIZE: u32 = 512;

/// Initialize block-device request slots and queue heads.
///
/// This keeps registered device drivers intact and only resets runtime
/// request-chain state.
pub fn blk_dev_init() {
    unsafe {
        BLOCK_MANAGER.exclusive_unchecked(|manager| {
            for device in &mut manager.devices {
                device.current_request = None;
            }
            for entry in &mut manager.request_pool {
                entry.request = None;
                entry.next = None;
            }
        });
    }
}

/// Low-level block request submission API.
pub fn submit_block_request(cmd: RequestCmd, prefetch: bool, buffer_handle: Arc<BufferHandle>) {
    let Some(key) = buffer_handle.state.exclusive(|state| state.key) else {
        warn!("Buffer key not set");
        return;
    };

    // Check if the device is available.
    let major = key.dev.major() as usize;
    if major >= BLOCK_DEVICE_SLOT_COUNT
        || BLOCK_MANAGER.exclusive(|manager| manager.devices[major].driver.is_none())
    {
        warn!("Device not available");
        return;
    }

    // If the buffer is locked, we don't prefetch.
    if prefetch && buffer_handle.state.exclusive(|state| state.io_locked) {
        return;
    }

    buffer_handle.lock();

    if buffer_handle.state.exclusive(|state| {
        cmd == RequestCmd::Read && state.uptodate || cmd == RequestCmd::Write && !state.dirty
    }) {
        buffer_handle.unlock();
        return;
    }

    let request_slot = loop {
        let candidate = BLOCK_MANAGER.exclusive(|manager| manager.find_free_request_slot(cmd));
        match candidate {
            Some(slot) => break slot,
            None if prefetch => {
                buffer_handle.unlock();
                return;
            }
            None => WaitQueue::sleep_on(&WAIT_FOR_REQUEST),
        }
    };
    let driver_to_run = BLOCK_MANAGER.exclusive(|manager| {
        manager.request_pool[request_slot] = RequestPoolEntry {
            request: Some(BlockRequest {
                dev: key.dev,
                cmd,
                errors: 0,
                first_sector: key.block_nr << 1,
                sector_count: 2,
                data_addr: buffer_handle.data,
                payload: RequestPayload::BufferCache(buffer_handle),
            }),
            next: None,
        };
        manager.add_request(major, request_slot)
    });

    if let Some(driver) = driver_to_run {
        driver.process_pending_requests();
    }
}

/// Complete the current request of one major queue.
pub fn complete_current_request(major: usize, is_uptodate: bool) {
    let Some((driver, request)) =
        BLOCK_MANAGER.exclusive(|manager| manager.take_current_request(major))
    else {
        warn!("No current request to complete for major {}", major);
        return;
    };

    if let Some(driver) = driver {
        driver.on_request_complete(request.dev);
    }

    match request.payload {
        RequestPayload::BufferCache(buffer_handle) => {
            buffer_handle
                .state
                .exclusive(|state| state.uptodate = is_uptodate);
            buffer_handle.unlock();
        }
        RequestPayload::Paging(wait_queue) => {
            WaitQueue::wake_up(&wait_queue);
        }
    }

    WaitQueue::wake_up(&WAIT_FOR_REQUEST);
}

/// Read/write command carried by one block request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RequestCmd {
    /// Read sectors from device to memory.
    Read = 0,
    /// Write sectors from memory to device.
    Write = 1,
}

/// Driver contract for one block-device slot.
trait BlockDeviceDriver: Sync {
    /// Process queued requests for this device slot.
    fn process_pending_requests(&self);

    /// Run device-specific completion hook for one request.
    fn on_request_complete(&self, _dev: DevNum) {}
}

/// Origin metadata of one block request payload.
enum RequestPayload {
    /// Request originated from buffer-cache metadata.
    BufferCache(Arc<BufferHandle>),
    /// Request originated from paging path and waits on its own queue.
    Paging(WaitQueue),
}

/// One block request entry stored in the global fixed slot pool.
struct BlockRequest {
    /// Encoded `major:minor` device number.
    pub dev: DevNum,
    /// Read or write command.
    pub cmd: RequestCmd,
    /// Error/retry counter for this request.
    pub errors: i32,
    /// First hardware sector (512-byte unit).
    pub first_sector: u32,
    /// Number of sectors covered by this request.
    pub sector_count: u32,
    /// Non-null start pointer of request data buffer.
    pub data_addr: NonNull<u8>,
    /// Request payload metadata.
    pub payload: RequestPayload,
}

/// One slot inside the global fixed request array.
struct RequestPoolEntry {
    /// Occupied request payload, or `None` when this slot is free.
    pub request: Option<BlockRequest>,
    /// Next request-pool index in the same major queue.
    pub next: Option<usize>,
}

/// Dispatch state for one block-device slot.
struct BlockDevice {
    /// Driver mapped to this device slot.
    pub driver: Option<&'static dyn BlockDeviceDriver>,
    /// Head/current request slot index for this device queue.
    pub current_request: Option<usize>,
}

/// Global block-layer request state.
struct BlockManager {
    /// Per-device dispatch slots.
    pub devices: [BlockDevice; BLOCK_DEVICE_SLOT_COUNT],
    /// Fixed request slot pool.
    pub request_pool: [RequestPoolEntry; REQUEST_POOL_CAPACITY],
}

lazy_static! {
    /// Global singleton block manager protected by kernel critical sections.
    static ref BLOCK_MANAGER: KernelCell<BlockManager> = KernelCell::new(BlockManager::new());
    /// Wait queue for tasks waiting on one free request slot.
    static ref WAIT_FOR_REQUEST: WaitQueue = WaitQueue::new();
}

// SAFETY: Request slots are only mutated under `KernelCell` critical sections
// in a single-core kernel model. `data_addr` is an address token to kernel
// memory and does not provide ownership or aliasing guarantees by itself.
unsafe impl Send for BlockRequest {}

impl RequestPoolEntry {
    /// Build an empty slot.
    const fn empty() -> Self {
        Self {
            request: None,
            next: None,
        }
    }
}

impl BlockDevice {
    /// Build an empty device dispatch slot.
    const fn empty() -> Self {
        Self {
            driver: None,
            current_request: None,
        }
    }
}

impl BlockManager {
    /// Build a manager with all queues empty and all slots free.
    fn new() -> Self {
        Self {
            devices: array::from_fn(|_| BlockDevice::empty()),
            request_pool: array::from_fn(|_| RequestPoolEntry::empty()),
        }
    }

    /// Register one block driver for the given major slot during early boot.
    fn register_block_driver(&mut self, major: usize, driver: &'static dyn BlockDeviceDriver) {
        assert!(
            major < BLOCK_DEVICE_SLOT_COUNT,
            "invalid block major {}",
            major
        );

        let slot = &mut self.devices[major];
        assert!(
            slot.driver.is_none(),
            "block major {} already registered",
            major
        );
        slot.driver = Some(driver);
    }

    /// Return the current queued request as a stable raw pointer.
    ///
    /// The request pool is a fixed array, so request entries do not move in
    /// memory while they stay queued. The returned pointer remains valid until
    /// the caller completes the current request for this major slot.
    fn current_request(&self, major: usize) -> Option<NonNull<BlockRequest>> {
        if major >= BLOCK_DEVICE_SLOT_COUNT {
            return None;
        }

        let current_slot = self.devices[major].current_request?;
        let request = self.request_pool[current_slot].request.as_ref()?;
        Some(NonNull::from(request))
    }

    /// Find one free request slot using read/write reservation policy.
    ///
    /// Read requests can use the whole pool, while write requests are
    /// restricted to the first two thirds so read requests still have room.
    fn find_free_request_slot(&self, cmd: RequestCmd) -> Option<usize> {
        let search_end = match cmd {
            RequestCmd::Read => REQUEST_POOL_CAPACITY,
            RequestCmd::Write => (REQUEST_POOL_CAPACITY * 2) / 3,
        };

        self.request_pool[..search_end]
            .iter()
            .rposition(|entry| entry.request.is_none())
    }

    /// Compare two queued requests in elevator sort order.
    fn in_order(&self, lhs_slot: usize, rhs_slot: usize) -> bool {
        let lhs = self.request_pool[lhs_slot]
            .request
            .as_ref()
            .expect("request slot must be occupied");
        let rhs = self.request_pool[rhs_slot]
            .request
            .as_ref()
            .expect("request slot must be occupied");

        (lhs.cmd as u8, lhs.dev.0, lhs.first_sector) < (rhs.cmd as u8, rhs.dev.0, rhs.first_sector)
    }

    /// Insert one prepared request into the per-device queue.
    ///
    /// Returns the driver that should be started immediately when this
    /// request becomes the first entry of an empty queue.
    fn add_request(
        &mut self,
        major: usize,
        request_slot: usize,
    ) -> Option<&'static dyn BlockDeviceDriver> {
        debug_assert!(major < BLOCK_DEVICE_SLOT_COUNT);
        debug_assert!(request_slot < REQUEST_POOL_CAPACITY);
        debug_assert!(self.request_pool[request_slot].request.is_some());

        self.request_pool[request_slot].next = None;

        if let Some(BlockRequest {
            payload: RequestPayload::BufferCache(buffer_handle),
            ..
        }) = self.request_pool[request_slot].request.as_ref()
        {
            buffer_handle.state.exclusive(|state| state.dirty = false);
        }

        let Some(mut current_slot) = self.devices[major].current_request else {
            self.devices[major].current_request = Some(request_slot);
            return self.devices[major].driver;
        };

        while let Some(next_slot) = self.request_pool[current_slot].next {
            // A queue can wrap once at the seek "turning point". We insert
            // into the first position where the new request fits that ordering.
            let should_insert = (self.in_order(current_slot, request_slot)
                || !self.in_order(current_slot, next_slot))
                && self.in_order(request_slot, next_slot);

            if should_insert {
                break;
            }

            current_slot = next_slot;
        }

        self.request_pool[request_slot].next = self.request_pool[current_slot].next;
        self.request_pool[current_slot].next = Some(request_slot);
        None
    }

    /// Remove and return current request and its driver for one major queue.
    fn take_current_request(
        &mut self,
        major: usize,
    ) -> Option<(Option<&'static dyn BlockDeviceDriver>, BlockRequest)> {
        if major >= BLOCK_DEVICE_SLOT_COUNT {
            return None;
        }

        let driver = self.devices[major].driver;
        let current_slot = self.devices[major].current_request?;
        debug_assert!(current_slot < REQUEST_POOL_CAPACITY);

        let next_slot = self.request_pool[current_slot].next;
        let request = self.request_pool[current_slot]
            .request
            .take()
            .expect("current request slot must be occupied");

        self.request_pool[current_slot].next = None;
        self.devices[major].current_request = next_slot;

        Some((driver, request))
    }
}
