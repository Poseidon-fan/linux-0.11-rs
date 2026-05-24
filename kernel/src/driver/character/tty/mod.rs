//! TTY core layer.
//!
//! The TTY core owns the queues, line discipline, termios state, and wait
//! queues for the fixed Linux 0.11-style terminal table:
//!
//! ```text
//!   hardware ISR ──► raw_input ──► line discipline ──► cooked_input ──► read()
//!   write() / printk ──► output ──► backend flush ──► console / serial
//! ```
//!
//! Hardware backends do not access queue fields directly. They feed input with
//! [`receive_input`] and drain output with [`take_output`].

mod line_discipline;
mod ring_buffer;

use ring_buffer::RingBuffer;
use user_lib::syscall::tty::{ControlChar, LocalMode, OutputMode, Termio, Termios, TtyRequest};

use crate::{
    error::{Errno, Result},
    mm,
    segment::uaccess,
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
/// This is the Rust equivalent of the Linux 0.11 `printk` path, which writes
/// formatted kernel output through TTY channel 0 after making the buffer
/// readable as kernel data.
pub fn write(channel: usize, bytes: &[u8]) -> Result<usize> {
    device(channel)?.write(channel, bytes)
}

/// Handle a TTY ioctl request.
pub fn ioctl(channel: usize, cmd: u32, arg: u32) -> Result<u32> {
    device(channel)?.ioctl(channel, cmd, arg)
}

/// Feed hardware input into a TTY and run the line discipline.
pub fn receive_input(channel: usize, bytes: &[u8]) {
    let Ok(tty) = device(channel) else {
        return;
    };
    tty.receive_input(channel, bytes);
}

/// Ask a TTY backend to flush pending output.
pub fn flush_output(channel: usize) {
    if let Ok(tty) = device(channel) {
        (tty.backend_flush)(channel);
    }
}

/// Drain pending output bytes from a TTY into `out`.
pub fn take_output(channel: usize, out: &mut [u8]) -> usize {
    device(channel)
        .map(|tty| tty.take_output(out))
        .unwrap_or_default()
}

/// Wake writers waiting for output queue space.
pub fn wake_output(channel: usize) {
    if let Ok(tty) = device(channel) {
        tty.output_wait.wake();
    }
}

/// Clear the foreground process group for a TTY.
pub fn clear_foreground_group(channel: usize) {
    if let Ok(tty) = device(channel) {
        tty.state
            .exclusive(|state| state.foreground_group = NO_FOREGROUND_GROUP);
    }
}

type FlushOutputFn = fn(usize);

const NO_FOREGROUND_GROUP: i32 = 0;
const WRITE_WAKE_THRESHOLD: usize = 128;
const READ_CANONICAL_LOW_WATER: usize = 20;

const TCFLSH: u32 = 0x540B;
const TIOCOUTQ: u32 = 0x5411;
const TIOCINQ: u32 = 0x541B;

const FLUSH_INPUT: u32 = 0;
const FLUSH_OUTPUT: u32 = 1;
const FLUSH_BOTH: u32 = 2;

static DEVICES: [TtyDevice; DEVICE_COUNT] = [
    TtyDevice::new(Termios::console_default(), super::console::flush_output),
    TtyDevice::new(Termios::serial_default(), nop_flush),
    TtyDevice::new(Termios::serial_default(), nop_flush),
];

struct TtyDevice {
    state: KernelCell<TtyState>,
    backend_flush: FlushOutputFn,
    cooked_wait: WaitQueue,
    output_wait: WaitQueue,
}

struct TtyState {
    termios: Termios,
    foreground_group: i32,
    stopped: bool,
    raw_input: RingBuffer,
    output: RingBuffer,
    cooked_input: RingBuffer,
    pending_lines: usize,
    output_cr_pending: bool,
}

fn nop_flush(_channel: usize) {}

fn device(channel: usize) -> Result<&'static TtyDevice> {
    DEVICES.get(channel).ok_or(Errno::NODEV)
}

