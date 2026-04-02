use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    syscall::{SYSCALL_TABLE, context::SyscallContext},
};

define_syscall_handler!(
    user_lib::NR_MKNOD = 14,
    fn sys_mknod(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_MOUNT = 21,
    fn sys_mount(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_UMOUNT = 22,
    fn sys_umount(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_PIPE = 42,
    fn sys_pipe(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_IOCTL = 54,
    fn sys_ioctl(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_FCNTL = 55,
    fn sys_fcntl(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_IAM = 72,
    fn sys_iam(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
define_syscall_handler!(
    user_lib::NR_WHOAMI = 73,
    fn sys_whoami(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);
