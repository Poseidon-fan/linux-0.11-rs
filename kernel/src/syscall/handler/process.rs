use linkme::distributed_slice;

#[allow(unused_imports)]
use crate::syscall::SYSCALL_TABLE;

use alloc::sync::Arc;

use crate::{
    define_syscall_handler, mm,
    mm::space::TASK_LINEAR_SIZE,
    segment::{self, KERNEL_DS, uaccess},
    signal::{NSIG, SA_NOMASK, SA_ONESHOT, SIGCHLD, SIGKILL},
    syscall::{EAGAIN, ECHILD, EINTR, EINVAL, EPERM, ESRCH, context::SyscallContext},
    task::{self, HZ, TASK_MANAGER, is_super, task_struct::*},
    time,
};

define_syscall_handler!(
    user_lib::NR_EXIT = 1,
    fn sys_exit(_ctx: &SyscallContext) -> Result<u32, u32> {
        task::do_exit(0)
    }
);

define_syscall_handler!(
    user_lib::NR_FORK = 2,
    fn sys_fork(ctx: &SyscallContext) -> Result<u32, u32> {
        unsafe extern "C" {
            static pg_dir: u8;
        }

        // 1. Find a free slot and allocate a new task page.
        let (slot, pid) = TASK_MANAGER
            .exclusive(|manager| manager.find_empty_process())
            .ok_or(EAGAIN)?;
        let mut new_task = Task::new().ok_or(EAGAIN)?;

        // 2. Build child PCB from parent state with COW memory.
        let parent = task::current_task();
        let new_base = slot as u32 * TASK_LINEAR_SIZE;
        let stack_top = new_task.stack_top();
        let cr3 = unsafe { &pg_dir as *const u8 as u32 };

        let child_inner = parent.pcb.inner.exclusive(|p| {
            let data_base = p.ldt.data_segment().base();
            let code_base = p.ldt.code_segment().base();
            assert_eq!(data_base, code_base, "separate I&D not supported");

            let code_limit = p.ldt.code_segment().byte_limit();
            let data_limit = p.ldt.data_segment().byte_limit();
            assert!(
                data_limit >= code_limit,
                "bad data_limit: data < code (0x{:x} < 0x{:x})",
                data_limit,
                code_limit
            );

            let child_memory_space = p
                .memory_space
                .as_ref()
                .expect("parent has no memory space")
                .cow_copy(slot, data_limit)
                .map_err(|_| EAGAIN)?;

            let mut child_ldt = p.ldt.clone();
            child_ldt.set_base(new_base);

            Ok::<_, u32>(TaskControlBlockInner {
                sched: TaskSchedInfo {
                    state: TaskState::Running,
                    counter: p.sched.priority,
                    priority: p.sched.priority,
                },
                relation: TaskRelationInfo {
                    father: parent.pcb.pid,
                    pgrp: p.relation.pgrp,
                    session: p.relation.session,
                    leader: 0,
                },
                identity: p.identity,
                acct: TaskAcctInfo::default(),
                memory_space: Some(child_memory_space),
                mem_layout: p.mem_layout,
                exit_code: 0,
                tty: p.tty,
                fs: p.fs.clone(),
                ldt: child_ldt,
                tss: child_tss(ctx, stack_top, cr3, slot),
                signal_info: TaskSignalInfo {
                    signal: 0,
                    blocked: p.signal_info.blocked,
                    sigaction: p.signal_info.sigaction.clone(),
                    alarm: 0,
                },
            })
        })?;

        new_task.pcb = TaskControlBlock::new(slot, pid, child_inner);

        // 3. Install TSS and LDT descriptors in GDT.
        let (tss_addr, ldt_addr) = new_task.pcb.inner.exclusive(|inner| {
            (
                &inner.tss as *const TaskStateSegment as u32,
                inner.ldt.as_ptr(),
            )
        });
        task::set_tss_desc(slot as u16, tss_addr);
        task::set_ldt_desc(slot as u16, ldt_addr);

        // 4. Insert into task table.
        TASK_MANAGER.exclusive(|manager| {
            manager.tasks[slot] = Some(Arc::new(new_task));
        });

        Ok(pid)
    }
);

