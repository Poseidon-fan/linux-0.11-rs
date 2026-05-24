//! Process arguments and environment accessors.
//!
//! This module intentionally mirrors the high-level shape of [`std::env`].
//! Runtime startup records the raw `argc / argv / envp` pointers, while public
//! functions expose owned [`String`] values backed by the user-space allocator.

use alloc::string::{String, ToString};
use core::{
    ffi::{CStr, c_char},
    ptr, str,
};

/// Snapshot of the process argument and environment pointer tables.
#[derive(Clone, Copy)]
struct ProcessEnvironment {
    argc: usize,
    argv: *const *const u8,
    envp: *const *const u8,
}

impl ProcessEnvironment {
    /// Returns an empty environment used before runtime initialization.
    const fn empty() -> Self {
        Self {
            argc: 0,
            argv: ptr::null(),
            envp: ptr::null(),
        }
    }
}

static mut PROCESS_ENVIRONMENT: ProcessEnvironment = ProcessEnvironment::empty();

/// Initializes the process argument and environment views.
///
/// This is called once by the runtime before user `main` runs.
///
/// # Safety
///
/// `argv` must point to an array containing at least `argc` valid
/// NUL-terminated string pointers followed by a NULL terminator. `envp` must
/// point to a NULL-terminated array of valid NUL-terminated string pointers, or
/// be NULL. The pointed-to data must remain valid for the process lifetime.
pub unsafe fn init(argc: usize, argv: *const *const u8, envp: *const *const u8) {
    unsafe {
        PROCESS_ENVIRONMENT = ProcessEnvironment { argc, argv, envp };
    }
}

/// Returns an iterator over the current process arguments.
///
/// This follows `std::env::args`: invalid UTF-8 causes a panic during
/// iteration.
#[inline]
pub fn args() -> Args {
    let environment = snapshot();
    Args {
        argc: environment.argc,
        argv: environment.argv,
        index: 0,
    }
}

/// Returns an iterator over the current process environment.
///
/// This follows `std::env::vars`: invalid UTF-8 causes a panic during
/// iteration, and malformed entries without `=` are skipped.
#[inline]
pub fn vars() -> Vars {
    Vars {
        next: snapshot().envp,
    }
}

/// Looks up an environment variable by UTF-8 name.
pub fn var(name: &str) -> Result<String, VarError> {
    let value = find_var_value(name.as_bytes()).ok_or(VarError::NotPresent)?;
    str::from_utf8(value)
        .map(String::from)
        .map_err(|_| VarError::NotUnicode)
}

/// Copies the global process environment pointers into a local value.
#[inline]
fn snapshot() -> ProcessEnvironment {
    unsafe { PROCESS_ENVIRONMENT }
}

/// Error returned by [`var`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarError {
    /// The requested variable is not present.
    NotPresent,
    /// The variable exists but its value is not valid UTF-8.
    NotUnicode,
}

/// Iterator over owned UTF-8 process arguments.
pub struct Args {
    argc: usize,
    argv: *const *const u8,
    index: usize,
}

impl Iterator for Args {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.argc || self.argv.is_null() {
            return None;
        }

        let ptr = unsafe { *self.argv.add(self.index) };
        self.index += 1;
        let arg = cstr_from_ptr(ptr)?;
        Some(cstr_to_string(arg, "argument is not valid UTF-8"))
    }
}

/// Iterator over owned UTF-8 environment key/value pairs.
pub struct Vars {
    next: *const *const u8,
}

impl Iterator for Vars {
    type Item = (String, String);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = next_env_entry(&mut self.next)?;
            let bytes = entry.to_bytes();
            let Some(eq) = bytes.iter().position(|&byte| byte == b'=') else {
                continue;
            };

            let name = bytes_to_string(&bytes[..eq], "environment name is not valid UTF-8");
            let value = bytes_to_string(&bytes[eq + 1..], "environment value is not valid UTF-8");
            return Some((name, value));
        }
    }
}

/// Finds one environment variable value as raw bytes.
fn find_var_value(name: &[u8]) -> Option<&'static [u8]> {
    if name.contains(&b'=') {
        return None;
    }

    let mut next = snapshot().envp;
    while let Some(entry) = next_env_entry(&mut next) {
        let bytes = entry.to_bytes();
        if bytes.len() > name.len()
            && bytes[..name.len()] == *name
            && bytes.get(name.len()) == Some(&b'=')
        {
            return Some(&bytes[name.len() + 1..]);
        }
    }

    None
}

/// Reads the next raw environment entry and advances `next`.
fn next_env_entry<'a>(next: &mut *const *const u8) -> Option<&'a CStr> {
    let table = *next;
    if table.is_null() {
        return None;
    }

    let ptr = unsafe { *table };
    if ptr.is_null() {
        return None;
    }

    *next = unsafe { table.add(1) };
    cstr_from_ptr(ptr)
}

/// Converts a raw user-stack string pointer into a [`CStr`] reference.
#[inline]
fn cstr_from_ptr<'a>(ptr: *const u8) -> Option<&'a CStr> {
    if ptr.is_null() {
        return None;
    }

    Some(unsafe { CStr::from_ptr(ptr.cast::<c_char>()) })
}

/// Converts one C string into an owned UTF-8 string.
fn cstr_to_string(value: &CStr, panic_message: &str) -> String {
    bytes_to_string(value.to_bytes(), panic_message)
}

/// Converts bytes into an owned UTF-8 string.
fn bytes_to_string(bytes: &[u8], panic_message: &str) -> String {
    str::from_utf8(bytes)
        .unwrap_or_else(|_| panic!("{}", panic_message))
        .to_string()
}
