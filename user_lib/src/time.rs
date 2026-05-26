//! Temporal quantification.
//!
//! Counterpart to [`std::time`], adapted for this kernel:
//!
//! - [`Duration`] is re-exported from [`core::time`] unchanged.
//! - [`Instant`] internally stores a [`Duration`] (= time since boot),
//!   mirroring `std`'s opaque-clock-tick model. The underlying source is
//!   the jiffy counter returned by `times(2)` (10 ms granularity on a
//!   100 Hz kernel), but the public arithmetic surface speaks in
//!   `Duration`, so `Instant` itself isn't restricted to the kernel's u32
//!   ABI for things like `checked_add`.
//! - [`SystemTime`] stores `i64` seconds plus `u32` sub-second
//!   nanoseconds, matching `std`'s `Timespec`. The `time(2)` syscall only
//!   produces u32 seconds, so `now()` always returns a value with
//!   `nanos == 0`; arithmetic that crosses 2106 or 1970 still works
//!   correctly inside the type.
//! - [`sleep`] uses `alarm(2) + pause(2)` for the whole-second portion of
//!   the requested delay and a jiffy-resolution busy-wait for any
//!   sub-second remainder. It clobbers any pending `alarm` set by the
//!   caller — `std::thread::sleep` doesn't have this concern because real
//!   Linux uses `nanosleep`, which we do not.
//!
//! `Instant` and `SystemTime` mirror std's full method surface
//! (`duration_since`, `checked_duration_since`, `saturating_duration_since`,
//! `elapsed`, `checked_add`, `checked_sub`, and the corresponding
//! `Add`/`Sub`/`AddAssign`/`SubAssign` impls).

pub use core::time::Duration;
use core::{
    error, fmt,
    ops::{Add, AddAssign, Sub, SubAssign},
};

use crate::syscall;

/// Kernel tick rate (jiffies per second). Must match
/// `kernel::task::timer::HZ`.
const HZ: u32 = 100;

/// Nanoseconds in one kernel jiffy. Derived from [`HZ`].
const NANOS_PER_TICK: u64 = 1_000_000_000 / HZ as u64;

/// Nanoseconds in one second.
const NANOS_PER_SEC: u32 = 1_000_000_000;

// ---------------------------------------------------------------------------
// Instant
// ---------------------------------------------------------------------------

/// A measurement of a monotonically nondecreasing clock.
///
/// Opaque and useful only with [`Duration`]. Internally the value is the
/// elapsed [`Duration`] since some unspecified epoch (boot, in our case),
/// so all arithmetic uses the full `Duration` range. The clock source is
/// the kernel's jiffy counter so observed resolution is 10 ms, and the
/// underlying counter is a u32 that wraps in roughly 497 days — long
/// enough that we don't model wraparound.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Instant {
    since_boot: Duration,
}

impl Instant {
    /// Returns an instant corresponding to "now".
    #[must_use]
    pub fn now() -> Instant {
        // `times(NULL)` returns the wall-clock tick count since boot as a
        // u32 (kernel ABI). Widen at the boundary so internal arithmetic
        // operates on a normal `Duration`.
        let jiffies = syscall::process::times(core::ptr::null_mut()).unwrap_or(0);
        Instant {
            since_boot: Duration::from_nanos(u64::from(jiffies) * NANOS_PER_TICK),
        }
    }

    /// Returns the amount of time elapsed from another instant to this
    /// one, or zero duration if that instant is later than this one.
    #[must_use]
    pub fn duration_since(&self, earlier: Instant) -> Duration {
        self.checked_duration_since(earlier).unwrap_or_default()
    }

