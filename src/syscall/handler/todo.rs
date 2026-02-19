//! Placeholder handlers for syscalls not yet implemented.
//! Each function body is `todo!()` until the real implementation is added.

use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    syscall::{SYSCALL_TABLE, context::SyscallContext},
};

define_syscall_handler!(
    NR_SETUP = 0,
    fn sys_setup(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_EXIT = 1,
    fn sys_exit(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_WRITE = 4,
    fn sys_write(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_OPEN = 5,
    fn sys_open(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_CLOSE = 6,
    fn sys_close(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_WAITPID = 7,
    fn sys_waitpid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_CREAT = 8,
    fn sys_creat(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_LINK = 9,
    fn sys_link(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_UNLINK = 10,
    fn sys_unlink(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_EXECVE = 11,
    fn sys_execve(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_CHDIR = 12,
    fn sys_chdir(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_TIME = 13,
    fn sys_time(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_MKNOD = 14,
    fn sys_mknod(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_CHMOD = 15,
    fn sys_chmod(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_CHOWN = 16,
    fn sys_chown(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_BREAK = 17,
    fn sys_break(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_STAT = 18,
    fn sys_stat(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_LSEEK = 19,
    fn sys_lseek(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_GETPID = 20,
    fn sys_getpid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_MOUNT = 21,
    fn sys_mount(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_UMOUNT = 22,
    fn sys_umount(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SETUID = 23,
    fn sys_setuid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_GETUID = 24,
    fn sys_getuid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_STIME = 25,
    fn sys_stime(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_PTRACE = 26,
    fn sys_ptrace(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_ALARM = 27,
    fn sys_alarm(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_FSTAT = 28,
    fn sys_fstat(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_UTIME = 30,
    fn sys_utime(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_STTY = 31,
    fn sys_stty(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_GTTY = 32,
    fn sys_gtty(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_ACCESS = 33,
    fn sys_access(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_NICE = 34,
    fn sys_nice(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_FTIME = 35,
    fn sys_ftime(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SYNC = 36,
    fn sys_sync(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_KILL = 37,
    fn sys_kill(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_RENAME = 38,
    fn sys_rename(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_MKDIR = 39,
    fn sys_mkdir(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_RMDIR = 40,
    fn sys_rmdir(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_DUP = 41,
    fn sys_dup(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_PIPE = 42,
    fn sys_pipe(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_TIMES = 43,
    fn sys_times(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_PROF = 44,
    fn sys_prof(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_BRK = 45,
    fn sys_brk(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SETGID = 46,
    fn sys_setgid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_GETGID = 47,
    fn sys_getgid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SIGNAL = 48,
    fn sys_signal(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_GETEUID = 49,
    fn sys_geteuid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_GETEGID = 50,
    fn sys_getegid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_ACCT = 51,
    fn sys_acct(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_PHYS = 52,
    fn sys_phys(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_LOCK = 53,
    fn sys_lock(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_IOCTL = 54,
    fn sys_ioctl(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_FCNTL = 55,
    fn sys_fcntl(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_MPX = 56,
    fn sys_mpx(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SETPGID = 57,
    fn sys_setpgid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_ULIMIT = 58,
    fn sys_ulimit(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_UNAME = 59,
    fn sys_uname(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_UMASK = 60,
    fn sys_umask(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_CHROOT = 61,
    fn sys_chroot(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_USTAT = 62,
    fn sys_ustat(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_DUP2 = 63,
    fn sys_dup2(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_GETPPID = 64,
    fn sys_getppid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_GETPGRP = 65,
    fn sys_getpgrp(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SETSID = 66,
    fn sys_setsid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SIGACTION = 67,
    fn sys_sigaction(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SGETMASK = 68,
    fn sys_sgetmask(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SSETMASK = 69,
    fn sys_ssetmask(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SETREUID = 70,
    fn sys_setreuid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_SETREGID = 71,
    fn sys_setregid(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_IAM = 72,
    fn sys_iam(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    NR_WHOAMI = 73,
    fn sys_whoami(_ctx: &SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
