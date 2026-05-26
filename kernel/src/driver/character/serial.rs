//! 8250-compatible RS-232 serial ports.
//!
//! The serial driver owns the two Linux 0.11-style RS-232 channels:
//!
//! ```text
//! TTY channel 1 ──► COM1 base 0x3f8 ──► IRQ4 / IDT vector 0x24
//! TTY channel 2 ──► COM2 base 0x2f8 ──► IRQ3 / IDT vector 0x23
//! ```
//!
//! Each interrupt loops over the UART interrupt-identification register until
//! the device reports no pending cause. Receive interrupts feed raw bytes into
//! the TTY line discipline, and transmit-empty interrupts drain the TTY output
//! queue one byte at a time, matching the original interrupt-driven design.

use core::arch::naked_asm;

use user_lib::syscall::tty::{ControlMode, Termios};

use super::tty;
use crate::{
    pmio::{inb, inb_p, outb, outb_p},
    trap,
};

/// Number of serial ports implemented by this driver.
const PORT_COUNT: usize = 2;

/// TTY channel number for the first serial port.
const FIRST_SERIAL_TTY: usize = 1;

/// Master PIC interrupt mask register.
const PIC_MASTER_MASK: u16 = 0x21;

/// End-of-interrupt command value for the 8259A PIC.
const PIC_EOI: u8 = 0x20;

/// IRQ3 and IRQ4 mask bits in the master PIC interrupt mask register.
const SERIAL_IRQ_MASK: u8 = 0x18;

/// COM1 hardware descriptor.
const COM1: SerialPort = SerialPort {
    base: 0x3f8,
    tty_channel: 1,
};

/// COM2 hardware descriptor.
const COM2: SerialPort = SerialPort {
    base: 0x2f8,
    tty_channel: 2,
};

/// Static serial-port table indexed by `tty_channel - FIRST_SERIAL_TTY`.
const PORTS: [SerialPort; PORT_COUNT] = [COM1, COM2];

/// Initialize both serial ports and install IRQ gates.
pub fn init() {
    trap::set_intr_gate(0x24, serial1_interrupt);
    trap::set_intr_gate(0x23, serial2_interrupt);

    for port in PORTS {
        port.init(Termios::serial_default());
    }

    // Unmask IRQ3 and IRQ4 on the master PIC.
    outb(inb_p(PIC_MASTER_MASK) & !SERIAL_IRQ_MASK, PIC_MASTER_MASK);
}

/// TTY backend flush callback for serial channels.
///
/// The TTY core has already queued bytes in the channel output ring. Enabling
/// the UART transmit-holding-register-empty interrupt starts or resumes the
/// hardware-driven drain path.
pub fn flush_output(channel: usize) {
    let Some(port) = port_for_channel(channel) else {
        return;
    };

    if tty::has_output(channel) {
        port.enable_transmit_interrupt();
    } else {
        port.disable_transmit_interrupt();
        tty::wake_output(channel);
    }
}

/// Reconfigure a serial port from the current TTY termios state.
pub fn configure(channel: usize, termios: Termios) {
    let Some(port) = port_for_channel(channel) else {
        return;
    };

    port.configure(termios);
}

/// Return the serial port serving `channel`.
fn port_for_channel(channel: usize) -> Option<SerialPort> {
    let index = channel.checked_sub(FIRST_SERIAL_TTY)?;
    PORTS.get(index).copied()
}

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
/// base + 7  SCR          scratch
/// ```
#[derive(Clone, Copy)]
struct SerialPort {
    base: u16,
    tty_channel: usize,
}

/// UART register offsets from the port base address.
#[repr(u16)]
enum Register {
    Data = 0,
    InterruptEnable = 1,
    InterruptIdentifyOrFifoControl = 2,
    LineControl = 3,
    ModemControl = 4,
    LineStatus = 5,
    ModemStatus = 6,
}