    /// Returns the amount of time elapsed from another instant to this
    /// one, or `None` if that instant is later than this one.
    #[must_use]
    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        self.since_boot.checked_sub(earlier.since_boot)
    }

    /// Returns the amount of time elapsed from another instant to this
    /// one, or zero duration if that instant is later than this one.
    #[must_use]
    pub fn saturating_duration_since(&self, earlier: Instant) -> Duration {
        self.checked_duration_since(earlier).unwrap_or_default()
    }

    /// Returns the amount of time elapsed since this instant.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        Instant::now().saturating_duration_since(*self)
    }

    /// Returns `Some(t)` where `t` is the time `self + duration`, or
    /// `None` on overflow.
    #[must_use]
    pub fn checked_add(&self, duration: Duration) -> Option<Instant> {
        self.since_boot
            .checked_add(duration)
            .map(|since_boot| Instant { since_boot })
    }

    /// Returns `Some(t)` where `t` is the time `self - duration`, or
    /// `None` on underflow.
    #[must_use]
    pub fn checked_sub(&self, duration: Duration) -> Option<Instant> {
        self.since_boot
            .checked_sub(duration)
            .map(|since_boot| Instant { since_boot })
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;
    /// # Panics
    ///
    /// Panics on overflow. Use [`Instant::checked_add`] otherwise.
    fn add(self, rhs: Duration) -> Instant {
        self.checked_add(rhs)
            .expect("overflow when adding duration to instant")
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, rhs: Duration) {
        *self = *self + rhs;
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;
    /// # Panics
    ///
    /// Panics on underflow. Use [`Instant::checked_sub`] otherwise.
    fn sub(self, rhs: Duration) -> Instant {
        self.checked_sub(rhs)
            .expect("overflow when subtracting duration from instant")
    }
}

impl SubAssign<Duration> for Instant {
    fn sub_assign(&mut self, rhs: Duration) {
        *self = *self - rhs;
    }
}

impl Sub<Instant> for Instant {
    type Output = Duration;
    fn sub(self, rhs: Instant) -> Duration {
        self.duration_since(rhs)
    }
}

impl fmt::Debug for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Instant")
            .field("since_boot", &self.since_boot)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// SystemTime
// ---------------------------------------------------------------------------

/// A measurement of the system clock, useful for talking to external
/// entities like the file system or other processes.
///
/// Stores `(secs: i64, nanos: u32)` like `std::time::SystemTime`'s
/// underlying `Timespec`. The `time(2)` syscall used by [`Self::now`]
/// only produces u32 seconds, so `now()` always returns `nanos == 0` and
/// represents a moment between 1970 and 2106; arithmetic that crosses
/// those bounds still works correctly inside the type.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SystemTime {
    secs: i64,
    nanos: u32,
}

/// An anchor in time which can be used to create new [`SystemTime`]
/// instances or learn about where in time a `SystemTime` lies.
pub const UNIX_EPOCH: SystemTime = SystemTime { secs: 0, nanos: 0 };

impl SystemTime {
    /// An anchor in time equivalent to [`UNIX_EPOCH`].
    pub const UNIX_EPOCH: SystemTime = UNIX_EPOCH;

    /// Returns the system time corresponding to "now".
    #[must_use]
    pub fn now() -> SystemTime {
        let secs = syscall::process::time(core::ptr::null_mut()).unwrap_or(0);
        SystemTime {
            secs: i64::from(secs),
            nanos: 0,
        }
    }

    /// Returns the amount of time elapsed from an earlier point in time.
    ///
    /// Returns [`Err`] if `earlier` is later than `self`.
    pub fn duration_since(&self, earlier: SystemTime) -> Result<Duration, SystemTimeError> {
        match self.signed_difference(&earlier) {
            Ok(d) => Ok(d),
            Err(d) => Err(SystemTimeError(d)),
        }
    }

    /// Returns the difference from this system time to the current clock
    /// time, or [`Err`] if `self` is later than the current time.
    pub fn elapsed(&self) -> Result<Duration, SystemTimeError> {
        SystemTime::now().duration_since(*self)
    }

    /// Returns `Some(t)` where `t` is the time `self + duration`, or
    /// `None` on overflow.
    #[must_use]
    pub fn checked_add(&self, duration: Duration) -> Option<SystemTime> {
        let dur_secs = i64::try_from(duration.as_secs()).ok()?;
        let dur_nanos = duration.subsec_nanos();

        let (nanos, carry) = if self.nanos + dur_nanos >= NANOS_PER_SEC {
            (self.nanos + dur_nanos - NANOS_PER_SEC, 1_i64)
        } else {
            (self.nanos + dur_nanos, 0)
        };
        let secs = self.secs.checked_add(dur_secs)?.checked_add(carry)?;
        Some(SystemTime { secs, nanos })
    }

    /// Returns `Some(t)` where `t` is the time `self - duration`, or
    /// `None` on underflow.
    #[must_use]
    pub fn checked_sub(&self, duration: Duration) -> Option<SystemTime> {
        let dur_secs = i64::try_from(duration.as_secs()).ok()?;
        let dur_nanos = duration.subsec_nanos();

        let (nanos, borrow) = if self.nanos >= dur_nanos {
            (self.nanos - dur_nanos, 0_i64)
        } else {
            (NANOS_PER_SEC + self.nanos - dur_nanos, 1)
        };
        let secs = self.secs.checked_sub(dur_secs)?.checked_sub(borrow)?;
        Some(SystemTime { secs, nanos })
    }

