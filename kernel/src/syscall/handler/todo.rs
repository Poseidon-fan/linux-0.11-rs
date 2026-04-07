use alloc::sync::Arc;
use linkme::distributed_slice;
use user_lib::fs::F_DUPFD;
use user_lib::fs::F_GETFD;
use user_lib::fs::F_GETFL;
use user_lib::fs::F_SETFD;
use user_lib::fs::F_SETFL;

use crate::{
    define_syscall_handler,
    syscall::{SYSCALL_TABLE, context::SyscallContext, *},
    task::{self, task_struct::TASK_OPEN_FILES_LIMIT},
};

use super::fs::get_file;

define_syscall_handler!(
    user_lib::NR_PIPE = 42,
    fn sys_pipe(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        todo!()
    }
);

define_syscall_handler!(
    user_lib::NR_IOCTL = 54,
    fn sys_ioctl(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, cmd, arg) = ctx.args();
        let file = get_file(fd)?;
        file.ioctl(cmd, arg)
    }
);

define_syscall_handler!(
    user_lib::NR_FCNTL = 55,
    fn sys_fcntl(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (fd, cmd, arg) = ctx.args();
        let file = get_file(fd)?;

        match cmd {
            F_DUPFD => task::current_task().pcb.inner.exclusive(|inner| {
                let new_fd = (arg as usize..TASK_OPEN_FILES_LIMIT)
                    .find(|&i| inner.fs.open_files[i].is_none())
                    .ok_or(EMFILE)?;
                inner.fs.open_files[new_fd] = Some(Arc::clone(&file));
                inner.fs.close_on_exec &= !(1 << new_fd);
                Ok(new_fd as u32)
            }),

            F_GETFD => {
                let cloexec = task::current_task()
                    .pcb
                    .inner
                    .exclusive(|inner| (inner.fs.close_on_exec >> fd) & 1);
                Ok(cloexec)
            }

            F_SETFD => {
                task::current_task().pcb.inner.exclusive(|inner| {
                    if arg & 1 != 0 {
                        inner.fs.close_on_exec |= 1 << fd;
                    } else {
                        inner.fs.close_on_exec &= !(1 << fd);
                    }
                });
                Ok(0)
            }

            F_GETFL | F_SETFL => Ok(0),

            _ => Err(EINVAL),
        }
    }
);
