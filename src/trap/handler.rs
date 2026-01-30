//! Interrupt/Exception Handlers
//!
//! This module provides the `InterruptFrame` struct and macros for generating
//! interrupt entry points using `#[naked]` functions.

use core::arch::naked_asm;
use log::{error, info};

/// Interrupt stack frame containing saved register state and CPU-pushed values.
///
/// Stack layout (from high to low address):
///
/// ```text
/// +------------------+ <- High address
/// | SS (if CPL change)|
/// | ESP (if CPL change)|
/// | EFLAGS           |
/// | CS               |
/// | EIP              |  <- Pushed by CPU
/// +------------------+
/// | Error Code       |  <- Pushed by CPU (some exceptions) or 0 by us
/// +------------------+
/// | FS               |
/// | ES               |  <- Saved by us
/// | DS               |
/// +------------------+
/// | EAX              |
/// | ECX              |
/// | EDX              |
/// | EBX              |  <- Saved by pusha
/// | ESP (original)   |
/// | EBP              |
/// | ESI              |
/// | EDI              |
/// +------------------+ <- Low address (ESP)
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InterruptFrame {
    // Registers saved by pusha (in reverse push order)
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp_dummy: u32, // ESP pushed by pusha, not useful
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,

    // Segment registers saved by us
    pub ds: u32,
    pub es: u32,
    pub fs: u32,

    // Error code (pushed by CPU for some exceptions, or 0 by us)
    pub error_code: u32,

    // Pushed by CPU automatically
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,

    // Only present if privilege level changes (ring 3 -> ring 0)
    pub user_esp: u32,
    pub user_ss: u32,
}

// ============================================================================
// Entry Point Generation Macros
// ============================================================================

/// Generate an exception entry point for exceptions without error code.
///
/// For exceptions that don't push an error code (e.g., divide error, breakpoint),
/// we push 0 as a placeholder to unify the stack frame layout.
#[macro_export]
macro_rules! exception_no_error {
    ($entry:ident => $handler:ident) => {
        #[naked]
        pub extern "C" fn $entry() {
            unsafe {
                naked_asm!(
                    "push 0",           // Push fake error code for uniform layout
                    "push fs",
                    "push es",
                    "push ds",
                    "pusha",            // Save EAX, ECX, EDX, EBX, ESP, EBP, ESI, EDI
                    "mov eax, 0x10",    // Load kernel data segment
                    "mov ds, ax",
                    "mov es, ax",
                    "mov fs, ax",
                    "push esp",         // Pass frame pointer as argument
                    "call {handler}",
                    "add esp, 4",       // Pop argument
                    "popa",             // Restore general purpose registers
                    "pop ds",
                    "pop es",
                    "pop fs",
                    "add esp, 4",       // Skip error code
                    "iretd",
                    handler = sym $handler,
                );
            }
        }
    };
}

/// Generate an exception entry point for exceptions with error code.
///
/// For exceptions that push an error code (e.g., page fault, GPF),
/// the CPU has already pushed the error code onto the stack.
#[macro_export]
macro_rules! exception_with_error {
    ($entry:ident => $handler:ident) => {
        #[naked]
        pub extern "C" fn $entry() {
            unsafe {
                naked_asm!(
                    // Error code already pushed by CPU
                    "push fs",
                    "push es",
                    "push ds",
                    "pusha",
                    "mov eax, 0x10",
                    "mov ds, ax",
                    "mov es, ax",
                    "mov fs, ax",
                    "push esp",
                    "call {handler}",
                    "add esp, 4",
                    "popa",
                    "pop ds",
                    "pop es",
                    "pop fs",
                    "add esp, 4",       // Skip error code
                    "iretd",
                    handler = sym $handler,
                );
            }
        }
    };
}

// ============================================================================
// Exception Handlers
// ============================================================================

use core::arch::asm;

/// Print exception info and halt the system.
///
/// In the original Linux 0.11, this calls `do_exit(11)` to terminate the process.
/// For now, we just print the information and halt.
fn die(message: &str, frame: &InterruptFrame) -> ! {
    error!("{}: {:04x}", message, frame.error_code & 0xffff);
    error!(
        "EIP: {:04x}:{:08x}  EFLAGS: {:08x}  ESP: {:04x}:{:08x}",
        frame.cs, frame.eip, frame.eflags, frame.user_ss, frame.user_esp
    );
    error!("fs: {:04x}", frame.fs);
    // TODO: print base, limit, pid, process nr, etc.

    loop {
        unsafe { asm!("cli", "hlt") }
    }
}

// --- Exceptions without error code ---

fn do_divide_error(frame: &InterruptFrame) {
    die("divide error", frame);
}

fn do_debug(frame: &InterruptFrame) {
    die("debug", frame);
}

fn do_nmi(frame: &InterruptFrame) {
    die("nmi", frame);
}

/// Breakpoint exception handler.
///
/// Unlike other exceptions, int3 is used for debugging and should not halt.
/// It prints register state and returns to continue execution.
fn do_int3(frame: &InterruptFrame) {
    let tr: u32;
    unsafe { asm!("str {0:x}", out(reg) tr, options(nomem, nostack)) }

    info!("eax\t\tebx\t\tecx\t\tedx");
    info!(
        "{:08x}\t{:08x}\t{:08x}\t{:08x}",
        frame.eax, frame.ebx, frame.ecx, frame.edx
    );
    info!("esi\t\tedi\t\tebp\t\tesp");
    info!(
        "{:08x}\t{:08x}\t{:08x}\t{:08x}",
        frame.esi, frame.edi, frame.ebp, frame as *const _ as u32
    );
    info!("ds\t\tes\t\tfs\t\ttr");
    info!(
        "{:04x}\t\t{:04x}\t\t{:04x}\t\t{:04x}",
        frame.ds, frame.es, frame.fs, tr
    );
    info!(
        "EIP: {:08x}   CS: {:04x}  EFLAGS: {:08x}",
        frame.eip, frame.cs, frame.eflags
    );
}

fn do_overflow(frame: &InterruptFrame) {
    die("overflow", frame);
}

fn do_bounds(frame: &InterruptFrame) {
    die("bounds", frame);
}

fn do_invalid_op(frame: &InterruptFrame) {
    die("invalid operand", frame);
}

fn do_reserved(frame: &InterruptFrame) {
    die("reserved (15,17-47) error", frame);
}

// --- Exceptions with error code ---

fn do_double_fault(frame: &InterruptFrame) {
    die("double fault", frame);
}

fn do_invalid_tss(frame: &InterruptFrame) {
    die("invalid TSS", frame);
}

fn do_segment_not_present(frame: &InterruptFrame) {
    die("segment not present", frame);
}

fn do_stack_segment(frame: &InterruptFrame) {
    die("stack segment", frame);
}

fn do_general_protection(frame: &InterruptFrame) {
    die("general protection", frame);
}

exception_no_error!(divide_error => do_divide_error);
exception_no_error!(debug => do_debug);
exception_no_error!(nmi => do_nmi);
exception_no_error!(int3 => do_int3);
exception_no_error!(overflow => do_overflow);
exception_no_error!(bounds => do_bounds);
exception_no_error!(invalid_op => do_invalid_op);
exception_with_error!(double_fault => do_double_fault);
exception_with_error!(invalid_tss => do_invalid_tss);
exception_with_error!(segment_not_present => do_segment_not_present);
exception_with_error!(stack_segment => do_stack_segment);
exception_with_error!(general_protection => do_general_protection);
exception_no_error!(reserved => do_reserved);