    /// Returns `Ok(self - earlier)` if `self >= earlier`, or `Err(earlier
    /// - self)` otherwise. Both directions are returned as a non-negative
    /// [`Duration`].
    fn signed_difference(&self, earlier: &SystemTime) -> Result<Duration, Duration> {
        let (greater, lesser, positive) = if self.cmp_secs_nanos(earlier).is_ge() {
            (self, earlier, true)
        } else {
            (earlier, self, false)
        };
        // Borrow nanoseconds across the second boundary if needed.
        let (secs_diff, nanos_diff) = if greater.nanos >= lesser.nanos {
            (
                (greater.secs - lesser.secs) as u64,
                greater.nanos - lesser.nanos,
            )
        } else {
            (
                (greater.secs - lesser.secs - 1) as u64,
                NANOS_PER_SEC - (lesser.nanos - greater.nanos),
            )
        };
        let duration = Duration::new(secs_diff, nanos_diff);
        if positive {
            Ok(duration)
        } else {
            Err(duration)
        }
    }

    fn cmp_secs_nanos(&self, other: &SystemTime) -> core::cmp::Ordering {
        (self.secs, self.nanos).cmp(&(other.secs, other.nanos))
    }
}

impl Add<Duration> for SystemTime {
    type Output = SystemTime;
    /// # Panics
    ///
    /// Panics on overflow. Use [`SystemTime::checked_add`] otherwise.
    fn add(self, rhs: Duration) -> SystemTime {
        self.checked_add(rhs)
            .expect("overflow when adding duration to system time")
    }
}

impl AddAssign<Duration> for SystemTime {
    fn add_assign(&mut self, rhs: Duration) {
        *self = *self + rhs;
    }
}

impl Sub<Duration> for SystemTime {
    type Output = SystemTime;
    /// # Panics
    ///
    /// Panics on underflow. Use [`SystemTime::checked_sub`] otherwise.
    fn sub(self, rhs: Duration) -> SystemTime {
        self.checked_sub(rhs)
            .expect("overflow when subtracting duration from system time")
    }
}

impl SubAssign<Duration> for SystemTime {
    fn sub_assign(&mut self, rhs: Duration) {
        *self = *self - rhs;
    }
}

impl fmt::Debug for SystemTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemTime")
            .field("secs_since_epoch", &self.secs)
            .field("nanos", &self.nanos)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// SystemTimeError
// ---------------------------------------------------------------------------

/// An error returned from the `duration_since` and `elapsed` methods on
/// [`SystemTime`], used to learn how far in the opposite direction a
/// system time lies.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SystemTimeError(Duration);

impl SystemTimeError {
    /// Returns the positive duration which represents how far forward the
    /// second system time was from the first.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.0
    }
}

impl fmt::Display for SystemTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "second time provided was later than self")
    }
}

impl error::Error for SystemTimeError {}

// ---------------------------------------------------------------------------
// sleep
// ---------------------------------------------------------------------------

/// Blocks the current process for at least `duration`.
///
/// Sleep accuracy is limited by the kernel's 10 ms tick. The whole-second
/// portion is delegated to `alarm(2) + pause(2)`; any sub-second remainder
/// is busy-waited on the jiffy counter, which is correct but burns CPU
/// for up to one tick.
///
/// `pause(2)` returns on any signal, not just `SIGALRM`, so this function
/// re-arms the alarm and re-blocks until the deadline is reached.
///
/// # Caveats
///
/// This function calls `alarm(2)` and therefore overwrites any pending
/// alarm the caller had previously set. Real Linux's `nanosleep` does not
/// have this conflict; this kernel does not provide `nanosleep`, so we
/// trade a small interaction risk for the simplest possible
/// implementation.
pub fn sleep(duration: Duration) {
    let deadline = match Instant::now().checked_add(duration) {
        Some(d) => d,
        None => return,
    };
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let remaining = deadline.duration_since(now);
        let remaining_secs = remaining.as_secs();
        if remaining_secs >= 1 {
            // The `alarm(2)` argument is u32 per kernel ABI; cap accordingly.
            let n = u32::try_from(remaining_secs).unwrap_or(u32::MAX);
            let _ = syscall::process::alarm(n);
            let _ = syscall::process::pause();
        } else {
            core::hint::spin_loop();
        }
    }
}
