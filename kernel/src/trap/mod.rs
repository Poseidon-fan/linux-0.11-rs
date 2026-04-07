//! Exception and interrupt vector setup.
//!
//! - [`gate`] — IDT gate descriptor construction (`set_trap_gate`, etc.).
//! - [`handler`] — Per-vector entry stubs and exception handlers.

mod gate;
mod handler;

pub use gate::{TrapHandler, set_intr_gate, set_system_gate, set_trap_gate};

/// Installs exception handlers into the IDT.
pub fn init() {
    // Vectors 0–7: no error code
    set_trap_gate(0, handler::divide_error);
    set_trap_gate(1, handler::debug);
    set_trap_gate(2, handler::nmi);
    set_system_gate(3, handler::int3); // vectors 3–5: DPL=3 (user-callable)
    set_system_gate(4, handler::overflow);
    set_system_gate(5, handler::bounds);
    set_trap_gate(6, handler::invalid_op);
    set_trap_gate(7, handler::device_not_available);

    // Vectors 8–14: with error code
    set_trap_gate(8, handler::double_fault);
    set_trap_gate(9, handler::coprocessor_segment_overrun);
    set_trap_gate(10, handler::invalid_tss);
    set_trap_gate(11, handler::segment_not_present);
    set_trap_gate(12, handler::stack_segment);
    set_trap_gate(13, handler::general_protection);
    set_trap_gate(14, handler::page_fault);

    // Vector 16: no error code
    set_trap_gate(16, handler::coprocessor_error);

    // Fill unused vectors with a catch-all handler
    set_trap_gate(15, handler::reserved);
    for i in 17..48 {
        set_trap_gate(i, handler::reserved);
    }

    // Override specific reserved slots
    set_trap_gate(39, handler::parallel_interrupt);
    set_trap_gate(45, handler::irq13);
}
