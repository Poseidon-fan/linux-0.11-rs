//! Placeholder handlers for unimplemented syscall numbers.

use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    syscall::{ENOSYS, SYSCALL_TABLE, context::SyscallContext},
};

define_syscall_handler!(
    user_lib::syscall::NR_BREAK = 17,
    fn sys_break(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_STTY = 31,
    fn sys_stty(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_GTTY = 32,
    fn sys_gtty(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_FTIME = 35,
    fn sys_ftime(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_PROF = 44,
    fn sys_prof(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_ACCT = 51,
    fn sys_acct(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_PHYS = 52,
    fn sys_phys(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_LOCK = 53,
    fn sys_lock(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_MPX = 56,
    fn sys_mpx(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_ULIMIT = 58,
    fn sys_ulimit(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_PTRACE = 26,
    fn sys_ptrace(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_USTAT = 62,
    fn sys_ustat(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
define_syscall_handler!(
    user_lib::syscall::NR_RENAME = 38,
    fn sys_rename(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Err(ENOSYS)
    }
);
