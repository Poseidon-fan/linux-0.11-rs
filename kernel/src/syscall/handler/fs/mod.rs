//! Filesystem-related syscall handlers.
//!
//! - [`fd`]    — open, read, write, close, creat, lseek, dup, dup2, fcntl, ioctl, pipe, fstat
//! - [`path`]  — link, unlink, stat, chmod, chown, utime, access, chdir, chroot, mkdir, mknod, rmdir
//! - [`mount`] — setup, mount, umount, sync

mod fd;
mod mount;
mod path;
