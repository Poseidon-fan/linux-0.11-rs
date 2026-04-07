use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    syscall::{SYSCALL_TABLE, context::SyscallContext},
};

define_syscall_handler!(
    user_lib::NR_PIPE = 42,
    fn sys_pipe(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
