//! IDT gate descriptor construction and installation.

use crate::segment::KERNEL_CS;

unsafe extern "C" {
    static mut idt: [GateDescriptor; 256];
}

pub type TrapHandler = extern "C" fn();

/// Install a kernel-only **interrupt gate** at IDT `vector`.
///
/// The CPU clears `IF` on entry, masking further maskable interrupts until
/// the handler explicitly re-enables them. Use this for hardware IRQs that
/// must not be preempted by another device interrupt during their critical
/// prologue. `DPL = 0` rejects software `int N` from user space with `#GP`.
#[inline]
pub fn set_intr_gate(vector: usize, handler: TrapHandler) {
    set_gate(vector, GateDescriptor::interrupt(handler, 0));
}

/// Install a kernel-only **trap gate** at IDT `vector`.
///
/// Unlike an interrupt gate, `IF` is preserved on entry, so a handler that
/// re-faults or services nested events can run with interrupts already
/// enabled. Used for CPU exceptions (divide-by-zero, page fault, etc.).
/// `DPL = 0` rejects software `int N` from user space with `#GP`.
#[inline]
pub fn set_trap_gate(vector: usize, handler: TrapHandler) {
    set_gate(vector, GateDescriptor::trap(handler, 0));
}

/// Install a **trap gate callable from user space** at IDT `vector`.
///
/// `DPL = 3` allows ring-3 code to enter via `int N`, which is how user
/// programs invoke the syscall vector (`int 0x80`) and trigger the
/// breakpoint vector (`int3`). `IF` is preserved on entry, matching
/// [`set_trap_gate`].
#[inline]
pub fn set_system_gate(vector: usize, handler: TrapHandler) {
    set_gate(vector, GateDescriptor::trap(handler, 3));
}

/// An i386 IDT gate descriptor (interrupt gate or trap gate).
///
/// ```text
///  63       48 47 46-45 44 43-40 39-32 31     16 15        0
/// ┌──────────┬──┬─────┬──┬─────┬─────┬──────────┬──────────┐
/// │offset_hi │P │ DPL │0 │type │  0  │ selector │offset_lo │
/// └──────────┴──┴─────┴──┴─────┴─────┴──────────┴──────────┘
/// ```
///
/// - Interrupt gate (type `0xE`): clears IF on entry.
/// - Trap gate (type `0xF`): leaves IF unchanged.
#[repr(C)]
struct GateDescriptor {
    offset_low: u16,
    selector: u16,
    _reserved: u8,
    flags: u8,
    offset_high: u16,
}

#[inline]
fn set_gate(vector: usize, desc: GateDescriptor) {
    unsafe {
        idt[vector] = desc;
    }
}

impl GateDescriptor {
    #[inline]
    fn interrupt(handler: TrapHandler, dpl: u8) -> Self {
        Self::new(handler, dpl, 0xE)
    }

    #[inline]
    fn trap(handler: TrapHandler, dpl: u8) -> Self {
        Self::new(handler, dpl, 0xF)
    }

    #[inline]
    fn new(handler: TrapHandler, dpl: u8, gate_type: u8) -> Self {
        let addr = handler as usize;
        Self {
            offset_low: (addr & 0xFFFF) as u16,
            selector: KERNEL_CS.as_u16(),
            _reserved: 0,
            flags: 0x80 | ((dpl & 0x3) << 5) | (gate_type & 0x1F),
            offset_high: (addr >> 16) as u16,
        }
    }
}
