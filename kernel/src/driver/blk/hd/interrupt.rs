//! Hard disk interrupt entry and state-based dispatch.

use core::arch::naked_asm;

use crate::{
    driver::blk::{
        self, BLOCK_MANAGER, SECTOR_SIZE,
        hd::{
            HARD_DISK_MAJOR, SECTOR_WORD_COUNT,
            controller::{self, command_succeeded},
        },
    },
    pmio::{outb, port_read_words, port_write_words},
    println,
};

use super::{HARD_DISK_MANAGER, InterruptPhase, handle_request, mark_request_error};

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

    // Clear the pending phase before handing control to the concrete continuation.
    let interrupt_phase = HARD_DISK_MANAGER.exclusive(|manager| {
        core::mem::replace(&mut manager.interrupt_phase, InterruptPhase::None)
    });

    match interrupt_phase {
        InterruptPhase::None => handle_unexpected_interrupt(),
        InterruptPhase::Read => handle_read_interrupt(),
        InterruptPhase::Write => handle_write_interrupt(),
        InterruptPhase::Recalibrate => handle_recalibration_interrupt(),
    }
}

/// Handle a spurious or late ATA interrupt with no pending continuation.
fn handle_unexpected_interrupt() {
    println!("Unexpected hard disk interrupt");
}

/// Continue a pending ATA read command.
fn handle_read_interrupt() {
    if !command_succeeded() {
        mark_request_error();
        handle_request();
        return;
    }

    let buffer = BLOCK_MANAGER.exclusive(|manager| {
        manager
            .current_request_mut(HARD_DISK_MAJOR)
            .map(|request| request.io.data_addr.cast::<u16>())
    });
    let Some(buffer) = buffer else {
        handle_unexpected_interrupt();
        return;
    };

    port_read_words(controller::DATA_PORT, buffer.as_ptr(), SECTOR_WORD_COUNT);

    let has_more_sectors = BLOCK_MANAGER.exclusive(|manager| {
        let request = manager
            .current_request_mut(HARD_DISK_MAJOR)
            .expect("hard disk request should exist while handling read interrupt");
        request.error_count = 0;
        request.io.data_addr = unsafe {
            core::ptr::NonNull::new_unchecked(request.io.data_addr.as_ptr().add(SECTOR_SIZE))
        };
        request.io.first_sector += 1;
        request.io.sector_count -= 1;
        request.io.sector_count != 0
    });

    if has_more_sectors {
        HARD_DISK_MANAGER.exclusive(|manager| {
            manager.interrupt_phase = InterruptPhase::Read;
        });
        return;
    }

    blk::complete_current_request(HARD_DISK_MAJOR, true);
    handle_request();
}

/// Continue a pending ATA write command.
fn handle_write_interrupt() {
    if !command_succeeded() {
        mark_request_error();
        handle_request();
        return;
    }

    let next_buffer = BLOCK_MANAGER.exclusive(|manager| {
        let request = manager
            .current_request_mut(HARD_DISK_MAJOR)
            .expect("hard disk request should exist while handling write interrupt");
        request.io.sector_count -= 1;
        if request.io.sector_count == 0 {
            return None;
        }

        request.io.first_sector += 1;
        request.io.data_addr = unsafe {
            core::ptr::NonNull::new_unchecked(request.io.data_addr.as_ptr().add(SECTOR_SIZE))
        };
        Some(request.io.data_addr.cast::<u16>())
    });

    let Some(next_buffer) = next_buffer else {
        blk::complete_current_request(HARD_DISK_MAJOR, true);
        handle_request();
        return;
    };

    HARD_DISK_MANAGER.exclusive(|manager| {
        manager.interrupt_phase = InterruptPhase::Write;
    });
    port_write_words(
        controller::DATA_PORT,
        next_buffer.as_ptr(),
        SECTOR_WORD_COUNT,
    );
}

/// Continue a pending ATA recalibration or specify sequence.
fn handle_recalibration_interrupt() {
    (!command_succeeded()).then(mark_request_error);
    handle_request();
}
