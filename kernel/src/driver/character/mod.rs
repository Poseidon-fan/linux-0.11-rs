//! Character device drivers.
//!
//! - [`console`] — VGA text-mode display + PS/2 keyboard (TTY channel 0 backend).
//! - [`tty`] — terminal core layer with line discipline, ring buffers, and
//!   `termios` configuration.

pub mod console;
pub mod tty;
