//! Placeholder handlers for syscalls not yet implemented.
//! Each function body is `todo!()` until the real implementation is added.

use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    syscall::{SYSCALL_TABLE, context::SyscallContext},
};

define_syscall_handler!(
    user_lib::NR_CREAT = 8,
    fn sys_creat(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_LINK = 9,
    fn sys_link(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_EXECVE = 11,
    fn sys_execve(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_CHDIR = 12,
    fn sys_chdir(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_MKNOD = 14,
    fn sys_mknod(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_CHMOD = 15,
    fn sys_chmod(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_CHOWN = 16,
    fn sys_chown(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_BREAK = 17,
    fn sys_break(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_STAT = 18,
    fn sys_stat(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_MOUNT = 21,
    fn sys_mount(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_UMOUNT = 22,
    fn sys_umount(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_STIME = 25,
    fn sys_stime(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_PTRACE = 26,
    fn sys_ptrace(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_FSTAT = 28,
    fn sys_fstat(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_UTIME = 30,
    fn sys_utime(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_STTY = 31,
    fn sys_stty(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_GTTY = 32,
    fn sys_gtty(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_ACCESS = 33,
    fn sys_access(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_NICE = 34,
    fn sys_nice(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_FTIME = 35,
    fn sys_ftime(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_SYNC = 36,
    fn sys_sync(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_RENAME = 38,
    fn sys_rename(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_MKDIR = 39,
    fn sys_mkdir(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_RMDIR = 40,
    fn sys_rmdir(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_DUP = 41,
    fn sys_dup(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_PIPE = 42,
    fn sys_pipe(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_PROF = 44,
    fn sys_prof(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_BRK = 45,
    fn sys_brk(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_ACCT = 51,
    fn sys_acct(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_PHYS = 52,
    fn sys_phys(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_LOCK = 53,
    fn sys_lock(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_IOCTL = 54,
    fn sys_ioctl(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_FCNTL = 55,
    fn sys_fcntl(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_MPX = 56,
    fn sys_mpx(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_ULIMIT = 58,
    fn sys_ulimit(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_UMASK = 60,
    fn sys_umask(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_CHROOT = 61,
    fn sys_chroot(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_USTAT = 62,
    fn sys_ustat(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_DUP2 = 63,
    fn sys_dup2(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_IAM = 72,
    fn sys_iam(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_WHOAMI = 73,
    fn sys_whoami(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
