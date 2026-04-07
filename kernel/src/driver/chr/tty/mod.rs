//! TTY core layer.
//!
//! Provides the central TTY abstraction that sits between hardware backends
//! (console, serial) and user-space read/write system calls. Each device is
//! represented by a [`Tty`] slot in the fixed-size [`DEVICES`] table.
//!
//! Three ring buffers per device carry data through the pipeline:
//!
//! ```text
//!   Hardware ISR ──► raw_rx ──► LineDiscipline ──► cooked_rx ──► user read
//!   user write  ──► tx ──► flush_output() ──► hardware
//! ```

pub mod line_discipline;
pub mod ring_buffer;

use user_lib::termios::*;

use crate::{
    segment::uaccess,
    sync::KernelCell,
    syscall::*,
    task::{self, wait_queue::WaitQueue},
};

use ring_buffer::RingBuffer;

/// Signature of the backend flush callback. Receives the channel index so the
/// backend can locate the correct [`Tty`] in [`DEVICES`] and drain `tx`.
pub type FlushOutputFn = fn(usize);

/// Per-device mutable state, protected by [`KernelCell`].
pub struct TtyState {
    pub termios: Termios,
    /// Process group that receives signals (SIGINT, SIGQUIT, etc.).
    pub foreground_group: i32,
    /// Set by XOFF (STOP_CHAR), cleared by XON (START_CHAR).
    pub stopped: bool,

    /// Raw bytes received from hardware, not yet processed by the line discipline.
    pub raw_rx: RingBuffer,
    /// Bytes ready for transmission to the output device.
    pub tx: RingBuffer,
    /// Processed input bytes ready for user-space reads.
    pub cooked_rx: RingBuffer,

    /// Number of complete lines in `cooked_rx` (canonical mode).
    pub pending_lines: usize,
    /// Per-device flag for deferred CR insertion during NL → CR+NL expansion.
    pub output_cr_pending: bool,
}

/// A single TTY device slot.
pub struct Tty {
    pub state: KernelCell<TtyState>,
    /// Backend flush callback — NOT inside KernelCell so backends can
    /// re-acquire `state` to drain `tx` without nesting borrows.
    pub flush_output: FlushOutputFn,
    /// Readers block here when `cooked_rx` is empty.
    pub cooked_wait: WaitQueue,
    /// Writers block here when `tx` is full.
    pub output_wait: WaitQueue,
}

fn nop_flush(_channel: usize) {}

pub const DEVICE_COUNT: usize = 3;

/// Fixed device table: channel 0 = console, channels 1–2 = serial ports.
static DEVICES: [Tty; DEVICE_COUNT] = [
    Tty::new(Termios::console_default(), super::console::flush_output),
    Tty::new(Termios::serial_default(), nop_flush),
    Tty::new(Termios::serial_default(), nop_flush),
];

impl Tty {
    pub const DEVICE_COUNT: usize = DEVICE_COUNT;

    const fn new(termios: Termios, flush: FlushOutputFn) -> Self {
        Self {
            state: KernelCell::new(TtyState {
                termios,
                foreground_group: 0,
                stopped: false,
                raw_rx: RingBuffer::new(),
                tx: RingBuffer::new(),
                cooked_rx: RingBuffer::new(),
                pending_lines: 0,
                output_cr_pending: false,
            }),
            flush_output: flush,
            cooked_wait: WaitQueue::new(),
            output_wait: WaitQueue::new(),
        }
    }

