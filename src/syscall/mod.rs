mod context;
mod error;
mod handler;

use core::arch::global_asm;

use log::debug;

use crate::syscall::{context::SyscallContext, error::ENOSYS, handler::SYSCALL_TABLE};
pub use handler::*;

global_asm!(include_str!("syscall_entry.s"), options(att_syntax));

#[unsafe(no_mangle)]
pub extern "C" fn syscall_rust_entry(ctx: &SyscallContext) -> isize {
    debug!("syscall_rust_entry: {:?}", ctx);
    // Check if the syscall number is valid.
    if (ctx.syscall_nr() as usize) >= SYSCALL_TABLE.len() {
        return -(ENOSYS as isize);
    }
    // Call the syscall handler.
    let handler = SYSCALL_TABLE[ctx.syscall_nr() as usize];
    let result = handler(ctx);

    match result {
        Ok(result) => result as isize,
        Err(errno) => -(errno as isize),
    }
}
