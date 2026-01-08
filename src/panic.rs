use log::error;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(location) = info.location() {
        error!(
            "[kernel] Panicked at {}:{} {}",
            location.file(),
            location.line(),
            info.message().as_str().ok_or("unknown panic msg").unwrap()
        );
    } else {
        error!(
            "[kernel] Panicked: {}",
            info.message().as_str().ok_or("unknown panic msg").unwrap()
        );
    }
    loop {}
}
