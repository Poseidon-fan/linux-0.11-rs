//! Filesystem manipulation operations.
//!
//! Counterpart to [`std::fs`], adapted for this kernel:
//!
//! - Kernel file offsets and sizes are 32-bit, but std-compatible public
//!   methods expose Rust's normal `u64` sizes and seek positions. Values that
//!   do not fit the kernel ABI are rejected at the syscall boundary.
//! - Directory iteration is implemented in user space by reading the raw
//!   Minix v1 directory bytes exposed by this kernel and decoding their fixed
//!   16-byte directory entries.
//! - There is no `set_len`, `sync_all`, or `sync_data`: there is no
//!   `ftruncate` and no per-inode `fsync`.
//! - There are no symbolic links, so [`FileType::is_symlink`] always
//!   returns `false` and `symlink` / `symlink_metadata` / `read_link` are
//!   absent.
//! - [`File`] and the path-based free functions accept anything implementing
//!   [`AsRef<Path>`](crate::path::Path), where `Path` wraps [`str`] —
//!   string literals work directly, no `OsStr` indirection.

use alloc::{borrow::ToOwned, string::String, vec::Vec};
use core::fmt;

use crate::{
    ffi::CString,
    io::{Error, ErrorKind, Read, Result, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    syscall::{
        self,
        fs::{AccessMode, OpenFlags, Stat, TimeUpdate, Whence},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const S_IFMT: u16 = 0o170_000;
const S_IFREG: u16 = 0o100_000;
const S_IFDIR: u16 = 0o040_000;
const S_IFCHR: u16 = 0o020_000;
const S_IFBLK: u16 = 0o060_000;
const S_IFIFO: u16 = 0o010_000;

const PERM_MASK: u16 = 0o7_777;
const ALL_WRITE_BITS: u16 = 0o0_222;

const MINIX_DIRECTORY_ENTRY_SIZE: usize = 16;
const MINIX_DIRECTORY_NAME_LENGTH: usize = 14;
const MINIX_BLOCK_SIZE: u64 = 1024;
const POSIX_BLOCK_SIZE: u64 = 512;
const MAX_RW_COUNT: usize = i32::MAX as usize;

/// Default mode for newly created files: `rw-rw-rw-` (the kernel will mask
/// this with the process `umask`).
const DEFAULT_CREATE_MODE: u32 = 0o666;

/// Default mode for newly created directories: `rwxrwxrwx` (masked by
/// `umask`).
const DEFAULT_DIR_MODE: u32 = 0o777;

// ---------------------------------------------------------------------------
// File
// ---------------------------------------------------------------------------

/// An object providing access to an open file on the filesystem.
///
/// Files are automatically closed when they go out of scope.
pub struct File {
    fd: u32,
    path: Option<PathBuf>,
}

impl File {
    /// Attempts to open a file in read-only mode.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<File> {
        OpenOptions::new().read(true).open(path)
    }

    /// Opens a file in write-only mode, creating or truncating it.
    pub fn create<P: AsRef<Path>>(path: P) -> Result<File> {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
    }

    /// Opens a file in write-only mode, failing if it already exists.
    pub fn create_new<P: AsRef<Path>>(path: P) -> Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
    }

    /// Returns a new blank [`OpenOptions`] for ergonomic configuration.
    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }

    /// Queries metadata about the underlying file.
    pub fn metadata(&self) -> Result<Metadata> {
        let mut stat = empty_stat();
        syscall::fs::fstat(self.fd, &mut stat).map_err(Error::from)?;
        Ok(Metadata { stat })
    }

    /// Returns the raw underlying file descriptor.
    #[inline]
    pub(crate) fn raw_fd(&self) -> u32 {
        self.fd
    }

    /// Wraps a raw file descriptor as an owned [`File`].
    ///
    /// # Safety
    ///
    /// `fd` must be a valid open file descriptor in the current process.
    /// The returned [`File`] takes ownership and will close `fd` when
    /// dropped.
    #[inline]
    pub(crate) unsafe fn from_raw_fd(fd: u32) -> Self {
        File { fd, path: None }
    }

    /// Changes one or more timestamps of the underlying file.
    ///
    /// Mirrors [`std::fs::File::set_times`]. This kernel only exposes
    /// path-based `utime(2)`, so the operation is supported for files opened
    /// by path through this module.
    pub fn set_times(&self, times: FileTimes) -> Result<()> {
        let Some(path) = self.path.as_ref() else {
            return Err(Error::from(ErrorKind::Unsupported));
        };
        set_path_times(path.as_path(), times)
    }

    /// Changes the modification time of the underlying file.
    ///
    /// This is an alias for `set_times(FileTimes::new().set_modified(time))`.
    pub fn set_modified(&self, time: SystemTime) -> Result<()> {
        self.set_times(FileTimes::new().set_modified(time))
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let count = core::cmp::min(buf.len(), MAX_RW_COUNT) as i32;
        match syscall::fs::read(self.fd, buf.as_mut_ptr(), count) {
            Ok(count) => Ok(count as usize),
            Err(errno) => Err(Error::from(errno)),
        }
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let count = core::cmp::min(buf.len(), MAX_RW_COUNT) as i32;
        match syscall::fs::write(self.fd, buf.as_ptr(), count) {
            Ok(count) => Ok(count as usize),
            Err(errno) => Err(Error::from(errno)),
        }
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let (offset, whence) = seek_parts(pos)?;
        match syscall::fs::lseek(self.fd, offset, whence) {
            Ok(n) => Ok(u64::from(n)),
            Err(errno) => Err(Error::from(errno)),
        }
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = syscall::fs::close(self.fd);
    }
}

