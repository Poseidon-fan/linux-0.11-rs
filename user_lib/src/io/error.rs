//! Error and result types for I/O operations.
//!
//! Mirrors the design of [`std::io::Error`]: a single [`Error`] type that can
//! represent four kinds of underlying cause:
//!
//! - a raw OS error code,
//! - an [`ErrorKind`] alone,
//! - an [`ErrorKind`] plus a static `&'static str` message, or
//! - an [`ErrorKind`] plus an arbitrary boxed
//!   `core::error::Error + Send + Sync`.
//!
//! The `Send + Sync` bounds on the boxed payload are kept (despite the
//! kernel being single-threaded) for compatibility with ecosystem crates
//! such as `anyhow` that require errors to be `Send + Sync` for storage
//! in their universal error type.
//!
//! Construction goes through [`Error::new`], [`Error::other`],
//! [`Error::from_raw_os_error`], or `Error::from(ErrorKind)`. Inspection goes
//! through [`Error::kind`], [`Error::raw_os_error`], [`Error::get_ref`],
//! [`Error::get_mut`], [`Error::into_inner`], and [`Error::downcast`].
//!
//! The variant set of [`ErrorKind`] is tailored to the categories that can
//! actually arise on this kernel; Linux 0.11 has no networking and no file
//! locking, so the corresponding `std::io::ErrorKind` variants are absent.

use alloc::boxed::Box;
use core::{error, fmt};

use crate::syscall::errno::Errno;

/// A specialized [`Result`] type for I/O operations.
pub type Result<T> = core::result::Result<T, Error>;

/// The error type for I/O operations.
///
/// Errors carry an [`ErrorKind`] tag describing the rough category, plus an
/// optional payload — either a raw OS code, a static message, or a boxed
/// `core::error::Error` — providing the specific cause.
pub struct Error {
    repr: Repr,
}

enum Repr {
    Os(i32),
    Simple(ErrorKind),
    SimpleMessage(ErrorKind, &'static str),
    Custom(Box<Custom>),
}

struct Custom {
    kind: ErrorKind,
    error: Box<dyn error::Error + Send + Sync>,
}

impl Error {
    /// Creates a new I/O error from a known kind and an arbitrary payload.
    ///
    /// Accepts anything convertible into a `Box<dyn core::error::Error +
    /// Send + Sync>`, including `String`, `&'static str`, and any custom
    /// error type implementing [`core::error::Error`] + `Send` + `Sync`.
    /// The auto-trait bounds are kept (despite the kernel being
    /// single-threaded) so that this type composes with ecosystem crates
    /// such as `anyhow` that store errors in `Send + Sync` containers.
    pub fn new<E>(kind: ErrorKind, error: E) -> Self
    where E: Into<Box<dyn error::Error + Send + Sync>> {
        Self::_new(kind, error.into())
    }

    fn _new(kind: ErrorKind, error: Box<dyn error::Error + Send + Sync>) -> Self {
        Self {
            repr: Repr::Custom(Box::new(Custom { kind, error })),
        }
    }

    /// Creates a new I/O error of kind [`ErrorKind::Other`] from an arbitrary
    /// payload.
    pub fn other<E>(error: E) -> Self
    where E: Into<Box<dyn error::Error + Send + Sync>> {
        Self::_new(ErrorKind::Other, error.into())
    }

    /// Creates a new I/O error from a raw OS error code.
    ///
    /// The kind reported by [`Error::kind`] is derived by mapping the
    /// underlying errno to the closest [`ErrorKind`] variant.
    #[inline]
    pub const fn from_raw_os_error(code: i32) -> Self {
        Self {
            repr: Repr::Os(code),
        }
    }

    /// Returns the raw OS error code if this error originated from one.
    #[inline]
    pub fn raw_os_error(&self) -> Option<i32> {
        match self.repr {
            Repr::Os(code) => Some(code),
            _ => None,
        }
    }

    /// Returns the [`ErrorKind`] tag describing this error.
    #[inline]
    pub fn kind(&self) -> ErrorKind {
        match &self.repr {
            Repr::Os(code) => errno_to_kind(*code),
            Repr::Simple(kind) => *kind,
            Repr::SimpleMessage(kind, _) => *kind,
            Repr::Custom(custom) => custom.kind,
        }
    }

    /// Returns a reference to the inner payload, if there is one.
    pub fn get_ref(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.repr {
            Repr::Custom(custom) => Some(&*custom.error),
            _ => None,
        }
    }

