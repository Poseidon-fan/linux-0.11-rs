//! Kernel panic handler and unknown-interrupt fallback.

use core::{
    hint::spin_loop,
    panic::PanicInfo,
    sync::atomic::{AtomicBool, Ordering},
};

use log::error;

use crate::{fs, println, task};

/// Fallback handler for IDT vectors that have no dedicated service routine.
#[unsafe(no_mangle)]
pub extern "C" fn handle_unknown_interrupt() {
    error!("Unknown interrupt");
}

/// Set to `true` the first time the panic handler runs, so a panic raised
/// while the handler itself is unwinding does not spiral into infinite
/// recursion.
static IN_PANIC: AtomicBool = AtomicBool::new(false);

/// Kernel panic handler: prints the panic message and location, flushes the
/// buffer cache when safe, then halts the CPU.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // If we're already inside the panic handler, don't try to print again
    // (the inner panic was triggered by the cleanup code). Just halt.
    if IN_PANIC.swap(true, Ordering::Relaxed) {
        loop {
            spin_loop();
        }
    }

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