impl fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("File")
            .field("fd", &self.fd)
            .field("path", &self.path)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// ReadDir and DirEntry
// ---------------------------------------------------------------------------

/// Iterator over the entries in a directory.
///
/// Returned by [`read_dir`]. Each yielded item is a [`Result<DirEntry>`] to
/// match [`std::fs::ReadDir`]'s shape. This implementation reads and decodes
/// the directory contents up front because this kernel exposes directory data
/// as ordinary read-only inode bytes instead of a `getdents`-style cursor.
pub struct ReadDir {
    root: crate::path::PathBuf,
    entries: alloc::vec::IntoIter<Result<DirEntry>>,
}

impl Iterator for ReadDir {
    type Item = Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next()
    }
}

impl core::iter::FusedIterator for ReadDir {}

impl fmt::Debug for ReadDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ReadDir").field(&self.root).finish()
    }
}

/// An entry inside a directory.
///
/// `DirEntry` mirrors the high-level shape of [`std::fs::DirEntry`]. Because
/// this kernel treats all paths as UTF-8, [`file_name`](Self::file_name)
/// returns a [`String`] rather than an `OsString`.
pub struct DirEntry {
    root: crate::path::PathBuf,
    file_name: String,
}

impl DirEntry {
    /// Returns the full path to the file represented by this entry.
    #[must_use]
    pub fn path(&self) -> crate::path::PathBuf {
        self.root.join(self.file_name.as_str())
    }

    /// Returns metadata for the file that this entry points at.
    pub fn metadata(&self) -> Result<Metadata> {
        metadata(self.path())
    }

    /// Returns the file type for the file that this entry points at.
    pub fn file_type(&self) -> Result<FileType> {
        self.metadata().map(|metadata| metadata.file_type())
    }

    /// Returns the file name of this directory entry without leading path
    /// components.
    #[must_use]
    pub fn file_name(&self) -> String {
        self.file_name.clone()
    }
}

impl fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DirEntry").field(&self.path()).finish()
    }
}

// ---------------------------------------------------------------------------
// OpenOptions
// ---------------------------------------------------------------------------

/// Options and flags configuring how a [`File`] is opened.
///
/// Mirrors [`std::fs::OpenOptions`]. Configure flags via the builder methods
/// then call [`OpenOptions::open`] with a path.
#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    mode: u32,
}