/// Interrupt-enable register bits.
mod interrupt_enable {
    /// Received data available interrupt.
    pub const RECEIVED_DATA_AVAILABLE: u8 = 1 << 0;
    /// Transmit holding register empty interrupt.
    pub const TRANSMIT_EMPTY: u8 = 1 << 1;
    /// Receiver line status interrupt.
    pub const LINE_STATUS: u8 = 1 << 2;
    /// Modem status interrupt.
    pub const MODEM_STATUS: u8 = 1 << 3;
}

/// Interrupt-identification register bits and cause values.
mod interrupt_identify {
    /// Set when the UART has no pending interrupt.
    pub const NO_INTERRUPT_PENDING: u8 = 1 << 0;
    /// Mask selecting the interrupt cause.
    pub const CAUSE_MASK: u8 = 0x0e;
    /// Modem-status change interrupt.
    pub const MODEM_STATUS: u8 = 0x00;
    /// Transmit holding register empty interrupt.
    pub const TRANSMIT_EMPTY: u8 = 0x02;
    /// Received data available interrupt.
    pub const RECEIVED_DATA_AVAILABLE: u8 = 0x04;
    /// Receiver line-status interrupt.
    pub const LINE_STATUS: u8 = 0x06;
    /// Character timeout interrupt used by FIFO-capable UARTs.
    pub const CHARACTER_TIMEOUT: u8 = 0x0c;
}

/// Line-control register bits.
mod line_control {
    /// Five data bits per character.
    pub const WORD_LEN_5: u8 = 0x00;
    /// Six data bits per character.
    pub const WORD_LEN_6: u8 = 0x01;
    /// Seven data bits per character.
    pub const WORD_LEN_7: u8 = 0x02;
    /// Eight data bits per character.
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
    /// Data terminal ready.
    pub const DATA_TERMINAL_READY: u8 = 1 << 0;
    /// Request to send.
    pub const REQUEST_TO_SEND: u8 = 1 << 1;
    /// Auxiliary output 2, required on PC UARTs to route interrupts to the PIC.
    pub const OUT2: u8 = 1 << 3;
}

/// Line-status register bits.
mod line_status {
    /// At least one byte is available in the receive buffer.
    pub const DATA_READY: u8 = 1 << 0;
}

impl SerialPort {
    /// Initialize one UART with the supplied terminal settings.
    fn init(self, termios: Termios) {
        self.write(Register::InterruptEnable, 0x00);
        self.write(Register::InterruptIdentifyOrFifoControl, 0x00);
        self.configure(termios);
        self.write(
            Register::InterruptEnable,
            interrupt_enable::RECEIVED_DATA_AVAILABLE
                | interrupt_enable::LINE_STATUS
                | interrupt_enable::MODEM_STATUS,
        );

        // Clear any stale pending UART conditions before IRQs are unmasked.
        let _ = self.read(Register::Data);
        let _ = self.read(Register::LineStatus);
        let _ = self.read(Register::ModemStatus);
    }

