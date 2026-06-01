//! Kernel logging and formatted text output.
//!
//! Two output modes, switched by a one-way `TTY_READY` flag:
//!
//! - **Early boot** (`TTY_READY = false`): direct VGA writes at 0xB8000,
//!   no scrolling, wraps to top. Used before `console::init()`.
//!
//! - **TTY mode** (`TTY_READY = true`): formats into a static 1024-byte
//!   buffer, then writes those kernel-owned bytes through TTY channel 0. This
//!   keeps the kernel buffer kernel-owned instead of pretending it is a
//!   user-space pointer.

use core::{
    fmt::{self, Write},
    ptr::{self, addr_of, addr_of_mut},
    sync::atomic::{AtomicBool, Ordering},
};

use log::{LevelFilter, Log, Metadata};

use crate::driver::character::console::{ORIG_X, ORIG_Y};

/// Mark the TTY subsystem as ready for kernel output.
/// Called at the end of `driver::character::console::init()`.
pub fn set_tty_ready() {
    TTY_READY.store(true, Ordering::Release);
}

/// Initializes the kernel logger and continues early VGA output from the
/// cursor position the bootloader left behind.
pub fn init() {
    // Start early VGA output from where the bootloader left the cursor,
    // so bootloader messages stay visible and our output follows after them.
    let orig_x = unsafe { ptr::read_volatile(ORIG_X) } as usize;
    let orig_y = unsafe { ptr::read_volatile(ORIG_Y) } as usize;
    EARLY_VGA_POS.store(orig_y * EARLY_VGA_COLUMNS + orig_x, Ordering::Relaxed);

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

/// Format and output one kernel message.
pub fn put_fmt(args: fmt::Arguments) {
    if !TTY_READY.load(Ordering::Acquire) {
        EarlyConsole.write_fmt(args).unwrap();
        return;
    }

    // Format into a static buffer before handing the bytes to the TTY layer.
    unsafe {
        LOG_LEN = 0;
        LogBufWriter.write_fmt(args).unwrap();
        let len = LOG_LEN;
        let bytes = core::slice::from_raw_parts(addr_of!(LOG_BUF) as *const u8, len);
        let _ = crate::driver::character::tty::write(0, bytes);
    }
}

/// Return the early VGA cursor as (column, row) so the VGA console can
/// continue output from where early boot left off.
pub fn early_vga_cursor() -> (usize, usize) {
    let pos = EARLY_VGA_POS.load(Ordering::Relaxed);
    (pos % EARLY_VGA_COLUMNS, pos / EARLY_VGA_COLUMNS)
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

/// Set to `true` after `console::init()` completes. Once set, all output
/// is routed through the TTY layer instead of direct VGA.
static TTY_READY: AtomicBool = AtomicBool::new(false);

/// Static format buffer used to bridge formatted output into the TTY path.
/// This buffer is not reentrant.
static mut LOG_BUF: [u8; 1024] = [0u8; 1024];
/// Number of valid bytes currently held in `LOG_BUF`.
static mut LOG_LEN: usize = 0;

/// Cursor position for early boot VGA output (character index, not byte offset).
/// Tracked at module level so `console::init` can read it to continue seamlessly.
static EARLY_VGA_POS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Base address of the VGA text-mode framebuffer.
const EARLY_VGA_BASE: *mut u8 = 0xb8000 as *mut u8;
/// Columns per row in VGA text mode.
const EARLY_VGA_COLUMNS: usize = 80;
/// Rows on the VGA text-mode screen.
const EARLY_VGA_ROWS: usize = 25;
/// Total addressable character cells.
const EARLY_VGA_CELLS: usize = EARLY_VGA_COLUMNS * EARLY_VGA_ROWS;
/// Attribute byte (light gray on black) written alongside each character.
const EARLY_VGA_ATTR: u8 = 0x07;

/// Byte writer over the static `LOG_BUF`, used during formatting.
struct LogBufWriter;

/// Direct VGA text-mode writer used before the TTY layer is available.
struct EarlyConsole;

/// Adapter that routes the `log` crate's records to [`put_fmt`].
struct KernelLogger;

/// Writes one character to the early VGA framebuffer, advancing and wrapping
/// the cursor as needed.
fn early_put_char(c: u8) {
    let mut pos = EARLY_VGA_POS.load(Ordering::Relaxed);

    match c {
        b'\n' => {
            let vga = EARLY_VGA_BASE;
            let line_end = (pos / EARLY_VGA_COLUMNS + 1) * EARLY_VGA_COLUMNS;
            for i in pos..line_end.min(EARLY_VGA_CELLS) {
                unsafe {
                    ptr::write_volatile(vga.add(i * 2), b' ');
                    ptr::write_volatile(vga.add(i * 2 + 1), EARLY_VGA_ATTR);
                }
            }
            pos = line_end;
            if pos >= EARLY_VGA_CELLS {
                pos = 0;
            }
        }
        _ => {
            let vga = EARLY_VGA_BASE;
            unsafe {
                ptr::write_volatile(vga.add(pos * 2), c);
                ptr::write_volatile(vga.add(pos * 2 + 1), EARLY_VGA_ATTR);
            }
            pos += 1;
            if pos == EARLY_VGA_CELLS {
                pos = 0;
            }
        }
    }

    EARLY_VGA_POS.store(pos, Ordering::Relaxed);
}

impl Write for LogBufWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        unsafe {
            let buf = addr_of_mut!(LOG_BUF) as *mut u8;
            for &b in s.as_bytes() {
                if LOG_LEN < 1024 {
                    buf.add(LOG_LEN).write(b);
                    LOG_LEN += 1;
                }
            }
        }
        Ok(())
    }
}

impl Write for EarlyConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            early_put_char(c as u8);
        }
        Ok(())
    }
}

impl Log for KernelLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        println!("[{:>5}] {}", record.level(), record.args());
    }

    fn flush(&self) {}
}
