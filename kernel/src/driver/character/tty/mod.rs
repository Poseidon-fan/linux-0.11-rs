//! TTY core layer.
//!
//! The core owns the queues, line discipline, `termios` state, and wait queues
//! for a fixed table of terminal devices. Data moves through it like this:
//!
//! ```text
//!   hardware ISR ──► raw_input ──► line discipline ──► cooked_input ──► read()
//!   write() / printk ──► output ──► backend ──► console / serial hardware
//! ```
//!
//! Hardware is abstracted behind the [`TtyBackend`] trait: the core never
//! touches device registers, and backends never touch queue internals. A
//! backend pulls bytes with [`TtyDevice::take_output`] and pushes received
//! bytes with [`TtyDevice::receive_input`].

mod ioctl;
mod line_discipline;
mod ring_buffer;

use ring_buffer::RingBuffer;
use user_lib::syscall::tty::{ControlChar, LocalMode, OutputMode, Termios};

use super::{console, serial};
use crate::{
    error::{Errno, Result},
    sync::KernelCell,
    task::{self, WaitQueue},
};

/// Number of fixed TTY devices: console, serial port 1, and serial port 2.
pub const DEVICE_COUNT: usize = 3;

/// Read cooked input from a TTY into a kernel-owned buffer.
pub fn read(channel: usize, buffer: &mut [u8]) -> Result<usize> {
    device(channel)?.read(buffer)
}

/// Write kernel-owned bytes to a TTY.
///
/// This is also the kernel log output path: formatted kernel output is written
/// to channel 0 as ordinary kernel data.
pub fn write(channel: usize, bytes: &[u8]) -> Result<usize> {
    device(channel)?.write(channel, bytes)
}

/// Handle a terminal ioctl request.
pub fn ioctl(channel: usize, request: u32, arg: u32) -> Result<u32> {
    device(channel)?.ioctl(channel, request, arg)
}

/// Feed hardware input into a TTY and run the line discipline.
pub fn receive_input(channel: usize, bytes: &[u8]) {
    if let Ok(tty) = device(channel) {
        tty.receive_input(channel, bytes);
    }
}

/// Drain pending output bytes from a TTY into `out`, returning the count moved.
pub fn take_output(channel: usize, out: &mut [u8]) -> usize {
    device(channel).map_or(0, |tty| tty.take_output(out))
}

/// Wake a blocked writer once enough output-queue space has freed up.
///
/// Backends call this after draining bytes. Gating on the wake threshold
/// avoids waking a writer on every transmitted byte only for it to find the
/// queue still nearly full and sleep again.
pub fn notify_output_drained(channel: usize) {
    if let Ok(tty) = device(channel) {
        if tty
            .state
            .exclusive(|state| state.output.remaining() >= WRITE_WAKE_THRESHOLD)
        {
            tty.output_wait.wake();
        }
    }
}

/// Whether a TTY has output that is ready to transmit (queued and not
/// flow-control stopped).
pub fn has_pending_output(channel: usize) -> bool {
    device(channel).is_ok_and(|tty| {
        tty.state
            .exclusive(|state| !state.stopped && !state.output.is_empty())
    })
}

/// Clear the foreground process group for a TTY (e.g. when its session leader
/// exits).
pub fn clear_foreground_group(channel: usize) {
    if let Ok(tty) = device(channel) {
        tty.state
            .exclusive(|state| state.foreground_group = NO_FOREGROUND_GROUP);
    }
}

/// Hardware backend driving one TTY channel.
///
/// The core calls these on its own thread of control; implementations must not
/// assume any particular lock is held. Output draining goes through
/// [`TtyDevice::take_output`], so backends only react to "there is now work"
/// ([`start_output`]) and configuration changes ([`configure`]).
///
/// [`start_output`]: TtyBackend::start_output
/// [`configure`]: TtyBackend::configure
pub trait TtyBackend: Sync {
    /// Begin or resume draining the channel's output queue.
    fn start_output(&self, channel: usize);

    /// Reconfigure the hardware from updated `termios` state.
    ///
    /// The default does nothing, which suits backends like the console whose
    /// line settings are fixed.
    fn configure(&self, channel: usize, termios: &Termios) {
        let _ = (channel, termios);
    }
}

static DEVICES: [TtyDevice; DEVICE_COUNT] = [
    TtyDevice::new(Termios::console_default(), &console::CONSOLE_BACKEND),
    TtyDevice::new(Termios::serial_default(), &serial::SERIAL_BACKEND),
    TtyDevice::new(Termios::serial_default(), &serial::SERIAL_BACKEND),
];

/// Sentinel foreground process group meaning "no controlling group".
const NO_FOREGROUND_GROUP: i32 = 0;

