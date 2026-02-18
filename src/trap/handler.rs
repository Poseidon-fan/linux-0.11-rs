//! Interrupt/Exception Handlers
//!
//! Each vector has a thin entry stub bound to a concrete Rust handler.
//! Shared save/restore logic is centralized in two common paths:
//! exceptions without CPU error code and exceptions with CPU error code.

use core::arch::{asm, naked_asm};
use log::{error, info};

use crate::mm::page_fault;

/// Exception stack frame built by the common entry assembly stubs.
///
/// ```text
/// +--------------------+ <- High address
/// | SS  (if CPL change)|
/// | ESP (if CPL change)|
/// | EFLAGS             |
/// | CS                 |
/// | EIP                | <- Pushed by CPU
/// +--------------------+
/// | GP & segment regs  |
/// | ...                | <- Pushed manually
/// +--------------------+
/// | Error Code         | <- Saved by our entry code
/// +--------------------+ <- &ExceptionFrame (ESP)
/// ```
///
/// `user_esp` / `user_ss` are not part of this struct because they only exist
/// on privilege-level transitions. Use [`ExceptionFrame::user_stack`] to read them
/// when present.
#[repr(C)]
pub struct ExceptionFrame {
    pub error_code: u32,
    pub fs: u32,
    pub es: u32,
    pub ds: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,
    pub edx: u32,
    pub ecx: u32,
    pub ebx: u32,
    pub eax: u32,
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
}

impl ExceptionFrame {
    #[inline]
    fn is_from_user_mode(&self) -> bool {
        (self.cs & 0x3) == 0x3
    }

    /// Returns `(user_esp, user_ss)` when present.
    pub fn user_stack(&self) -> Option<(u32, u32)> {
        if !self.is_from_user_mode() {
            return None;
        }

        let base = self as *const Self as *const u32;
        let words = core::mem::size_of::<Self>() / core::mem::size_of::<u32>();
        unsafe {
            let user_esp = *base.add(words);
            let user_ss = *base.add(words + 1);
            Some((user_esp, user_ss))
        }
    }
}

/// Common entry path for exceptions **without** a CPU error code.
///
/// On entry the stub has pushed the handler address onto the stack.
/// This code builds a [`ExceptionFrame`] (with `error_code = 0`) and calls
/// the handler via `call *%eax`.
#[naked]
extern "C" fn common_no_error_entry() {
    unsafe {
        naked_asm!(
            "xchgl %eax, (%esp)", // eax <- handler, [esp] <- saved eax
            "pushl %ebx",
            "pushl %ecx",
            "pushl %edx",
            "pushl %edi",
            "pushl %esi",
            "pushl %ebp",
            "pushl %ds",
            "pushl %es",
            "pushl %fs",
            "pushl $0",   // error_code = 0 (no CPU error code)
            "pushl %esp", // arg: &ExceptionFrame
            "movl $0x10, %edx",
            "movw %dx, %ds",
            "movw %dx, %es",
            "movw %dx, %fs",
            "call *%eax",
            "addl $8, %esp", // drop arg + error_code
            "popl %fs",
            "popl %es",
            "popl %ds",
            "popl %ebp",
            "popl %esi",
            "popl %edi",
            "popl %edx",
            "popl %ecx",
            "popl %ebx",
            "popl %eax",
            "iret",
            options(att_syntax),
        );
    }
}

/// Common entry path for exceptions **with** a CPU error code.
///
/// On entry the stub has pushed the handler address on top of the CPU error
/// code. Two `xchgl` instructions extract both values into registers while
/// saving the original EAX/EBX in their place, then builds a [`ExceptionFrame`].
#[naked]
extern "C" fn common_with_error_entry() {
    unsafe {
        naked_asm!(
            "xchgl %eax, 4(%esp)", // eax <- error_code, [esp+4] <- saved eax
            "xchgl %ebx, (%esp)",  // ebx <- handler,    [esp]   <- saved ebx
            "pushl %ecx",
            "pushl %edx",
            "pushl %edi",
            "pushl %esi",
            "pushl %ebp",
            "pushl %ds",
            "pushl %es",
            "pushl %fs",
            "pushl %eax", // error_code (the real CPU value)
            "pushl %esp", // arg: &ExceptionFrame
            "movl $0x10, %eax",
            "movw %ax, %ds",
            "movw %ax, %es",
            "movw %ax, %fs",
            "call *%ebx",
            "addl $8, %esp", // drop arg + error_code
            "popl %fs",
            "popl %es",
            "popl %ds",
            "popl %ebp",
            "popl %esi",
            "popl %edi",
            "popl %edx",
            "popl %ecx",
            "popl %ebx",
            "popl %eax",
            "iret",
            options(att_syntax),
        );
    }
}

macro_rules! trap_stub_no_error {
    ($entry:ident => $handler:ident) => {
        #[naked]
        pub extern "C" fn $entry() {
            unsafe {
                naked_asm!(
                    "pushl ${handler}",
                    "jmp {common}",
                    handler = sym $handler,
                    common = sym common_no_error_entry,
                    options(att_syntax),
                );
            }
        }
    };
}

