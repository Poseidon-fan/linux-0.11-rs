//! Placeholder handlers for unimplemented syscall numbers.

use user_lib::syscall::Syscall;

use crate::{
    define_syscall_handler,
    error::{Errno, Result},
    syscall::context::SyscallContext,
};

define_syscall_handler!(
    Syscall::Break = 17,
    fn sys_break(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Stty = 31,
    fn sys_stty(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Gtty = 32,
    fn sys_gtty(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Ftime = 35,
    fn sys_ftime(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Prof = 44,
    fn sys_prof(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Acct = 51,
    fn sys_acct(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Phys = 52,
    fn sys_phys(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Lock = 53,
    fn sys_lock(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Mpx = 56,
    fn sys_mpx(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Ulimit = 58,
    fn sys_ulimit(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Ptrace = 26,
    fn sys_ptrace(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
define_syscall_handler!(
    Syscall::Ustat = 62,
    fn sys_ustat(_ctx: &mut SyscallContext) -> Result<u32> {
        Err(Errno::NOSYS)
    }
);
