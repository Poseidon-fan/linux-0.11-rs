//! Process identity syscall handlers (pid, uid, gid, session, group).

use crate::{
    define_syscall_handler,
    syscall::{EPERM, ESRCH, context::SyscallContext},
    task::{self, TASK_MANAGER, is_superuser},
};

define_syscall_handler!(
    user_lib::syscall::NR_GETPID = 20,
    fn sys_getpid(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Ok(task::current_pid())
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_GETPPID = 64,
    fn sys_getppid(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Ok(task::with_current(|inner| inner.relation.father))
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_GETUID = 24,
    fn sys_getuid(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Ok(task::with_current(|inner| inner.identity.uid as u32))
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_SETUID = 23,
    fn sys_setuid(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (uid, _, _) = ctx.args();
        sys_setreuid_impl(uid, uid)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_GETEUID = 49,
    fn sys_geteuid(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Ok(task::with_current(|inner| inner.identity.euid as u32))
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_GETGID = 47,
    fn sys_getgid(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Ok(task::with_current(|inner| inner.identity.gid as u32))
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_SETGID = 46,
    fn sys_setgid(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (gid, _, _) = ctx.args();
        sys_setregid_impl(gid, gid)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_GETEGID = 50,
    fn sys_getegid(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Ok(task::with_current(|inner| inner.identity.egid as u32))
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_SETREUID = 70,
    fn sys_setreuid(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (ruid, euid, _) = ctx.args();
        sys_setreuid_impl(ruid, euid)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_SETREGID = 71,
    fn sys_setregid(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (rgid, egid, _) = ctx.args();
        sys_setregid_impl(rgid, egid)
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_SETPGID = 57,
    fn sys_setpgid(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (pid_arg, pgid_arg, _) = ctx.args();
        let pid = task::current_pid();
        let target_pid = if pid_arg == 0 { pid } else { pid_arg };
        let target_pgid = if pgid_arg == 0 { pid } else { pgid_arg };
        let current_session = task::with_current(|inner| inner.relation.session);

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
                        .exclusive(|inner| (inner.relation.leader, inner.relation.session));
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
    user_lib::syscall::NR_GETPGRP = 65,
    fn sys_getpgrp(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        Ok(task::with_current(|inner| inner.relation.pgrp))
    }
);

define_syscall_handler!(
    user_lib::syscall::NR_SETSID = 66,
    fn sys_setsid(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        let pid = task::current_pid();
        let is_leader = task::with_current(|inner| inner.relation.leader);
        if is_leader && !is_superuser() {
            return Err(EPERM);
        }
        task::with_current(|inner| {
            inner.relation.leader = true;
            inner.relation.session = pid;
            inner.relation.pgrp = pid;
            inner.tty = -1;
        });
        Ok(pid)
    }
);

fn sys_setreuid_impl(ruid: u32, euid: u32) -> Result<u32, u32> {
    let superuser = is_superuser();
    task::with_current(|inner| {
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
    let superuser = is_superuser();
    task::with_current(|inner| {
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
