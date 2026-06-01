//! 8250-compatible RS-232 serial ports.
//!
//! The driver owns two RS-232 channels wired to the master PIC:
//!
//! ```text
//! TTY channel 1 ──► COM1 base 0x3f8 ──► IRQ4 / IDT vector 0x24
//! TTY channel 2 ──► COM2 base 0x2f8 ──► IRQ3 / IDT vector 0x23
//! ```
//!
//! Each interrupt loops over the UART interrupt-identification register until
//! no cause remains. Receive interrupts feed raw bytes into the TTY line
//! discipline; transmit-holding-empty interrupts drain the TTY output queue one
//! byte at a time, so the whole transmit path is interrupt-driven.

use core::arch::naked_asm;

use user_lib::syscall::tty::{ControlMode, Termios};

use super::tty::{self, TtyBackend};
use crate::{
    pmio::{inb, inb_p, outb, outb_p},
    trap,
};

/// Backend instance shared by both serial TTY channels.
pub static SERIAL_BACKEND: SerialBackend = SerialBackend;

/// Initialize both serial ports and install their IRQ gates.
pub fn init() {
    trap::set_intr_gate(0x24, serial1_interrupt);
    trap::set_intr_gate(0x23, serial2_interrupt);

    for port in PORTS {
        port.init(&Termios::serial_default());
    }

    // Unmask IRQ3 and IRQ4 on the master PIC.
    outb(inb_p(PIC_MASTER_MASK) & !SERIAL_IRQ_MASK, PIC_MASTER_MASK);
}

/// TTY backend for the serial channels.
pub struct SerialBackend;

/// Build a naked IRQ stub that saves state and calls [`serial_interrupt_entry`]
/// with the given port index.
macro_rules! serial_isr {
    ($name:ident, $index:literal) => {
        #[naked]
        extern "C" fn $name() {
            unsafe {
                naked_asm!(
                    "pushl %eax",
                    "pushl %ebx",
                    "pushl %ecx",
                    "pushl %edx",
                    "push %ds",
                    "push %es",
                    "movl $0x10, %eax",
                    "movw %ax, %ds",
                    "movw %ax, %es",
                    concat!("pushl $", stringify!($index)),
                    "call {entry}",
                    "addl $4, %esp",
                    "pop %es",
                    "pop %ds",
                    "popl %edx",
                    "popl %ecx",
                    "popl %ebx",
                    "popl %eax",
                    "iret",
                    entry = sym serial_interrupt_entry,
                    options(att_syntax),
                );
            }
        }
    };
}

serial_isr!(serial1_interrupt, 0);
serial_isr!(serial2_interrupt, 1);

/// Number of serial ports implemented by this driver.
const PORT_COUNT: usize = 2;

/// TTY channel number of the first serial port.
const FIRST_SERIAL_TTY: usize = 1;

/// Master PIC interrupt mask register.
const PIC_MASTER_MASK: u16 = 0x21;

/// Master PIC command register (used to send end-of-interrupt).
const PIC_MASTER_COMMAND: u16 = 0x20;

/// End-of-interrupt command for the 8259A PIC.
const PIC_EOI: u8 = 0x20;

/// IRQ3 and IRQ4 mask bits in the master PIC interrupt mask register.
const SERIAL_IRQ_MASK: u8 = 0x18;

/// Maximum bytes drained from the receive FIFO per interrupt.
const RX_BATCH: usize = 16;

/// Static serial-port table, indexed by `tty_channel - FIRST_SERIAL_TTY`.
const PORTS: [SerialPort; PORT_COUNT] = [
    SerialPort {
        base: 0x3f8,
        tty_channel: 1,
    },
    SerialPort {
        base: 0x2f8,
        tty_channel: 2,
    },
];

/// At least one byte is available in the receive buffer (line-status bit 0).
const LINE_STATUS_DATA_READY: u8 = 1 << 0;

/// One 8250-compatible UART port.
///
/// ```text
/// base + 0  RBR/THR/DLL  receive buffer, transmit holding, divisor low
/// base + 1  IER/DLM      interrupt enable, divisor high
/// base + 2  IIR/FCR      interrupt identification, FIFO control
/// base + 3  LCR          line control and DLAB
/// base + 4  MCR          modem control
/// base + 5  LSR          line status
/// base + 6  MSR          modem status
/// ```
#[derive(Clone, Copy)]
struct SerialPort {
    /// Base I/O port address of the UART.
    base: u16,
    /// TTY channel this port is bound to.
    tty_channel: usize,
}

/// UART register offsets from the port base address.
#[derive(Clone, Copy)]
#[repr(u16)]
enum Register {
    /// Receive buffer / transmit holding / divisor low.
    Data = 0,
    /// Interrupt enable / divisor high.
    InterruptEnable = 1,
    /// Interrupt identification (read) or FIFO control (write).
    InterruptId = 2,
    /// Line control register.
    LineControl = 3,
    /// Modem control register.
    ModemControl = 4,
    /// Line status register.
    LineStatus = 5,
    /// Modem status register.
    ModemStatus = 6,
}

