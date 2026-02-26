use linkme::distributed_slice;

#[allow(unused_imports)]
use crate::syscall::SYSCALL_TABLE;

use crate::{
    define_syscall_handler, mm, segment,
    syscall::{EAGAIN, ECHILD, EPERM, ESRCH, context::SyscallContext},
    task::{self, TASK_MANAGER, is_super, task_struct::TaskState},
};

define_syscall_handler!(
    NR_EXIT = 1,
    fn sys_exit(_ctx: &SyscallContext) -> Result<u32, u32> {
        task::do_exit(0)
    }
);

define_syscall_handler!(
    NR_FORK = 2,
    fn sys_fork(ctx: &SyscallContext) -> Result<u32, u32> {
        TASK_MANAGER
            .exclusive(|manager| manager.fork(ctx))
            .map_err(|()| EAGAIN)
    }
);

define_syscall_handler!(
    NR_WAITPID = 7,
    fn sys_waitpid(ctx: &SyscallContext) -> Result<u32, u32> {
        const WNOHANG: u32 = 1;
        const WUNTRACED: u32 = 2;

        #[derive(Clone, Copy)]
        struct ChildView {
            slot: usize,
            pid: u32,
            pgrp: u32,
            state: TaskState,
            exit_code: i32,
            utime: u32,
            stime: u32,
            father: u32,
        }

        #[derive(Clone, Copy)]
        enum ScanResult {
            Stopped {
                pid: u32,
                status: u32,
            },
            Zombie {
                slot: usize,
                pid: u32,
                status: u32,
                utime: u32,
                stime: u32,
            },
            NeedWait,
            NoChild,
        }

        let (pid, stat_addr, options) = ctx.args();
        let pid = pid as i32;
        let stat_addr = stat_addr as *mut u32;
        mm::ensure_user_area_writable(stat_addr as u32, 4);

        let pid_matches = |child: &ChildView, current_pgrp: u32| -> bool {
            match pid {
                p if p > 0 => child.pid == p as u32,
                0 => child.pgrp == current_pgrp,
                -1 => true,
                p => child.pgrp == (-p) as u32,
            }
        };

        loop {
            let scan_result = TASK_MANAGER.exclusive(|manager| {
                let current = task::current_task();
                let current_slot = current.pcb.slot;
                let current_pid = current.pcb.pid;
                let current_pgrp = current.pcb.inner.exclusive(|inner| inner.relation.pgrp);

                let children = || {
                    manager
                        .tasks
                        .iter()
                        .enumerate()
                        .rev()
                        .filter_map(|(slot, task)| {
                            let task = task.as_ref()?;
                            if slot == current_slot {
                                return None;
                            }

                            let view = task.pcb.inner.exclusive(|inner| ChildView {
                                slot,
                                pid: task.pcb.pid,
                                pgrp: inner.relation.pgrp,
                                state: inner.sched.state,
                                exit_code: inner.exit_code,
                                utime: inner.acct.utime,
                                stime: inner.acct.stime,
                                father: inner.relation.father,
                            });
                            (view.father == current_pid && pid_matches(&view, current_pgrp))
                                .then_some(view)
                        })
                };

                if let Some(result) = children().find_map(|child| match child.state {
                    TaskState::Stopped if (options & WUNTRACED) != 0 => Some(ScanResult::Stopped {
                        pid: child.pid,
                        status: 0x7f,
                    }),
                    TaskState::Zombie => Some(ScanResult::Zombie {
                        slot: child.slot,
                        pid: child.pid,
                        status: child.exit_code as u32,
                        utime: child.utime,
                        stime: child.stime,
                    }),
                    _ => None,
                }) {
                    if let ScanResult::Zombie {
                        slot, utime, stime, ..
                    } = result
                    {
                        current.pcb.inner.exclusive(|inner| {
                            inner.acct.cutime = inner.acct.cutime.wrapping_add(utime);
                            inner.acct.cstime = inner.acct.cstime.wrapping_add(stime);
                        });
                        manager.tasks[slot] = None;
                    }
                    return result;
                }

                if children()
                    .any(|child| !matches!(child.state, TaskState::Stopped | TaskState::Zombie))
                {
                    ScanResult::NeedWait
                } else {
                    ScanResult::NoChild
                }
            });

            match scan_result {
                ScanResult::Stopped { pid, status } | ScanResult::Zombie { pid, status, .. } => {
                    segment::put_fs_long(status, stat_addr);
                    return Ok(pid);
                }
                ScanResult::NeedWait if (options & WNOHANG) != 0 => return Ok(0),
                ScanResult::NeedWait => {
                    task::current_task()
                        .pcb
                        .inner
                        .exclusive(|inner| inner.sched.state = TaskState::Interruptible);
                    task::schedule();
                }
                ScanResult::NoChild => return Err(ECHILD),
            }
        }
    }
);

define_syscall_handler!(
    NR_PAUSE = 29,
    fn sys_pause(_ctx: &SyscallContext) -> Result<u32, u32> {
        task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.sched.state = TaskState::Interruptible);
        task::schedule();
        Ok(0)
    }
);

// ---------------------------------------------------------------------------
// Process/group/identity syscalls
// ---------------------------------------------------------------------------

define_syscall_handler!(
    NR_GETPID = 20,
    fn sys_getpid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task().pcb.pid)
    }
);

