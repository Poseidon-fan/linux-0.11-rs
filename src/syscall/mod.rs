mod context;
mod error;
mod handler;

use core::arch::global_asm;

pub use context::SyscallContext;
pub use error::*;
pub use handler::*;

use crate::task::{self, TASK_MANAGER, current_task, task_struct::TaskState};

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

    // Schedule if needed.
    let should_schedule = current_task().pcb.inner.exclusive(|current| {
        current.sched.state != TaskState::Running || current.sched.counter == 0
    });
    if should_schedule {
        if let Some(next) = TASK_MANAGER.exclusive(|m| m.schedule()) {
            task::switch_to(next);
        }
    }

    match result {
        Ok(value) => value as i32,
        Err(errno) => -(errno as i32),
    }
}
