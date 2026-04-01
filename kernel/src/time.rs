//! Kernel timekeeping support.
//!
//! Tracks the Unix timestamp captured during boot and provides helpers
//! for retrieving the current kernel time.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::{pmio, task};

/// Initializes kernel timekeeping from the CMOS RTC.
///
/// The RTC can tick while the kernel is reading its registers, so the time is
/// sampled repeatedly until the seconds field is stable across the read.
pub fn init() {
    let startup_timestamp = unix_timestamp_from_rtc(read_rtc_time());
    STARTUP_TIMESTAMP.store(startup_timestamp, Ordering::Relaxed);
}

/// Returns the Unix timestamp captured at boot.
#[inline]
pub fn startup_time() -> u32 {
    STARTUP_TIMESTAMP.load(Ordering::Relaxed)
}

/// Returns the current Unix timestamp based on the boot timestamp and timer ticks.
#[inline]
pub fn current_time() -> u32 {
    startup_time() + task::jiffies() / task::HZ
}

// Unix timestamp captured during kernel initialization.
//
// The value is written once during boot and then read without any additional
// synchronization requirements beyond atomicity.
static STARTUP_TIMESTAMP: AtomicU32 = AtomicU32::new(0);

// RTC register indices in the CMOS address space.
const RTC_SECONDS_REGISTER: u8 = 0x00;
const RTC_MINUTES_REGISTER: u8 = 0x02;
const RTC_HOURS_REGISTER: u8 = 0x04;
const RTC_DAY_REGISTER: u8 = 0x07;
const RTC_MONTH_REGISTER: u8 = 0x08;
const RTC_YEAR_REGISTER: u8 = 0x09;

// Time unit constants in seconds.
const MINUTE: u32 = 60;
const HOUR: u32 = 60 * MINUTE;
const DAY: u32 = 24 * HOUR;
const COMMON_YEAR: u32 = 365 * DAY;
const UNIX_EPOCH_YEAR: u32 = 1970;
const UNIX_EPOCH_YEAR_IN_CENTURY: u32 = UNIX_EPOCH_YEAR % 100;
const MONTH_START_DAYS_IN_LEAP_YEAR: [u32; 12] = build_month_start_days_in_leap_year();

// Calendar fields decoded from the RTC CMOS registers.
struct RtcTime {
    second: u32,          // Seconds in [0, 60] to allow for a leap second.
    minute: u32,          // Minutes in [0, 59].
    hour: u32,            // Hours in [0, 23].
    day_of_month: u32,    // Day of month in [1, 31].
    month_index: usize,   // Zero-based month index in [0, 11].
    year_of_century: u32, // Last two digits of the Gregorian year in [0, 99].
}

// Reads the RTC until the seconds register remains unchanged across the sample.
fn read_rtc_time() -> RtcTime {
    loop {
        let second = pmio::read_cmos(RTC_SECONDS_REGISTER);
        let minute = pmio::read_cmos(RTC_MINUTES_REGISTER);
        let hour = pmio::read_cmos(RTC_HOURS_REGISTER);
        let day = pmio::read_cmos(RTC_DAY_REGISTER);
        let month = pmio::read_cmos(RTC_MONTH_REGISTER);
        let year = pmio::read_cmos(RTC_YEAR_REGISTER);

        if second == pmio::read_cmos(RTC_SECONDS_REGISTER) {
            return RtcTime {
                second: decode_bcd(second),
                minute: decode_bcd(minute),
                hour: decode_bcd(hour),
                day_of_month: decode_bcd(day),
                month_index: (decode_bcd(month) - 1) as usize,
                year_of_century: decode_bcd(year),
            };
        }
    }
}

// Converts the RTC packed-BCD encoding into a binary integer.
const fn decode_bcd(value: u8) -> u32 {
    ((value & 0x0F) + (value >> 4) * 10) as u32
}

// Converts a decoded RTC value to a Unix timestamp.
//
// The RTC only stores a two-digit year. Values `70..=99` are interpreted as
// `1970..=1999`, and values `0..=69` are interpreted as `2000..=2069`.
fn unix_timestamp_from_rtc(rtc: RtcTime) -> u32 {
    let full_year = full_year_from_rtc(rtc.year_of_century);
    let years_since_epoch = full_year - UNIX_EPOCH_YEAR;

    // `MONTH_START_DAYS_IN_LEAP_YEAR` assumes February has 29 days. Remove
    // that extra day when the current year is not a leap year and the date is
    // already past February.
    let leap_day_adjustment = if rtc.month_index > 1 && !is_leap_year(full_year) {
        DAY
    } else {
        0
    };

    COMMON_YEAR * years_since_epoch
        + DAY * ((years_since_epoch + 1) / 4)
        + DAY * MONTH_START_DAYS_IN_LEAP_YEAR[rtc.month_index]
        + DAY * (rtc.day_of_month - 1)
        + HOUR * rtc.hour
        + MINUTE * rtc.minute
        + rtc.second
        - leap_day_adjustment
}

// Expands the RTC two-digit year into the full-year range supported by the kernel.
const fn full_year_from_rtc(year_of_century: u32) -> u32 {
    if year_of_century >= UNIX_EPOCH_YEAR_IN_CENTURY {
        1900 + year_of_century
    } else {
        2000 + year_of_century
    }
}

// Checks leap-year status for the RTC range supported by the kernel.
//
// The supported RTC window stays below 2100, so the simple divisibility-by-4
// rule matches the Gregorian calendar for every reachable year.
const fn is_leap_year(year: u32) -> bool {
    year % 4 == 0
}

// Builds the day offset from January 1 to the start of each month in a leap year.
const fn build_month_start_days_in_leap_year() -> [u32; 12] {
    let month_lengths = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut offsets = [0u32; 12];
    let mut month = 1;

    while month < 12 {
        offsets[month] = offsets[month - 1] + month_lengths[month - 1];
        month += 1;
    }

    offsets
}