macro_rules! trap_stub_with_error {
    ($entry:ident => $handler:ident) => {
        #[naked]
        pub extern "C" fn $entry() {
            unsafe {
                naked_asm!(
                    "pushl ${handler}",
                    "jmp {common}",
                    handler = sym $handler,
                    common = sym common_with_error_entry,
                    options(att_syntax),
                );
            }
        }
    };
}

/// Print exception info and halt the system.
///
/// In Linux 0.11 this eventually exits the current task. For now we halt.
fn die(message: &str, frame: &ExceptionFrame) -> ! {
    error!("{}: {:04x}", message, frame.error_code & 0xffff);
    match frame.user_stack() {
        Some((user_esp, user_ss)) => {
            error!(
                "EIP: {:04x}:{:08x}  EFLAGS: {:08x}  ESP: {:04x}:{:08x}",
                frame.cs, frame.eip, frame.eflags, user_ss, user_esp
            );
        }
        None => {
            error!(
                "EIP: {:04x}:{:08x}  EFLAGS: {:08x}  ESP: <kernel-mode>",
                frame.cs, frame.eip, frame.eflags
            );
        }
    }
    error!("fs: {:04x}", frame.fs);

    loop {
        unsafe { asm!("cli", "hlt", options(att_syntax)) }
    }
}

extern "C" fn do_divide_error(frame: &ExceptionFrame) {
    die("divide error", frame);
}

extern "C" fn do_nmi(frame: &ExceptionFrame) {
    die("nmi", frame);
}

extern "C" fn do_int3(frame: &ExceptionFrame) {
    let tr: u32;
    unsafe { asm!("str {0:x}", out(reg) tr, options(nomem, nostack, att_syntax)) }

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

extern "C" fn do_overflow(frame: &ExceptionFrame) {
    die("overflow", frame);
}

extern "C" fn do_bounds(frame: &ExceptionFrame) {
    die("bounds", frame);
}

extern "C" fn do_invalid_op(frame: &ExceptionFrame) {
    die("invalid operand", frame);
}

extern "C" fn do_reserved(frame: &ExceptionFrame) {
    die("reserved (15,17-47) error", frame);
}

extern "C" fn do_double_fault(frame: &ExceptionFrame) {
    die("double fault", frame);
}

extern "C" fn do_invalid_tss(frame: &ExceptionFrame) {
    die("invalid TSS", frame);
}

extern "C" fn do_segment_not_present(frame: &ExceptionFrame) {
    die("segment not present", frame);
}

extern "C" fn do_stack_segment(frame: &ExceptionFrame) {
    die("stack segment", frame);
}

extern "C" fn do_general_protection(frame: &ExceptionFrame) {
    die("general protection", frame);
}

extern "C" fn do_page_fault(frame: &ExceptionFrame) {
    let fault_addr: u32;
    unsafe {
        asm!(
            "movl %cr2, {fault_addr:e}",
            fault_addr = out(reg) fault_addr,
            options(att_syntax, nomem, nostack, preserves_flags),
        );
    }

    let error_code = frame.error_code;
    if error_code & 0x1 == 0 {
        page_fault::handle_no_page(error_code, fault_addr);
    } else {
        page_fault::handle_wp_page(fault_addr);
    }
}

/// Temporary handler for vectors that are not implemented yet.
///
/// Prints the vector number first, then halts with the common trap dump.
fn fake_unimplemented_vector(vector: u8, frame: &ExceptionFrame) -> ! {
    error!("unimplemented trap vector: {}", vector);
    die("unimplemented trap", frame);
}

extern "C" fn do_device_not_available(frame: &ExceptionFrame) {
    fake_unimplemented_vector(7, frame);
}

extern "C" fn do_coprocessor_segment_overrun(frame: &ExceptionFrame) {
    fake_unimplemented_vector(9, frame);
}

extern "C" fn do_coprocessor_error(frame: &ExceptionFrame) {
    fake_unimplemented_vector(16, frame);
}

extern "C" fn do_parallel_interrupt(frame: &ExceptionFrame) {
    fake_unimplemented_vector(39, frame);
}

extern "C" fn do_irq13(frame: &ExceptionFrame) {
    fake_unimplemented_vector(45, frame);
}

trap_stub_no_error!(divide_error => do_divide_error);
trap_stub_no_error!(debug => do_int3);
trap_stub_no_error!(nmi => do_nmi);
trap_stub_no_error!(int3 => do_int3);
trap_stub_no_error!(overflow => do_overflow);
trap_stub_no_error!(bounds => do_bounds);
trap_stub_no_error!(invalid_op => do_invalid_op);
trap_stub_no_error!(device_not_available => do_device_not_available);
trap_stub_no_error!(coprocessor_segment_overrun => do_coprocessor_segment_overrun);
trap_stub_with_error!(double_fault => do_double_fault);
trap_stub_with_error!(invalid_tss => do_invalid_tss);
trap_stub_with_error!(segment_not_present => do_segment_not_present);
trap_stub_with_error!(stack_segment => do_stack_segment);
trap_stub_with_error!(general_protection => do_general_protection);
trap_stub_with_error!(page_fault => do_page_fault);
trap_stub_no_error!(coprocessor_error => do_coprocessor_error);
trap_stub_no_error!(parallel_interrupt => do_parallel_interrupt);
trap_stub_no_error!(irq13 => do_irq13);
trap_stub_no_error!(reserved => do_reserved);
