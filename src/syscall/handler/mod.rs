pub mod process;
pub mod todo;

use linkme::distributed_slice;

use crate::syscall::context::SyscallContext;

#[distributed_slice]
pub static SYSCALL_TABLE: [fn(&SyscallContext) -> Result<u32, u32>];

// linkme requires an integer literal in `distributed_slice(..., N)`.
// This helper keeps a named syscall number and the required literal in one place.
#[macro_export]
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
    NR_TEST = 74,
    fn sys_test(ctx: &SyscallContext) -> Result<u32, u32> {
        let (value, _, _) = ctx.args();
        crate::println!("test value: {}", value as i32);
        Ok(0)
    }
);
