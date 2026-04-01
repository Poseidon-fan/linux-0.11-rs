pub mod fs;
pub mod process;
pub mod todo;

use linkme::distributed_slice;

use crate::{define_syscall_handler, syscall::SyscallContext};

#[distributed_slice]
pub static SYSCALL_TABLE: [fn(&SyscallContext) -> Result<u32, u32>];

// linkme requires an integer literal in `distributed_slice(..., N)`, so the
// syscall number must be written as a literal at the call site. A compile-time
// assertion then verifies it matches the corresponding NR_* constant exported
// by user_lib, catching any accidental mismatch.
#[macro_export]
macro_rules! define_syscall_handler {
    (
        $nr_path:path = $nr:literal,
        fn $fn_name:ident($ctx:ident : &SyscallContext) -> $ret:ty $body:block
    ) => {
        const _: () = assert!($nr_path == $nr, "syscall number mismatch with user_lib");

        #[distributed_slice(SYSCALL_TABLE, $nr)]
        fn $fn_name($ctx: &SyscallContext) -> $ret $body
    };
}

define_syscall_handler!(
    user_lib::NR_TEST = 74,
    fn sys_test(ctx: &SyscallContext) -> Result<u32, u32> {
        let (path_ptr, _, _) = ctx.args();
        let pathname = crate::segment::get_fs_string(path_ptr as *const u8, 256);

        let flags = user_lib::fs::OpenFlags::new(
            user_lib::fs::AccessMode::ReadOnly,
            user_lib::fs::OpenOptions::empty(),
        );
        let inode = crate::fs::path::open_path(&pathname, flags, 0)?;

        let mut buf = [0u8; 4096];
        let n = inode.read_at(0, &mut buf)?;
        let content = core::str::from_utf8(&buf[..n]).unwrap_or("<invalid utf8>");
        crate::println!("{}", content);

        Ok(n as u32)
    }
);
