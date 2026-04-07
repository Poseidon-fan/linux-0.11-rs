//! User-space console I/O — writes to stdout (fd 1) via the `write` syscall.
//!
//! Provides `print!` and `println!` macros for formatted output, equivalent
//! to the original `printf` used in `init/main.c`.

use core::fmt::{self, Write};

use crate::fs;

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let _ = fs::write(1, s.as_ptr(), s.len() as u32);
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! print {
    ($fmt:literal $(, $($arg:tt)+)?) => {
        $crate::console::_print(format_args!($fmt $(, $($arg)+)?));
    };
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($fmt:literal $(, $($arg:tt)+)?) => {
        $crate::console::_print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?));
    };
}
