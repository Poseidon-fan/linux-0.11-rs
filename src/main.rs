#![no_std]
#![no_main]

mod mm;
mod sched;

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

#[unsafe(no_mangle)]
pub extern "C" fn printk() {
    #[allow(clippy::empty_loop)]
    loop {}
}