define_syscall_handler!(
    NR_GETUID = 24,
    fn sys_getuid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.identity.uid as u32))
    }
);

define_syscall_handler!(
    NR_SETUID = 23,
    fn sys_setuid(ctx: &SyscallContext) -> Result<u32, u32> {
        let (uid, _, _) = ctx.args();
        sys_setreuid_impl(uid, uid)
    }
);

define_syscall_handler!(
    NR_GETGID = 47,
    fn sys_getgid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.identity.gid as u32))
    }
);

define_syscall_handler!(
    NR_SETGID = 46,
    fn sys_setgid(ctx: &SyscallContext) -> Result<u32, u32> {
        let (gid, _, _) = ctx.args();
        sys_setregid_impl(gid, gid)
    }
);

define_syscall_handler!(
    NR_GETEUID = 49,
    fn sys_geteuid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.identity.euid as u32))
    }
);

define_syscall_handler!(
    NR_GETEGID = 50,
    fn sys_getegid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.identity.egid as u32))
    }
);

define_syscall_handler!(
    NR_SETPGID = 57,
    fn sys_setpgid(ctx: &SyscallContext) -> Result<u32, u32> {
        let (pid_arg, pgid_arg, _) = ctx.args();
        let current = task::current_task();
        let target_pid = if pid_arg == 0 {
            current.pcb.pid
        } else {
            pid_arg
        };
        let target_pgid = if pgid_arg == 0 {
            current.pcb.pid
        } else {
            pgid_arg
        };

        let current_session = current.pcb.inner.exclusive(|inner| inner.relation.session);

        TASK_MANAGER.exclusive(|manager| {
            manager
                .tasks
                .iter()
                .enumerate()
                .find_map(|(slot, opt_task)| {
                    let task = opt_task.as_ref()?;
                    (task.pcb.pid == target_pid).then_some((slot, task))
                })
                .map(|(_, task)| {
                    let (is_leader, task_session) = task
                        .pcb
                        .inner
                        .exclusive(|inner| (inner.relation.leader != 0, inner.relation.session));
                    if is_leader || task_session != current_session {
                        Err(EPERM)
                    } else {
                        task.pcb
                            .inner
                            .exclusive(|inner| inner.relation.pgrp = target_pgid);
                        Ok(0u32)
                    }
                })
                .unwrap_or(Err(ESRCH))
        })
    }
);

define_syscall_handler!(
    NR_GETPGRP = 65,
    fn sys_getpgrp(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.relation.pgrp))
    }
);

define_syscall_handler!(
    NR_SETSID = 66,
    fn sys_setsid(_ctx: &SyscallContext) -> Result<u32, u32> {
        let current = task::current_task();
        let (is_leader, pid) = current
            .pcb
            .inner
            .exclusive(|inner| (inner.relation.leader != 0, current.pcb.pid));
        if is_leader && !is_super() {
            return Err(EPERM);
        }
        current.pcb.inner.exclusive(|inner| {
            inner.relation.leader = 1;
            inner.relation.session = pid;
            inner.relation.pgrp = pid;
            inner.tty = -1;
        });
        Ok(pid)
    }
);

define_syscall_handler!(
    NR_GETPPID = 64,
    fn sys_getppid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.relation.father))
    }
);

define_syscall_handler!(
    NR_SETREUID = 70,
    fn sys_setreuid(ctx: &SyscallContext) -> Result<u32, u32> {
        let (ruid, euid, _) = ctx.args();
        sys_setreuid_impl(ruid, euid)
    }
);

define_syscall_handler!(
    NR_SETREGID = 71,
    fn sys_setregid(ctx: &SyscallContext) -> Result<u32, u32> {
        let (rgid, egid, _) = ctx.args();
        sys_setregid_impl(rgid, egid)
    }
);

// ---------------------------------------------------------------------------
// Helpers for setreuid/setregid (shared permission logic)
// ---------------------------------------------------------------------------

fn sys_setreuid_impl(ruid: u32, euid: u32) -> Result<u32, u32> {
    let superuser = is_super();
    task::current_task().pcb.inner.exclusive(|inner| {
        let old_ruid = inner.identity.uid;
        if ruid > 0 {
            let allow = inner.identity.euid == ruid as u16 || old_ruid == ruid as u16 || superuser;
            if !allow {
                return Err(EPERM);
            }
            inner.identity.uid = ruid as u16;
        }
        if euid > 0 {
            let allow = old_ruid == euid as u16 || inner.identity.euid == euid as u16 || superuser;
            if !allow {
                inner.identity.uid = old_ruid;
                return Err(EPERM);
            }
            inner.identity.euid = euid as u16;
        }
        Ok(0)
    })
}

fn sys_setregid_impl(rgid: u32, egid: u32) -> Result<u32, u32> {
    let superuser = is_super();
    task::current_task().pcb.inner.exclusive(|inner| {
        if rgid > 0 {
            let allow = inner.identity.gid == rgid as u16 || superuser;
            if !allow {
                return Err(EPERM);
            }
            inner.identity.gid = rgid as u16;
        }
        if egid > 0 {
            let allow = inner.identity.gid == egid as u16
                || inner.identity.egid == egid as u16
                || inner.identity.sgid == egid as u16
                || superuser;
            if !allow {
                return Err(EPERM);
            }
            inner.identity.egid = egid as u16;
        }
        Ok(0)
    })
}