impl OpenOptions {
    /// Creates a blank set of options ready for configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,
            create_new: false,
            mode: DEFAULT_CREATE_MODE,
        }
    }

    /// Sets the option for read access.
    pub fn read(&mut self, read: bool) -> &mut Self {
        self.read = read;
        self
    }

    /// Sets the option for write access.
    pub fn write(&mut self, write: bool) -> &mut Self {
        self.write = write;
        self
    }

    /// Sets the option for the append mode.
    pub fn append(&mut self, append: bool) -> &mut Self {
        self.append = append;
        self
    }

    /// Sets the option for truncating a previous file.
    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    /// Sets the option to create a new file, or open it if it already exists.
    pub fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    /// Sets the option to create a new file, failing if it already exists.
    pub fn create_new(&mut self, create_new: bool) -> &mut Self {
        self.create_new = create_new;
        self
    }

    /// Sets the permission mode used when creating a new file.
    pub fn mode(&mut self, mode: u32) -> &mut Self {
        self.mode = mode;
        self
    }

    /// Opens a file at `path` with the configured options.
    pub fn open<P: AsRef<Path>>(&self, path: P) -> Result<File> {
        self._open(path.as_ref())
    }

    fn _open(&self, path: &Path) -> Result<File> {
        let access = self.access_mode()?;
        self.validate_creation()?;

        let mut option_bits = syscall::fs::OpenOptions::empty();
        if self.create {
            option_bits |= syscall::fs::OpenOptions::CREATE;
        }
        if self.create_new {
            option_bits |= syscall::fs::OpenOptions::CREATE | syscall::fs::OpenOptions::EXCLUSIVE;
        }
        if self.truncate && !self.create_new {
            option_bits |= syscall::fs::OpenOptions::TRUNCATE;
        }
        if self.append {
            option_bits |= syscall::fs::OpenOptions::APPEND;
        }
        let flags = OpenFlags::new(access, option_bits);

        let path_c = path_cstring(path)?;
        match syscall::fs::open(path_c.as_ptr().cast(), flags, self.mode) {
            Ok(fd) => Ok(File {
                fd,
                path: Some(path.to_path_buf()),
            }),
            Err(errno) => Err(Error::from(errno)),
        }
    }

    fn access_mode(&self) -> Result<AccessMode> {
        match (self.read, self.write, self.append) {
            (true, false, false) => Ok(AccessMode::ReadOnly),
            (false, true, false) => Ok(AccessMode::WriteOnly),
            (true, true, false) => Ok(AccessMode::ReadWrite),
            (false, _, true) => Ok(AccessMode::WriteOnly),
            (true, _, true) => Ok(AccessMode::ReadWrite),
            (false, false, false) => Err(Error::from(ErrorKind::InvalidInput)),
        }
    }

    fn validate_creation(&self) -> Result<()> {
        match (self.write, self.append) {
            (true, false) => Ok(()),
            (false, false) => {
                if self.truncate || self.create || self.create_new {
                    Err(Error::from(ErrorKind::InvalidInput))
                } else {
                    Ok(())
                }
            }
            (_, true) => {
                if self.truncate && !self.create_new {
                    Err(Error::from(ErrorKind::InvalidInput))
                } else {
                    Ok(())
                }
            }
        }
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Metadata, Permissions, FileType
// ---------------------------------------------------------------------------

/// Representation of the various timestamps on a file.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileTimes {
    accessed: Option<SystemTime>,
    modified: Option<SystemTime>,
}

impl FileTimes {
    /// Creates a new `FileTimes` with no times set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the last access time of a file.
    pub fn set_accessed(mut self, time: SystemTime) -> Self {
        self.accessed = Some(time);
        self
    }

    /// Sets the last modified time of a file.
    pub fn set_modified(mut self, time: SystemTime) -> Self {
        self.modified = Some(time);
        self
    }
}

/// Metadata information about a file.
#[derive(Clone)]
pub struct Metadata {
    stat: Stat,
}

impl Metadata {
    /// Returns the size of the file, in bytes.
    pub fn len(&self) -> u64 {
        self.stat.st_size as u64
    }

