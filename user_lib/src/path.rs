//! Cross-platform path manipulation.
//!
//! Counterpart to [`std::path`], adapted for this kernel:
//!
//! - `/` is the only path separator.
//! - Paths are committed to UTF-8: [`Path`] is a transparent wrapper around
//!   [`str`] and [`PathBuf`] around [`String`]. We do not provide
//!   [`std::ffi::OsStr`] / [`OsString`] because there is no second platform
//!   whose paths look different, and [`crate::env`] already commits the
//!   process to UTF-8 for `argv` / `envp`.
//! - Windows-only concepts (drive prefixes, verbatim paths, UNC) are absent.
//!
//! The general shape of the API matches `std::path`: [`Path`] for borrowed
//! paths, [`PathBuf`] for owned paths, [`Components`] / [`Component`] for
//! iteration. Methods follow the same naming where possible; the only
//! systematic deviation is `as_os_str` → [`as_str`](Path::as_str).
//!
//! [`OsString`]: std::ffi::OsString

use alloc::{borrow::ToOwned, string::String};
use core::{
    borrow::Borrow,
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
};

/// The primary separator character on this system.
pub const MAIN_SEPARATOR: char = '/';

/// The primary separator as a string slice.
pub const MAIN_SEPARATOR_STR: &str = "/";

#[inline]
fn is_sep_byte(b: u8) -> bool {
    b == b'/'
}

/// A slice of a path (akin to [`str`]).
///
/// [`Path`] is borrowed; [`PathBuf`] is the owned counterpart. Construct one
/// with [`Path::new`].
#[repr(transparent)]
pub struct Path {
    inner: str,
}

/// An owned, mutable path (akin to [`String`]).
#[derive(Clone, Default)]
pub struct PathBuf {
    inner: String,
}

/// A single component of a path.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Component<'a> {
    /// The root directory component, `/`.
    RootDir,
    /// A reference to the current directory, `.`.
    CurDir,
    /// A reference to the parent directory, `..`.
    ParentDir,
    /// A normal component, e.g. `foo` in `/foo/bar`.
    Normal(&'a str),
}

impl<'a> Component<'a> {
    /// Extracts the underlying `str` slice for this component.
    pub fn as_str(self) -> &'a str {
        match self {
            Component::RootDir => MAIN_SEPARATOR_STR,
            Component::CurDir => ".",
            Component::ParentDir => "..",
            Component::Normal(s) => s,
        }
    }
}

impl AsRef<str> for Component<'_> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<Path> for Component<'_> {
    fn as_ref(&self) -> &Path {
        Path::new(self.as_str())
    }
}

/// Iterator over the [`Component`]s of a [`Path`].
///
/// Forward-only and tolerant of the same canonicalisation rules as
/// `std::path::Components`: leading separators collapse to a single
/// [`Component::RootDir`], repeated internal separators are skipped, `.`
/// components are dropped except as a leading bare-`.` (so `Path::new(".")`
/// yields `[CurDir]`), and a trailing separator is absorbed.
#[derive(Clone)]
pub struct Components<'a> {
    rest: &'a str,
    yielded_root: bool,
    yielded_cur: bool,
    initial: bool,
}

impl<'a> Iterator for Components<'a> {
    type Item = Component<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.initial {
            self.initial = false;
            let bytes = self.rest.as_bytes();
            if !bytes.is_empty() && is_sep_byte(bytes[0]) {
                while !self.rest.is_empty() && is_sep_byte(self.rest.as_bytes()[0]) {
                    self.rest = &self.rest[1..];
                }
                self.yielded_root = true;
                return Some(Component::RootDir);
            }
        }

        loop {
            if self.rest.is_empty() {
                return None;
            }

            let bytes = self.rest.as_bytes();
            let end = bytes
                .iter()
                .position(|b| is_sep_byte(*b))
                .unwrap_or(bytes.len());
            let (head, tail) = self.rest.split_at(end);
            self.rest = tail;
            while !self.rest.is_empty() && is_sep_byte(self.rest.as_bytes()[0]) {
                self.rest = &self.rest[1..];
            }

            match head {
                "" => continue,
                "." => {
                    if !self.yielded_root && !self.yielded_cur {
                        self.yielded_cur = true;
                        return Some(Component::CurDir);
                    }
                    continue;
                }
                ".." => return Some(Component::ParentDir),
                normal => return Some(Component::Normal(normal)),
            }
        }
    }
}

impl core::iter::FusedIterator for Components<'_> {}

impl Path {
    /// Wraps a string slice as a [`Path`] slice.
    ///
    /// This is a constant-time conversion that does not allocate.
    pub fn new<S: AsRef<str> + ?Sized>(s: &S) -> &Path {
        // SAFETY: `Path` is `repr(transparent)` over `str`, so the layouts
        // are identical and the cast is a no-op at runtime.
        unsafe { &*(s.as_ref() as *const str as *const Path) }
    }

