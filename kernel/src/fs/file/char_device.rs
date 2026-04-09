//! Character device file — dispatches read/write by major number.
//!
//! The character device dispatch table is indexed by major number:
//!
//! ```text
//! Major 0: (unused)
//! Major 1: /dev/mem, /dev/kmem, /dev/null, /dev/port — memory devices
//! Major 2: /dev/fd  (floppy — not yet wired)
//! Major 3: /dev/hd  (hard disk — not yet wired)
//! Major 4: /dev/ttyN — specific TTY by minor number
//! Major 5: /dev/tty  — current process's controlling terminal
//! Major 6: /dev/lp   (printer — not yet wired)
//! Major 7: unnamed pipes (handled separately)
//! ```

use alloc::sync::Arc;

use user_lib::fs::Stat;

use super::File;
use crate::{
    driver::{DevNum, chr::tty::Tty},
    fs::minix::Inode,
    segment::uaccess,
    syscall::*,
    task,
};

/// Opened character device file.
///
/// Holds a reference to the backing inode (for `stat`) and the device
/// number extracted from `direct_zones[0]`.
pub struct CharDeviceFile {
    dev: DevNum,
    inode: Arc<Inode>,
}

impl CharDeviceFile {
    pub fn new(inode: Arc<Inode>) -> Self {
        let dev = inode.device_number();
        Self { inode, dev }
    }
}

impl File for CharDeviceFile {
    fn read(&self, buffer: &mut [u8]) -> Result<usize, u32> {
        rw_char(RwDir::Read, self.dev, buffer.as_mut_ptr(), buffer.len())
    }

    fn write(&self, buffer: &[u8]) -> Result<usize, u32> {
        rw_char(RwDir::Write, self.dev, buffer.as_ptr(), buffer.len())
    }

    fn stat(&self) -> Result<Stat, u32> {
        Ok(self.inode.stat())
    }

    fn ioctl(&self, cmd: u32, arg: u32) -> Result<u32, u32> {
        ioctl_char(self.dev, cmd, arg)
    }
}

/// Character device ioctl dispatcher — equivalent of `sys_ioctl`'s
/// `ioctl_table[MAJOR(dev)]` lookup.
fn ioctl_char(dev: DevNum, cmd: u32, arg: u32) -> Result<u32, u32> {
    let minor = dev.minor() as usize;
    match dev.major() {
        4 => {
            if minor >= Tty::DEVICE_COUNT {
                return Err(ENODEV);
            }
            Tty::device(minor).ioctl(minor, cmd, arg)
        }
        5 => {
            let tty_nr = task::with_current(|inner| inner.tty);
            if tty_nr < 0 {
                return Err(EPERM);
            }
            let minor = tty_nr as usize;
            Tty::device(minor).ioctl(minor, cmd, arg)
        }
        _ => Err(ENOTTY),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RwDir {
    Read,
    Write,
}

/// Top-level character device dispatcher — equivalent of `rw_char()`.
fn rw_char(dir: RwDir, dev: DevNum, buf: *const u8, count: usize) -> Result<usize, u32> {
    match dev.major() {
        1 => rw_memory(dir, dev.minor(), buf, count),
        4 => rw_ttyx(dir, dev.minor() as usize, buf, count),
        5 => rw_tty(dir, buf, count),
        _ => Err(ENODEV),
    }
}

/// Major 4 — read/write a specific TTY device by minor number.
///
/// The `File` layer has already copied user data into a kernel buffer, but
/// `Tty::read/write` use `uaccess::read_u8/write_u8` (which go through
/// `%fs`). We set `%fs` to the kernel data segment so those accessors
/// operate on our kernel buffer.
fn rw_ttyx(dir: RwDir, minor: usize, buf: *const u8, count: usize) -> Result<usize, u32> {
    if minor >= Tty::DEVICE_COUNT {
        return Err(ENODEV);
    }
    let tty = Tty::device(minor);
    uaccess::with_kernel_fs(|| match dir {
        RwDir::Read => tty.read(minor, buf as *mut u8, count).map(|n| n as usize),
        RwDir::Write => tty.write(minor, buf, count).map(|n| n as usize),
    })
}

/// Major 5 — read/write the calling process's controlling terminal.
fn rw_tty(dir: RwDir, buf: *const u8, count: usize) -> Result<usize, u32> {
    let tty_nr = task::with_current(|inner| inner.tty);
    if tty_nr < 0 {
        return Err(EPERM);
    }
    rw_ttyx(dir, tty_nr as usize, buf, count)
}

/// Major 1 — memory pseudo-devices dispatched by minor number.
fn rw_memory(dir: RwDir, minor: u8, _buf: *const u8, count: usize) -> Result<usize, u32> {
    match minor {
        // 0 = /dev/ram, 1 = /dev/mem, 2 = /dev/kmem — stub EIO
        0..=2 => Err(EIO),
        // 3 = /dev/null — reads return 0 bytes, writes succeed silently
        3 => match dir {
            RwDir::Read => Ok(0),
            RwDir::Write => Ok(count),
        },
        // 4 = /dev/port — stub EIO
        4 => Err(EIO),
        _ => Err(EIO),
    }
}