    /// Returns a mutable reference to the inner payload, if there is one.
    pub fn get_mut(&mut self) -> Option<&mut (dyn error::Error + 'static)> {
        match &mut self.repr {
            Repr::Custom(custom) => Some(&mut *custom.error),
            _ => None,
        }
    }

    /// Consumes the error and returns the boxed inner payload, if any.
    pub fn into_inner(self) -> Option<Box<dyn error::Error + Send + Sync>> {
        match self.repr {
            Repr::Custom(custom) => Some(custom.error),
            _ => None,
        }
    }

    /// Attempts to downcast the boxed inner payload to a concrete type.
    ///
    /// Returns the original error untouched if the payload is missing or
    /// does not have the requested type.
    pub fn downcast<E>(self) -> core::result::Result<Box<E>, Self>
    where E: error::Error + 'static {
        match self.repr {
            Repr::Custom(custom) => match custom.error.downcast::<E>() {
                Ok(downcast) => Ok(downcast),
                Err(error) => Err(Self {
                    repr: Repr::Custom(Box::new(Custom {
                        kind: custom.kind,
                        error,
                    })),
                }),
            },
            repr => Err(Self { repr }),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            Repr::Os(code) => f
                .debug_struct("Os")
                .field("code", code)
                .field("kind", &errno_to_kind(*code))
                .field("message", &errno_message(*code))
                .finish(),
            Repr::Simple(kind) => f.debug_tuple("Kind").field(kind).finish(),
            Repr::SimpleMessage(kind, message) => f
                .debug_struct("Error")
                .field("kind", kind)
                .field("message", message)
                .finish(),
            Repr::Custom(custom) => fmt::Debug::fmt(&custom.error, f),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            Repr::Os(code) => write!(f, "{} (os error {})", errno_message(*code), code),
            Repr::Simple(kind) => fmt::Display::fmt(kind, f),
            Repr::SimpleMessage(_, message) => f.write_str(message),
            Repr::Custom(custom) => fmt::Display::fmt(&custom.error, f),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.repr {
            Repr::Custom(custom) => custom.error.source(),
            _ => None,
        }
    }
}

impl From<ErrorKind> for Error {
    /// Builds an error carrying just an [`ErrorKind`] with no extra payload.
    #[inline]
    fn from(kind: ErrorKind) -> Self {
        Self {
            repr: Repr::Simple(kind),
        }
    }
}

impl From<Errno> for Error {
    #[inline]
    fn from(errno: Errno) -> Self {
        Self::from_raw_os_error(errno.code() as i32)
    }
}

/// Internal constructor for static-message errors used by trait default
/// methods (e.g. `Read::read_exact` produces `UnexpectedEof`).
#[inline]
pub(crate) const fn const_io_error(kind: ErrorKind, message: &'static str) -> Error {
    Error {
        repr: Repr::SimpleMessage(kind, message),
    }
}

/// A list specifying general categories of I/O error.
///
/// The set is tailored to the conditions that can arise on this kernel: file
/// system access, pipes/TTYs, the kernel heap, and process/exec failures.
/// Marked `#[non_exhaustive]` so additional variants can be introduced
/// without breaking exhaustive matches.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// An entity was not found, often a file.
    NotFound,
    /// The operation lacked the necessary privileges to complete.
    PermissionDenied,
    /// A pipe was closed before all of its bytes were read.
    BrokenPipe,
    /// An entity already exists, often a file.
    AlreadyExists,
    /// The operation would block but was requested non-blocking.
    WouldBlock,
    /// A filesystem object expected to be a directory was not.
    NotADirectory,
    /// The filesystem object is a directory; the operation requires otherwise.
    IsADirectory,
    /// A non-empty directory was given to an operation that requires empty.
    DirectoryNotEmpty,
    /// The filesystem or storage medium is read-only.
    ReadOnlyFilesystem,
    /// A parameter was incorrect.
    InvalidInput,
    /// Data not valid for the operation were encountered.
    InvalidData,
    /// `write` returned `Ok(0)` to indicate the underlying object refused.
    WriteZero,
    /// The underlying storage is full.
    StorageFull,
    /// Seek on an unseekable object.
    NotSeekable,
    /// Filesystem-level limit on file size was exceeded.
    FileTooLarge,
    /// The resource is busy.
    ResourceBusy,
    /// Executable file is busy.
    ExecutableFileBusy,
    /// Cross-device link or rename.
    CrossesDevices,
    /// Too many hard links to a single filesystem object.
    TooManyLinks,
    /// The argument list passed to `execve(2)` was too large.
    ArgumentListTooLong,
    /// The operation was interrupted; it can typically be retried.
    Interrupted,
    /// The operation is not supported.
    Unsupported,
    /// An end-of-file was reached unexpectedly.
    UnexpectedEof,
    /// An operation could not be completed because it failed to allocate
    /// enough memory.
    OutOfMemory,
    /// A custom error that does not fall under any other category.
    Other,
}

