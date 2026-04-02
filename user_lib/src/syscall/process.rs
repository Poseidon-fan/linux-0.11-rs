/// Number of signals supported (signals 1–32).
pub const NSIG: usize = 32;

// Signal numbers (POSIX subset).
pub const SIGHUP: u32 = 1;
pub const SIGKILL: u32 = 9;
pub const SIGSEGV: u32 = 11;
pub const SIGALRM: u32 = 14;
pub const SIGCHLD: u32 = 17;
pub const SIGSTOP: u32 = 19;

// Signal handler sentinels.
pub const SIG_DFL: u32 = 0;
pub const SIG_IGN: u32 = 1;

// Sigaction flags.
pub const SA_NOMASK: u32 = 0x4000_0000;
pub const SA_ONESHOT: u32 = 0x8000_0000;
