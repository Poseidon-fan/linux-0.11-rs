mod address;
mod frame;
mod heap;
mod page;

pub use frame::PAGE_SIZE;

pub fn init(start_mem: u32, end_mem: u32) {
    heap::init();
    frame::init(start_mem, end_mem);
}