    /// Returns `true` if the file size is zero.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if this metadata is for a regular file.
    pub fn is_file(&self) -> bool {
        (self.stat.st_mode & S_IFMT) == S_IFREG
    }

    /// Returns `true` if this metadata is for a directory.
    pub fn is_dir(&self) -> bool {
        (self.stat.st_mode & S_IFMT) == S_IFDIR
    }

    /// Returns the file type.
    pub fn file_type(&self) -> FileType {
        FileType {
            mode: self.stat.st_mode & S_IFMT,
        }
    }

    /// Returns the permissions of the file.
    pub fn permissions(&self) -> Permissions {
        Permissions {
            mode: self.stat.st_mode & PERM_MASK,
        }
    }

    /// Returns the inode number. Equivalent to `std::os::unix::fs::MetadataExt::ino`.
    pub fn ino(&self) -> u64 {
        u64::from(self.stat.st_ino)
    }

    /// Returns the raw mode bits. Equivalent to
    /// `std::os::unix::fs::MetadataExt::mode`.
    pub fn mode(&self) -> u32 {
        u32::from(self.stat.st_mode)
    }

    /// Returns the number of hard links. Equivalent to
    /// `std::os::unix::fs::MetadataExt::nlink`.
    pub fn nlink(&self) -> u64 {
        u64::from(self.stat.st_nlink)
    }

    /// Returns the user ID of the owner. Equivalent to
    /// `std::os::unix::fs::MetadataExt::uid`.
    pub fn uid(&self) -> u32 {
        u32::from(self.stat.st_uid)
    }

    /// Returns the group ID of the owner. Equivalent to
    /// `std::os::unix::fs::MetadataExt::gid`.
    pub fn gid(&self) -> u32 {
        u32::from(self.stat.st_gid)
    }

    /// Returns the device id holding this file. Equivalent to
    /// `std::os::unix::fs::MetadataExt::dev`.
    pub fn dev(&self) -> u64 {
        u64::from(self.stat.st_dev)
    }

    /// Returns the device ID for special files. Equivalent to
    /// `std::os::unix::fs::MetadataExt::rdev`.
    pub fn rdev(&self) -> u64 {
        u64::from(self.stat.st_rdev)
    }

    /// Returns the total size of this file in bytes. Equivalent to
    /// `std::os::unix::fs::MetadataExt::size`.
    pub fn size(&self) -> u64 {
        self.len()
    }

    /// Returns the last access time as seconds since the Unix epoch.
    /// Equivalent to `std::os::unix::fs::MetadataExt::atime`.
    pub fn atime(&self) -> i64 {
        i64::from(self.stat.st_atime)
    }

    /// Returns the last modification time as seconds since the Unix epoch.
    /// Equivalent to `std::os::unix::fs::MetadataExt::mtime`.
    pub fn mtime(&self) -> i64 {
        i64::from(self.stat.st_mtime)
    }

    /// Returns the last metadata-change time as seconds since the Unix
    /// epoch. Equivalent to `std::os::unix::fs::MetadataExt::ctime`.
    pub fn ctime(&self) -> i64 {
        i64::from(self.stat.st_ctime)
    }

    /// Returns the nanosecond portion of the last access time.
    ///
    /// Minix v1 stores timestamps with one-second precision, so this is
    /// always zero.
    pub fn atime_nsec(&self) -> i64 {
        0
    }

    /// Returns the nanosecond portion of the last modification time.
    ///
    /// Minix v1 stores timestamps with one-second precision, so this is
    /// always zero.
    pub fn mtime_nsec(&self) -> i64 {
        0
    }

    /// Returns the nanosecond portion of the last metadata-change time.
    ///
    /// Minix v1 stores timestamps with one-second precision, so this is
    /// always zero.
    pub fn ctime_nsec(&self) -> i64 {
        0
    }

    /// Returns the preferred block size for filesystem I/O. Equivalent to
    /// `std::os::unix::fs::MetadataExt::blksize`.
    pub fn blksize(&self) -> u64 {
        MINIX_BLOCK_SIZE
    }

