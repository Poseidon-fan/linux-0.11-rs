//! Kernel logging and formatted text output.
//!
//! Two output modes, switched by a one-way `TTY_READY` flag:
//!
//! - **Early boot** (`TTY_READY = false`): direct VGA writes at 0xB8000,
//!   no scrolling, wraps to top. Used before `console::init()`.
//!
//! - **TTY mode** (`TTY_READY = true`): formats into a static 1024-byte
//!   buffer, then calls `Tty::write(0, buf, len)` — the same path used by
//!   user-space writes. To make `uaccess::read_u8` (which reads through
//!   `%fs`) work on kernel memory, `%fs` is temporarily set to the kernel
//!   data segment (0x10) before the call so the output path can read from
//!   the kernel-owned format buffer.

use core::{
    fmt::{self, Write},
    ptr::{self, addr_of, addr_of_mut},
    sync::atomic::{AtomicBool, Ordering},
};

use log::{LevelFilter, Log, Metadata};

/// Set to `true` after `console::init()` completes. Once set, all output
/// is routed through the TTY layer instead of direct VGA.
static TTY_READY: AtomicBool = AtomicBool::new(false);

/// Mark the TTY subsystem as ready for kernel output.
/// Called at the end of `driver::chr::console::init()`.
pub fn set_tty_ready() {
    TTY_READY.store(true, Ordering::Release);
}

/// Static format buffer used to bridge formatted output into the TTY path.
/// This buffer is not reentrant.
static mut LOG_BUF: [u8; 1024] = [0u8; 1024];
static mut LOG_LEN: usize = 0;

pub fn init() {
    // Start early VGA output from where the bootloader left the cursor,
    // so bootloader messages stay visible and our output follows after them.
    let orig_x = unsafe { ptr::read_volatile(0x90000 as *const u8) } as usize;
    let orig_y = unsafe { ptr::read_volatile(0x90001 as *const u8) } as usize;
    EARLY_VGA_POS.store(orig_y * 80 + orig_x, Ordering::Relaxed);

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
        let buf_ptr = addr_of!(LOG_BUF) as *const u8;

        crate::segment::uaccess::with_kernel_fs(|| {
            let _ = crate::driver::chr::tty::Tty::device(0).write(0, buf_ptr, len);
        });
    }
}

/// Byte writer over the static `LOG_BUF`, used during formatting.
struct LogBufWriter;

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

// ---- Early boot VGA (before TTY is available) ----

/// Cursor position for early boot VGA output (character index, not byte offset).
/// Tracked at module level so `console::init` can read it to continue seamlessly.
static EARLY_VGA_POS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Return the early VGA cursor as (column, row) so the VGA console can
/// continue output from where early boot left off.
pub fn early_vga_cursor() -> (usize, usize) {
    let pos = EARLY_VGA_POS.load(Ordering::Relaxed);
    (pos % 80, pos / 80)
}

fn early_put_char(c: u8) {
    let mut pos = EARLY_VGA_POS.load(Ordering::Relaxed);

    match c {
        b'\n' => {
            let vga = 0xb8000 as *mut u8;
            let line_end = (pos / 80 + 1) * 80;
            for i in pos..line_end.min(80 * 25) {
                unsafe {
                    ptr::write_volatile(vga.add(i * 2), b' ');
                    ptr::write_volatile(vga.add(i * 2 + 1), 0x07);
                }
            }
            pos = line_end;
            if pos >= 80 * 25 {
                pos = 0;
            }
        }
        _ => {
            let vga = 0xb8000 as *mut u8;
            unsafe {
                ptr::write_volatile(vga.add(pos * 2), c);
                ptr::write_volatile(vga.add(pos * 2 + 1), 0x07);
            }
            pos += 1;
            if pos == 80 * 25 {
                pos = 0;
            }
        }
    }

    EARLY_VGA_POS.store(pos, Ordering::Relaxed);
}

struct EarlyConsole;

impl Write for EarlyConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            early_put_char(c as u8);
        }
        Ok(())
    }
}

// ---- log crate integration ----

struct KernelLogger;

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
