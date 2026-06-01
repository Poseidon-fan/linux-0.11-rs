//! `waitpid` syscall — reap child processes.

use user_lib::syscall::{Syscall, signal::Signal};

use crate::{
    define_syscall_handler,
    error::{Errno, Result},
    mm,
    segment::uaccess,
    syscall::context::SyscallContext,
    task::{self, TASK_MANAGER, TaskState},
};

define_syscall_handler!(
    Syscall::Waitpid = 7,
    fn sys_waitpid(ctx: &mut SyscallContext) -> Result<u32> {
        const WNOHANG: u32 = 1;
        const WUNTRACED: u32 = 2;

        /// Snapshot of a child task's fields taken under the task-table lock,
        /// so the rest of the scan can run without holding the child's PCB.
        #[derive(Clone, Copy)]
        struct ChildView {
            /// Index of the child in the task table.
            slot: usize,
            /// Child process ID.
            pid: u32,
            /// Child process group ID.
            pgrp: u32,
            /// Current scheduling state.
            state: TaskState,
            /// Exit/stop status word.
            exit_code: i32,
            /// Accumulated user-mode time (jiffies).
            utime: u32,
            /// Accumulated system-mode time (jiffies).
            stime: u32,
            /// Parent process ID.
            father: u32,
        }

        /// Outcome of one pass over the caller's children.
        #[derive(Clone, Copy)]
        enum ScanResult {
            /// A stopped child was found (reported when `WUNTRACED` is set).
            Stopped {
                /// PID to report.
                pid: u32,
                /// Status word to write back to user space.
                status: u32,
            },
            /// A zombie child was found and reaped.
            Zombie {
                /// Task-table slot freed by reaping.
                slot: usize,
                /// PID to report.
                pid: u32,
                /// Status word to write back to user space.
                status: u32,
                /// Child user-mode time, folded into the parent's totals.
                utime: u32,
                /// Child system-mode time, folded into the parent's totals.
                stime: u32,
            },
            /// Matching live children exist; the caller must block and retry.
            NeedWait,
            /// No matching children exist.
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
                        inner.signal_info.clear(Signal::Chld as u32);
                        inner.signal_info.signal != 0
                    }) {
                        return Err(Errno::INTR);
                    }
                }
                ScanResult::NoChild => return Err(Errno::CHILD),
            }
        }
    }
);