    /// Returns the number of 512-byte blocks allocated to this file.
    /// Equivalent to `std::os::unix::fs::MetadataExt::blocks`.
    pub fn blocks(&self) -> u64 {
        self.size().div_ceil(POSIX_BLOCK_SIZE)
    }

    /// Returns the last modification time.
    ///
    /// Mirrors [`std::fs::Metadata::modified`].
    pub fn modified(&self) -> Result<SystemTime> {
        Ok(system_time_from_unix_seconds(self.stat.st_mtime))
    }

    /// Returns the last access time.
    ///
    /// Mirrors [`std::fs::Metadata::accessed`].
    pub fn accessed(&self) -> Result<SystemTime> {
        Ok(system_time_from_unix_seconds(self.stat.st_atime))
    }

    /// Returns the creation time listed in this metadata.
    ///
    /// This filesystem does not store a birth time. The Unix `ctime` field
    /// is a metadata-change time, not a creation time, so this returns
    /// [`ErrorKind::Unsupported`] instead of reporting misleading data.
    pub fn created(&self) -> Result<SystemTime> {
        Err(Error::from(ErrorKind::Unsupported))
    }
}

impl fmt::Debug for Metadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metadata")
            .field("file_type", &self.file_type())
            .field("permissions", &self.permissions())
            .field("len", &self.len())
            .finish()
    }
}

/// Representation of the various permissions on a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Permissions {
    mode: u16,
}

impl Permissions {
    /// Creates permissions from raw Unix mode bits.
    ///
    /// This is the local equivalent of `std::os::unix::fs::PermissionsExt`
    /// for this single-platform library.
    #[must_use]
    pub fn from_mode(mode: u32) -> Self {
        Self {
            mode: mode as u16 & PERM_MASK,
        }
    }

    /// Returns `true` if these permissions describe a readonly file.
    ///
    /// On this kernel "readonly" mirrors std's POSIX mapping: any owner /
    /// group / other write bit being unset.
    pub fn readonly(&self) -> bool {
        self.mode & ALL_WRITE_BITS == 0
    }

    /// Modifies the readonly flag for these permissions.
    pub fn set_readonly(&mut self, readonly: bool) {
        if readonly {
            self.mode &= !ALL_WRITE_BITS;
        } else {
            self.mode |= ALL_WRITE_BITS;
        }
    }

    /// Returns the underlying raw mode bits.
    pub fn mode(&self) -> u32 {
        u32::from(self.mode)
    }
}

/// A structure representing a type of file with accessors for each file type.
///
/// `is_symlink` always returns `false` because this kernel has no symlinks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileType {
    mode: u16,
}

impl FileType {
    /// Tests whether this file type represents a directory.
    pub fn is_dir(&self) -> bool {
        self.mode == S_IFDIR
    }

    /// Tests whether this file type represents a regular file.
    pub fn is_file(&self) -> bool {
        self.mode == S_IFREG
    }

    /// Always returns `false`; this kernel has no symbolic links.
    pub fn is_symlink(&self) -> bool {
        false
    }

    /// Tests whether this file type represents a character device.
    pub fn is_char_device(&self) -> bool {
        self.mode == S_IFCHR
    }

    /// Tests whether this file type represents a block device.
    pub fn is_block_device(&self) -> bool {
        self.mode == S_IFBLK
    }

    /// Tests whether this file type represents a FIFO (named pipe).
    pub fn is_fifo(&self) -> bool {
        self.mode == S_IFIFO
    }

