mod context;
mod error;
mod handler;

use core::arch::global_asm;

pub use context::SyscallContext;
pub use error::*;
pub use handler::*;

use crate::task::TASK_MANAGER;

global_asm!(include_str!("syscall_entry.s"), options(att_syntax));

#[unsafe(no_mangle)]
pub extern "C" fn syscall_rust_entry(ctx: &SyscallContext) -> i32 {
    // Check if the syscall number is valid.
    if (ctx.syscall_nr() as usize) >= SYSCALL_TABLE.len() {
        return -(ENOSYS as i32);
    }
    // Call the syscall handler.
    let handler = SYSCALL_TABLE[ctx.syscall_nr() as usize];
    let result = handler(ctx);

    // Schedule if possible.
    TASK_MANAGER.with_mut(|manager| manager.try_schedule());

    match result {
        Ok(value) => value as i32,
        Err(errno) => -(errno as i32),
    }
}
