//! Filesystem manipulation operations.
//!
//! Counterpart to [`std::fs`], adapted for this kernel:
//!
//! - File offsets and sizes are exposed as 32-bit values to match the
//!   i386/Linux 0.11 ABI and the Minix v1 on-disk format.
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
    path::Path,
    syscall::{
        self,
        fs::{AccessMode, OpenFlags, Stat, Whence},
    },
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
        File { fd }
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        match syscall::fs::read(self.fd, buf.as_mut_ptr(), buf.len() as u32) {
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
        match syscall::fs::write(self.fd, buf.as_ptr(), buf.len() as u32) {
            Ok(count) => Ok(count as usize),
            Err(errno) => Err(Error::from(errno)),
        }
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> Result<u32> {
        let (offset, whence) = seek_parts(pos)?;
        match syscall::fs::lseek(self.fd, offset, whence) {
            Ok(n) => Ok(n),
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
        f.debug_struct("File").field("fd", &self.fd).finish()
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
            Ok(fd) => Ok(File { fd }),
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

/// Metadata information about a file.
#[derive(Clone)]
pub struct Metadata {
    stat: Stat,
}

impl Metadata {
    /// Returns the size of the file, in bytes.
    pub fn len(&self) -> u32 {
        self.stat.st_size
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
    pub fn ino(&self) -> u32 {
        u32::from(self.stat.st_ino)
    }

    /// Returns the device id holding this file. Equivalent to
    /// `std::os::unix::fs::MetadataExt::dev`.
    pub fn dev(&self) -> u32 {
        u32::from(self.stat.st_dev)
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
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Reads the entire contents of a file into a bytes vector.
pub fn read<P: AsRef<Path>>(path: P) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let initial = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
    let mut bytes = Vec::with_capacity(initial);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Reads the entire contents of a file into a string.
pub fn read_to_string<P: AsRef<Path>>(path: P) -> Result<String> {
    let mut file = File::open(path)?;
    let initial = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
    let mut buf = String::with_capacity(initial);
    file.read_to_string(&mut buf)?;
    Ok(buf)
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

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
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

fn seek_parts(pos: SeekFrom) -> Result<(i32, Whence)> {
    // This library mirrors `std::io::Seek` structurally, but exposes 32-bit
    // offsets because the target kernel and filesystem are 32-bit. The
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
        SeekFrom::Current(offset) => Ok((offset, Whence::Current)),
        SeekFrom::End(offset) => Ok((offset, Whence::End)),
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
