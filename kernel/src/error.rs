//! Kernel-wide error types.
//!
//! Re-exports [`Errno`] from `user_lib`, defines kernel-local errno constants,
//! and defines the [`Result`] alias used throughout the kernel.

pub use user_lib::syscall::errno::Errno;

pub const EPERM: Errno = Errno::PERM;
pub const ENOENT: Errno = Errno::NOENT;
pub const ESRCH: Errno = Errno::SRCH;
pub const EINTR: Errno = Errno::INTR;
pub const EIO: Errno = Errno::IO;
pub const ENXIO: Errno = Errno::NXIO;
pub const E2BIG: Errno = Errno::TOOBIG;
pub const ENOEXEC: Errno = Errno::NOEXEC;
pub const EBADF: Errno = Errno::BADF;
pub const ECHILD: Errno = Errno::CHILD;
pub const EAGAIN: Errno = Errno::AGAIN;
pub const ENOMEM: Errno = Errno::NOMEM;
pub const EACCES: Errno = Errno::ACCESS;
pub const EFAULT: Errno = Errno::FAULT;
pub const ENOTBLK: Errno = Errno::NOTBLK;
pub const EBUSY: Errno = Errno::BUSY;
pub const EEXIST: Errno = Errno::EXIST;
pub const EXDEV: Errno = Errno::XDEV;
pub const ENODEV: Errno = Errno::NODEV;
pub const ENOTDIR: Errno = Errno::NOTDIR;
pub const EISDIR: Errno = Errno::ISDIR;
pub const EINVAL: Errno = Errno::INVAL;
pub const ENFILE: Errno = Errno::NFILE;
pub const EMFILE: Errno = Errno::MFILE;
pub const ENOTTY: Errno = Errno::NOTTY;
pub const ETXTBSY: Errno = Errno::TXTBSY;
pub const EFBIG: Errno = Errno::FBIG;
pub const ENOSPC: Errno = Errno::NOSPC;
pub const ESPIPE: Errno = Errno::SPIPE;
pub const EROFS: Errno = Errno::ROFS;
pub const EMLINK: Errno = Errno::MLINK;
pub const EPIPE: Errno = Errno::PIPE;
pub const EDOM: Errno = Errno::DOM;
pub const ERANGE: Errno = Errno::RANGE;
pub const EDEADLK: Errno = Errno::DEADLK;
pub const ENAMETOOLONG: Errno = Errno::NAMETOOLONG;
pub const ENOLCK: Errno = Errno::NOLCK;
pub const ENOSYS: Errno = Errno::NOSYS;
pub const ENOTEMPTY: Errno = Errno::NOTEMPTY;
pub const ERROR: Errno = Errno::ERROR;

/// Standard kernel [`Result`] with [`Errno`] as the error type.
pub type Result<T> = core::result::Result<T, Errno>;
