//! Process arguments and environment accessors.
//!
//! This module intentionally mirrors the high-level shape of [`std::env`].
//! Runtime startup records the raw `argc / argv / envp` pointers, while public
//! functions expose owned [`String`] values backed by the user-space allocator.

use alloc::{string::String, vec::Vec};
use core::{ffi::CStr, ptr, str};

use crate::{
    fs,
    path::{Path, PathBuf},
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

/// Returns the full filesystem path of the current working directory.
///
/// Mirrors [`std::env::current_dir`]. This kernel has no `getcwd` syscall,
/// so the path is reconstructed by walking parent directories and matching
/// directory entries by `(dev, ino)`.
pub fn current_dir() -> crate::io::Result<PathBuf> {
    let original = fs::metadata(".")?;
    let mut names = Vec::new();

    loop {
        let current = fs::metadata(".")?;
        let parent = fs::metadata("..")?;
        if same_file(&current, &parent) {
            break;
        }

        fs::set_current_dir("..")?;
        names.push(find_child_name(&current)?);
    }

    let cwd = build_absolute_path(&names);
    fs::set_current_dir(cwd.as_path())?;

    let restored = fs::metadata(".")?;
    if same_file(&original, &restored) {
        Ok(cwd)
    } else {
        Err(crate::io::Error::new(
            crate::io::ErrorKind::NotFound,
            "current directory changed while resolving path",
        ))
    }
}

/// Changes the current working directory to the specified path.
///
/// Mirrors [`std::env::set_current_dir`].
pub fn set_current_dir<P: AsRef<Path>>(path: P) -> crate::io::Result<()> {
    fs::set_current_dir(path)
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
        if ptr.is_null() {
            return None;
        }

        let arg = unsafe { CStr::from_ptr(ptr.cast()) };
        Some(arg.to_str().expect("argument is not valid UTF-8").into())
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

            let name = str::from_utf8(&bytes[..eq])
                .expect("environment name is not valid UTF-8")
                .into();
            let value = str::from_utf8(&bytes[eq + 1..])
                .expect("environment value is not valid UTF-8")
                .into();
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
    Some(unsafe { CStr::from_ptr(ptr.cast()) })
}

fn find_child_name(target: &fs::Metadata) -> crate::io::Result<String> {
    for entry in fs::read_dir(".")? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if same_file(&metadata, target) {
            return Ok(entry.file_name());
        }
    }

    Err(crate::io::Error::new(
        crate::io::ErrorKind::NotFound,
        "current directory entry not found in parent",
    ))
}

fn build_absolute_path(reversed_names: &[String]) -> PathBuf {
    let mut path = PathBuf::new();
    path.push("/");
    for name in reversed_names.iter().rev() {
        path.push(name.as_str());
    }
    path
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}
