use core::ptr::NonNull;

use alloc::sync::Arc;

use crate::{driver::DevNum, fs::buffer::BufferHandle, task::wait_queue::WaitQueue};

pub fn init() {}

#[repr(u8)]
enum BlockRequestType {
    Read = 0,
    Write = 1,
}

enum RequestPayload {
    /// Request originated from buffer-cache metadata.
    BufferCache(Arc<BufferHandle>),
    /// Request originated from paging path and waits on its own queue.
    Paging(WaitQueue),
}

struct BlockRequest {
    dev: DevNum,
    ty: BlockRequestType,
    error_count: u32,
    first_sector: u32,
    sector_count: u32,
    data_addr: NonNull<u8>,
    payload: RequestPayload,
}
