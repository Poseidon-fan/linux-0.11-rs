//! Process-management syscall wrappers.

use crate::{syscall::Syscall, use_syscall};

use_syscall!(Syscall::Setup => setup(drive_info_addr: *const u8) -> u32);
use_syscall!(Syscall::Exit => exit(status: u32) -> u32);
use_syscall!(Syscall::Fork  => fork() -> u32);
use_syscall!(Syscall::Waitpid => waitpid(
    pid: i32,
    stat_addr: *mut u32,
    options: u32
) -> u32);

use_syscall!(Syscall::Execve => execve(
    filename: *const u8,
    argv: *const *const u8,
    envp: *const *const u8
) -> u32);

use_syscall!(Syscall::Time => time(tloc: *mut u32) -> u32);
use_syscall!(Syscall::Stime => stime(tptr: *const u32) -> u32);
use_syscall!(Syscall::Alarm => alarm(seconds: u32) -> u32);
use_syscall!(Syscall::Pause => pause() -> u32);
use_syscall!(Syscall::Times => times(tbuf: *mut u8) -> u32);
use_syscall!(Syscall::Brk => brk(end_data_segment: u32) -> u32);
use_syscall!(Syscall::Getpid => getpid() -> u32);
use_syscall!(Syscall::Setuid => setuid(uid: u32) -> u32);
use_syscall!(Syscall::Getuid => getuid() -> u32);
use_syscall!(Syscall::Nice => nice(increment: u32) -> u32);
use_syscall!(Syscall::Setgid => setgid(gid: u32) -> u32);
use_syscall!(Syscall::Getgid => getgid() -> u32);
use_syscall!(Syscall::Geteuid => geteuid() -> u32);
use_syscall!(Syscall::Getegid => getegid() -> u32);
use_syscall!(Syscall::Setpgid => setpgid(pid: u32, pgid: u32) -> u32);
use_syscall!(Syscall::Uname => uname(buf: *mut u8) -> u32);
use_syscall!(Syscall::Setreuid => setreuid(ruid: u32, euid: u32) -> u32);
use_syscall!(Syscall::Setregid => setregid(rgid: u32, egid: u32) -> u32);
use_syscall!(Syscall::Getppid => getppid() -> u32);
use_syscall!(Syscall::Getpgrp => getpgrp() -> u32);
use_syscall!(Syscall::Setsid => setsid() -> u32);
