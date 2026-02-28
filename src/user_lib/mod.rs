//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

mod syscall;

pub use syscall::*;

/// struct tms (POSIX <sys/times.h>), 16 bytes.
#[repr(C)]
struct Tms {
    tms_utime: u32,
    tms_stime: u32,
    tms_cutime: u32,
    tms_cstime: u32,
}

/// struct utsname (POSIX <sys/utsname.h>), 45 bytes.
#[repr(C)]
struct Utsname {
    sysname: [u8; 9],
    nodename: [u8; 9],
    release: [u8; 9],
    version: [u8; 9],
    machine: [u8; 9],
}

pub fn init() -> ! {
    // Test sys_time: get current Unix timestamp.
    let t = time(core::ptr::null_mut()).unwrap();
    test(t as i32).unwrap();

    // Burn user-mode CPU so utime > 0 when timer fires (HZ=100, ~10ms/tick).
    for _ in 0..1_000_000 {
        core::hint::spin_loop();
    }

    // Test sys_times: get process CPU times and jiffies.
    let mut tms = Tms {
        tms_utime: 0,
        tms_stime: 0,
        tms_cutime: 0,
        tms_cstime: 0,
    };
    let jiffies = times(&mut tms as *mut Tms as *mut u8).unwrap();
    test(jiffies as i32).unwrap();
    // Verify tms was filled: tms_utime should be > 0 (process ran in user mode).
    test(tms.tms_utime as i32).unwrap();

    // Test sys_uname: get system info.
    let mut uts = Utsname {
        sysname: [0; 9],
        nodename: [0; 9],
        release: [0; 9],
        version: [0; 9],
        machine: [0; 9],
    };
    uname(&mut uts as *mut Utsname as *mut u8).unwrap();
    // Verify uts was filled: sysname[0] should be 'l' (108) from "linux .0".
    test(uts.sysname[0] as i32).unwrap();

    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
