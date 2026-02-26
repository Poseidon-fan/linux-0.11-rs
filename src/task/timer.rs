//! PIT timer interrupt entry and tick handling.

use core::{arch::naked_asm, ptr};

use crate::{pmio::outb, task};

/// Number of timer ticks since boot.
static mut JIFFIES: u32 = 0;

/// Returns current jiffies value.
#[inline]
pub fn jiffies() -> u32 {
    unsafe { ptr::read_volatile(ptr::addr_of!(JIFFIES)) }
}

/// IRQ0 entry stub.
#[naked]
pub extern "C" fn timer_interrupt() {
    unsafe {
        naked_asm!(
            "push %ds",
            "push %es",
            "push %fs",
            "pushl %edx",
            "pushl %ecx",
            "pushl %ebx",
            "pushl %eax",
            "movl $0x10, %eax",
            "movw %ax, %ds",
            "movw %ax, %es",
            "movl $0x17, %eax",
            "movw %ax, %fs",
            "movl {saved_cs_off}(%esp), %eax",
            "andl $3, %eax",
            "pushl %eax",
            "call {entry}",
            "addl $4, %esp",
            "popl %eax",
            "popl %ebx",
            "popl %ecx",
            "popl %edx",
            "pop %fs",
            "pop %es",
            "pop %ds",
            "iret",
            saved_cs_off = const 32,
            entry = sym timer_interrupt_rust_entry,
            options(att_syntax),
        );
    }
}

/// Rust-side timer tick logic for IRQ0.
extern "C" fn timer_interrupt_rust_entry(cpl: u32) {
    // Safety: single-core kernel; IRQ0 handler runs with interrupts masked by gate semantics.
    unsafe {
        let jiffies_ptr = ptr::addr_of_mut!(JIFFIES);
        let next = ptr::read_volatile(jiffies_ptr).wrapping_add(1);
        ptr::write_volatile(jiffies_ptr, next);
    }

    // Send End-Of-Interrupt to master 8259A PIC.
    outb(0x20, 0x20);

    // Safety: IRQ0 runs through an interrupt gate, so hardware already
    // masked interrupts on entry. This satisfies `exclusive_unchecked`.
    let should_schedule = unsafe {
        task::current_task()
            .pcb
            .inner
            .exclusive_unchecked(|current| {
                if cpl != 0 {
                    current.acct.utime = current.acct.utime.wrapping_add(1);
                } else {
                    current.acct.stime = current.acct.stime.wrapping_add(1);
                }

                if current.sched.counter > 0 {
                    current.sched.counter -= 1;
                }

                current.sched.counter == 0 && cpl != 0
            })
    };

    if should_schedule {
        task::schedule();
    }
}