/// Free output-queue space, in bytes, below which a blocked writer is woken.
const WRITE_WAKE_THRESHOLD: usize = 128;

/// In canonical mode a read may also return once cooked input drops within this
/// many bytes of the buffer limit, so a pathologically long line still makes
/// progress instead of deadlocking on a full queue.
const READ_CANONICAL_LOW_WATER: usize = 20;

/// One TTY device: queues, configuration, backend, and waiters.
struct TtyDevice {
    /// Mutable TTY state behind an IRQ-masked cell.
    state: KernelCell<TtyState>,
    /// Hardware backend driving this channel.
    backend: &'static dyn TtyBackend,
    /// Readers waiting for cooked input.
    cooked_wait: WaitQueue,
    /// Writers waiting for output-queue space.
    output_wait: WaitQueue,
}

/// Mutable per-device TTY state.
struct TtyState {
    /// Terminal configuration.
    termios: Termios,
    /// Foreground process group receiving terminal-generated signals.
    foreground_group: i32,
    /// Whether output is currently suspended by XOFF flow control.
    stopped: bool,
    /// Raw bytes received from hardware, awaiting the line discipline.
    raw_input: RingBuffer,
    /// Line-discipline output ready to be read.
    cooked_input: RingBuffer,
    /// Bytes queued for transmission to the backend.
    output: RingBuffer,
    /// Number of complete lines available in canonical mode.
    pending_lines: usize,
    /// Whether a carriage return was already emitted for the pending newline.
    output_cr_pending: bool,
}

/// Look up a TTY device by channel number.
fn device(channel: usize) -> Result<&'static TtyDevice> {
    DEVICES.get(channel).ok_or(Errno::NODEV)
}

/// Whether the current task has a pending signal.
fn signal_pending() -> bool {
    task::with_current(|inner| inner.signal_info.signal != 0)
}

/// Post `signal` to every task in the given foreground process group.
fn signal_foreground_group(foreground_group: i32, signal: u32) {
    let Ok(pgrp) = u32::try_from(foreground_group) else {
        return;
    };
    if pgrp == NO_FOREGROUND_GROUP as u32 {
        return;
    }

    task::TASK_MANAGER.exclusive(|manager| {
        for task in manager.tasks.iter().flatten() {
            task.pcb.inner.exclusive(|inner| {
                if inner.relation.pgrp == pgrp {
                    inner.signal_info.raise(signal);
                    inner.sched.wake_if_interruptible();
                }
            });
        }
    });
}

impl TtyDevice {
    /// Build a TTY device with empty queues and the given configuration.
    const fn new(termios: Termios, backend: &'static dyn TtyBackend) -> Self {
        Self {
            state: KernelCell::new(TtyState {
                termios,
                foreground_group: NO_FOREGROUND_GROUP,
                stopped: false,
                raw_input: RingBuffer::new(),
                cooked_input: RingBuffer::new(),
                output: RingBuffer::new(),
                pending_lines: 0,
                output_cr_pending: false,
            }),
            backend,
            cooked_wait: WaitQueue::new(),
            output_wait: WaitQueue::new(),
        }
    }

    /// Read cooked input into `buffer`, blocking until data or a signal arrives.
    fn read(&'static self, buffer: &mut [u8]) -> Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }

        let mut written = 0;
        while !signal_pending() {
            if !self.state.exclusive(|state| state.has_readable_data()) {
                self.cooked_wait.sleep_interruptible();
                continue;
            }

            written = self.state.exclusive(|state| state.drain_cooked(buffer));
            break;
        }

        if written == 0 && signal_pending() {
            return Err(Errno::INTR);
        }
        Ok(written)
    }

    /// Write `buffer` to the output queue, applying output post-processing, and
    /// kick the backend to drain it. Blocks while the queue stays full.
    fn write(&'static self, channel: usize, buffer: &[u8]) -> Result<usize> {
        let mut sent = 0;
        while sent < buffer.len() {
            if signal_pending() {
                break;
            }

            if self.state.exclusive(|state| state.output.is_full()) {
                self.backend.start_output(channel);
                if self
                    .state
                    .exclusive(|state| state.output.remaining() < WRITE_WAKE_THRESHOLD)
                {
                    self.output_wait.sleep_interruptible();
                    continue;
                }
            }

            sent += self
                .state
                .exclusive(|state| state.fill_output(&buffer[sent..]));
            self.backend.start_output(channel);

            if sent < buffer.len() {
                task::schedule();
            }
        }

        Ok(sent)
    }

    /// Handle a terminal ioctl request for this device.
    fn ioctl(&'static self, channel: usize, request: u32, arg: u32) -> Result<u32> {
        ioctl::dispatch(self, channel, request, arg)
    }