/// EAX = 0 so `fork()` returns 0 in the child; all other registers
/// and segment selectors are copied from the parent's syscall context.
fn child_tss(ctx: &SyscallContext, stack_top: u32, cr3: u32, slot: usize) -> TaskStateSegment {
    TaskStateSegment {
        back_link: 0,
        esp0: stack_top,
        ss0: KERNEL_DS.as_u32(),
        esp1: 0,
        ss1: 0,
        esp2: 0,
        ss2: 0,
        cr3,
        eip: ctx.eip,
        eflags: ctx.eflags,
        eax: 0,
        ecx: ctx.ecx,
        edx: ctx.edx,
        ebx: ctx.ebx,
        esp: ctx.user_esp,
        ebp: ctx.ebp,
        esi: ctx.esi,
        edi: ctx.edi,
        es: ctx.es & 0xffff,
        cs: ctx.cs & 0xffff,
        ss: ctx.user_ss & 0xffff,
        ds: ctx.ds & 0xffff,
        fs: ctx.fs & 0xffff,
        gs: ctx.gs & 0xffff,
        ldt: segment::ldt_selector(slot as u16).as_u32(),
        trace_bitmap: 0x8000_0000,
        i387: I387Struct::empty(),
    }
}

define_syscall_handler!(
    user_lib::NR_WAITPID = 7,
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
                    uaccess::write_u32(status, stat_addr);
                    return Ok(pid);
                }
                ScanResult::NeedWait if (options & WNOHANG) != 0 => return Ok(0),
                ScanResult::NeedWait => {
                    task::current_task()
                        .pcb
                        .inner
                        .exclusive(|inner| inner.sched.state = TaskState::Interruptible);
                    task::schedule();
                    if task::current_task().pcb.inner.exclusive(|inner| {
                        inner.signal_info.signal &= !(1 << (SIGCHLD - 1));
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

define_syscall_handler!(
    user_lib::NR_ALARM = 27,
    fn sys_alarm(ctx: &SyscallContext) -> Result<u32, u32> {
        let (seconds, _, _) = ctx.args();
        let old_seconds = task::current_task().pcb.inner.exclusive(|inner| {
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
    user_lib::NR_PAUSE = 29,
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
// Signal syscalls
// ---------------------------------------------------------------------------

define_syscall_handler!(
    user_lib::NR_KILL = 37,
    fn sys_kill(ctx: &SyscallContext) -> Result<u32, u32> {
        let (pid_arg, sig_arg, _) = ctx.args();
        let pid = pid_arg as i32;
        let sig = sig_arg;

        (1..=NSIG as u32)
            .contains(&sig)
            .then_some(())
            .ok_or(EINVAL)?;

        let current = task::current_task();
        let current_pid = current.pcb.pid;
        let current_euid = current.pcb.inner.exclusive(|inner| inner.identity.euid);

        fn send_sig(sig: u32, task: &Task, priv_flag: bool, current_euid: u16) -> Result<(), u32> {
            let idx = (sig - 1) as usize;
            let allowed = priv_flag
                || task.pcb.inner.exclusive(|inner| inner.identity.euid) == current_euid
                || is_super();
            allowed.then_some(()).ok_or(EPERM)?;
            task.pcb.inner.exclusive(|inner| {
                inner.signal_info.signal |= 1u32 << idx;
                (inner.sched.state == TaskState::Interruptible)
                    .then(|| inner.sched.state = TaskState::Running);
            });
            Ok(())
        }

        let mut retval = Ok(0u32);
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
                    if let Err(e) = send_sig(sig, task, pid == 0, current_euid) {
                        retval = Err(e);
                    }
                }
            }
        });
        retval
    }
);

define_syscall_handler!(
    user_lib::NR_SIGNAL = 48,
    fn sys_signal(ctx: &SyscallContext) -> Result<u32, u32> {
        let (signum, handler, restorer) = ctx.args();

        (1..=NSIG as u32)
            .contains(&signum)
            .then_some(signum)
            .filter(|&s| s != SIGKILL)
            .ok_or(EPERM)?;

        let idx = (signum - 1) as usize;
        let old_handler = task::current_task().pcb.inner.exclusive(|inner| {
            let old = inner.signal_info.sigaction[idx].sa_handler;
            inner.signal_info.sigaction[idx] = SigAction {
                sa_handler: handler,
                sa_mask: 0,
                sa_flags: SA_ONESHOT | SA_NOMASK,
                sa_restorer: restorer,
            };
            old
        });
        Ok(old_handler)
    }
);

define_syscall_handler!(
    user_lib::NR_SGETMASK = 68,
    fn sys_sgetmask(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.signal_info.blocked))
    }
);

define_syscall_handler!(
    user_lib::NR_SSETMASK = 69,
    fn sys_ssetmask(ctx: &SyscallContext) -> Result<u32, u32> {
        let (newmask, _, _) = ctx.args();
        let old = task::current_task().pcb.inner.exclusive(|inner| {
            core::mem::replace(
                &mut inner.signal_info.blocked,
                newmask & !(1u32 << (SIGKILL - 1)),
            )
        });
        Ok(old)
    }
);

define_syscall_handler!(
    user_lib::NR_SIGACTION = 67,
    fn sys_sigaction(ctx: &SyscallContext) -> Result<u32, u32> {
        let (signum, action_ptr, oldaction_ptr) = ctx.args();

        (1..=NSIG as u32)
            .contains(&signum)
            .then_some(signum)
            .filter(|&s| s != SIGKILL)
            .ok_or(EPERM)?;

        let idx = (signum - 1) as usize;

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

        let old_sa = task::current_task().pcb.inner.exclusive(|inner| {
            let old = inner.signal_info.sigaction[idx].clone();
            (action_ptr != 0).then(|| {
                inner.signal_info.sigaction[idx] = read_sigaction_from_user(action_ptr);
            });
            let current = inner.signal_info.sigaction[idx].clone();
            inner.signal_info.sigaction[idx].sa_mask = ((current.sa_flags & SA_NOMASK) == 0)
                .then(|| current.sa_mask | (1u32 << idx))
                .unwrap_or(0);
            old
        });

        (oldaction_ptr != 0).then(|| write_sigaction_to_user(oldaction_ptr, &old_sa));
        Ok(0)
    }
);

// ---------------------------------------------------------------------------
// Process/group/identity syscalls
// ---------------------------------------------------------------------------

define_syscall_handler!(
    user_lib::NR_GETPID = 20,
    fn sys_getpid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task().pcb.pid)
    }
);

