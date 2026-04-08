//! Kernel panic handler and unknown-interrupt fallback.

use core::{hint::spin_loop, panic::PanicInfo};

use log::error;

use crate::{fs, println, task};

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    match info.location() {
        Some(location) => {
            println!(
                "Kernel panic: {} ({}:{})",
                info.message(),
                location.file(),
                location.line(),
            );
        }
        None => {
            println!("Kernel panic: {}", info.message());
        }
    }
    match task::try_current_slot() {
        Some(0) => {
            println!("In swapper task - not syncing");
        }
        Some(_) => fs::sync(),
        None => {}
    }

    loop {
        spin_loop();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn handle_unknown_interrupt() {
    error!("Unknown interrupt");
}
