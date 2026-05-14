//! Process-related syscall handlers.
//!
//! Large or logically cohesive groups are split into dedicated submodules:
//!
//! - [`exec`]     — execve
//! - [`fork`]     — fork
//! - [`waitpid`]  — waitpid
//! - [`signal`]   — kill, signal, sigaction, alarm, pause, signal masks
//! - [`identity`] — pid/uid/gid/session/group getters and setters
//! - [`time`]     — time, stime, times, uname
//!
//! This file contains the remaining small miscellaneous handlers:
//! exit, nice, brk, umask.

mod exec;
mod fork;
mod identity;
mod signal;
mod time;
mod waitpid;

use user_lib::syscall::nr::Syscall;

use crate::{define_syscall_handler, error::Result, syscall::context::SyscallContext, task};

define_syscall_handler!(
    Syscall::Exit = 1,
    fn sys_exit(ctx: &mut SyscallContext) -> Result<u32> {
        let (status, _, _) = ctx.args();
        task::exit_process(((status & 0xff) << 8) as i32)
    }
);

define_syscall_handler!(
    Syscall::Nice = 34,
    fn sys_nice(ctx: &mut SyscallContext) -> Result<u32> {
        let (increment, _, _) = ctx.args();
        task::with_current(|inner| {
            if inner.sched.priority > increment {
                inner.sched.priority -= increment;
            }
        });
        Ok(0)
    }
);

const MIN_STACK_GAP: u32 = 16384;

define_syscall_handler!(
    Syscall::Brk = 45,
    fn sys_brk(ctx: &mut SyscallContext) -> Result<u32> {
        let (end_data_seg, _, _) = ctx.args();
        Ok(task::with_current(|inner| {
            let layout = &mut inner.mem_layout;
            if end_data_seg >= layout.end_code && end_data_seg < layout.start_stack - MIN_STACK_GAP
            {
                layout.brk = end_data_seg;
            }
            layout.brk
        }))
    }
);

define_syscall_handler!(
    Syscall::Umask = 60,
    fn sys_umask(ctx: &mut SyscallContext) -> Result<u32> {
        let (mask, _, _) = ctx.args();
        Ok(task::with_current(|inner| {
            let old = inner.fs.umask as u32;
            inner.fs.umask = (mask & 0o777) as u16;
            old
        }))
    }
);
