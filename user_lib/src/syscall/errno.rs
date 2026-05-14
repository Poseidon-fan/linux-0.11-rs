//! POSIX errno error codes.
//!
//! [`Errno`] is a newtype over `u32` representing a POSIX error number.
//! The inner value is always a positive integer; it is transmitted over the
//! `int $0x80` ABI as `-(code as i32)` in EAX.
//!
//! Error codes are associated constants on the type:
//!
//! ```rust,ignore
//! use user_lib::syscall::errno::Errno;
//!
//! let e = Errno::PERM;
//! ```

/// POSIX errno error code, compatible with the `int $0x80` ABI.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Errno(pub u32);

impl Errno {
    /// Returns the raw numeric code (a positive integer).
    #[inline]
    pub const fn code(self) -> u32 {
        self.0
    }

    pub const PERM: Self = Self(1);
    pub const NOENT: Self = Self(2);
    pub const SRCH: Self = Self(3);
    pub const INTR: Self = Self(4);
    pub const IO: Self = Self(5);
    pub const NXIO: Self = Self(6);
    pub const TOOBIG: Self = Self(7);
    pub const NOEXEC: Self = Self(8);
    pub const BADF: Self = Self(9);
    pub const CHILD: Self = Self(10);
    pub const AGAIN: Self = Self(11);
    pub const NOMEM: Self = Self(12);
    pub const ACCESS: Self = Self(13);
    pub const FAULT: Self = Self(14);
    pub const NOTBLK: Self = Self(15);
    pub const BUSY: Self = Self(16);
    pub const EXIST: Self = Self(17);
    pub const XDEV: Self = Self(18);
    pub const NODEV: Self = Self(19);
    pub const NOTDIR: Self = Self(20);
    pub const ISDIR: Self = Self(21);
    pub const INVAL: Self = Self(22);
    pub const NFILE: Self = Self(23);
    pub const MFILE: Self = Self(24);
    pub const NOTTY: Self = Self(25);
    pub const TXTBSY: Self = Self(26);
    pub const FBIG: Self = Self(27);
    pub const NOSPC: Self = Self(28);
    pub const SPIPE: Self = Self(29);
    pub const ROFS: Self = Self(30);
    pub const MLINK: Self = Self(31);
    pub const PIPE: Self = Self(32);
    pub const DOM: Self = Self(33);
    pub const RANGE: Self = Self(34);
    pub const DEADLK: Self = Self(35);
    pub const NAMETOOLONG: Self = Self(36);
    pub const NOLCK: Self = Self(37);
    pub const NOSYS: Self = Self(38);
    pub const NOTEMPTY: Self = Self(39);

    /// Non-POSIX generic error code used internally by the kernel filesystem layer.
    pub const ERROR: Self = Self(99);
}

impl From<u32> for Errno {
    #[inline]
    fn from(v: u32) -> Self {
        Self(v)
    }
}
