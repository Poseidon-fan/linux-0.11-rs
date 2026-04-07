use core::arch::{asm, naked_asm};

use log::{error, info};

use crate::mm;

/// CPU exception frame built by the common entry stubs.
///
/// ```text
/// +--------------------+ ← high address
/// | SS  (if CPL change)|
/// | ESP (if CPL change)|
/// | EFLAGS             |
/// | CS                 |
/// | EIP                | ← pushed by CPU
/// +--------------------+
/// | EAX                |
/// | EBX                |
/// | ECX                |
/// | EDX                |
/// | EDI                |
/// | ESI                |
/// | EBP                |
/// | DS                 |
/// | ES                 |
/// | FS                 | ← pushed by entry stub
/// +--------------------+
/// | error_code         | ← 0 if CPU didn't push one
/// +--------------------+ ← &ExceptionFrame (ESP)
/// ```
///
/// `user_esp`/`user_ss` sit above the CPU-pushed fields and only exist on
/// privilege-level transitions (see [`user_stack`](Self::user_stack)).
#[repr(C)]
struct ExceptionFrame {
    error_code: u32,
    fs: u32,
    es: u32,
    ds: u32,
    ebp: u32,
    esi: u32,
    edi: u32,
    edx: u32,
    ecx: u32,
    ebx: u32,
    eax: u32,
    eip: u32,
    cs: u32,
    eflags: u32,
}

impl ExceptionFrame {
    #[inline]
    fn is_from_user_mode(&self) -> bool {
        (self.cs & 0x3) == 0x3
    }