    /// Yields the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Returns `true` if the path has no components.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Converts a [`Path`] to an owned [`PathBuf`].
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf {
            inner: self.inner.to_owned(),
        }
    }

    /// Returns `true` if the path begins with the root separator `/`.
    pub fn is_absolute(&self) -> bool {
        self.has_root()
    }

    /// Returns `true` if the path is not absolute.
    pub fn is_relative(&self) -> bool {
        !self.is_absolute()
    }

    /// Returns `true` if the path begins with the root separator `/`.
    pub fn has_root(&self) -> bool {
        self.inner
            .as_bytes()
            .first()
            .copied()
            .map(is_sep_byte)
            .unwrap_or(false)
    }

    /// Produces an iterator over the [`Component`]s of the path.
    pub fn components(&self) -> Components<'_> {
        Components {
            rest: &self.inner,
            yielded_root: false,
            yielded_cur: false,
            initial: true,
        }
    }

    /// Returns the path without its final component, if there is one.
    ///
    /// Returns `None` if the path is the root, empty, or terminates in
    /// `..`.
    pub fn parent(&self) -> Option<&Path> {
        let trimmed = trim_trailing_seps(&self.inner);
        let bytes = trimmed.as_bytes();

        let last_sep = bytes.iter().rposition(|b| is_sep_byte(*b))?;
        let last_component = &trimmed[last_sep + 1..];
        if last_component == ".." {
            return None;
        }

        if last_sep == 0 {
            // path was like "/foo" → parent is "/"
            return Some(Path::new("/"));
        }

        // collapse trailing separators in the parent prefix
        let mut end = last_sep;
        while end > 0 && is_sep_byte(bytes[end - 1]) {
            end -= 1;
        }
        if end == 0 {
            return Some(Path::new("/"));
        }
        Some(Path::new(&trimmed[..end]))
    }

    /// Returns the final normal component of the path, if there is one.
    ///
    /// Returns `None` for the root, empty path, or paths ending in `..`.
    pub fn file_name(&self) -> Option<&str> {
        let trimmed = trim_trailing_seps(&self.inner);
        if trimmed.is_empty() {
            return None;
        }
        let bytes = trimmed.as_bytes();
        let start = bytes
            .iter()
            .rposition(|b| is_sep_byte(*b))
            .map(|i| i + 1)
            .unwrap_or(0);
        let name = &trimmed[start..];
        match name {
            "" | "." | ".." => None,
            other => Some(other),
        }
    }

    /// Extracts the stem (non-extension) portion of [`file_name`](Self::file_name).
    pub fn file_stem(&self) -> Option<&str> {
        let name = self.file_name()?;
        let (stem, _) = rsplit_at_dot(name);
        stem
    }

    /// Extracts the extension portion of [`file_name`](Self::file_name).
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name()?;
        let (_, ext) = rsplit_at_dot(name);
        ext
    }

    /// Creates an owned [`PathBuf`] with `path` adjoined to `self`.
    pub fn join<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        let mut buf = self.to_path_buf();
        buf.push(path);
        buf
    }

    /// Determines whether `base` is a prefix of `self`.
    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        starts_with_impl(self.components(), base.as_ref().components())
    }

    /// Determines whether `child` is a suffix of `self`.
    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        let mut self_iter: alloc::vec::Vec<_> = self.components().collect();
        let child_iter: alloc::vec::Vec<_> = child.as_ref().components().collect();
        if child_iter.len() > self_iter.len() {
            return false;
        }
        let tail = self_iter.split_off(self_iter.len() - child_iter.len());
        tail == child_iter
    }
}

impl PathBuf {
    /// Allocates an empty `PathBuf`.
    pub fn new() -> Self {
        PathBuf {
            inner: String::new(),
        }
    }

