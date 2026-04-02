//! Exception and interrupt vector setup.
//!
//! - [`gate`] — IDT gate descriptor construction (`set_trap_gate`, etc.).
//! - [`handler`] — Per-vector entry stubs and exception handlers.

mod gate;
mod handler;

pub use gate::{TrapHandler, set_intr_gate, set_system_gate, set_trap_gate};
use handler::*;

/// Installs exception handlers into the IDT.
pub fn init() {
    // Vectors 0–7: no error code
    set_trap_gate(0, divide_error);
    set_trap_gate(1, debug);
    set_trap_gate(2, nmi);
    set_system_gate(3, int3); // vectors 3–5: DPL=3 (user-callable)
    set_system_gate(4, overflow);
    set_system_gate(5, bounds);
    set_trap_gate(6, invalid_op);
    set_trap_gate(7, device_not_available);

    // Vectors 8–14: with error code
    set_trap_gate(8, double_fault);
    set_trap_gate(9, coprocessor_segment_overrun);
    set_trap_gate(10, invalid_tss);
    set_trap_gate(11, segment_not_present);
    set_trap_gate(12, stack_segment);
    set_trap_gate(13, general_protection);
    set_trap_gate(14, page_fault);

    // Vector 16: no error code
    set_trap_gate(16, coprocessor_error);

    // Fill unused vectors with a catch-all handler
    set_trap_gate(15, reserved);
    for i in 17..48 {
        set_trap_gate(i, reserved);
    }

    // Override specific reserved slots
    set_trap_gate(39, parallel_interrupt);
    set_trap_gate(45, irq13);
}
