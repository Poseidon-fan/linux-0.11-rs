use linkme::distributed_slice;

use crate::{
    define_syscall_handler, mm, segment,
    syscall::{EAGAIN, ECHILD, SYSCALL_TABLE, context::SyscallContext},
    task::{self, TASK_MANAGER, current_task, task_struct::TaskState},
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
                    current_task()
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
        current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.sched.state = TaskState::Interruptible);
        task::schedule();
        Ok(0)
    }
);
