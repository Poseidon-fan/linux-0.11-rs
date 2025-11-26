use core::ptr;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let msg = b"panic!";
    for (i, &ch) in msg.iter().enumerate() {
        putc(ch, i);
    }
    loop {}
}

fn putc(c: u8, pos: usize) {
    let vga = 0xb8000 as *mut u8;

    unsafe {
        ptr::write_volatile(vga.add(pos * 2), c);
        ptr::write_volatile(vga.add(pos * 2 + 1), 0x07);
    }
}