    /// Push raw hardware bytes through the line discipline, waking readers and
    /// flushing any echoed output.
    fn receive_input(&'static self, channel: usize, bytes: &[u8]) {
        let echoed = self.state.exclusive(|state| {
            for &byte in bytes {
                state.raw_input.push(byte);
            }
            line_discipline::process_raw_input(state)
        });

        self.cooked_wait.wake();
        if echoed {
            self.backend.start_output(channel);
        }
    }

    /// Pop queued output bytes into `out`, returning the count drained.
    ///
    /// Yields nothing while flow-control-stopped so XOFF actually suspends the
    /// transmitter.
    fn take_output(&'static self, out: &mut [u8]) -> usize {
        self.state.exclusive(|state| {
            if state.stopped {
                return 0;
            }
            let mut count = 0;
            while count < out.len() {
                let Some(byte) = state.output.pop() else {
                    break;
                };
                out[count] = byte;
                count += 1;
            }
            count
        })
    }

    /// Reconfigure the backend from the current `termios` state.
    fn reconfigure(&'static self, channel: usize) {
        let termios = self.state.exclusive(|state| state.termios);
        self.backend.configure(channel, &termios);
    }
}

impl TtyState {
    /// Whether the terminal is in canonical (line-buffered) mode.
    fn is_canonical(&self) -> bool {
        self.termios.local_mode.contains(LocalMode::ICANON)
    }

    /// Whether a read can return data without blocking.
    fn has_readable_data(&self) -> bool {
        if self.is_canonical() {
            self.pending_lines > 0 || self.cooked_input.remaining() <= READ_CANONICAL_LOW_WATER
        } else {
            !self.cooked_input.is_empty()
        }
    }

    /// Whether `byte` terminates a canonical-mode line.
    fn is_line_boundary(&self, byte: u8) -> bool {
        byte == b'\n' || byte == self.termios.control_char(ControlChar::Eof)
    }

    /// Copy as much cooked input as fits into `buffer`, returning the byte count.
    ///
    /// In canonical mode the EOF character terminates the read without being
    /// copied, and crossing a line boundary decrements the pending-line count.
    fn drain_cooked(&mut self, buffer: &mut [u8]) -> usize {
        let eof = self.termios.control_char(ControlChar::Eof);
        let canonical = self.is_canonical();

        let mut written = 0;
        while written < buffer.len() {
            let Some(byte) = self.cooked_input.pop() else {
                break;
            };

            if self.is_line_boundary(byte) && self.pending_lines > 0 {
                self.pending_lines -= 1;
            }
            if canonical && byte == eof {
                break;
            }

            buffer[written] = byte;
            written += 1;
        }
        written
    }

    /// Apply output post-processing and enqueue as many bytes of `buffer` as
    /// fit, returning how many input bytes were consumed.
    ///
    /// A newline expanded to CR+LF may consume an output slot for the inserted
    /// carriage return without advancing the input cursor; that partial state
    /// is remembered in [`output_cr_pending`](Self::output_cr_pending) so the
    /// expansion survives a queue-full boundary.
    fn fill_output(&mut self, buffer: &[u8]) -> usize {
        let post_process = self.termios.output_mode.contains(OutputMode::OPOST);

        let mut consumed = 0;
        while consumed < buffer.len() && !self.output.is_full() {
            let mut byte = buffer[consumed];
            if post_process {
                byte = self.map_output_byte(byte);
                if self.should_insert_output_cr(byte) {
                    self.output_cr_pending = true;
                    self.output.push(b'\r');
                    continue;
                }
            }
            self.output_cr_pending = false;
            self.output.push(byte);
            consumed += 1;
        }
        consumed
    }

    /// Apply output-mode character translations to one byte.
    fn map_output_byte(&self, byte: u8) -> u8 {
        let output_mode = self.termios.output_mode;
        match byte {
            b'\r' if output_mode.contains(OutputMode::OCRNL) => b'\n',
            b'\n' if output_mode.contains(OutputMode::ONLRET) => b'\r',
            _ if output_mode.contains(OutputMode::OLCUC) => byte.to_ascii_uppercase(),
            _ => byte,
        }
    }

    /// Whether a carriage return must be inserted before `byte` (ONLCR).
    fn should_insert_output_cr(&self, byte: u8) -> bool {
        byte == b'\n'
            && self.termios.output_mode.contains(OutputMode::ONLCR)
            && !self.output_cr_pending
    }

    /// Discard all queued raw and cooked input.
    fn flush_input(&mut self) {
        self.raw_input.clear();
        self.cooked_input.clear();
        self.pending_lines = 0;
    }

    /// Discard all queued output.
    fn flush_output(&mut self) {
        self.output.clear();
        self.output_cr_pending = false;
    }
}
