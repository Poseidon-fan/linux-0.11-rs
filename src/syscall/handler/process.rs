use linkme::distributed_slice;
use log::debug;

use crate::{
    define_syscall_handler,
    syscall::{SYSCALL_TABLE, context::SyscallContext, error::EAGAIN},
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
        let slot = TASK_MANAGER
            .with_mut(|manager| manager.find_empty_process())
            .ok_or(EAGAIN)?;

        debug!(
            "sys_fork: slot={}, gs={:#x}, esi={:#x}, edi={:#x}, ebp={:#x}, eip={:#x}, esp={:#x}",
            slot, ctx.gs, ctx.esi, ctx.edi, ctx.ebp, ctx.eip, ctx.user_esp
        );

        // TODO: copy_process(slot, ctx)
        Ok(0)
    }
);
