use core::ptr;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    fill_screen(b'!');
    loop {}
}

fn fill_screen(c: u8) {
    let vga = 0xb8000 as *mut u8;
    const SCREEN_SIZE: usize = 80 * 25; // 80 columns x 25 rows

    unsafe {
        for pos in 0..SCREEN_SIZE {
            ptr::write_volatile(vga.add(pos * 2), c);
            ptr::write_volatile(vga.add(pos * 2 + 1), 0x07);
        }
    }
}
