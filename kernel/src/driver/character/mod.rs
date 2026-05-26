//! Character device drivers.
//!
//! - [`console`] — VGA text-mode display + PS/2 keyboard (TTY channel 0 backend).
//! - [`serial`] — 8250-compatible RS-232 ports (TTY channels 1 and 2).
//! - [`tty`] — terminal core layer with line discipline, ring buffers, and
//!   `termios` configuration.

pub mod console;
pub mod serial;
pub mod tty;
