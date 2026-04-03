#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    let _ = user_lib::test();
    let _ = user_lib::exit();

    #[allow(clippy::empty_loop)]
    loop {}
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    let _ = user_lib::exit();
    loop {}
}
