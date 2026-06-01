//! Syscall handler definitions and the [`SYSCALL_TABLE`] dispatch slice.
//!
//! Each handler is registered at compile time via the [`define_syscall_handler!`]
//! macro, which places it at a fixed index in the `linkme` distributed slice.
//!
//! - [`process`] — fork, execve, exit, waitpid, kill, signal, identity, etc.
//! - [`fs`] — open, read, write, close, link, mkdir, stat, pipe, mount, etc.
//! - [`nosys`] — `-Errno::NOSYS` stubs for unimplemented syscall numbers.

mod fs;
mod nosys;
mod process;

use crate::{define_syscall_handler, error::Result, syscall::SyscallContext};

/// Dispatch table mapping each syscall number to its handler function.
///
/// Populated at link time: every [`define_syscall_handler!`] invocation inserts
/// its function at a fixed index via the `linkme` distributed slice.
#[::linkme::distributed_slice]
pub static SYSCALL_TABLE: [fn(&mut SyscallContext) -> Result<u32>];

/// Registers a syscall handler at a fixed index in [`SYSCALL_TABLE`].
///
/// `$number` must be written as an integer literal because `linkme` requires
/// one in `distributed_slice(..., N)`. A compile-time assertion verifies the
/// literal matches the value of the corresponding `Syscall` variant.
#[macro_export]
macro_rules! define_syscall_handler {
    (
        $number_path:path = $number:literal,
        fn $fn_name:ident($ctx:ident : &mut SyscallContext) -> $ret:ty $body:block
    ) => {
        const _: () = assert!(
            $number_path as u32 == $number,
            "syscall number mismatch with user_lib"
        );

        #[::linkme::distributed_slice($crate::syscall::SYSCALL_TABLE, $number)]
        fn $fn_name($ctx: &mut SyscallContext) -> $ret $body
    };
}

define_syscall_handler!(
    user_lib::syscall::Syscall::Test = 72,
    fn sys_test(_ctx: &mut SyscallContext) -> Result<u32> {
        crate::println!("hello linux");
        Ok(0)
    }
);
