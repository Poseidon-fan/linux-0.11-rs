//! `waitpid` syscall — reap child processes.

use user_lib::syscall::process::SIGCHLD;

use crate::{
    define_syscall_handler,
    error::{ECHILD, EINTR, Result},
    mm,
    segment::uaccess,
    syscall::context::SyscallContext,
    task::{self, TASK_MANAGER, TaskState},
};

define_syscall_handler!(
    user_lib::syscall::NR_WAITPID = 7,
    fn sys_waitpid(ctx: &mut SyscallContext) -> Result<u32> {
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

        let current_slot = task::current_slot();
        let current_pid = task::current_pid();

        loop {
            let scan_result = TASK_MANAGER.exclusive(|manager| {
                let current_pgrp = task::with_current(|inner| inner.relation.pgrp);

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
                        task::with_current(|inner| {
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
                    uaccess::write_u32(status, stat_addr);
                    return Ok(pid);
                }
                ScanResult::NeedWait if (options & WNOHANG) != 0 => return Ok(0),
                ScanResult::NeedWait => {
                    task::with_current(|inner| inner.sched.state = TaskState::Interruptible);
                    task::schedule();
                    if task::with_current(|inner| {
                        inner.signal_info.clear(SIGCHLD);
                        inner.signal_info.signal != 0
                    }) {
                        return Err(EINTR);
                    }
                }
                ScanResult::NoChild => return Err(ECHILD),
            }
        }
    }
);
