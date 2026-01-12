mod task_struct;

use core::ptr::addr_of_mut;

use crate::mm::PAGE_SIZE;

static mut USER_STACK: [i32; (PAGE_SIZE >> 2) as usize] = [0; (PAGE_SIZE >> 2) as usize];

#[repr(C)]
struct StackStart {
    a: *mut i32,
    b: i16,
}

#[unsafe(export_name = "stack_start")]
static mut STACK_START: StackStart = StackStart {
    a: unsafe {
        addr_of_mut!(USER_STACK)
            .cast::<i32>()
            .add((PAGE_SIZE >> 2) as usize)
    },
    b: 0x10,
};
