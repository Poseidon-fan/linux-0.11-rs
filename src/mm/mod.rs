mod address;
mod frame;
mod heap;
mod page;
mod space;

pub use address::PhysPageNum;
pub use frame::{PAGE_SIZE, PhysFrame};
pub use space::MemorySpace;

pub fn init(start_mem: u32, end_mem: u32) {
    heap::init();
    frame::init(start_mem, end_mem);
}
