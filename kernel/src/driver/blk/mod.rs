pub mod hd;

use core::ptr::NonNull;

use alloc::sync::Arc;
use log::warn;

use crate::{
    driver::DevNum,
    fs::{BLOCK_SIZE, buffer::BufferHandle},
    sync::KernelCell,
    task::wait_queue::WaitQueue,
};

const REQUEST_POOL_CAPACITY: usize = 32;
const BLOCK_DEVICE_SLOT_COUNT: usize = 7;
/// All block requests are addressed in 512-byte sectors.
pub(super) const SECTOR_SIZE: usize = 512;
const BUFFER_BLOCK_SECTOR_COUNT: u32 = (BLOCK_SIZE / SECTOR_SIZE) as u32;

static BLOCK_MANAGER: KernelCell<BlockManager> = KernelCell::new(BlockManager::new());
static DEVICE_WAIT_QUEUE: WaitQueue = WaitQueue::new();

// Register one block-device major with the shared request queue.
pub(super) fn register_device(
    major: usize,
    request_handler: fn(),
    activate: Option<fn(DevNum)>,
    deactivate: Option<fn(DevNum)>,
) {
    unsafe {
        BLOCK_MANAGER.exclusive_unchecked(|manager| {
            manager.register_device(major, request_handler, activate, deactivate);
        });
    }
}

pub fn submit_request(ty: BlockRequestType, prefetch: bool, buffer_handle: Arc<BufferHandle>) {
    let Some(key) = buffer_handle.key() else {
        warn!("Buffer key not set");
        return;
    };

    // Check if the device is available.
    let major = key.dev.major() as usize;
    if major >= BLOCK_DEVICE_SLOT_COUNT
        || BLOCK_MANAGER.exclusive(|manager| manager.devices[major].is_none())
    {
        warn!("Device not available");
        return;
    }

    // If the buffer is locked, we don't prefetch.
    if prefetch && buffer_handle.io_lock.is_locked() {
        return;
    }

    buffer_handle.io_lock.acquire();
    if ty == BlockRequestType::Read && buffer_handle.is_uptodate()
        || ty == BlockRequestType::Write && !buffer_handle.is_dirty()
    {
        buffer_handle.io_lock.release();
        return;
    }

    let request_slot = loop {
        let candidate = BLOCK_MANAGER.exclusive(|manager| manager.find_free_request_slot(ty));
        match candidate {
            Some(slot) => break slot,
            None if prefetch => {
                buffer_handle.io_lock.release();
                return;
            }
            None => WaitQueue::sleep_on(&DEVICE_WAIT_QUEUE),
        }
    };

    if let Some(request_handler) = BLOCK_MANAGER.exclusive(|manager| {
        manager.requests[request_slot] = Some(BlockRequest {
            io: BlockRequestIo {
                dev: key.dev,
                ty,
                first_sector: key.block_nr * BUFFER_BLOCK_SECTOR_COUNT,
                sector_count: BUFFER_BLOCK_SECTOR_COUNT,
                data_addr: buffer_handle.data,
            },
            error_count: 0,
            payload: RequestPayload::BufferCache(buffer_handle),
            next_request: None,
        });
        manager.add_request(major, request_slot)
    }) {
        request_handler();
    }
}

