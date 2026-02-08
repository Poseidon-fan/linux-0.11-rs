use linkme::distributed_slice;

use crate::{println, syscall::context::SyscallContext};

#[distributed_slice]
pub static SYSCALL_TABLE: [fn(&SyscallContext) -> Result<u32, u32>];

// linkme requires an integer literal in `distributed_slice(..., N)`.
// This helper keeps a named syscall number and the required literal in one place.
macro_rules! define_syscall_handler {
    (
        $nr_name:ident = $nr:literal,
        fn $fn_name:ident($ctx:ident : &SyscallContext) -> $ret:ty $body:block
    ) => {
        pub const $nr_name: u32 = $nr;

        #[distributed_slice(SYSCALL_TABLE, $nr)]
        fn $fn_name($ctx: &SyscallContext) -> $ret $body
    };
}

define_syscall_handler!(
    NR_TEST = 0,
    fn sys_test(_ctx: &SyscallContext) -> Result<u32, u32> {
        println!("entry sys_test!");
        Ok(0)
    }
);
