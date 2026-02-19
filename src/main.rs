#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(naked_functions)]
#![feature(asm_goto)]
#![feature(used_with_arg)]
#![allow(dead_code)]

extern crate alloc;

mod boot;
mod logging;
mod mm;
mod panic;
mod pmio;
mod segment;
mod sync;
mod syscall;
mod task;
mod time;
mod trap;
mod user_lib;

use core::arch::global_asm;

global_asm!(include_str!("boot/head.s"), options(att_syntax));

#[unsafe(no_mangle)]
pub extern "C" fn rust_main() -> ! {
    let memory_end = ((1 << 20) + ((ext_mem_k() as u32) << 10)) & 0xfffff000;
    let memory_end = memory_end.min(16 * 1024 * 1024);
    let buffer_memory_end = match memory_end {
        m if m > 12 * 1024 * 1024 => 4 * 1024 * 1024,
        m if m > 6 * 1024 * 1024 => 2 * 1024 * 1024,
        _ => 1024 * 1024,
    };
    let main_memory_start = buffer_memory_end;

    logging::init();
    println!("logging initialized");

    mm::init(main_memory_start, memory_end);
    trap::init();
    time::init();
    task::init();
    println!("init complete");

    sync::sti();
    sync::move_to_user_mode();
    (user_lib::fork().unwrap() == 0).then(|| user_lib::init());

    user_lib::test(true).unwrap();

    loop {
        user_lib::pause().unwrap();
    }
}

#[inline]
pub fn ext_mem_k() -> u16 {
    /// BIOS extended memory info address (set up by setup.s).
    const EXT_MEM_K_ADDR: u32 = 0x90002;
    unsafe { core::ptr::read_volatile(EXT_MEM_K_ADDR as *const u16) }
}

// Dummy function, currently referenced by `ignore_int` in `head.s`.
// Remove it later, replace with rust kernel print.
#[unsafe(no_mangle)]
pub extern "C" fn printk() {
    panic!("printk is not implemented");
}