/// Complete the current request for one block-device major.
pub fn complete_current_request(major: usize, is_uptodate: bool) {
    let (request, device) = BLOCK_MANAGER.exclusive(|manager| manager.take_current_request(major));
    let BlockRequest { io, payload, .. } = request;
    if let Some(deactivate) = device.deactivate {
        deactivate(io.dev);
    }

    match payload {
        RequestPayload::BufferCache(buffer_handle) => {
            buffer_handle.set_uptodate(is_uptodate);
            buffer_handle.io_lock.release();
        }
        RequestPayload::Paging(wait_queue) => WaitQueue::wake_up(&wait_queue),
    }

    if !is_uptodate {
        warn!(
            "block I/O error: dev {:04x}, sector {}",
            io.dev.0, io.first_sector
        );
    }

    WaitQueue::wake_up(&DEVICE_WAIT_QUEUE);
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockRequestType {
    Read = 0,
    Write = 1,
}

/// Cloneable request fields consumed by device request handlers.
#[derive(Clone)]
pub(super) struct BlockRequestIo {
    pub dev: DevNum,
    pub ty: BlockRequestType,
    pub first_sector: u32,
    pub sector_count: u32,
    pub data_addr: NonNull<u8>,
}

enum RequestPayload {
    /// Request originated from buffer-cache metadata.
    BufferCache(Arc<BufferHandle>),
    /// Request originated from paging path and waits on its own queue.
    Paging(WaitQueue),
}

struct BlockRequest {
    io: BlockRequestIo,
    error_count: u32,
    payload: RequestPayload,
    next_request: Option<usize>,
}

/// Registered callbacks and queue head for one block-device major.
#[derive(Clone, Copy)]
struct BlockDevice {
    // Callback to process the next request.
    request_handler: fn(),
    // Optional hook for hardware that must be activated before I/O starts.
    activate: Option<fn(DevNum)>,
    // Optional hook for hardware that must be released after I/O completes.
    deactivate: Option<fn(DevNum)>,
    current_request: Option<usize>,
}

struct BlockManager {
    requests: [Option<BlockRequest>; REQUEST_POOL_CAPACITY],
    devices: [Option<BlockDevice>; BLOCK_DEVICE_SLOT_COUNT],
}

impl BlockManager {
    const fn new() -> Self {
        Self {
            requests: [const { None }; REQUEST_POOL_CAPACITY],
            devices: [const { None }; BLOCK_DEVICE_SLOT_COUNT],
        }
    }

    /// Register one block-device major and its driver callbacks.
    pub fn register_device(
        &mut self,
        major: usize,
        request_handler: fn(),
        activate: Option<fn(DevNum)>,
        deactivate: Option<fn(DevNum)>,
    ) {
        debug_assert!(major < BLOCK_DEVICE_SLOT_COUNT);
        debug_assert!(self.devices[major].is_none());
        self.devices[major] = Some(BlockDevice {
            request_handler,
            activate,
            deactivate,
            current_request: None,
        });
    }

    pub fn current_request_mut(&mut self, major: usize) -> Option<&mut BlockRequest> {
        let device = self.devices[major].expect("block device not found");
        let request_slot = device.current_request?;
        Some(
            self.requests[request_slot]
                .as_mut()
                .expect("request slot must contain a request"),
        )
    }

    /// Find one free request slot using read/write reservation policy.
    ///
    /// Read requests can use the whole pool, while write requests are
    /// restricted to the first two thirds so read requests still have room.
    fn find_free_request_slot(&self, ty: BlockRequestType) -> Option<usize> {
        let search_end = match ty {
            BlockRequestType::Read => REQUEST_POOL_CAPACITY,
            BlockRequestType::Write => (REQUEST_POOL_CAPACITY * 2) / 3,
        };

        self.requests[..search_end]
            .iter()
            .rposition(|entry| entry.is_none())
    }

    fn request(&self, slot: usize) -> &BlockRequest {
        self.requests[slot]
            .as_ref()
            .expect("request slot must contain a request")
    }

    fn request_mut(&mut self, slot: usize) -> &mut BlockRequest {
        self.requests[slot]
            .as_mut()
            .expect("request slot must contain a request")
    }

    fn take_current_request(&mut self, major: usize) -> (BlockRequest, BlockDevice) {
        let device = self.devices[major].expect("block device not found");
        let current_slot = device.current_request.expect("current request missing");
        let request = self.requests[current_slot]
            .take()
            .expect("request slot must contain a request");

        self.devices[major] = Some(BlockDevice {
            current_request: request.next_request,
            ..device
        });
        (request, device)
    }

    fn request_in_order(left: &BlockRequest, right: &BlockRequest) -> bool {
        (left.io.ty as u8, left.io.dev.0, left.io.first_sector)
            < (right.io.ty as u8, right.io.dev.0, right.io.first_sector)
    }

    fn add_request(&mut self, major: usize, request_slot: usize) -> Option<fn()> {
        debug_assert!(major < BLOCK_DEVICE_SLOT_COUNT);
        debug_assert!(request_slot < REQUEST_POOL_CAPACITY);
        debug_assert!(self.requests[request_slot].is_some());
        debug_assert_eq!(
            self.request(request_slot).io.dev.major() as usize,
            major,
            "request major must match target device queue"
        );

        if let RequestPayload::BufferCache(buffer_handle) = &self.request(request_slot).payload {
            buffer_handle.set_dirty(false);
        }

        self.request_mut(request_slot).next_request = None;

        let (request_handler, current_request) = match &self.devices[major] {
            Some(device) => (device.request_handler, device.current_request),
            None => panic!("block device not found"),
        };
        let Some(mut current_slot) = current_request else {
            self.devices[major]
                .as_mut()
                .expect("block device not found")
                .current_request = Some(request_slot);
            return Some(request_handler);
        };

        // Keep the current head as the in-flight request and splice the new
        // node into the singly linked request chain using the original
        // elevator rule. The list therefore stays ordered with at most one
        // wrap point after the current head.
        loop {
            let Some(next_slot) = self.request(current_slot).next_request else {
                self.request_mut(current_slot).next_request = Some(request_slot);
                return None;
            };
            let current_request = self.request(current_slot);
            let new_request = self.request(request_slot);
            let next_request = self.request(next_slot);
            let current_before_new = Self::request_in_order(current_request, new_request);
            let current_before_next = Self::request_in_order(current_request, next_request);
            let new_before_next = Self::request_in_order(new_request, next_request);

            if (current_before_new || !current_before_next) && new_before_next {
                self.request_mut(request_slot).next_request = Some(next_slot);
                self.request_mut(current_slot).next_request = Some(request_slot);
                return None;
            }

            current_slot = next_slot;
        }
    }
}
