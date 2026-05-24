//! Process arguments and environment accessors.
//!
//! The runtime initializes this module from the initial user stack before it
//! calls the program's `main` function. Public access mirrors Rust's standard
//! library shape: programs call [`args`], [`vars`], or [`var`] instead of
//! receiving `argv` and `envp` as direct `main` parameters.

use core::{
    ffi::{CStr, c_char},
    marker::PhantomData,
    ptr,
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

/// Returns the current process arguments.
#[inline]
pub fn args() -> Args<'static> {
    let environment = snapshot();
    unsafe { Args::from_raw(environment.argc, environment.argv) }
}

/// Returns the current process environment entries.
#[inline]
pub fn vars() -> Vars<'static> {
    let environment = snapshot();
    unsafe { Vars::from_raw(environment.envp) }
}

/// Looks up an environment variable by byte name.
///
/// The name must not contain `=`. The returned value excludes the `NAME=`
/// prefix and is not NUL-terminated.
pub fn var(name: &[u8]) -> Option<&'static [u8]> {
    vars().get(name)
}

/// Copies the global process environment pointers into a local value.
#[inline]
fn snapshot() -> ProcessEnvironment {
    unsafe { PROCESS_ENVIRONMENT }
}

/// Process argument vector view.
///
/// The pointed-to strings live on the initial user stack and remain valid until
/// the process replaces its image with `execve` or exits.
#[derive(Clone, Copy, Debug)]
pub struct Args<'a> {
    argc: usize,
    argv: *const *const u8,
    lifetime: PhantomData<&'a CStr>,
}

impl<'a> Args<'a> {
    /// Builds an argument view from raw process startup pointers.
    ///
    /// # Safety
    ///
    /// `argv` must point to an array containing at least `argc` valid
    /// NUL-terminated string pointers followed by a NULL terminator.
    pub unsafe fn from_raw(argc: usize, argv: *const *const u8) -> Self {
        Self {
            argc,
            argv,
            lifetime: PhantomData,
        }
    }

    /// Returns the number of arguments.
    #[inline]
    pub const fn len(self) -> usize {
        self.argc
    }

    /// Returns true when no arguments were supplied.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.argc == 0
    }

    /// Returns the argument at `index`, if it exists and is non-NULL.
    #[inline]
    pub fn get(self, index: usize) -> Option<&'a CStr> {
        if index >= self.argc || self.argv.is_null() {
            return None;
        }

        let ptr = unsafe { *self.argv.add(index) };
        cstr_from_ptr(ptr)
    }

    /// Iterates over all argument strings.
    #[inline]
    pub const fn iter(self) -> ArgsIter<'a> {
        ArgsIter {
            args: self,
            index: 0,
        }
    }
}

/// Iterator over process arguments.
pub struct ArgsIter<'a> {
    args: Args<'a>,
    index: usize,
}

impl<'a> Iterator for ArgsIter<'a> {
    type Item = &'a CStr;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.args.get(self.index)?;
        self.index += 1;
        Some(item)
    }
}

/// Process environment vector view.
///
/// Environment entries are stored as `NAME=value` C strings and terminated by a
/// NULL pointer.
#[derive(Clone, Copy, Debug)]
pub struct Vars<'a> {
    envp: *const *const u8,
    lifetime: PhantomData<&'a CStr>,
}

impl<'a> Vars<'a> {
    /// Builds an environment view from raw process startup pointers.
    ///
    /// # Safety
    ///
    /// `envp` must point to a NULL-terminated array of valid NUL-terminated
    /// string pointers, or be NULL.
    pub unsafe fn from_raw(envp: *const *const u8) -> Self {
        Self {
            envp,
            lifetime: PhantomData,
        }
    }

    /// Returns the raw environment pointer table.
    #[inline]
    pub const fn as_ptr(self) -> *const *const u8 {
        self.envp
    }

    /// Iterates over all environment entries.
    #[inline]
    pub const fn iter(self) -> VarsIter<'a> {
        VarsIter {
            next: self.envp,
            lifetime: PhantomData,
        }
    }

    /// Looks up an environment variable by byte name.
    ///
    /// The name must not contain `=`. The returned value excludes the `NAME=`
    /// prefix and is not NUL-terminated.
    pub fn get(self, name: &[u8]) -> Option<&'a [u8]> {
        if name.contains(&b'=') {
            return None;
        }

        for entry in self.iter() {
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
}

/// Iterator over process environment entries.
pub struct VarsIter<'a> {
    next: *const *const u8,
    lifetime: PhantomData<&'a CStr>,
}

impl<'a> Iterator for VarsIter<'a> {
    type Item = &'a CStr;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next.is_null() {
            return None;
        }

        let ptr = unsafe { *self.next };
        if ptr.is_null() {
            return None;
        }

        self.next = unsafe { self.next.add(1) };
        cstr_from_ptr(ptr)
    }
}

/// Converts a raw user-stack string pointer into a [`CStr`] reference.
#[inline]
fn cstr_from_ptr<'a>(ptr: *const u8) -> Option<&'a CStr> {
    if ptr.is_null() {
        return None;
    }

    Some(unsafe { CStr::from_ptr(ptr.cast::<c_char>()) })
}
