pub mod fs;
pub mod nosys;
pub mod process;
pub mod todo;

use linkme::distributed_slice;

use crate::{define_syscall_handler, syscall::SyscallContext};

#[distributed_slice]
pub static SYSCALL_TABLE: [fn(&mut SyscallContext) -> Result<u32, u32>];

// linkme requires an integer literal in `distributed_slice(..., N)`, so the
// syscall number must be written as a literal at the call site. A compile-time
// assertion then verifies it matches the corresponding NR_* constant exported
// by user_lib, catching any accidental mismatch.
#[macro_export]
macro_rules! define_syscall_handler {
    (
        $nr_path:path = $nr:literal,
        fn $fn_name:ident($ctx:ident : &mut SyscallContext) -> $ret:ty $body:block
    ) => {
        const _: () = assert!($nr_path == $nr, "syscall number mismatch with user_lib");

        #[distributed_slice(SYSCALL_TABLE, $nr)]
        fn $fn_name($ctx: &mut SyscallContext) -> $ret $body
    };
}

define_syscall_handler!(
    user_lib::NR_TEST = 72,
    fn sys_test(_ctx: &mut SyscallContext) -> Result<u32, u32> {
        crate::println!("hello linux");
        Ok(0)
    }
);
