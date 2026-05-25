//! Utilities related to FFI bindings.
//!
//! Counterpart to [`std::ffi`]. This module is a thin re-export of the
//! `CStr`/`CString` types from [`core::ffi`] and [`alloc::ffi`], plus the
//! `c_*` primitive type aliases. They are the lingua franca at the boundary
//! to system calls, where pointers to NUL-terminated C strings are required.
//!
//! `OsStr`/`OsString` are intentionally absent: this kernel runs a single
//! platform whose paths are treated as UTF-8, so [`crate::path::Path`] wraps
//! [`str`] directly. See the [`path`](crate::path) module for the rationale.

pub use alloc::ffi::{CString, FromVecWithNulError, IntoStringError, NulError};
pub use core::ffi::{
    CStr, FromBytesUntilNulError, FromBytesWithNulError, c_char, c_double, c_float, c_int, c_long,
    c_longlong, c_schar, c_short, c_uchar, c_uint, c_ulong, c_ulonglong, c_ushort, c_void,
};