    /// Always returns `false`; this kernel has no Unix-domain sockets.
    pub fn is_socket(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Reads the entire contents of a file into a bytes vector.
pub fn read<P: AsRef<Path>>(path: P) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let initial = file.metadata().map(|m| len_capacity(m.len())).unwrap_or(0);
    let mut bytes = Vec::with_capacity(initial);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Reads the entire contents of a file into a string.
pub fn read_to_string<P: AsRef<Path>>(path: P) -> Result<String> {
    let mut file = File::open(path)?;
    let initial = file.metadata().map(|m| len_capacity(m.len())).unwrap_or(0);
    let mut buf = String::with_capacity(initial);
    file.read_to_string(&mut buf)?;
    Ok(buf)
}

/// Copies the contents and permissions of one regular file to another.
///
/// Mirrors [`std::fs::copy`].
pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<u64> {
    let metadata = metadata(from.as_ref())?;
    if metadata.is_dir() {
        return Err(Error::from(ErrorKind::IsADirectory));
    }

    let mut reader = File::open(from.as_ref())?;
    let mut writer = File::create(to.as_ref())?;
    let mut buf = [0u8; 8 * 1024];
    let mut written_total = 0u64;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        written_total = written_total
            .checked_add(n as u64)
            .ok_or_else(|| Error::new(ErrorKind::Other, "copied byte count overflowed"))?;
    }
    writer.flush()?;
    set_permissions(to, metadata.permissions())?;
    Ok(written_total)
}

/// Returns an iterator over the entries within a directory.
///
/// Entries for the current and parent directories (`.` and `..`) are skipped,
/// matching [`std::fs::read_dir`].
///
/// The order is the on-disk Minix directory-entry order. As with
/// [`std::fs::read_dir`], callers that need reproducible ordering should
/// collect and sort the returned paths explicitly.
pub fn read_dir<P: AsRef<Path>>(path: P) -> Result<ReadDir> {
    let path = path.as_ref();
    let root = path.to_path_buf();
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() {
        return Err(Error::from(ErrorKind::NotADirectory));
    }

    let mut bytes = Vec::with_capacity(len_capacity(metadata.len()));
    file.read_to_end(&mut bytes)?;
    let entries = decode_directory_entries(root.clone(), &bytes)?;

    Ok(ReadDir {
        root,
        entries: entries.into_iter(),
    })
}

/// Writes a slice as the entire contents of a file, creating it (truncating
/// any previous contents) if necessary.
pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> Result<()> {
    let mut file = File::create(path)?;
    file.write_all(contents.as_ref())
}

/// Returns metadata for the file at `path`.
pub fn metadata<P: AsRef<Path>>(path: P) -> Result<Metadata> {
    let path_c = path_cstring(path.as_ref())?;
    let mut stat = empty_stat();
    syscall::fs::stat(path_c.as_ptr().cast(), &mut stat).map_err(Error::from)?;
    Ok(Metadata { stat })
}

/// Changes the permissions found on a file or directory.
///
/// Mirrors [`std::fs::set_permissions`]. The file type bits are preserved by
/// the kernel; only the permission and special mode bits are changed.
pub fn set_permissions<P: AsRef<Path>>(path: P, permissions: Permissions) -> Result<()> {
    let path_c = path_cstring(path.as_ref())?;
    syscall::fs::chmod(path_c.as_ptr().cast(), permissions.mode())
        .map(|_| ())
        .map_err(Error::from)
}

/// Removes a file from the filesystem.
pub fn remove_file<P: AsRef<Path>>(path: P) -> Result<()> {
    let path_c = path_cstring(path.as_ref())?;
    syscall::fs::unlink(path_c.as_ptr().cast())
        .map(|_| ())
        .map_err(Error::from)
}

/// Removes an empty directory.
pub fn remove_dir<P: AsRef<Path>>(path: P) -> Result<()> {
    let path_c = path_cstring(path.as_ref())?;
    syscall::fs::rmdir(path_c.as_ptr().cast())
        .map(|_| ())
        .map_err(Error::from)
}

/// Renames a file or directory to a new name, replacing the destination if
/// it already exists.
pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<()> {
    let from_c = path_cstring(from.as_ref())?;
    let to_c = path_cstring(to.as_ref())?;
    syscall::fs::rename(from_c.as_ptr().cast(), to_c.as_ptr().cast())
        .map(|_| ())
        .map_err(Error::from)
}

/// Creates a new, empty directory at the given path.
pub fn create_dir<P: AsRef<Path>>(path: P) -> Result<()> {
    let path_c = path_cstring(path.as_ref())?;
    syscall::fs::mkdir(path_c.as_ptr().cast(), DEFAULT_DIR_MODE)
        .map(|_| ())
        .map_err(Error::from)
}

/// Recursively creates a directory and all of its missing parents.
///
/// Mirrors [`std::fs::create_dir_all`]. Existing directory components are
/// accepted.
pub fn create_dir_all<P: AsRef<Path>>(path: P) -> Result<()> {
    let path = path.as_ref();
    if path.is_empty() {
        return Ok(());
    }
    if metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
        return Ok(());
    }

