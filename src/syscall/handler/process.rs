use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    syscall::{SYSCALL_TABLE, context::SyscallContext},
    task::TASK_MANAGER,
};

define_syscall_handler!(
    NR_EXIT = 1,
    fn sys_exit(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!();
    }
);

define_syscall_handler!(
    NR_FORK = 2,
    fn sys_fork(ctx: &SyscallContext) -> Result<u32, u32> {
        TASK_MANAGER.with_mut(|manager| manager.fork(ctx))
    }
);
