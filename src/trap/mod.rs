#![allow(unused)]
//! Trap (Exception/Interrupt) handling.

mod gate;
mod handler;

pub use gate::{set_intr_gate, set_system_gate, set_trap_gate};
use handler::*;
use log::info;

/// Initialize trap handlers.
///
/// Sets up exception handlers in the IDT. This corresponds to `trap_init()` in
/// the original Linux 0.11 kernel.
pub fn init() {
    // Exceptions without error code
    set_trap_gate(0, divide_error);
    set_trap_gate(1, debug);
    set_trap_gate(2, nmi);
    set_system_gate(3, int3); // int3-5 can be called from user mode
    set_system_gate(4, overflow);
    set_system_gate(5, bounds);
    set_trap_gate(6, invalid_op);

    // Exceptions with error code
    set_trap_gate(8, double_fault);
    set_trap_gate(10, invalid_tss);
    set_trap_gate(11, segment_not_present);
    set_trap_gate(12, stack_segment);
    set_trap_gate(13, general_protection);

    // Reserved vectors
    set_trap_gate(15, reserved);
    for i in 17..48 {
        set_trap_gate(i, reserved);
    }
}