    /// Apply baud rate, data bits, stop bits, and parity from termios.
    fn configure(self, termios: Termios) {
        let Some(divisor) = baud_divisor(termios.control_mode) else {
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
        self.write(
            Register::ModemControl,
            modem_control::DATA_TERMINAL_READY
                | modem_control::REQUEST_TO_SEND
                | modem_control::OUT2,
        );
        self.enable_receive_interrupts();
    }

    /// Enable receive, line-status, and modem-status interrupts.
    fn enable_receive_interrupts(self) {
        let enabled = self.read(Register::InterruptEnable);
        self.write(
            Register::InterruptEnable,
            enabled
                | interrupt_enable::RECEIVED_DATA_AVAILABLE
                | interrupt_enable::LINE_STATUS
                | interrupt_enable::MODEM_STATUS,
        );
    }

    /// Enable transmit-empty interrupts without changing receive-related bits.
    fn enable_transmit_interrupt(self) {
        let enabled = self.read(Register::InterruptEnable);
        self.write(
            Register::InterruptEnable,
            enabled | interrupt_enable::TRANSMIT_EMPTY,
        );
    }

    /// Disable transmit-empty interrupts without changing receive-related bits.
    fn disable_transmit_interrupt(self) {
        let enabled = self.read(Register::InterruptEnable);
        self.write(
            Register::InterruptEnable,
            enabled & !interrupt_enable::TRANSMIT_EMPTY,
        );
    }

    /// Dispatch all pending interrupt causes for this UART.
    fn handle_interrupt(self) {
        loop {
            let ident = self.read(Register::InterruptIdentifyOrFifoControl);
            if ident & interrupt_identify::NO_INTERRUPT_PENDING != 0 {
                break;
            }

            match ident & interrupt_identify::CAUSE_MASK {
                interrupt_identify::MODEM_STATUS => self.clear_modem_status(),
                interrupt_identify::TRANSMIT_EMPTY => self.write_next_byte(),
                interrupt_identify::RECEIVED_DATA_AVAILABLE
                | interrupt_identify::CHARACTER_TIMEOUT => self.read_available_bytes(),
                interrupt_identify::LINE_STATUS => self.clear_line_status(),
                _ => break,
            }
        }
    }

    /// Clear a modem-status interrupt by reading the modem-status register.
    fn clear_modem_status(self) {
        let _ = self.read(Register::ModemStatus);
    }

    /// Clear a line-status interrupt by reading the line-status register.
    fn clear_line_status(self) {
        let _ = self.read(Register::LineStatus);
    }

    /// Drain all currently available received bytes into the TTY layer.
    fn read_available_bytes(self) {
        let mut bytes = [0u8; 16];
        let mut count = 0;

        while count < bytes.len() && self.read(Register::LineStatus) & line_status::DATA_READY != 0
        {
            bytes[count] = self.read(Register::Data);
            count += 1;
        }

        if count != 0 {
            tty::receive_input(self.tty_channel, &bytes[..count]);
        }
    }

    /// Transmit one queued byte, or disable transmit interrupts if empty.
    fn write_next_byte(self) {
        let mut byte = [0u8; 1];
        if tty::take_output(self.tty_channel, &mut byte) == 0 {
            self.disable_transmit_interrupt();
            tty::wake_output(self.tty_channel);
            return;
        }

        self.write(Register::Data, byte[0]);

        if !tty::has_output(self.tty_channel) {
            self.disable_transmit_interrupt();
        }
        tty::wake_output(self.tty_channel);
    }

    /// Read one UART register.
    #[inline]
    fn read(self, register: Register) -> u8 {
        inb(self.base + register as u16)
    }

    /// Write one UART register with an ISA I/O delay.
    #[inline]
    fn write(self, register: Register, value: u8) {
        outb_p(value, self.base + register as u16);
    }
}

/// Return the UART divisor for the low four `c_cflag` baud bits.
fn baud_divisor(control_mode: ControlMode) -> Option<u16> {
    match control_mode & ControlMode::CBAUD {
        mode if mode.is_empty() => None,
        ControlMode::B50 => Some(2304),
        ControlMode::B75 => Some(1536),
        ControlMode::B110 => Some(1047),
        ControlMode::B134 => Some(857),
        ControlMode::B150 => Some(768),
        ControlMode::B200 => Some(576),
        ControlMode::B300 => Some(384),
        ControlMode::B600 => Some(192),
        ControlMode::B1200 => Some(96),
        ControlMode::B1800 => Some(64),
        ControlMode::B2400 => Some(48),
        ControlMode::B4800 => Some(24),
        ControlMode::B9600 => Some(12),
        ControlMode::B19200 => Some(6),
        ControlMode::B38400 => Some(3),
        _ => Some(48),
    }
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

/// Naked ISR stub for IRQ4, used by COM1.
#[naked]
pub extern "C" fn serial1_interrupt() {
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
            "pushl $0",
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

/// Naked ISR stub for IRQ3, used by COM2.
#[naked]
pub extern "C" fn serial2_interrupt() {
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
            "pushl $1",
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

/// Rust-side dispatcher shared by the COM1 and COM2 interrupt stubs.
extern "C" fn serial_interrupt_entry(index: usize) {
    if let Some(port) = PORTS.get(index).copied() {
        port.handle_interrupt();
    }

    outb(PIC_EOI, 0x20);
}
