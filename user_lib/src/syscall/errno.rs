//! POSIX errno error codes.
//!
//! [`Errno`] is a newtype over `u32` representing a POSIX error number.
//! The inner value is always a positive integer; it is transmitted over the
//! `int $0x80` ABI as `-(code as i32)` in EAX.
//!
//! Constants follow the `SCREAMING_SNAKE_CASE` convention and are defined at
//! module level so callers can import them directly:
//!
//! ```rust,ignore
//! use user_lib::syscall::errno::{Errno, EPERM, ENOENT};
//! ```

/// POSIX errno error code, compatible with the `int $0x80` ABI.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Errno(pub u32);

impl Errno {
    /// Returns the raw numeric code (a positive integer).
    #[inline(always)]
    pub const fn code(self) -> u32 {
        self.0
    }
}

impl From<u32> for Errno {
    #[inline(always)]
    fn from(v: u32) -> Self {
        Self(v)
    }
}

pub const EPERM: Errno = Errno(1);
pub const ENOENT: Errno = Errno(2);
pub const ESRCH: Errno = Errno(3);
pub const EINTR: Errno = Errno(4);
pub const EIO: Errno = Errno(5);
pub const ENXIO: Errno = Errno(6);
pub const E2BIG: Errno = Errno(7);
pub const ENOEXEC: Errno = Errno(8);
pub const EBADF: Errno = Errno(9);
pub const ECHILD: Errno = Errno(10);
pub const EAGAIN: Errno = Errno(11);
pub const ENOMEM: Errno = Errno(12);
pub const EACCES: Errno = Errno(13);
pub const EFAULT: Errno = Errno(14);
pub const ENOTBLK: Errno = Errno(15);
pub const EBUSY: Errno = Errno(16);
pub const EEXIST: Errno = Errno(17);
pub const EXDEV: Errno = Errno(18);
pub const ENODEV: Errno = Errno(19);
pub const ENOTDIR: Errno = Errno(20);
pub const EISDIR: Errno = Errno(21);
pub const EINVAL: Errno = Errno(22);
pub const ENFILE: Errno = Errno(23);
pub const EMFILE: Errno = Errno(24);
pub const ENOTTY: Errno = Errno(25);
pub const ETXTBSY: Errno = Errno(26);
pub const EFBIG: Errno = Errno(27);
pub const ENOSPC: Errno = Errno(28);
pub const ESPIPE: Errno = Errno(29);
pub const EROFS: Errno = Errno(30);
pub const EMLINK: Errno = Errno(31);
pub const EPIPE: Errno = Errno(32);
pub const EDOM: Errno = Errno(33);
pub const ERANGE: Errno = Errno(34);
pub const EDEADLK: Errno = Errno(35);
pub const ENAMETOOLONG: Errno = Errno(36);
pub const ENOLCK: Errno = Errno(37);
pub const ENOSYS: Errno = Errno(38);
pub const ENOTEMPTY: Errno = Errno(39);

/// Non-POSIX generic error code used internally by the kernel filesystem layer.
pub const ERROR: Errno = Errno(99);