fn signal_foreground_group(foreground_group: i32, signal_mask: u32) {
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

impl TtyDevice {
    const fn new(termios: Termios, backend_flush: FlushOutputFn) -> Self {
        Self {
            state: KernelCell::new(TtyState {
                termios,
                foreground_group: NO_FOREGROUND_GROUP,
                stopped: false,
                raw_input: RingBuffer::new(),
                output: RingBuffer::new(),
                cooked_input: RingBuffer::new(),
                pending_lines: 0,
                output_cr_pending: false,
            }),
            backend_flush,
            cooked_wait: WaitQueue::new(),
            output_wait: WaitQueue::new(),
        }
    }

    fn read(&'static self, buffer: &mut [u8]) -> Result<usize> {
        let count = buffer.len();
        if count == 0 {
            return Ok(0);
        }

        let mut written = 0usize;

        loop {
            if task::with_current(|inner| inner.signal_info.signal != 0) {
                break;
            }

            let data_available = self.state.exclusive(|state| state.has_readable_data());
            if !data_available {
                self.cooked_wait.sleep_interruptible();
                continue;
            }

            self.state.exclusive(|state| {
                while written < count {
                    let Some(byte) = state.cooked_input.pop() else {
                        break;
                    };

                    if state.is_line_boundary(byte) && state.pending_lines > 0 {
                        state.pending_lines -= 1;
                    }

                    if byte == state.termios.control_char(ControlChar::Eof) && state.is_canonical()
                    {
                        break;
                    }

                    buffer[written] = byte;
                    written += 1;
                }
            });

            break;
        }

        if written == 0 && task::with_current(|inner| inner.signal_info.signal != 0) {
            return Err(Errno::INTR);
        }

        Ok(written)
    }

    fn write(&'static self, channel: usize, buffer: &[u8]) -> Result<usize> {
        let mut sent = 0usize;
        let count = buffer.len();

        while sent < count {
            if task::with_current(|inner| inner.signal_info.signal != 0) {
                break;
            }

            if self.state.exclusive(|state| state.output.is_full()) {
                flush_output(channel);

                if self
                    .state
                    .exclusive(|state| state.output.remaining() < WRITE_WAKE_THRESHOLD)
                {
                    self.output_wait.sleep_interruptible();
                    continue;
                }
            }

            self.state.exclusive(|state| {
                while sent < count && !state.output.is_full() {
                    let mut byte = buffer[sent];

                    if state.termios.output_mode.contains(OutputMode::OPOST) {
                        byte = state.map_output_byte(byte);
                        if state.should_insert_output_cr(byte) {
                            state.output_cr_pending = true;
                            state.output.push(b'\r');
                            continue;
                        }
                    }

                    state.output_cr_pending = false;
                    state.output.push(byte);
                    sent += 1;
                }
            });

            flush_output(channel);

            if sent < count {
                task::schedule();
            }
        }

        Ok(sent)
    }

    fn ioctl(&'static self, channel: usize, cmd: u32, arg: u32) -> Result<u32> {
        match cmd {
            x if x == TtyRequest::GetTermios as u32 => self.get_termios_to_user(arg),
            x if x == TtyRequest::SetTermiosFlush as u32 => {
                self.state.exclusive(TtyState::flush_input);
                flush_output(channel);
                self.set_termios_from_user(arg)
            }
            x if x == TtyRequest::SetTermiosWait as u32 => {
                flush_output(channel);
                self.set_termios_from_user(arg)
            }
            x if x == TtyRequest::SetTermios as u32 => self.set_termios_from_user(arg),
            x if x == TtyRequest::GetTermio as u32 => self.get_termio_to_user(arg),
            x if x == TtyRequest::SetTermioFlush as u32 => {
                self.state.exclusive(TtyState::flush_input);
                flush_output(channel);
                self.set_termio_from_user(arg)
            }
            x if x == TtyRequest::SetTermioWait as u32 => {
                flush_output(channel);
                self.set_termio_from_user(arg)
            }
            x if x == TtyRequest::SetTermio as u32 => self.set_termio_from_user(arg),
            x if x == TtyRequest::GetPgrp as u32 => {
                let pgrp = self.state.exclusive(|state| state.foreground_group);
                mm::ensure_user_area_writable(arg, core::mem::size_of::<u32>());
                uaccess::write_u32(pgrp as u32, arg as *mut u32);
                Ok(0)
            }
            x if x == TtyRequest::SetPgrp as u32 => {
                let pgrp = uaccess::read_u32(arg as *const u32);
                self.state
                    .exclusive(|state| state.foreground_group = pgrp as i32);
                Ok(0)
            }
            TCFLSH => self.flush_for_ioctl(arg),
            TIOCOUTQ => {
                let count = self.state.exclusive(|state| state.output.len());
                mm::ensure_user_area_writable(arg, core::mem::size_of::<u32>());
                uaccess::write_u32(count as u32, arg as *mut u32);
                Ok(0)
            }
            TIOCINQ => {
                let count = self.state.exclusive(|state| state.cooked_input.len());
                mm::ensure_user_area_writable(arg, core::mem::size_of::<u32>());
                uaccess::write_u32(count as u32, arg as *mut u32);
                Ok(0)
            }
            _ => Err(Errno::INVAL),
        }
    }

    fn receive_input(&'static self, channel: usize, bytes: &[u8]) {
        let has_echo = self.state.exclusive(|state| {
            for &byte in bytes {
                let _ = state.raw_input.push(byte);
            }
            line_discipline::process_raw_input(state)
        });

        self.cooked_wait.wake();

        if has_echo {
            flush_output(channel);
        }
    }

    fn take_output(&'static self, out: &mut [u8]) -> usize {
        self.state.exclusive(|state| {
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

    fn get_termios_to_user(&'static self, user_ptr: u32) -> Result<u32> {
        let termios = self.state.exclusive(|state| state.termios);
        mm::ensure_user_area_writable(user_ptr, core::mem::size_of::<Termios>());
        uaccess::write_struct(&termios, user_ptr as *mut Termios);
        Ok(0)
    }

    fn set_termios_from_user(&'static self, user_ptr: u32) -> Result<u32> {
        let termios = uaccess::read_struct(user_ptr as *const Termios);
        self.state.exclusive(|state| state.termios = termios);
        Ok(0)
    }

    fn get_termio_to_user(&'static self, user_ptr: u32) -> Result<u32> {
        let termio = self.state.exclusive(|state| state.termios.to_termio());
        mm::ensure_user_area_writable(user_ptr, core::mem::size_of::<Termio>());
        uaccess::write_struct(&termio, user_ptr as *mut Termio);
        Ok(0)
    }

    fn set_termio_from_user(&'static self, user_ptr: u32) -> Result<u32> {
        let termio = uaccess::read_struct(user_ptr as *const Termio);
        self.state
            .exclusive(|state| state.termios.apply_termio(termio));
        Ok(0)
    }

    fn flush_for_ioctl(&'static self, arg: u32) -> Result<u32> {
        match arg {
            FLUSH_INPUT => self.state.exclusive(TtyState::flush_input),
            FLUSH_OUTPUT => self.state.exclusive(TtyState::flush_output),
            FLUSH_BOTH => self.state.exclusive(|state| {
                state.flush_input();
                state.flush_output();
            }),
            _ => return Err(Errno::INVAL),
        }
        Ok(0)
    }
}

impl TtyState {
    fn is_canonical(&self) -> bool {
        self.termios.local_mode.contains(LocalMode::ICANON)
    }

    fn has_readable_data(&self) -> bool {
        if self.is_canonical() {
            self.pending_lines > 0 || self.cooked_input.remaining() <= READ_CANONICAL_LOW_WATER
        } else {
            !self.cooked_input.is_empty()
        }
    }

    fn is_line_boundary(&self, byte: u8) -> bool {
        byte == b'\n' || byte == self.termios.control_char(ControlChar::Eof)
    }

    fn map_output_byte(&self, byte: u8) -> u8 {
        if byte == b'\r' && self.termios.output_mode.contains(OutputMode::OCRNL) {
            return b'\n';
        }
        if byte == b'\n' && self.termios.output_mode.contains(OutputMode::ONLRET) {
            return b'\r';
        }
        if self.termios.output_mode.contains(OutputMode::OLCUC) && byte.is_ascii_lowercase() {
            return byte.to_ascii_uppercase();
        }
        byte
    }

    fn should_insert_output_cr(&self, byte: u8) -> bool {
        byte == b'\n'
            && self.termios.output_mode.contains(OutputMode::ONLCR)
            && !self.output_cr_pending
    }

    fn flush_input(&mut self) {
        self.raw_input.flush();
    }

    fn flush_output(&mut self) {
        self.output.flush();
        self.output_cr_pending = false;
    }
}
