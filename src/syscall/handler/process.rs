use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    syscall::{SYSCALL_TABLE, context::SyscallContext},
    task::{self, TASK_MANAGER, task_struct::TaskState},
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
        TASK_MANAGER.with_mut_irqsave(|manager| manager.fork(ctx))
    }
);

define_syscall_handler!(
    NR_PAUSE = 29,
    fn sys_pause(_ctx: &SyscallContext) -> Result<u32, u32> {
        let next = TASK_MANAGER.with_mut_irqsave(|manager| {
            manager.current().pcb.inner.borrow_mut().sched.state = TaskState::Interruptible;
            manager.schedule()
        });
        if let Some(next) = next {
            task::switch_to(next);
        }
        Ok(0)
    }
);
