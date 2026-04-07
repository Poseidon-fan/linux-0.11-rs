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
    driver::chr::console::init();
    driver::blk::hd::init();
    fs::buffer::init(buffer_memory_end);
    println!("init complete");

    segment::move_to_user_mode();
    (user_lib::fork().unwrap() == 0).then(|| user_init());

    loop {
        user_lib::pause().unwrap();
    }
}

/// Process 1 — the init process.
///
/// 1. Call `setup()` to initialise the root filesystem.
/// 2. Open `/dev/tty0` as stdin/stdout/stderr.
/// 3. Fork a child to run `/bin/sh` with `/etc/rc` as stdin.
/// 4. After the rc-shell exits, loop forever spawning interactive shells.
fn user_init() -> ! {
    use user_lib::{fs, process};

    const DRIVE_INFO_ADDR: *const u8 = 0x90080 as *const u8;
    user_lib::setup(DRIVE_INFO_ADDR).unwrap();

    // Open /dev/tty0 as fd 0 (stdin), then dup to fd 1 (stdout) and fd 2 (stderr).
    fs::open(
        c"/dev/tty0".as_ptr().cast(),
        fs::OpenFlags::from_raw(fs::AccessMode::ReadWrite as u32),
        0,
    )
    .unwrap();
    fs::dup(0).unwrap();
    fs::dup(0).unwrap();

    user_lib::println!("hello linux");

    // --- Phase 1: run /bin/sh with /etc/rc as stdin ---
    let pid = user_lib::fork().unwrap();
    if pid == 0 {
        fs::close(0).unwrap();
        if fs::open(
            c"/etc/rc".as_ptr().cast(),
            fs::OpenFlags::from_raw(fs::AccessMode::ReadOnly as u32),
            0,
        )
        .is_err()
        {
            user_lib::exit().unwrap();
        }
        let argv_rc: [*const u8; 2] = [c"/bin/sh".as_ptr().cast(), core::ptr::null()];
        let envp_rc: [*const u8; 4] = [
            c"HOME=/".as_ptr().cast(),
            c"ENV=/.shinit".as_ptr().cast(),
            c"TERM=console".as_ptr().cast(),
            core::ptr::null(),
        ];
        let _ = process::execve(
            c"/bin/sh".as_ptr().cast(),
            argv_rc.as_ptr(),
            envp_rc.as_ptr(),
        );
        user_lib::exit().unwrap();
        #[allow(clippy::empty_loop)]
        loop {}
    }

    // Wait for the rc-shell to finish.
    if pid > 0 {
        let mut status = 0u32;
        loop {
            let waited = process::waitpid(-1, &mut status as *mut u32, 0);
            if waited == Ok(pid) {
                break;
            }
        }
    }

    // --- Phase 2: respawn interactive shells forever ---
    loop {
        let pid = match user_lib::fork() {
            Ok(p) => p,
            Err(_) => {
                user_lib::println!("Fork failed in init");
                continue;
            }
        };

        if pid == 0 {
            // Child: set up a new session with a fresh controlling terminal.
            fs::close(0).unwrap();
            fs::close(1).unwrap();
            fs::close(2).unwrap();
            process::setsid().unwrap();
            fs::open(
                c"/dev/tty0".as_ptr().cast(),
                fs::OpenFlags::from_raw(fs::AccessMode::ReadWrite as u32),
                0,
            )
            .unwrap();
            fs::dup(0).unwrap();
            fs::dup(0).unwrap();

            let argv: [*const u8; 2] = [c"/bin/sh".as_ptr().cast(), core::ptr::null()];
            let envp: [*const u8; 4] = [
                c"HOME=/".as_ptr().cast(),
                c"ENV=/.shinit".as_ptr().cast(),
                c"TERM=console".as_ptr().cast(),
                core::ptr::null(),
            ];
            let _ = process::execve(c"/bin/sh".as_ptr().cast(), argv.as_ptr(), envp.as_ptr());
            user_lib::exit().unwrap();
            #[allow(clippy::empty_loop)]
            loop {}
        }

        // Parent: wait for the shell to exit, then report and restart.
        let mut status = 0u32;
        loop {
            let waited = process::waitpid(-1, &mut status as *mut u32, 0);
            if waited == Ok(pid) {
                break;
            }
        }
        user_lib::println!("\nchild {} died with code {:04x}", pid, status);
        fs::sync().unwrap();
    }
}
