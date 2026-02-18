use crate::println;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    match info.location() {
        Some(location) => {
            println!(
                "[kernel] Panicked at {}:{} {}",
                location.file(),
                location.line(),
                info.message()
            );
        }
        None => {
            println!("[kernel] Panicked: {}", info.message());
        }
    }
    loop {}
}