impl ErrorKind {
    /// Returns a short string describing this error kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::NotFound => "entity not found",
            ErrorKind::PermissionDenied => "permission denied",
            ErrorKind::BrokenPipe => "broken pipe",
            ErrorKind::AlreadyExists => "entity already exists",
            ErrorKind::WouldBlock => "operation would block",
            ErrorKind::NotADirectory => "not a directory",
            ErrorKind::IsADirectory => "is a directory",
            ErrorKind::DirectoryNotEmpty => "directory not empty",
            ErrorKind::ReadOnlyFilesystem => "read-only filesystem or storage medium",
            ErrorKind::InvalidInput => "invalid input parameter",
            ErrorKind::InvalidData => "invalid data",
            ErrorKind::WriteZero => "write zero",
            ErrorKind::StorageFull => "no storage space",
            ErrorKind::NotSeekable => "seek on unseekable file",
            ErrorKind::FileTooLarge => "file too large",
            ErrorKind::ResourceBusy => "resource busy",
            ErrorKind::ExecutableFileBusy => "executable file busy",
            ErrorKind::CrossesDevices => "cross-device link or rename",
            ErrorKind::TooManyLinks => "too many links",
            ErrorKind::ArgumentListTooLong => "argument list too long",
            ErrorKind::Interrupted => "operation interrupted",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::UnexpectedEof => "unexpected end of file",
            ErrorKind::OutOfMemory => "out of memory",
            ErrorKind::Other => "other error",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Maps a raw OS errno value to the closest [`ErrorKind`] variant.
fn errno_to_kind(code: i32) -> ErrorKind {
    let raw = if code < 0 {
        (-code) as u32
    } else {
        code as u32
    };
    match raw {
        x if x == Errno::PERM.code() || x == Errno::ACCESS.code() => ErrorKind::PermissionDenied,
        x if x == Errno::NOENT.code()
            || x == Errno::SRCH.code()
            || x == Errno::NXIO.code()
            || x == Errno::NODEV.code() =>
        {
            ErrorKind::NotFound
        }
        x if x == Errno::INTR.code() => ErrorKind::Interrupted,
        x if x == Errno::AGAIN.code() => ErrorKind::WouldBlock,
        x if x == Errno::NOMEM.code() => ErrorKind::OutOfMemory,
        x if x == Errno::EXIST.code() => ErrorKind::AlreadyExists,
        x if x == Errno::XDEV.code() => ErrorKind::CrossesDevices,
        x if x == Errno::NOTDIR.code() => ErrorKind::NotADirectory,
        x if x == Errno::ISDIR.code() => ErrorKind::IsADirectory,
        x if x == Errno::INVAL.code()
            || x == Errno::FAULT.code()
            || x == Errno::DOM.code()
            || x == Errno::RANGE.code()
            || x == Errno::NAMETOOLONG.code() =>
        {
            ErrorKind::InvalidInput
        }
        x if x == Errno::BUSY.code() => ErrorKind::ResourceBusy,
        x if x == Errno::TXTBSY.code() => ErrorKind::ExecutableFileBusy,
        x if x == Errno::FBIG.code() => ErrorKind::FileTooLarge,
        x if x == Errno::NOSPC.code() => ErrorKind::StorageFull,
        x if x == Errno::SPIPE.code() => ErrorKind::NotSeekable,
        x if x == Errno::ROFS.code() => ErrorKind::ReadOnlyFilesystem,
        x if x == Errno::MLINK.code() => ErrorKind::TooManyLinks,
        x if x == Errno::PIPE.code() => ErrorKind::BrokenPipe,
        x if x == Errno::NOSYS.code() => ErrorKind::Unsupported,
        x if x == Errno::NOTEMPTY.code() => ErrorKind::DirectoryNotEmpty,
        x if x == Errno::TOOBIG.code() => ErrorKind::ArgumentListTooLong,
        _ => ErrorKind::Other,
    }
}

/// Returns the canned message for a raw OS errno value.
fn errno_message(code: i32) -> &'static str {
    errno_to_kind(code).as_str()
}
