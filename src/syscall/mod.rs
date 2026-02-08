mod context;

use core::arch::global_asm;

use linkme::distributed_slice;

use crate::{println, syscall::context::SyscallContext};

global_asm!(include_str!("syscall_entry.s"), options(att_syntax));

#[distributed_slice]
static SYSCALL_TABLE: [extern "C" fn() -> isize];

#[unsafe(no_mangle)]
pub extern "C" fn syscall_rust_entry(ctx: &SyscallContext) {
    println!("syscall_rust_entry: {:?}", ctx);
}
