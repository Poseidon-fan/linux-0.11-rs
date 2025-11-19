#![no_std]
#![no_main]

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
fn _start() -> ! {
    #[allow(clippy::empty_loop)]
    loop {}
}
