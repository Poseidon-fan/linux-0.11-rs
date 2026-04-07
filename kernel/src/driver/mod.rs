use core::sync::atomic::{AtomicU16, Ordering};

pub mod blk;
pub mod chr;

static ROOT_DEV: AtomicU16 = AtomicU16::new(0);

#[inline]
pub fn set_root_dev(dev: DevNum) {
    ROOT_DEV.store(dev.0, Ordering::Release);
}

#[inline]
pub fn root_dev() -> DevNum {
    DevNum(ROOT_DEV.load(Ordering::Acquire))
}

/// Encoded kernel device number (`major:minor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct DevNum(pub u16);

impl DevNum {
    /// Build a device number from major and minor components.
    #[inline]
    pub const fn new(major: u8, minor: u8) -> Self {
        Self(((major as u16) << 8) | minor as u16)
    }

    /// Extract major from an encoded device number.
    #[inline]
    pub const fn major(self) -> u8 {
        (self.0 >> 8) as u8
    }

    /// Extract minor from an encoded device number.
    #[inline]
    pub const fn minor(self) -> u8 {
        (self.0 & 0x00ff) as u8
    }
}
