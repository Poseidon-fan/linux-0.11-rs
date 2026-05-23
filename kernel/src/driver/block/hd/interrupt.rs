//! Hard disk interrupt entry.

use core::arch::naked_asm;

use crate::pmio::outb;

/// IRQ14 entry stub for the hard disk controller.
///
/// This keeps the assembly path minimal: save the caller-saved registers and
/// segment registers used by the kernel ABI, switch to kernel segments, and
/// tail into the Rust dispatcher.
#[naked]
pub extern "C" fn hd_interrupt() {
    unsafe {
        naked_asm!(
            "pushl %eax",
            "pushl %ecx",
            "pushl %edx",
            "push %ds",
            "push %es",
            "push %fs",
            "movl $0x10, %eax",
            "movw %ax, %ds",
            "movw %ax, %es",
            "movl $0x17, %eax",
            "movw %ax, %fs",
            "call {entry}",
            "pop %fs",
            "pop %es",
            "pop %ds",
            "popl %edx",
            "popl %ecx",
            "popl %eax",
            "iret",
            entry = sym hd_interrupt_rust_entry,
            options(att_syntax),
        );
    }
}

/// Rust-side dispatcher for IRQ14.
extern "C" fn hd_interrupt_rust_entry() {
    // Acknowledge the slave PIC first, then the master cascade line.
    outb(0x20, 0xA0);
    outb(0x20, 0x20);

    super::on_interrupt();
}