    let mut current = crate::path::PathBuf::new();
    for component in path.components() {
        let raw = component.as_str();
        if raw == "." {
            continue;
        }
        current.push(raw);
        match create_dir(current.as_path()) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                if !metadata(current.as_path())
                    .map(|m| m.is_dir())
                    .unwrap_or(false)
                {
                    return Err(Error::from(ErrorKind::AlreadyExists));
                }
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// Recursively removes a directory and all of its contents.
///
/// Mirrors [`std::fs::remove_dir_all`].
pub fn remove_dir_all<P: AsRef<Path>>(path: P) -> Result<()> {
    let path = path.as_ref();
    let root_metadata = metadata(path)?;
    if !root_metadata.is_dir() {
        return Err(Error::from(ErrorKind::NotADirectory));
    }

    let mut children = Vec::new();
    for item in read_dir(path)? {
        children.push(item?.path());
    }
    children.sort_by(|a, b| a.as_str().cmp(b.as_str()));

    for child in children {
        let child_metadata = metadata(child.as_path())?;
        if child_metadata.is_dir() {
            remove_dir_all(child.as_path())?;
        } else {
            remove_file(child.as_path())?;
        }
    }

    remove_dir(path)
}

/// Changes the current working directory to the specified path.
///
/// This mirrors [`std::env::set_current_dir`], but lives in [`crate::fs`]
/// because this library keeps process-environment helpers small and routes
/// filesystem syscalls through this module.
pub fn set_current_dir<P: AsRef<Path>>(path: P) -> Result<()> {
    let path_c = path_cstring(path.as_ref())?;
    syscall::fs::chdir(path_c.as_ptr().cast())
        .map(|_| ())
        .map_err(Error::from)
}

/// Creates a hard link from `original` to `link`.
pub fn hard_link<P: AsRef<Path>, Q: AsRef<Path>>(original: P, link: Q) -> Result<()> {
    let from_c = path_cstring(original.as_ref())?;
    let to_c = path_cstring(link.as_ref())?;
    syscall::fs::link(from_c.as_ptr().cast(), to_c.as_ptr().cast())
        .map(|_| ())
        .map_err(Error::from)
}

/// Changes the owner and group of a file or directory.
///
/// `None` values for `uid` or `gid` leave that field unchanged.
///
/// This is the local equivalent of [`std::os::unix::fs::chown`]; because
/// this library targets a single Unix-like kernel, it lives directly in
/// [`crate::fs`] rather than in a platform-specific module.
///
/// # Notes
///
/// - The caller must hold appropriate privileges (usually root) to change
///   the owner.
/// - Changing the owner clears the setuid and setgid bits.
pub fn chown<P: AsRef<Path>>(path: P, uid: Option<u32>, gid: Option<u32>) -> Result<()> {
    let path_c = path_cstring(path.as_ref())?;
    syscall::fs::chown(
        path_c.as_ptr().cast(),
        uid.unwrap_or(u32::MAX),
        gid.unwrap_or(u32::MAX),
    )
    .map(|_| ())
    .map_err(Error::from)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn path_cstring(path: &Path) -> Result<CString> {
    CString::new(path.as_str().to_owned()).map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            "path contains an interior NUL byte",
        )
    })
}

fn empty_stat() -> Stat {
    // SAFETY: `Stat` is `#[repr(C)]` and consists entirely of integer fields,
    // for which the all-zero bit pattern is a valid representation. The
    // value is overwritten by the kernel on successful return.
    unsafe { core::mem::zeroed() }
}

fn system_time_from_unix_seconds(secs: i32) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(u64::from(secs.unsigned_abs()))
    }
}