/// Interrupt cause read from the interrupt-identification register.
enum InterruptCause {
    ModemStatus,
    TransmitEmpty,
    ReceivedData,
    LineStatus,
}

/// Interrupt-enable register bits.
mod interrupt_enable {
    /// Received-data-available interrupt.
    pub const RECEIVED_DATA: u8 = 1 << 0;
    /// Transmit-holding-register-empty interrupt.
    pub const TRANSMIT_EMPTY: u8 = 1 << 1;
    /// Receiver line-status interrupt.
    pub const LINE_STATUS: u8 = 1 << 2;
    /// Modem-status interrupt.
    pub const MODEM_STATUS: u8 = 1 << 3;

    /// All receive-side interrupts enabled together.
    pub const RECEIVE_SET: u8 = RECEIVED_DATA | LINE_STATUS | MODEM_STATUS;
}

/// Line-control register bits.
mod line_control {
    pub const WORD_LEN_5: u8 = 0x00;
    pub const WORD_LEN_6: u8 = 0x01;
    pub const WORD_LEN_7: u8 = 0x02;
    pub const WORD_LEN_8: u8 = 0x03;
    /// Two stop bits for 6/7/8-bit words, or 1.5 stop bits for 5-bit words.
    pub const STOP_BITS: u8 = 1 << 2;
    /// Enable parity generation and checking.
    pub const PARITY_ENABLE: u8 = 1 << 3;
    /// Select even parity when parity is enabled.
    pub const EVEN_PARITY: u8 = 1 << 4;
    /// Divisor latch access bit.
    pub const DIVISOR_LATCH_ACCESS: u8 = 1 << 7;
}

/// Modem-control register bits.
mod modem_control {
    pub const DATA_TERMINAL_READY: u8 = 1 << 0;
    pub const REQUEST_TO_SEND: u8 = 1 << 1;
    /// Auxiliary output 2, required on PC UARTs to route interrupts to the PIC.
    pub const OUT2: u8 = 1 << 3;

    /// Lines asserted for an active, interrupt-routed connection.
    pub const ACTIVE_SET: u8 = DATA_TERMINAL_READY | REQUEST_TO_SEND | OUT2;
}

/// Return the serial port serving `channel`.
fn port_for_channel(channel: usize) -> Option<SerialPort> {
    let index = channel.checked_sub(FIRST_SERIAL_TTY)?;
    PORTS.get(index).copied()
}

/// Return the UART divisor for the baud bits of `control_mode`, or `None` for
/// B0 (which requests a hang-up).
fn baud_divisor(control_mode: ControlMode) -> Option<u16> {
    Some(match control_mode & ControlMode::CBAUD {
        mode if mode.is_empty() => return None,
        ControlMode::B50 => 2304,
        ControlMode::B75 => 1536,
        ControlMode::B110 => 1047,
        ControlMode::B134 => 857,
        ControlMode::B150 => 768,
        ControlMode::B200 => 576,
        ControlMode::B300 => 384,
        ControlMode::B600 => 192,
        ControlMode::B1200 => 96,
        ControlMode::B1800 => 64,
        ControlMode::B2400 => 48,
        ControlMode::B4800 => 24,
        ControlMode::B9600 => 12,
        ControlMode::B19200 => 6,
        ControlMode::B38400 => 3,
        _ => 48,
    })
}

/// Convert termios character-format flags into an 8250 line-control value.
fn line_control_from_termios(control_mode: ControlMode) -> u8 {
    let mut line = match control_mode & ControlMode::CSIZE {
        ControlMode::CS6 => line_control::WORD_LEN_6,
        ControlMode::CS7 => line_control::WORD_LEN_7,
        ControlMode::CS8 => line_control::WORD_LEN_8,
        _ => line_control::WORD_LEN_5,
    };

    if control_mode.contains(ControlMode::CSTOPB) {
        line |= line_control::STOP_BITS;
    }
    if control_mode.contains(ControlMode::CPARENB) {
        line |= line_control::PARITY_ENABLE;
        if !control_mode.contains(ControlMode::CPARODD) {
            line |= line_control::EVEN_PARITY;
        }
    }

    line
}

/// Rust-side dispatcher shared by the COM1 and COM2 interrupt stubs.
extern "C" fn serial_interrupt_entry(index: usize) {
    if let Some(port) = PORTS.get(index) {
        port.handle_interrupt();
    }
    outb(PIC_EOI, PIC_MASTER_COMMAND);
}

impl InterruptCause {
    /// Set when the UART reports no pending interrupt.
    const NONE_PENDING: u8 = 1 << 0;
    /// Mask selecting the cause field of the identification register.
    const CAUSE_MASK: u8 = 0x0e;

