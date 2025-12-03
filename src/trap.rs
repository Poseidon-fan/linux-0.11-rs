#![allow(unused)]

unsafe extern "C" {
    // Interrupt Descriptor Table, defined in boot/head.s
    static mut idt: [InterruptDescriptor; 256];
}

#[inline(always)]
pub fn set_intr_gate(n: usize, handler: fn()) {
    set_gate(n, InterruptDescriptor::intr(handler, 0x0));
}

#[inline(always)]
pub fn set_trap_gate(n: usize, handler: fn()) {
    set_gate(n, InterruptDescriptor::trap(handler, 0x0));
}

#[inline(always)]
pub fn set_system_gate(n: usize, handler: fn()) {
    set_gate(n, InterruptDescriptor::trap(handler, 0x11));
}

// Interrupt Descriptor Table Entry.
// In fact, there're three descriptor for i386:
// - Task Gate
// - Interrupt Gate
// - Trap Gate
// We'll not use task gate, so the following struct describes interrupt gate and trap gate.
#[repr(C, packed)]
struct InterruptDescriptor {
    offset_low: u16,
    selector: u16,
    zero: u8,
    flags: u8,
    offset_high: u16,
}

#[inline(always)]
fn set_gate(n: usize, descriptor: InterruptDescriptor) {
    unsafe {
        idt[n] = descriptor;
    }
}

impl InterruptDescriptor {
    pub fn intr(handler: fn(), dpl: u8) -> Self {
        Self::new(handler, dpl, 0xE)
    }

    pub fn trap(handler: fn(), dpl: u8) -> Self {
        Self::new(handler, dpl, 0xF)
    }

    fn new(handler: fn(), dpl: u8, gate_type: u8) -> Self {
        const KERNEL_CS: u16 = 0x08;
        let addr = handler as usize;

        Self {
            offset_low: (addr & 0xFFFF) as u16,
            selector: KERNEL_CS,
            zero: 0,
            flags: 0x80 | ((dpl & 0x3) << 5) | (gate_type & 0x1F),
            offset_high: ((addr >> 16) & 0xFFFF) as u16,
        }
    }
}
