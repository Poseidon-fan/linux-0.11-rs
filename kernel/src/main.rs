//! Kernel entry point and init process bootstrap.

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(naked_functions)]
#![feature(asm_goto)]
#![feature(used_with_arg)]
#![feature(stmt_expr_attributes)]

extern crate alloc;

mod boot;
mod driver;
mod error;
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

use core::{arch::global_asm, ffi::CStr};

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
    driver::character::serial::init();
    driver::character::console::init();
    driver::block::hd::init();
    fs::buffer::init(buffer_memory_end);
    println!("init complete");

    segment::move_to_user_mode();
    use user_lib::syscall::process;
    (process::fork().unwrap() == 0).then(|| user_init());

    loop {
        process::pause().unwrap();
    }
}

/// Process 1 — the init process.
///
/// 1. Call `setup()` to initialise the root filesystem.
/// 2. Open `/dev/tty0` as stdin/stdout/stderr.
/// 3. Run `/bin/sh` with `/etc/rc` as stdin (one-shot).
/// 4. After the rc-shell exits, loop forever spawning interactive shells.
fn user_init() -> ! {
    use user_lib::{
        fs::{File, OpenOptions},
        process::{Command, Stdio},
        syscall,
    };

    const DRIVE_INFO_ADDR: *const u8 = 0x90080 as *const u8;
    syscall::process::setup(DRIVE_INFO_ADDR).unwrap();

    // Which TTY device acts as the system console.
    #[cfg(not(feature = "serial-console"))]
    const CONSOLE_TTY: &CStr = c"/dev/tty0";
    #[cfg(feature = "serial-console")]
    const CONSOLE_TTY: &CStr = c"/dev/tty1";

    // Open console TTY as fd 0 (stdin), then dup to fd 1 (stdout) and fd 2 (stderr).
    syscall::fs::open(
        CONSOLE_TTY.as_ptr().cast(),
        syscall::fs::OpenFlags::from_raw(syscall::fs::AccessMode::ReadWrite as u32),
        0,
    )
    .unwrap();
    syscall::fs::dup(0).unwrap();
    syscall::fs::dup(0).unwrap();

    user_lib::println!("hello linux");

    // --- Phase 1: run /bin/sh with /etc/rc as stdin ---
    let rc = File::open("/etc/rc").expect("/etc/rc must be present");
    let _ = Command::new("/bin/sh")
        .env("HOME", "/")
        .stdin(Stdio::from(rc))
        .status();

    // --- Phase 2: respawn interactive shells forever ---
    loop {
        let stdin = match File::open(CONSOLE_TTY.to_str().unwrap()) {
            Ok(f) => f,
            Err(_) => {
                user_lib::println!("Failed to open {} in init", CONSOLE_TTY.to_str().unwrap());
                continue;
            }
        };
        let stdout = match OpenOptions::new()
            .write(true)
            .open(CONSOLE_TTY.to_str().unwrap())
        {
            Ok(f) => f,
            Err(_) => continue,
        };
        let stderr = match OpenOptions::new()
            .write(true)
            .open(CONSOLE_TTY.to_str().unwrap())
        {
            Ok(f) => f,
            Err(_) => continue,
        };

        let status = Command::new("/bin/sh")
            .arg0("-/bin/sh")
            .env("HOME", "/usr/root")
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .pre_exec(|| {
                let _ = syscall::process::setsid();
                Ok(())
            })
            .status();

        match status {
            Ok(status) => user_lib::println!("\nshell exited: {}", status),
            Err(_) => user_lib::println!("Fork failed in init"),
        }
        let _ = syscall::fs::sync();
    }
}
