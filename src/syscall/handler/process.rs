use linkme::distributed_slice;

use crate::{
    define_syscall_handler, sync,
    syscall::{SYSCALL_TABLE, context::SyscallContext},
    task::{self, TASK_MANAGER, task_struct::TaskState},
};

define_syscall_handler!(
    NR_FORK = 2,
    fn sys_fork(ctx: &SyscallContext) -> Result<u32, u32> {
        sync::cli();
        let result = TASK_MANAGER.with_mut(|manager| manager.fork(ctx));
        sync::sti();
        result
    }
);

define_syscall_handler!(
    NR_PAUSE = 29,
    fn sys_pause(_ctx: &SyscallContext) -> Result<u32, u32> {
        sync::cli();
        TASK_MANAGER
            .borrow_mut()
            .current()
            .pcb
            .inner
            .borrow_mut()
            .sched
            .state = TaskState::Interruptible;
        task::schedule();
        sync::sti();
        Ok(0)
    }
);