    /// Get a device reference by channel index.
    #[inline]
    pub fn device(channel: usize) -> &'static Tty {
        &DEVICES[channel]
    }

    /// Called from ISR context after hardware has pushed raw bytes into `raw_rx`.
    /// Runs the line discipline, then flushes any echo output to the backend.
    pub fn on_interrupt(&'static self, channel: usize) {
        let has_echo = self
            .state
            .exclusive(line_discipline::LineDiscipline::process_raw_input);

        WaitQueue::wake_up(&self.cooked_wait);

        if has_echo {
            (self.flush_output)(channel);
        }
    }

    /// Send `signal_mask` to every process whose pgrp matches `foreground_group`.
    pub fn signal_foreground_group(foreground_group: i32, signal_mask: u32) {
        if foreground_group <= 0 {
            return;
        }
        let pgrp = foreground_group as u32;
        task::TASK_MANAGER.exclusive(|manager| {
            for slot in manager.tasks.iter().flatten() {
                slot.pcb.inner.exclusive(|inner| {
                    if inner.relation.pgrp == pgrp {
                        inner.signal_info.signal |= signal_mask;
                    }
                });
            }
        });
    }

    /// Read cooked input into a user-space buffer.
    ///
    /// In canonical mode, blocks until a complete line is available.
    /// In non-canonical mode, honours VMIN / VTIME semantics.
    pub fn read(
        &'static self,
        _channel: usize,
        user_buf: *mut u8,
        count: usize,
    ) -> Result<u32, u32> {
        if count == 0 {
            return Ok(0);
        }

        let mut written = 0usize;

        loop {
            let has_signal = task::current_task()
                .pcb
                .inner
                .exclusive(|inner| inner.signal_info.signal != 0);
            if has_signal {
                break;
            }

            let data_available = self.state.exclusive(|state| {
                let canonical = state.termios.local_mode.contains(LocalMode::ICANON);

                if canonical {
                    state.pending_lines > 0 || state.cooked_rx.remaining() <= 20
                } else {
                    !state.cooked_rx.is_empty()
                }
            });

            if !data_available {
                WaitQueue::interruptible_sleep_on(&self.cooked_wait);
                continue;
            }

            // Drain cooked_rx into user buffer.
            self.state.exclusive(|state| {
                let canonical = state.termios.local_mode.contains(LocalMode::ICANON);
                let eof_char = state.termios.control_char(VEOF);

                while written < count {
                    let Some(c) = state.cooked_rx.pop() else {
                        break;
                    };

                    if (c == 10 || c == eof_char) && state.pending_lines > 0 {
                        state.pending_lines -= 1;
                    }

                    if c == eof_char && canonical {
                        // EOF itself is not delivered to user space.
                        break;
                    }

                    uaccess::write_u8(c, unsafe { user_buf.add(written) });
                    written += 1;
                }
            });

            break;
        }

        if written == 0 {
            let has_signal = task::current_task()
                .pcb
                .inner
                .exclusive(|inner| inner.signal_info.signal != 0);
            if has_signal {
                return Err(EINTR);
            }
        }

        Ok(written as u32)
    }

    /// Write user data through output processing to `tx`, then flush.
    pub fn write(
        &'static self,
        channel: usize,
        user_buf: *const u8,
        count: usize,
    ) -> Result<u32, u32> {
        let mut sent = 0usize;

        while sent < count {
            let has_signal = task::current_task()
                .pcb
                .inner
                .exclusive(|inner| inner.signal_info.signal != 0);
            if has_signal {
                break;
            }

            // Wait until tx has space.
            let is_full = self.state.exclusive(|state| state.tx.is_full());
            if is_full {
                (self.flush_output)(channel);

                let still_full = self.state.exclusive(|state| state.tx.remaining() < 128);
                if still_full {
                    WaitQueue::interruptible_sleep_on(&self.output_wait);
                    continue;
                }
            }

            // Fill tx with output-processed bytes.
            self.state.exclusive(|state| {
                let do_opost = state.termios.output_mode.contains(OutputMode::OPOST);
                let do_nlcr = state.termios.output_mode.contains(OutputMode::ONLCR);
                let do_crnl = state.termios.output_mode.contains(OutputMode::OCRNL);
                let do_nlret = state.termios.output_mode.contains(OutputMode::ONLRET);
                let do_lcuc = state.termios.output_mode.contains(OutputMode::OLCUC);

                while sent < count && !state.tx.is_full() {
                    let c = uaccess::read_u8(unsafe { user_buf.add(sent) });
                    let mut c = c as char;

                    if do_opost {
                        if c == '\r' && do_crnl {
                            c = '\n';
                        } else if c == '\n' && do_nlret {
                            c = '\r';
                        }

                        if c == '\n' && do_nlcr && !state.output_cr_pending {
                            state.output_cr_pending = true;
                            state.tx.push(b'\r');
                            continue;
                        }

                        if do_lcuc && c.is_ascii_lowercase() {
                            c = c.to_ascii_uppercase();
                        }
                    }

                    state.output_cr_pending = false;
                    state.tx.push(c as u8);
                    sent += 1;
                }
            });

            (self.flush_output)(channel);

            if sent < count {
                task::schedule();
            }
        }

        Ok(sent as u32)
    }
}
