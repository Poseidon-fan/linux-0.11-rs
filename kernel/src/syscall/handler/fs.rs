use linkme::distributed_slice;

use crate::{
    define_syscall_handler,
    driver::blk::hd,
    syscall::{EPERM, SYSCALL_TABLE, context::SyscallContext},
};

define_syscall_handler!(
    user_lib::NR_SETUP = 0,
    fn sys_setup(ctx: &SyscallContext) -> Result<u32, u32> {
        let (drive_info_addr, _, _) = ctx.args();
        hd::setup_from_bios(drive_info_addr as *const u8).map_err(|()| EPERM)?;
        Ok(0)
    }
);
