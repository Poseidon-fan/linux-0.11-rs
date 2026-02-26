use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    syscall::{EAGAIN, SYSCALL_TABLE, context::SyscallContext},
    task::{self, TASK_MANAGER, current_task, task_struct::TaskState},
};

define_syscall_handler!(
    NR_EXIT = 1,
    fn sys_exit(_ctx: &SyscallContext) -> Result<u32, u32> {
        task::do_exit(0)
    }
);

define_syscall_handler!(
    NR_FORK = 2,
    fn sys_fork(ctx: &SyscallContext) -> Result<u32, u32> {
        TASK_MANAGER
            .exclusive(|manager| manager.fork(ctx))
            .map_err(|()| EAGAIN)
    }
);

define_syscall_handler!(
    NR_PAUSE = 29,
    fn sys_pause(_ctx: &SyscallContext) -> Result<u32, u32> {
        current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.sched.state = TaskState::Interruptible);
        task::schedule();
        Ok(0)
    }
);
