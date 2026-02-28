//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

mod syscall;

pub use syscall::*;

/// C-compatible sigaction layout (16 bytes).
#[repr(C)]
pub struct SigAction {
    pub sa_handler: u32,
    pub sa_mask: u32,
    pub sa_flags: u32,
    pub sa_restorer: u32,
}

pub const SIGUSR1: u32 = 10;
const SA_ONESHOT: u32 = 0x8000_0000;
const SA_NOMASK: u32 = 0x4000_0000;

extern "C" fn sigusr1_handler(_signr: i32) {
    test(1).unwrap(); // handler received SIGUSR1
}

extern "C" fn sigusr1_restorer() -> ! {
    test(2999).unwrap(); // restorer: about to exit
    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}

pub fn init() -> ! {
    let act = SigAction {
        sa_handler: sigusr1_handler as usize as u32,
        sa_mask: 0,
        sa_flags: SA_ONESHOT | SA_NOMASK,
        sa_restorer: sigusr1_restorer as usize as u32,
    };
    sigaction(SIGUSR1 as i32, &act as *const SigAction as u32, 0).unwrap();
    test(100).unwrap(); // sigaction done, about to fork

    let pid = fork().unwrap();
    if pid == 0 {
        test(200).unwrap(); // child: about to kill parent
        kill(getppid().unwrap() as i32, SIGUSR1 as i32).unwrap();
        exit().unwrap();
    }

    pause().unwrap();
    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