    /// Decode the interrupt-identification register, returning `None` when no
    /// interrupt is pending or the cause is unrecognized.
    fn decode(ident: u8) -> Option<Self> {
        if ident & Self::NONE_PENDING != 0 {
            return None;
        }
        match ident & Self::CAUSE_MASK {
            0x00 => Some(Self::ModemStatus),
            0x02 => Some(Self::TransmitEmpty),
            // 0x0c is the FIFO character-timeout interrupt, drained like data.
            0x04 | 0x0c => Some(Self::ReceivedData),
            0x06 => Some(Self::LineStatus),
            _ => None,
        }
    }
}

impl TtyBackend for SerialBackend {
    fn start_output(&self, channel: usize) {
        let Some(port) = port_for_channel(channel) else {
            return;
        };

        if tty::has_pending_output(channel) {
            port.enable_transmit_interrupt();
        } else {
            port.disable_transmit_interrupt();
            tty::notify_output_drained(channel);
        }
    }

    fn configure(&self, channel: usize, termios: &Termios) {
        if let Some(port) = port_for_channel(channel) {
            port.configure(termios);
        }
    }
}

impl SerialPort {
    /// Initialize the UART with the supplied terminal settings.
    fn init(&self, termios: &Termios) {
        self.write(Register::InterruptEnable, 0x00);
        self.write(Register::InterruptId, 0x00);
        self.configure(termios);

        // Clear any stale pending conditions before IRQs are unmasked.
        self.read(Register::Data);
        self.read(Register::LineStatus);
        self.read(Register::ModemStatus);
    }

    /// Apply baud rate, data bits, stop bits, and parity from termios.
    fn configure(&self, termios: &Termios) {
        let Some(divisor) = baud_divisor(termios.control_mode) else {
            // B0: drop the line by disabling interrupts and modem control.
            self.write(Register::InterruptEnable, 0x00);
            self.write(Register::ModemControl, 0x00);
            return;
        };

        let line = line_control_from_termios(termios.control_mode);
        self.write(
            Register::LineControl,
            line_control::DIVISOR_LATCH_ACCESS | line,
        );
        self.write(Register::Data, divisor as u8);
        self.write(Register::InterruptEnable, (divisor >> 8) as u8);
        self.write(Register::LineControl, line);
        self.write(Register::ModemControl, modem_control::ACTIVE_SET);
        self.write(Register::InterruptEnable, interrupt_enable::RECEIVE_SET);
    }

    /// Enable transmit-empty interrupts without disturbing the receive bits.
    fn enable_transmit_interrupt(&self) {
        let enabled = self.read(Register::InterruptEnable);
        self.write(
            Register::InterruptEnable,
            enabled | interrupt_enable::TRANSMIT_EMPTY,
        );
    }

    /// Disable transmit-empty interrupts without disturbing the receive bits.
    fn disable_transmit_interrupt(&self) {
        let enabled = self.read(Register::InterruptEnable);
        self.write(
            Register::InterruptEnable,
            enabled & !interrupt_enable::TRANSMIT_EMPTY,
        );
    }

    /// Service every pending interrupt cause on this UART.
    fn handle_interrupt(&self) {
        while let Some(cause) = InterruptCause::decode(self.read(Register::InterruptId)) {
            match cause {
                // Reading the status register clears the corresponding cause.
                InterruptCause::ModemStatus => {
                    self.read(Register::ModemStatus);
                }
                InterruptCause::LineStatus => {
                    self.read(Register::LineStatus);
                }
                InterruptCause::ReceivedData => self.drain_receive_fifo(),
                InterruptCause::TransmitEmpty => self.transmit_next(),
            }
        }
    }

    /// Drain all currently available received bytes into the TTY layer.
    fn drain_receive_fifo(&self) {
        let mut bytes = [0u8; RX_BATCH];
        let mut count = 0;
        while count < bytes.len() && self.read(Register::LineStatus) & LINE_STATUS_DATA_READY != 0 {
            bytes[count] = self.read(Register::Data);
            count += 1;
        }

        if count > 0 {
            tty::receive_input(self.tty_channel, &bytes[..count]);
        }
    }

    /// Transmit one queued byte, disabling transmit interrupts once the queue
    /// is drained.
    fn transmit_next(&self) {
        let mut byte = [0u8; 1];
        if tty::take_output(self.tty_channel, &mut byte) == 0 {
            self.disable_transmit_interrupt();
        } else {
            self.write(Register::Data, byte[0]);
            if !tty::has_pending_output(self.tty_channel) {
                self.disable_transmit_interrupt();
            }
        }
        tty::notify_output_drained(self.tty_channel);
    }

    /// Read one UART register.
    #[inline]
    fn read(&self, register: Register) -> u8 {
        inb(self.base + register as u16)
    }

    /// Write one UART register with an ISA I/O delay.
    #[inline]
    fn write(&self, register: Register, value: u8) {
        outb_p(value, self.base + register as u16);
    }
}
