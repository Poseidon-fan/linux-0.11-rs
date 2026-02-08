use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    syscall::{SYSCALL_TABLE, context::SyscallContext},
};

define_syscall_handler!(
    NR_SETUP = 0,
    fn sys_setup(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!();
    }
);
