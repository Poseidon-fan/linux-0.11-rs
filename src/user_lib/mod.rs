//! User-space library — provides system call wrappers and utility functions
//! that run in ring 3 (user mode) after `move_to_user_mode()`.

mod syscall;

pub use syscall::*;

#[repr(C)]
pub struct SigAction {
    pub sa_handler: u32,
    pub sa_mask: u32,
    pub sa_flags: u32,
    pub sa_restorer: u32,
}

const SIGALRM: u32 = 14;
const SA_ONESHOT: u32 = 0x8000_0000;
const SA_NOMASK: u32 = 0x4000_0000;

extern "C" fn sigalrm_handler(_signr: i32) {
    test(1).unwrap();
    exit().unwrap();
}

extern "C" fn sigalrm_restorer() {}

pub fn init() -> ! {
    let act = SigAction {
        sa_handler: sigalrm_handler as usize as u32,
        sa_mask: 0,
        sa_flags: SA_ONESHOT | SA_NOMASK,
        sa_restorer: sigalrm_restorer as usize as u32,
    };
    sigaction(SIGALRM as i32, &act as *const SigAction as u32, 0).unwrap();
    alarm(1).unwrap();
    pause().unwrap();
    exit().unwrap();
    #[allow(clippy::empty_loop)]
    loop {}
}
