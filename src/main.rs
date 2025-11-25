#![no_std]
#![no_main]

use core::arch::global_asm;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

global_asm!(include_str!("boot/head.s"), options(att_syntax));

#[unsafe(no_mangle)]
pub extern "C" fn main() {
    todo!()
}

#[unsafe(no_mangle)]
pub extern "C" fn stack_start() {
    todo!()
}

#[unsafe(no_mangle)]
pub extern "C" fn printk() {
    todo!()
}
