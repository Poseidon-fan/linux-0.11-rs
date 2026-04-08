//! IDT gate descriptor construction and installation.

use crate::segment::KERNEL_CS;

unsafe extern "C" {
    static mut idt: [GateDescriptor; 256];
}

pub type TrapHandler = extern "C" fn();

#[inline]
pub fn set_intr_gate(n: usize, handler: TrapHandler) {
    set_gate(n, GateDescriptor::interrupt(handler, 0));
}

#[inline]
pub fn set_trap_gate(n: usize, handler: TrapHandler) {
    set_gate(n, GateDescriptor::trap(handler, 0));
}

#[inline]
pub fn set_system_gate(n: usize, handler: TrapHandler) {
    set_gate(n, GateDescriptor::trap(handler, 3));
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
fn set_gate(n: usize, desc: GateDescriptor) {
    unsafe {
        idt[n] = desc;
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
