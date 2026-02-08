use linkme::distributed_slice;

use crate::{
    define_syscall_handler, println,
    syscall::{SYSCALL_TABLE, context::SyscallContext},
};

define_syscall_handler!(
    NR_EXIT = 1,
    fn sys_exit(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!();
    }
);

define_syscall_handler!(
    NR_FORK = 2,
    fn sys_fork(_ctx: &SyscallContext) -> Result<u32, u32> {
        println!("entry sys_fork!");
        Ok(0)
    }
);
