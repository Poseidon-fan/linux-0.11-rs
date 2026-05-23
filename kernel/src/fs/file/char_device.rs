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

use user_lib::syscall::fs::Stat;

use super::File;
use crate::{
    driver::{DevNum, character::tty},
    error::{Errno, Result},
    fs::minix::Inode,
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
    fn read(&self, buffer: &mut [u8]) -> Result<usize> {
        match self.dev.major() {
            1 => read_memory(self.dev.minor(), buffer.len()),
            4 => tty::read(self.dev.minor() as usize, buffer),
            5 => tty::read(current_tty_channel()?, buffer),
            _ => Err(Errno::NODEV),
        }
    }

    fn write(&self, buffer: &[u8]) -> Result<usize> {
        match self.dev.major() {
            1 => write_memory(self.dev.minor(), buffer.len()),
            4 => tty::write(self.dev.minor() as usize, buffer),
            5 => tty::write(current_tty_channel()?, buffer),
            _ => Err(Errno::NODEV),
        }
    }

    fn stat(&self) -> Result<Stat> {
        Ok(self.inode.stat())
    }

    fn ioctl(&self, cmd: u32, arg: u32) -> Result<u32> {
        match self.dev.major() {
            4 => tty::ioctl(self.dev.minor() as usize, cmd, arg),
            5 => tty::ioctl(current_tty_channel()?, cmd, arg),
            _ => Err(Errno::NOTTY),
        }
    }
}

/// Return the current process's controlling terminal channel.
fn current_tty_channel() -> Result<usize> {
    let tty_index = task::with_current(|inner| inner.tty);
    if tty_index < 0 {
        return Err(Errno::PERM);
    }
    Ok(tty_index as usize)
}

/// Major 1 — memory pseudo-devices read by minor number.
fn read_memory(minor: u8, _count: usize) -> Result<usize> {
    match minor {
        // 0 = /dev/ram, 1 = /dev/mem, 2 = /dev/kmem — stub Errno::IO
        0..=2 => Err(Errno::IO),
        // 3 = /dev/null — reads return EOF
        3 => Ok(0),
        // 4 = /dev/port — stub Errno::IO
        4 => Err(Errno::IO),
        _ => Err(Errno::IO),
    }
}

/// Major 1 — memory pseudo-devices written by minor number.
fn write_memory(minor: u8, count: usize) -> Result<usize> {
    match minor {
        // 0 = /dev/ram, 1 = /dev/mem, 2 = /dev/kmem — stub Errno::IO
        0..=2 => Err(Errno::IO),
        // 3 = /dev/null — writes are discarded successfully
        3 => Ok(count),
        // 4 = /dev/port — stub Errno::IO
        4 => Err(Errno::IO),
        _ => Err(Errno::IO),
    }
}
