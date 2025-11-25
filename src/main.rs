#![no_std]
#![no_main]

use core::arch::global_asm;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

global_asm!(include_str!("boot/head.s"), options(att_syntax));

#[unsafe(no_mangle)]
pub extern "C" fn main() -> ! {
    #[allow(clippy::empty_loop)]
    loop {}
}

// Format is incorrect, should be modified later
#[unsafe(no_mangle)]
pub static stack_start: [u8; 4096] = [0; 4096];

#[unsafe(no_mangle)]
pub extern "C" fn printk() {
    #[allow(clippy::empty_loop)]
    loop {}
}
