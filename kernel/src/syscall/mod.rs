//! System call dispatch via `int 0x80`.
//!
//! The naked `system_call` entry stub saves registers and calls
//! [`syscall_rust_entry`], which indexes into the [`SYSCALL_TABLE`](handler::SYSCALL_TABLE)
//! distributed slice populated by [`define_syscall_handler!`](crate::define_syscall_handler).

mod context;
mod handler;

use core::arch::naked_asm;

pub use context::SyscallContext;
pub use handler::*;

use crate::{
    error::Errno,
    signal,
    task::{self, TaskState},
};

/// `int 0x80` entry from user mode.
///
/// Push order matches [`SyscallContext`](SyscallContext) layout. Callee-saved
/// registers are captured so fork/exec can read them from the frame. After
/// [`syscall_rust_entry`] returns, the saved EAX slot is overwritten with the
/// return value so `popl %eax` restores it before `iret`.
#[naked]
pub extern "C" fn system_call() {
    unsafe {
        naked_asm!(
            "push %ds",
            "push %es",
            "push %fs",
            "pushl %edx",
            "pushl %ecx",
            "pushl %ebx",
            "pushl %eax",
            "pushl %ebp",
            "pushl %edi",
            "pushl %esi",
            "push %gs",
            "movl $0x10, %edx",
            "mov %dx, %ds",
            "mov %dx, %es",
            "movl $0x17, %edx",
            "mov %dx, %fs",
            "movl %esp, %eax",
            "pushl %eax",
            "call {rust_entry}",
            "addl $4, %esp",
            "movl %eax, 16(%esp)",
            "addl $16, %esp",
            "popl %eax",
            "popl %ebx",
            "popl %ecx",
            "popl %edx",
            "pop %fs",
            "pop %es",
            "pop %ds",
            "iret",
            rust_entry = sym syscall_rust_entry,
            options(att_syntax),
        );
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn syscall_rust_entry(ctx: &mut SyscallContext) -> i32 {
    // Check if the syscall number is valid.
    if (ctx.syscall_nr() as usize) >= SYSCALL_TABLE.len() {
        return -(Errno::NOSYS.code() as i32);
    }
    // Call the syscall handler.
    let handler = SYSCALL_TABLE[ctx.syscall_nr() as usize];
    let result = handler(ctx);

    // Schedule if needed.
    let needs_schedule = task::with_current(|inner| {
        inner.sched.state != TaskState::Running || inner.sched.counter == 0
    });
    if needs_schedule {
        task::schedule();
    }

    let ret = match result {
        Ok(value) => value as i32,
        Err(e) => -(e.code() as i32),
    };
    ctx.eax = ret as u32;
    signal::handle_pending_signal(ctx);
    ret
}
