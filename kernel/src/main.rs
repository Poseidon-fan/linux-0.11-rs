#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(naked_functions)]
#![feature(asm_goto)]
#![feature(used_with_arg)]
#![feature(stmt_expr_attributes)]
#![allow(dead_code)]

extern crate alloc;

mod boot;
mod driver;
mod fs;
mod logging;
mod mm;
mod panic;
mod pmio;
mod segment;
mod signal;
mod sync;
mod syscall;
mod task;
mod time;
mod trap;

use core::arch::global_asm;

use crate::driver::DevNum;

global_asm!(include_str!("boot/head.s"), options(att_syntax));

#[unsafe(no_mangle)]
pub extern "C" fn rust_main() -> ! {
    let ext_mem_k = {
        // BIOS extended memory info address (set up by setup.s).
        const EXT_MEM_K_ADDR: u32 = 0x90002;
        unsafe { core::ptr::read_volatile(EXT_MEM_K_ADDR as *const u16) }
    };
    driver::set_root_dev({
        // BIOS root device address (set up by setup.s).
        const ROOT_DEV_ADDR: u32 = 0x901FC;
        DevNum(unsafe { core::ptr::read_volatile(ROOT_DEV_ADDR as *const u16) })
    });

    let memory_end = ((1 << 20) + ((ext_mem_k as u32) << 10)) & 0xfffff000;
    let memory_end = memory_end.min(16 * 1024 * 1024);
    let buffer_memory_end = match memory_end {
        m if m > 12 * 1024 * 1024 => 5 * 1024 * 1024,
        m if m > 6 * 1024 * 1024 => 3 * 1024 * 1024,
        _ => panic!("memory must be > 6MB"),
    };
    let main_memory_start = buffer_memory_end;

    logging::init();
    println!("logging initialized");

    mm::init(main_memory_start, memory_end);
    trap::init();
    time::init();
    task::init();
    driver::blk::hd::init();
    fs::buffer::init(buffer_memory_end);
    println!("init complete");

    segment::move_to_user_mode();
    (user_lib::fork().unwrap() == 0).then(|| user_init());

    loop {
        user_lib::pause().unwrap();
    }
}

fn user_init() -> ! {
    // Boot-time location of the BIOS drive table (set up by setup.s).
    const DRIVE_INFO_ADDR: *const u8 = 0x90080 as *const u8;
    user_lib::setup(DRIVE_INFO_ADDR).unwrap();

    let argv: [*const u8; 2] = [c"update".as_ptr().cast(), core::ptr::null()];
    let envp: [*const u8; 1] = [core::ptr::null()];
    user_lib::process::execve(c"/bin/update".as_ptr().cast(), argv.as_ptr(), envp.as_ptr())
        .unwrap();

    user_lib::exit().unwrap();

    #[allow(clippy::empty_loop)]
    loop {}
}