define_syscall_handler!(
    user_lib::NR_GETUID = 24,
    fn sys_getuid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.identity.uid as u32))
    }
);

define_syscall_handler!(
    user_lib::NR_SETUID = 23,
    fn sys_setuid(ctx: &SyscallContext) -> Result<u32, u32> {
        let (uid, _, _) = ctx.args();
        sys_setreuid_impl(uid, uid)
    }
);

define_syscall_handler!(
    user_lib::NR_GETGID = 47,
    fn sys_getgid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.identity.gid as u32))
    }
);

define_syscall_handler!(
    user_lib::NR_SETGID = 46,
    fn sys_setgid(ctx: &SyscallContext) -> Result<u32, u32> {
        let (gid, _, _) = ctx.args();
        sys_setregid_impl(gid, gid)
    }
);

define_syscall_handler!(
    user_lib::NR_GETEUID = 49,
    fn sys_geteuid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.identity.euid as u32))
    }
);

define_syscall_handler!(
    user_lib::NR_GETEGID = 50,
    fn sys_getegid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.identity.egid as u32))
    }
);

define_syscall_handler!(
    user_lib::NR_SETPGID = 57,
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
    user_lib::NR_GETPGRP = 65,
    fn sys_getpgrp(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.relation.pgrp))
    }
);