fn set_path_times(path: &Path, times: FileTimes) -> Result<()> {
    if times.accessed.is_none() && times.modified.is_none() {
        return Ok(());
    }

    let metadata = metadata(path)?;
    let access_time = match times.accessed {
        Some(time) => system_time_to_time_t(time)?,
        None => i32::try_from(metadata.atime())
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "access time is out of range"))?,
    };
    let modification_time = match times.modified {
        Some(time) => system_time_to_time_t(time)?,
        None => i32::try_from(metadata.mtime()).map_err(|_| {
            Error::new(ErrorKind::InvalidInput, "modification time is out of range")
        })?,
    };

    let update = TimeUpdate {
        access_time,
        modification_time,
    };
    let path_c = path_cstring(path)?;
    syscall::fs::utime(path_c.as_ptr().cast(), &update)
        .map(|_| ())
        .map_err(Error::from)
}

fn system_time_to_time_t(time: SystemTime) -> Result<i32> {
    let duration = time.duration_since(UNIX_EPOCH).map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            "timestamp is before the Unix epoch",
        )
    })?;
    i32::try_from(duration.as_secs())
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "timestamp is too large"))
}

fn len_capacity(len: u64) -> usize {
    usize::try_from(len).unwrap_or(0)
}

fn seek_parts(pos: SeekFrom) -> Result<(i32, Whence)> {
    // This library mirrors `std::io::Seek` publicly, while the kernel
    // syscall ABI itself uses a signed 32-bit offset word:
    //
    // +------------------+-----------------------------+
    // | user_lib type    | kernel syscall word         |
    // +------------------+-----------------------------+
    // | SeekFrom::Start  | signed 32-bit absolute off  |
    // | SeekFrom::End    | signed 32-bit relative off  |
    // | SeekFrom::Current| signed 32-bit relative off  |
    // +------------------+-----------------------------+
    //
    // Reject absolute offsets that cannot be represented by that signed ABI
    // instead of letting an integer cast wrap into an unrelated position.
    let invalid = || Error::new(ErrorKind::InvalidInput, "seek offset does not fit in i32");
    match pos {
        SeekFrom::Start(offset) => i32::try_from(offset)
            .map(|offset| (offset, Whence::Set))
            .map_err(|_| invalid()),
        SeekFrom::Current(offset) => i32::try_from(offset)
            .map(|offset| (offset, Whence::Current))
            .map_err(|_| invalid()),
        SeekFrom::End(offset) => i32::try_from(offset)
            .map(|offset| (offset, Whence::End))
            .map_err(|_| invalid()),
    }
}

fn decode_directory_entries(
    root: crate::path::PathBuf,
    bytes: &[u8],
) -> Result<Vec<Result<DirEntry>>> {
    let mut chunks = bytes.chunks_exact(MINIX_DIRECTORY_ENTRY_SIZE);
    if !chunks.remainder().is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "directory size is not aligned to Minix directory entries",
        ));
    }

    let mut entries = Vec::new();
    for chunk in &mut chunks {
        // Minix v1 directory entry layout:
        //
        // +----------------------+ byte offset
        // | inode number: u16le  | 0..2
        // +----------------------+
        // | NUL-padded name      | 2..16
        // | (14 bytes)           |
        // +----------------------+
        //
        // An inode number of zero marks a deleted/free slot.
        let inode_number = u16::from_le_bytes([chunk[0], chunk[1]]);
        if inode_number == 0 {
            continue;
        }

        let name_bytes = &chunk[2..2 + MINIX_DIRECTORY_NAME_LENGTH];
        let name_len = name_bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(MINIX_DIRECTORY_NAME_LENGTH);
        let name = core::str::from_utf8(&name_bytes[..name_len]).map_err(|_| {
            Error::new(
                ErrorKind::InvalidData,
                "directory entry name is not valid UTF-8",
            )
        })?;
        if name == "." || name == ".." {
            continue;
        }

        entries.push(Ok(DirEntry {
            root: root.clone(),
            file_name: name.to_owned(),
        }));
    }

    Ok(entries)
}
