//! Signal-related syscall handlers (kill, signal, sigaction, alarm, pause, mask).

use user_lib::syscall::{
    Syscall,
    signal::{NSIG, SigFlags, Signal},
};

use crate::{
    define_syscall_handler,
    error::{Errno, Result},
    mm,
    segment::uaccess,
    syscall::context::SyscallContext,
    task::{self, HZ, TASK_MANAGER, is_superuser, task_struct::*},
};

define_syscall_handler!(
    Syscall::Alarm = 27,
    fn sys_alarm(ctx: &mut SyscallContext) -> Result<u32> {
        let (seconds, _, _) = ctx.args();
        let old_seconds = task::with_current(|inner| {
            let j = task::jiffies();
            let alarm = inner.signal_info.alarm;
            let old = (alarm > 0 && alarm > j)
                .then(|| (alarm - j) / HZ)
                .unwrap_or(0);
            inner.signal_info.alarm = (seconds > 0).then(|| j + HZ * seconds).unwrap_or(0);
            old
        });
        Ok(old_seconds)
    }
);

define_syscall_handler!(
    Syscall::Pause = 29,
    fn sys_pause(_ctx: &mut SyscallContext) -> Result<u32> {
        task::with_current(|inner| inner.sched.state = TaskState::Interruptible);
        task::schedule();
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Kill = 37,
    fn sys_kill(ctx: &mut SyscallContext) -> Result<u32> {
        let (pid_raw, signal_number, _) = ctx.args();
        let pid = pid_raw as i32;

        (1..=NSIG as u32)
            .contains(&signal_number)
            .then_some(())
            .ok_or(Errno::INVAL)?;

        let current_pid = task::current_pid();
        let current_euid = task::with_current(|inner| inner.identity.euid);

        fn send_signal(
            signal_number: u32,
            task: &Task,
            skip_permission_check: bool,
            current_euid: u16,
        ) -> Result<()> {
            let allowed = skip_permission_check
                || task.pcb.inner.exclusive(|inner| inner.identity.euid) == current_euid
                || is_superuser();
            allowed.then_some(()).ok_or(Errno::PERM)?;
            task.pcb.inner.exclusive(|inner| {
                inner.signal_info.raise(signal_number);
                inner.sched.wake_if_interruptible();
            });
            Ok(())
        }

        let mut result = Ok(0u32);
        TASK_MANAGER.exclusive(|manager| {
            for task in manager.tasks.iter().filter_map(|t| t.as_ref()) {
                if task.pcb.slot == 0 {
                    continue;
                }
                let matches = match pid {
                    0 => task.pcb.inner.exclusive(|i| i.relation.pgrp) == current_pid,
                    p if p > 0 => task.pcb.pid == p as u32,
                    -1 => true,
                    p => task.pcb.inner.exclusive(|i| i.relation.pgrp) == (-p) as u32,
                };
                if matches {
                    if let Err(e) = send_signal(signal_number, task, pid == 0, current_euid) {
                        result = Err(e);
                    }
                }
            }
        });
        result
    }
);

define_syscall_handler!(
    Syscall::Signal = 48,
    fn sys_signal(ctx: &mut SyscallContext) -> Result<u32> {
        let (signal_number, handler, restorer) = ctx.args();

        (1..=NSIG as u32)
            .contains(&signal_number)
            .then_some(signal_number)
            .filter(|&n| n != Signal::Kill as u32)
            .ok_or(Errno::PERM)?;

        let signal_index = (signal_number - 1) as usize;
        let old_handler = task::with_current(|inner| {
            let old = inner.signal_info.sigaction[signal_index].sa_handler;
            inner.signal_info.sigaction[signal_index] = SigAction {
                sa_handler: handler,
                sa_mask: 0,
                sa_flags: (SigFlags::ONE_SHOT | SigFlags::NO_MASK).bits(),
                sa_restorer: restorer,
            };
            old
        });
        Ok(old_handler)
    }
);

define_syscall_handler!(
    Syscall::Sgetmask = 68,
    fn sys_sgetmask(_ctx: &mut SyscallContext) -> Result<u32> {
        Ok(task::with_current(|inner| inner.signal_info.blocked))
    }
);

define_syscall_handler!(
    Syscall::Ssetmask = 69,
    fn sys_ssetmask(ctx: &mut SyscallContext) -> Result<u32> {
        let (newmask, _, _) = ctx.args();
        let old = task::with_current(|inner| {
            core::mem::replace(
                &mut inner.signal_info.blocked,
                newmask & !(1u32 << (Signal::Kill as u32 - 1)),
            )
        });
        Ok(old)
    }
);

define_syscall_handler!(
    Syscall::Sigaction = 67,
    fn sys_sigaction(ctx: &mut SyscallContext) -> Result<u32> {
        let (signal_number, action_ptr, old_action_ptr) = ctx.args();

        (1..=NSIG as u32)
            .contains(&signal_number)
            .then_some(signal_number)
            .filter(|&n| n != Signal::Kill as u32)
            .ok_or(Errno::PERM)?;

        let signal_index = (signal_number - 1) as usize;

        fn read_sigaction_from_user(ptr: u32) -> SigAction {
            let base = ptr as *const u8;
            let mut bytes = [0u8; 16];
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = uaccess::read_u8(unsafe { base.add(i) });
            }
            unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const SigAction) }
        }

        fn write_sigaction_to_user(ptr: u32, sa: &SigAction) {
            mm::ensure_user_area_writable(ptr, 16);
            let base = ptr as *mut u8;
            let sa_bytes = sa as *const SigAction as *const [u8; 16];
            for (i, b) in unsafe { *sa_bytes }.iter().enumerate() {
                uaccess::write_u8(*b, unsafe { base.add(i) });
            }
        }

        let old_sa = task::with_current(|inner| {
            let old = inner.signal_info.sigaction[signal_index];
            (action_ptr != 0).then(|| {
                inner.signal_info.sigaction[signal_index] = read_sigaction_from_user(action_ptr);
            });
            let current = inner.signal_info.sigaction[signal_index];
            inner.signal_info.sigaction[signal_index].sa_mask =
                ((current.sa_flags & SigFlags::NO_MASK.bits()) == 0)
                    .then(|| current.sa_mask | (1u32 << signal_index))
                    .unwrap_or(0);
            old
        });

        (old_action_ptr != 0).then(|| write_sigaction_to_user(old_action_ptr, &old_sa));
        Ok(0)
    }
);
