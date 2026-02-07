//! Interrupt/Exception Handlers
//!
//! Each vector has a thin entry stub bound to a concrete Rust handler.
//! Shared save/restore logic is centralized in two common paths:
//! exceptions without CPU error code and exceptions with CPU error code.

use core::arch::{asm, naked_asm};
use log::{error, info};

/// Trap stack frame passed from common entry assembly.
///
/// Layout (low to high address):
/// `error_code, fs, es, ds, ebp, esi, edi, edx, ecx, ebx, eax, eip, cs, eflags`.
///
/// `user_esp/user_ss` are not embedded because they only exist when the trap
/// transitions from ring 3 to ring 0.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TrapFrame {
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

impl TrapFrame {
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

#[naked]
extern "C" fn common_no_error_entry() {
    unsafe {
        naked_asm!(
            // Entry stack:
            // [handler][eip][cs][eflags][opt user_esp][opt user_ss]
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
            "pushl $0",   // normalize error_code
            "pushl %esp", // arg: &TrapFrame (points to error_code)
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

#[naked]
extern "C" fn common_with_error_entry() {
    unsafe {
        naked_asm!(
            // Entry stack:
            // [handler][cpu_error][eip][cs][eflags][opt user_esp][opt user_ss]
            "xchgl %eax, 4(%esp)", // eax <- error_code, [4] <- saved eax
            "xchgl %ebx, (%esp)",  // ebx <- handler, [0] <- saved ebx
            "pushl %ecx",
            "pushl %edx",
            "pushl %edi",
            "pushl %esi",
            "pushl %ebp",
            "pushl %ds",
            "pushl %es",
            "pushl %fs",
            "pushl %eax", // normalized error_code
            "pushl %esp", // arg: &TrapFrame (points to error_code)
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
fn die(message: &str, frame: &TrapFrame) -> ! {
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

extern "C" fn do_divide_error(frame: &TrapFrame) {
    die("divide error", frame);
}

extern "C" fn do_debug(frame: &TrapFrame) {
    die("debug", frame);
}

extern "C" fn do_nmi(frame: &TrapFrame) {
    die("nmi", frame);
}

extern "C" fn do_int3(frame: &TrapFrame) {
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

extern "C" fn do_overflow(frame: &TrapFrame) {
    die("overflow", frame);
}

extern "C" fn do_bounds(frame: &TrapFrame) {
    die("bounds", frame);
}

extern "C" fn do_invalid_op(frame: &TrapFrame) {
    die("invalid operand", frame);
}

extern "C" fn do_reserved(frame: &TrapFrame) {
    die("reserved (15,17-47) error", frame);
}

extern "C" fn do_double_fault(frame: &TrapFrame) {
    die("double fault", frame);
}

extern "C" fn do_invalid_tss(frame: &TrapFrame) {
    die("invalid TSS", frame);
}

extern "C" fn do_segment_not_present(frame: &TrapFrame) {
    die("segment not present", frame);
}

extern "C" fn do_stack_segment(frame: &TrapFrame) {
    die("stack segment", frame);
}

extern "C" fn do_general_protection(frame: &TrapFrame) {
    die("general protection", frame);
}

trap_stub_no_error!(divide_error => do_divide_error);
trap_stub_no_error!(debug => do_debug);
trap_stub_no_error!(nmi => do_nmi);
trap_stub_no_error!(int3 => do_int3);
trap_stub_no_error!(overflow => do_overflow);
trap_stub_no_error!(bounds => do_bounds);
trap_stub_no_error!(invalid_op => do_invalid_op);
trap_stub_with_error!(double_fault => do_double_fault);
trap_stub_with_error!(invalid_tss => do_invalid_tss);
trap_stub_with_error!(segment_not_present => do_segment_not_present);
trap_stub_with_error!(stack_segment => do_stack_segment);
trap_stub_with_error!(general_protection => do_general_protection);
trap_stub_no_error!(reserved => do_reserved);
