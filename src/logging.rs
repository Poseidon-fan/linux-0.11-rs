// Manage VGA console manually for temporary use, will be rewritten when tty driver is implemented.

use core::{
    fmt::{self, Write},
    ptr,
};

use log::{Level, LevelFilter, Log, Metadata};

pub fn init() {
    // Clear the screen.
    for _ in 0..80 * 25 {
        put_char(b' ');
    }

    // Register the logger.
    static LOGGER: KernelLogger = KernelLogger;
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(match option_env!("LOG") {
        Some("ERROR") => LevelFilter::Error,
        Some("WARN") => LevelFilter::Warn,
        Some("INFO") => LevelFilter::Info,
        Some("DEBUG") => LevelFilter::Debug,
        Some("TRACE") => LevelFilter::Trace,
        _ => LevelFilter::Trace,
    });
}

#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::logging::put_fmt(format_args!($fmt $(, $($arg)+)?));
    };
}

#[macro_export]
macro_rules! println {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::logging::put_fmt(format_args!(concat!($fmt, "\n") $(, $($arg)+)?));
    };
}

struct Console;

fn put_char(c: u8) {
    static mut POS: usize = 0;

    match c {
        b'\n' => unsafe {
            // Move to the beginning of the next line
            // Current row = POS / 80, next row start = (current row + 1) * 80
            POS = (POS / 80 + 1) * 80;
            if POS >= 80 * 25 {
                POS = 0; // Wrap to top (or implement scrolling here)
            }
        },
        _ => {
            let vga = 0xb8000 as *mut u8;
            unsafe {
                ptr::write_volatile(vga.add(POS * 2), c);
                ptr::write_volatile(vga.add(POS * 2 + 1), 0x07);
                POS += 1;
                if POS == 80 * 25 {
                    POS = 0;
                }
            }
        }
    }
}

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            put_char(c as u8);
        }
        Ok(())
    }
}

pub fn put_fmt(args: fmt::Arguments) {
    Console.write_fmt(args).unwrap();
}

struct KernelLogger;

impl Log for KernelLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let color = match record.level() {
            Level::Error => 31, // Red
            Level::Warn => 93,  // BrightYellow
            Level::Info => 34,  // Blue
            Level::Debug => 32, // Green
            Level::Trace => 90, // BrightBlack
        };
        println!(
            "\u{1B}[{}m[{:>5}] {}\u{1B}[0m",
            color,
            record.level(),
            record.args(),
        );
    }

    fn flush(&self) {}
}