    /// Returns `(user_esp, user_ss)` if the exception came from user mode.
    fn user_stack(&self) -> Option<(u32, u32)> {
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

    /// Dumps all registers via `log::error!`.
    fn dump(&self) {
        error!(
            "EAX={:08x} EBX={:08x} ECX={:08x} EDX={:08x}",
            self.eax, self.ebx, self.ecx, self.edx
        );
        error!(
            "ESI={:08x} EDI={:08x} EBP={:08x}",
            self.esi, self.edi, self.ebp
        );
        error!(
            "EIP={:04x}:{:08x} EFLAGS={:08x}",
            self.cs, self.eip, self.eflags
        );
        error!("DS={:04x} ES={:04x} FS={:04x}", self.ds, self.es, self.fs);
        if let Some((esp, ss)) = self.user_stack() {
            error!("ESP={:04x}:{:08x}", ss, esp);
        }
    }
}

// ---------------------------------------------------------------------------
// Common entry paths (naked assembly)
// ---------------------------------------------------------------------------

/// Entry path for exceptions **without** a CPU error code.
///
/// The stub has pushed the handler address; this code saves all registers,
/// sets `error_code = 0`, switches to kernel data segments, and calls the
/// handler through `%eax`.
#[naked]
extern "C" fn entry_no_error() {
    unsafe {
        naked_asm!(
            "xchgl %eax, (%esp)", // eax ← handler, [esp] ← saved eax
            "pushl %ebx",
            "pushl %ecx",
            "pushl %edx",
            "pushl %edi",
            "pushl %esi",
            "pushl %ebp",
            "pushl %ds",
            "pushl %es",
            "pushl %fs",
            "pushl $0",   // error_code = 0
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

/// Entry path for exceptions **with** a CPU error code.
///
/// The stub has pushed the handler address on top of the CPU error code.
/// Two `xchgl` instructions extract both into registers while saving
/// `EAX`/`EBX` in their place, then the rest proceeds as above.
#[naked]
extern "C" fn entry_with_error() {
    unsafe {
        naked_asm!(
            "xchgl %eax, 4(%esp)", // eax ← error_code, [esp+4] ← saved eax
            "xchgl %ebx, (%esp)",  // ebx ← handler,    [esp]   ← saved ebx
            "pushl %ecx",
            "pushl %edx",
            "pushl %edi",
            "pushl %esi",
            "pushl %ebp",
            "pushl %ds",
            "pushl %es",
            "pushl %fs",
            "pushl %eax", // error_code
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

// ---------------------------------------------------------------------------
// Stub generation macros
// ---------------------------------------------------------------------------

macro_rules! stub_no_error {
    ($entry:ident => $handler:ident) => {
        #[naked]
        pub extern "C" fn $entry() {
            unsafe {
                naked_asm!(
                    "pushl ${handler}",
                    "jmp {common}",
                    handler = sym $handler,
                    common = sym entry_no_error,
                    options(att_syntax),
                );
            }
        }
    };
}

macro_rules! stub_with_error {
    ($entry:ident => $handler:ident) => {
        #[naked]
        pub extern "C" fn $entry() {
            unsafe {
                naked_asm!(
                    "pushl ${handler}",
                    "jmp {common}",
                    handler = sym $handler,
                    common = sym entry_with_error,
                    options(att_syntax),
                );
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Exception handlers
// ---------------------------------------------------------------------------

/// Dumps the exception frame and halts the CPU.
fn fatal(name: &str, frame: &ExceptionFrame) -> ! {
    error!("EXCEPTION: {} (error_code={:#06x})", name, frame.error_code);
    frame.dump();
    loop {
        unsafe { asm!("cli", "hlt", options(att_syntax)) }
    }
}

/// Prints register state for a debug/breakpoint trap (non-fatal).
fn dump_debug_info(tag: &str, frame: &ExceptionFrame) {
    let tr: u32;
    unsafe { asm!("str {0:x}", out(reg) tr, options(nomem, nostack, att_syntax)) }

    info!("--- {} ---", tag);
    info!(
        "EAX={:08x} EBX={:08x} ECX={:08x} EDX={:08x}",
        frame.eax, frame.ebx, frame.ecx, frame.edx
    );
    info!(
        "ESI={:08x} EDI={:08x} EBP={:08x} ESP={:08x}",
        frame.esi, frame.edi, frame.ebp, frame as *const _ as u32
    );
    info!(
        "DS={:04x} ES={:04x} FS={:04x} TR={:04x}",
        frame.ds, frame.es, frame.fs, tr
    );
    info!(
        "EIP={:04x}:{:08x} EFLAGS={:08x}",
        frame.cs, frame.eip, frame.eflags
    );
}

extern "C" fn on_divide_error(frame: &ExceptionFrame) {
    fatal("divide error", frame);
}

extern "C" fn on_debug(frame: &ExceptionFrame) {
    dump_debug_info("#DB debug exception", frame);
}

extern "C" fn on_nmi(frame: &ExceptionFrame) {
    fatal("NMI", frame);
}

extern "C" fn on_int3(frame: &ExceptionFrame) {
    dump_debug_info("#BP breakpoint", frame);
}

extern "C" fn on_overflow(frame: &ExceptionFrame) {
    fatal("overflow", frame);
}

extern "C" fn on_bounds(frame: &ExceptionFrame) {
    fatal("bounds check", frame);
}

extern "C" fn on_invalid_op(frame: &ExceptionFrame) {
    fatal("invalid opcode", frame);
}

extern "C" fn on_device_not_available(frame: &ExceptionFrame) {
    fatal("device not available", frame);
}

extern "C" fn on_double_fault(frame: &ExceptionFrame) {
    fatal("double fault", frame);
}

extern "C" fn on_coprocessor_segment_overrun(frame: &ExceptionFrame) {
    fatal("coprocessor segment overrun", frame);
}

extern "C" fn on_invalid_tss(frame: &ExceptionFrame) {
    fatal("invalid TSS", frame);
}

extern "C" fn on_segment_not_present(frame: &ExceptionFrame) {
    fatal("segment not present", frame);
}

extern "C" fn on_stack_segment(frame: &ExceptionFrame) {
    fatal("stack segment fault", frame);
}

extern "C" fn on_general_protection(frame: &ExceptionFrame) {
    fatal("general protection fault", frame);
}

extern "C" fn on_page_fault(frame: &ExceptionFrame) {
    let fault_addr: u32;
    unsafe {
        asm!(
            "movl %cr2, {fault_addr:e}",
            fault_addr = out(reg) fault_addr,
            options(att_syntax, nomem, nostack, preserves_flags),
        );
    }

    if frame.error_code & 0x1 == 0 {
        mm::handle_no_page(frame.error_code, fault_addr);
    } else {
        mm::handle_wp_page(fault_addr);
    }
}

extern "C" fn on_coprocessor_error(frame: &ExceptionFrame) {
    fatal("coprocessor error", frame);
}

extern "C" fn on_reserved(frame: &ExceptionFrame) {
    fatal("reserved vector", frame);
}

extern "C" fn on_parallel_interrupt(_frame: &ExceptionFrame) {
    // Spurious IRQ7 — nothing to do.
}

extern "C" fn on_irq13(frame: &ExceptionFrame) {
    fatal("IRQ13 (coprocessor)", frame);
}

// ---------------------------------------------------------------------------
// Stub → handler bindings
// ---------------------------------------------------------------------------

stub_no_error!(divide_error          => on_divide_error);
stub_no_error!(debug                 => on_debug);
stub_no_error!(nmi                   => on_nmi);
stub_no_error!(int3                  => on_int3);
stub_no_error!(overflow              => on_overflow);
stub_no_error!(bounds                => on_bounds);
stub_no_error!(invalid_op            => on_invalid_op);
stub_no_error!(device_not_available  => on_device_not_available);
stub_no_error!(coprocessor_segment_overrun => on_coprocessor_segment_overrun);
stub_with_error!(double_fault        => on_double_fault);
stub_with_error!(invalid_tss         => on_invalid_tss);
stub_with_error!(segment_not_present => on_segment_not_present);
stub_with_error!(stack_segment       => on_stack_segment);
stub_with_error!(general_protection  => on_general_protection);
stub_with_error!(page_fault          => on_page_fault);
stub_no_error!(coprocessor_error     => on_coprocessor_error);
stub_no_error!(parallel_interrupt    => on_parallel_interrupt);
stub_no_error!(irq13                 => on_irq13);
stub_no_error!(reserved              => on_reserved);
