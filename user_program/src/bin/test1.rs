#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    let _ = user_lib::test();
    user_lib::exit(0);
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    user_lib::exit(1);
}
