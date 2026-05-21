//! Time and system-info syscall handlers (time, stime, times, uname).

use user_lib::syscall::Syscall;

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
            let (utime, stime, cutime, cstime) = task::with_current(|inner| {
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
    Syscall::Uname = 59,
    fn sys_uname(ctx: &mut SyscallContext) -> Result<u32> {
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
            return Err(Errno::INVAL);
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
