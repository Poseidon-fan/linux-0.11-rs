//! Miscellaneous syscall wrappers for legacy or currently unimplemented entries.

use crate::{syscall::Syscall, use_syscall};

use_syscall!(Syscall::Break => break_() -> u32);
use_syscall!(Syscall::Stty => stty(fd: u32, arg: u32) -> u32);
use_syscall!(Syscall::Gtty => gtty(fd: u32, arg: u32) -> u32);
use_syscall!(Syscall::Ftime => ftime(buf: *mut u8) -> u32);
use_syscall!(Syscall::Prof => prof(buf: *mut u8) -> u32);
use_syscall!(Syscall::Ptrace => ptrace(request: u32, pid: u32, addr: u32) -> u32);
use_syscall!(Syscall::Acct => acct(filename: *const u8) -> u32);
use_syscall!(Syscall::Phys => phys() -> u32);
use_syscall!(Syscall::Lock => lock() -> u32);
use_syscall!(Syscall::Mpx => mpx() -> u32);
use_syscall!(Syscall::Ulimit => ulimit(command: u32, new_limit: u32) -> u32);
use_syscall!(Syscall::Test => test() -> u32);
