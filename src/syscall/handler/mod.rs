pub mod process;
pub mod todo;

use linkme::distributed_slice;

use crate::syscall::context::SyscallContext;
use crate::{
    signal,
    syscall::EINVAL,
    task,
    task::task_struct::{NSIG, SigAction},
};

#[distributed_slice]
pub static SYSCALL_TABLE: [fn(&SyscallContext) -> Result<u32, u32>];

// linkme requires an integer literal in `distributed_slice(..., N)`.
// This helper keeps a named syscall number and the required literal in one place.
#[macro_export]
macro_rules! define_syscall_handler {
    (
        $nr_name:ident = $nr:literal,
        fn $fn_name:ident($ctx:ident : &SyscallContext) -> $ret:ty $body:block
    ) => {
        pub const $nr_name: u32 = $nr;

        #[distributed_slice(SYSCALL_TABLE, $nr)]
        fn $fn_name($ctx: &SyscallContext) -> $ret $body
    };
}

define_syscall_handler!(
    NR_TEST = 74,
    fn sys_test(ctx: &SyscallContext) -> Result<u32, u32> {
        let (value, _, _) = ctx.args();
        crate::println!("test value: {}", value as i32);
        Ok(0)
    }
);

define_syscall_handler!(
    NR_TEST1 = 75,
    fn sys_test1(ctx: &SyscallContext) -> Result<u32, u32> {
        let (handler, restorer, signr) = ctx.args();
        if signr == 0 || signr > NSIG as u32 {
            return Err(EINVAL);
        }

        let idx = (signr - 1) as usize;
        task::current_task().pcb.inner.exclusive(|inner| {
            inner.signal_info.sigaction[idx] = SigAction {
                sa_handler: handler,
                sa_mask: 0,
                sa_flags: signal::SA_ONESHOT | signal::SA_NOMASK,
                sa_restorer: restorer,
            };
            inner.signal_info.signal |= 1u32 << idx;
        });

        crate::println!(
            "test1 inject signal={} handler={:#x} restorer={:#x}",
            signr,
            handler,
            restorer
        );
        Ok(0)
    }
);
