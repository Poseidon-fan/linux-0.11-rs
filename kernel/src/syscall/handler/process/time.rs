//! Time and system-info syscall handlers (time, stime, times, uname).

use core::mem;

use user_lib::syscall::{
    Syscall,
    process::{Tms, UtsName},
};

use crate::{
    define_syscall_handler,
    error::{Errno, Result},
    mm,
    segment::uaccess,
    syscall::context::SyscallContext,
    task::{self, HZ, is_superuser},
    time,
};

define_syscall_handler!(
    Syscall::Time = 13,
    fn sys_time(ctx: &mut SyscallContext) -> Result<u32> {
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
    Syscall::Stime = 25,
    fn sys_stime(ctx: &mut SyscallContext) -> Result<u32> {
        if !is_superuser() {
            return Err(Errno::PERM);
        }
        let (tptr, _, _) = ctx.args();
        let new_time = uaccess::read_u32(tptr as *const u32);
        time::set_startup_time(new_time - task::jiffies() / HZ);
        Ok(0)
    }
);

define_syscall_handler!(
    Syscall::Times = 43,
    fn sys_times(ctx: &mut SyscallContext) -> Result<u32> {
        let (tbuf, _, _) = ctx.args();
        if tbuf != 0 {
            let (utime, stime, cutime, cstime) = task::with_current(|inner| {
                (
                    inner.acct.utime,
                    inner.acct.stime,
                    inner.acct.cutime,
                    inner.acct.cstime,
                )
            });
            let tms = Tms {
                user_time: utime,
                system_time: stime,
                child_user_time: cutime,
                child_system_time: cstime,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(&tms as *const Tms as *const u8, mem::size_of::<Tms>())
            };
            mm::ensure_user_area_writable(tbuf, bytes.len());
            uaccess::write_bytes(bytes, tbuf as *mut u8);
        }
        Ok(task::jiffies())
    }
);

define_syscall_handler!(
    Syscall::Uname = 59,
    fn sys_uname(ctx: &mut SyscallContext) -> Result<u32> {
        let (name, _, _) = ctx.args();
        if name == 0 {
            return Err(Errno::INVAL);
        }
        let uts_name = UtsName {
            sysname: *b"linux .0\0",
            nodename: *b"nodename\0",
            release: *b"release \0",
            version: *b"version \0",
            machine: *b"machine \0",
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &uts_name as *const UtsName as *const u8,
                mem::size_of::<UtsName>(),
            )
        };
        mm::ensure_user_area_writable(name, bytes.len());
        uaccess::write_bytes(bytes, name as *mut u8);
        Ok(0)
    }
);
