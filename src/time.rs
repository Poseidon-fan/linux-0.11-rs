use crate::{
    pmio::{inb_p, outb_p},
    println,
};

// Partial implementation of the ISO C `broken-down time' structure.
struct Time {
    pub second: u32, // Seconds 	[0-60] (1 leap second)
    pub minute: u32, // Minutes 	[0-59]
    pub hour: u32,   // Hours	    [0-23]
    pub day: u32,    // Day		    [1-31]
    pub month: u32,  // Month	    [0-11]
    pub year: u32,   // Year	    [1900-...]
}

const MINUTE: u32 = 60;
const HOUR: u32 = 60 * MINUTE;
const DAY: u32 = 24 * HOUR;
const YEAR: u32 = 365 * DAY;
const MONTH: [u32; 12] = calculate_months();

pub fn init() {
    // Read a byte from the CMOS RTC at the specified address.
    let cmos_read = |addr: u8| {
        outb_p(0x80 | addr, 0x70);
        inb_p(0x71)
    };

    // Convert BCD (Binary-Coded Decimal) to binary.
    let bcd_to_bin = |val: u8| ((val & 0x0F) + (val >> 4) * 10) as u32;

    let time = loop {
        let second = cmos_read(0);
        let minute = cmos_read(2);
        let hour = cmos_read(4);
        let day = cmos_read(7);
        let month = cmos_read(8);
        let year = cmos_read(9);

        if second == cmos_read(0) {
            break Time {
                second: bcd_to_bin(second),
                minute: bcd_to_bin(minute),
                hour: bcd_to_bin(hour),
                day: bcd_to_bin(day),
                month: bcd_to_bin(month) - 1, // month is 0-11
                year: bcd_to_bin(year),
            };
        }
    };
    let startup_time = kernel_mktime(&time);
    println!("startup time: {}", startup_time);
}

// Convert a `Time` struct to the number of seconds since 1970-01-01 00:00:00 UTC.
fn kernel_mktime(tm: &Time) -> u32 {
    let year = match tm.year {
        y if y >= 70 => y - 70,
        y => y + 100 - 70, // Y2K bug fix
    };

    // Since we assume a leap year in `calculate_months`, may need to adjust a day time if not a leap year.
    let leap_day_adjustment = if tm.month > 1 && (year + 2) % 4 != 0 {
        DAY
    } else {
        0
    };

    [
        YEAR * year + DAY * ((year + 1) / 4), // magic number for calculating leap years since 1970
        MONTH[tm.month as usize],
        DAY * (tm.day - 1),
        HOUR * tm.hour,
        MINUTE * tm.minute,
        tm.second,
    ]
    .into_iter()
    .sum::<u32>()
        - leap_day_adjustment
}

// Calculate the number of seconds in each month, assuming a leap year.
const fn calculate_months() -> [u32; 12] {
    let days_in_month = [0, 31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30];
    let mut result = [0u32; 12];
    let mut i = 0;
    let mut total_days = 0;

    while i < 12 {
        total_days += days_in_month[i];
        result[i] = total_days * DAY;
        i += 1;
    }
    result
}