define_syscall_handler!(
    user_lib::NR_SETSID = 66,
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
    user_lib::NR_GETPPID = 64,
    fn sys_getppid(_ctx: &SyscallContext) -> Result<u32, u32> {
        Ok(task::current_task()
            .pcb
            .inner
            .exclusive(|inner| inner.relation.father))
    }
);

define_syscall_handler!(
    user_lib::NR_SETREUID = 70,
    fn sys_setreuid(ctx: &SyscallContext) -> Result<u32, u32> {
        let (ruid, euid, _) = ctx.args();
        sys_setreuid_impl(ruid, euid)
    }
);

define_syscall_handler!(
    user_lib::NR_SETREGID = 71,
    fn sys_setregid(ctx: &SyscallContext) -> Result<u32, u32> {
        let (rgid, egid, _) = ctx.args();
        sys_setregid_impl(rgid, egid)
    }
);

// ---------------------------------------------------------------------------
// Time and system info syscalls
// ---------------------------------------------------------------------------

define_syscall_handler!(
    user_lib::NR_TIME = 13,
    fn sys_time(ctx: &SyscallContext) -> Result<u32, u32> {
        let (tloc, _, _) = ctx.args();
        let t = time::current_time();
        if tloc != 0 {
            mm::ensure_user_area_writable(tloc, 4);
            uaccess::write_u32(t, tloc as *mut u32);
        }
        Ok(t)
    }
);

define_syscall_handler!(
    user_lib::NR_TIMES = 43,
    fn sys_times(ctx: &SyscallContext) -> Result<u32, u32> {
        // struct tms (POSIX <sys/times.h>), 16 bytes total, time_t = long (4 bytes)
        //
        //   offset  size  field       description
        //   ------  ----  ----------  -----------------------------------------
        //   0x00    4     tms_utime   User CPU time (clock ticks)
        //   0x04    4     tms_stime   System CPU time (clock ticks)
        //   0x08    4     tms_cutime  Child user CPU time (waited children)
        //   0x0C    4     tms_cstime  Child system CPU time (waited children)
        let (tbuf, _, _) = ctx.args();
        if tbuf != 0 {
            let (utime, stime, cutime, cstime) =
                task::current_task().pcb.inner.exclusive(|inner| {
                    (
                        inner.acct.utime,
                        inner.acct.stime,
                        inner.acct.cutime,
                        inner.acct.cstime,
                    )
                });
            mm::ensure_user_area_writable(tbuf, 16);
            let base = tbuf as *mut u32;
            unsafe {
                uaccess::write_u32(utime, base);
                uaccess::write_u32(stime, base.add(1));
                uaccess::write_u32(cutime, base.add(2));
                uaccess::write_u32(cstime, base.add(3));
            }
        }
        Ok(task::jiffies())
    }
);

define_syscall_handler!(
    user_lib::NR_UNAME = 59,
    fn sys_uname(ctx: &SyscallContext) -> Result<u32, u32> {
        // struct utsname (POSIX <sys/utsname.h>), 45 bytes total
        //
        //   offset  size  field      description
        //   ------  ----  ---------  -----------------------------------------
        //   0x00    9     sysname    Operating system name (e.g. "linux .0")
        //   0x09    9     nodename   Network node name
        //   0x12    9     release    Kernel release
        //   0x1B    9     version    Kernel version
        //   0x24    9     machine    Hardware identifier
        //
        // Each field is char[9], no null terminator in the struct.
        let (name, _, _) = ctx.args();
        if name == 0 {
            return Err(EINVAL);
        }
        // Match "linux .0", "nodename", "release ", "version ", "machine " (each char[9])
        const UTSNAME: &[u8; 45] = b"linux .0\0nodename\0release \0version \0machine \0";
        mm::ensure_user_area_writable(name, 45);
        let base = name as *mut u8;
        for (i, &b) in UTSNAME.iter().enumerate() {
            uaccess::write_u8(b, unsafe { base.add(i) });
        }
        Ok(0)
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