    /// Creates a new `PathBuf` with the given string capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        PathBuf {
            inner: String::with_capacity(capacity),
        }
    }

    /// Coerces to a [`Path`] slice.
    pub fn as_path(&self) -> &Path {
        Path::new(&self.inner)
    }

    /// Extends `self` with `path`.
    ///
    /// If `path` is absolute, it replaces `self` entirely; otherwise a
    /// separator is inserted as needed and `path` is appended.
    pub fn push<P: AsRef<Path>>(&mut self, path: P) {
        self._push(path.as_ref());
    }

    fn _push(&mut self, path: &Path) {
        if path.is_absolute() {
            self.inner.clear();
            self.inner.push_str(&path.inner);
            return;
        }

        let need_sep = self
            .inner
            .as_bytes()
            .last()
            .copied()
            .map(|b| !is_sep_byte(b))
            .unwrap_or(false);
        if need_sep {
            self.inner.push(MAIN_SEPARATOR);
        }
        self.inner.push_str(&path.inner);
    }

    /// Truncates `self` to [`Path::parent`].
    ///
    /// Returns `false` and does nothing if [`Path::parent`] is `None`.
    pub fn pop(&mut self) -> bool {
        match self.parent().map(|p| p.inner.len()) {
            Some(len) => {
                self.inner.truncate(len);
                true
            }
            None => false,
        }
    }

    /// Updates [`Path::file_name`] to `file_name`.
    ///
    /// If [`Path::file_name`] was `None`, this is equivalent to pushing
    /// `file_name`.
    pub fn set_file_name<S: AsRef<str>>(&mut self, file_name: S) {
        if self.file_name().is_some() {
            let popped = self.pop();
            debug_assert!(popped);
        }
        self.push(file_name.as_ref());
    }

    /// Updates [`Path::extension`] to `extension`.
    ///
    /// Returns `false` if [`Path::file_name`] is `None`.
    pub fn set_extension<S: AsRef<str>>(&mut self, extension: S) -> bool {
        let extension = extension.as_ref();
        for byte in extension.as_bytes() {
            if is_sep_byte(*byte) {
                panic!("extension cannot contain path separators: {extension:?}");
            }
        }

        let stem = match self.file_stem() {
            None => return false,
            Some(stem) => stem.len(),
        };
        let end_file_stem = self.inner.len() - self.file_name().unwrap().len() + stem;
        self.inner.truncate(end_file_stem);
        if !extension.is_empty() {
            self.inner.push('.');
            self.inner.push_str(extension);
        }
        true
    }

    /// Consumes the `PathBuf`, yielding its internal [`String`] storage.
    pub fn into_string(self) -> String {
        self.inner
    }
}

fn starts_with_impl<'a, I, J>(mut left: I, mut right: J) -> bool
where
    I: Iterator<Item = Component<'a>>,
    J: Iterator<Item = Component<'a>>,
{
    loop {
        match (left.next(), right.next()) {
            (Some(l), Some(r)) if l == r => continue,
            (_, None) => return true,
            _ => return false,
        }
    }
}

fn trim_trailing_seps(s: &str) -> &str {
    let mut bytes = s.as_bytes();
    while bytes.len() > 1 && is_sep_byte(bytes[bytes.len() - 1]) {
        bytes = &bytes[..bytes.len() - 1];
    }
    // SAFETY: trimmed from the byte end on ASCII separator only.
    unsafe { core::str::from_utf8_unchecked(bytes) }
}

fn rsplit_at_dot(name: &str) -> (Option<&str>, Option<&str>) {
    if name == ".." {
        return (Some(name), None);
    }
    let mut iter = name.rsplitn(2, '.');
    let after = iter.next();
    let before = iter.next();
    if before == Some("") {
        (Some(name), None)
    } else {
        (before, after)
    }
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

impl Deref for PathBuf {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.as_path()
    }
}

impl Borrow<Path> for PathBuf {
    fn borrow(&self) -> &Path {
        self.as_path()
    }
}

impl AsRef<Path> for Path {
    fn as_ref(&self) -> &Path {
        self
    }
}

impl AsRef<Path> for PathBuf {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl AsRef<Path> for str {
    fn as_ref(&self) -> &Path {
        Path::new(self)
    }
}

impl AsRef<Path> for String {
    fn as_ref(&self) -> &Path {
        Path::new(self)
    }
}

impl AsRef<str> for Path {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

impl AsRef<str> for PathBuf {
    fn as_ref(&self) -> &str {
        self.inner.as_str()
    }
}

impl From<&str> for PathBuf {
    fn from(s: &str) -> Self {
        PathBuf {
            inner: s.to_owned(),
        }
    }
}

impl From<String> for PathBuf {
    fn from(s: String) -> Self {
        PathBuf { inner: s }
    }
}

impl From<&Path> for PathBuf {
    fn from(p: &Path) -> Self {
        p.to_path_buf()
    }
}

impl ToOwned for Path {
    type Owned = PathBuf;

    fn to_owned(&self) -> PathBuf {
        self.to_path_buf()
    }
}

impl PartialEq for Path {
    fn eq(&self, other: &Path) -> bool {
        self.components().eq(other.components())
    }
}

impl Eq for Path {}

impl PartialOrd for Path {
    fn partial_cmp(&self, other: &Path) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Path {
    fn cmp(&self, other: &Path) -> Ordering {
        self.components().cmp(other.components())
    }
}

impl Hash for Path {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for component in self.components() {
            component.as_str().hash(state);
        }
    }
}

impl PartialEq for PathBuf {
    fn eq(&self, other: &PathBuf) -> bool {
        **self == **other
    }
}

impl Eq for PathBuf {}

impl PartialOrd for PathBuf {
    fn partial_cmp(&self, other: &PathBuf) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PathBuf {
    fn cmp(&self, other: &PathBuf) -> Ordering {
        (**self).cmp(&**other)
    }
}

impl Hash for PathBuf {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl fmt::Debug for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl fmt::Display for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}
