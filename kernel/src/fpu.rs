//! Lazy x87 FPU context switching.
//!
//! The FPU register file is large and most tasks never touch it, so the kernel
//! never saves or restores it eagerly. Instead a single task — the *owner* —
//! keeps its live state in the hardware registers, and `CR0.TS` is left set for
//! everyone else. The first floating-point instruction a non-owner executes
//! raises a #NM (device-not-available) exception, at which point the previous
//! owner's state is flushed to its PCB and the current task's state is loaded.
//!
//! Each task carries its register snapshot in [`FpuContext`] inside its PCB
//! (not the hardware TSS, which stays a pure CPU-defined structure). Ownership
//! is tracked by a single raw pointer mirroring the `current`-task scheme.

use alloc::sync::Arc;
use core::{
    arch::asm,
    ptr::null_mut,
    sync::atomic::{AtomicPtr, Ordering},
};

use user_lib::syscall::signal::Signal;

use crate::{sync::IrqSaveGuard, task, task::task_struct::Task};

/// Per-task x87 register snapshot, stored in the PCB.
///
/// Holds the 108-byte `FNSAVE`/`FRSTOR` register image plus a flag recording
/// whether the task has ever used the FPU, which decides whether its first
/// fault loads a saved image or a freshly initialized one.
#[derive(Clone, Copy, Default)]
pub struct FpuContext {
    regs: I387,
    used: bool,
}

/// Marks the boot CPU as having no live FPU owner with `CR0.TS` set, so the
/// first floating-point instruction in user space faults into [`switch_in`].
pub fn init() {
    unsafe { stts() };
}

/// Handles a #NM exception: hands the FPU to the current task on demand.
///
/// Clears `CR0.TS` so floating-point instructions run, then — unless the
/// current task already owns the live state — flushes the previous owner's
/// registers to its PCB and loads (or initializes) the current task's.
pub fn switch_in() {
    let _irq = IrqSaveGuard::enter();
    unsafe { clts() };

    // No hardware FPU present: deliver SIGFPE instead of emulating.
    if read_cr0() & CR0_EM != 0 {
        task::with_current(|inner| inner.signal_info.raise(Signal::Fpe as u32));
        return;
    }

    let current = task::current_task();
    let current_ptr = Arc::as_ptr(&current).cast_mut();
    let owner = FPU_OWNER.load(Ordering::Acquire);
    if owner == current_ptr {
        return;
    }

    unsafe {
        fwait();
        if !owner.is_null() {
            // SAFETY: a non-null owner pointer always refers to a live task
            // kept alive by the task table; it is cleared on exec/exit.
            (*owner)
                .pcb
                .inner
                .exclusive(|prev| fnsave(&mut prev.fpu.regs));
        }
    }

    FPU_OWNER.store(current_ptr, Ordering::Release);
    current.pcb.inner.exclusive(|inner| unsafe {
        if inner.fpu.used {
            frstor(&inner.fpu.regs);
        } else {
            fninit();
            inner.fpu.used = true;
        }
    });
}

/// Clears `CR0.TS` when a just-resumed task still owns the live FPU state,
/// sparing it a redundant #NM fault on its next floating-point instruction.
///
/// Called right after a hardware task switch resumes the current task.
pub fn on_resume() {
    let current = task::current_task();
    if FPU_OWNER.load(Ordering::Acquire) == Arc::as_ptr(&current).cast_mut() {
        unsafe { clts() };
    }
}

/// Returns a snapshot of the current task's FPU state for a forked child.
///
/// If the caller owns the live registers they are flushed to its PCB and
/// immediately reloaded, so the parent keeps ownership while the child
/// inherits an identical copy.
pub fn snapshot_current() -> FpuContext {
    let _irq = IrqSaveGuard::enter();
    let current = task::current_task();
    let is_owner = FPU_OWNER.load(Ordering::Acquire) == Arc::as_ptr(&current).cast_mut();
    current.pcb.inner.exclusive(|inner| {
        if is_owner {
            unsafe {
                clts();
                fnsave(&mut inner.fpu.regs);
                frstor(&inner.fpu.regs);
            }
        }
        inner.fpu
    })
}

/// Discards the current task's FPU state across `execve`.
pub fn reset_for_exec() {
    abandon(Arc::as_ptr(&task::current_task()).cast_mut());
    task::with_current(|inner| inner.fpu.used = false);
}

/// Releases FPU ownership when the current task exits, so the stale pointer is
/// never dereferenced after its PCB is freed.
pub fn abandon_on_exit() {
    abandon(Arc::as_ptr(&task::current_task()).cast_mut());
}

/// Handles a floating-point error (#MF / IRQ13) by clearing the exception and
/// posting SIGFPE to the task whose computation faulted.
pub fn handle_error() {
    let _irq = IrqSaveGuard::enter();
    unsafe { fnclex() };
    let owner = FPU_OWNER.load(Ordering::Acquire);
    if !owner.is_null() {
        // SAFETY: see `switch_in`; the owner pointer is always live here.
        unsafe {
            (*owner)
                .pcb
                .inner
                .exclusive(|inner| inner.signal_info.raise(Signal::Fpe as u32));
        }
    }
}

/// Drops FPU ownership if `task_ptr` currently holds it.
fn abandon(task_ptr: *mut Task) {
    let _ = FPU_OWNER.compare_exchange(task_ptr, null_mut(), Ordering::AcqRel, Ordering::Acquire);
}

/// Raw pointer to the task whose FPU state is live in the hardware registers,
/// or null when no task owns it. Mirrors the `current`-task pointer scheme:
/// the task table holds the owning `Arc`, and ownership is cleared on the
/// exec/exit paths before the referenced task can be freed.
static FPU_OWNER: AtomicPtr<Task> = AtomicPtr::new(null_mut());

/// `CR0.EM` — math emulation (set when no hardware FPU is present).
const CR0_EM: u32 = 1 << 2;
/// `CR0.TS` — task switched (a floating-point instruction faults while set).
const CR0_TS: u32 = 1 << 3;

/// x87 register file in `FNSAVE`/`FRSTOR` layout: the control/status/tag words,
/// the instruction and operand pointers, and the eight 80-bit stack registers.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct I387 {
    cwd: u32,
    swd: u32,
    twd: u32,
    fip: u32,
    fcs: u32,
    foo: u32,
    fos: u32,
    st_space: [u32; 20],
}

#[inline]
fn read_cr0() -> u32 {
    let cr0: u32;
    unsafe { asm!("movl %cr0, {0}", out(reg) cr0, options(att_syntax, nomem, nostack)) };
    cr0
}

/// Sets `CR0.TS`.
#[inline]
unsafe fn stts() {
    let cr0 = read_cr0() | CR0_TS;
    unsafe { asm!("movl {0}, %cr0", in(reg) cr0, options(att_syntax, nomem, nostack)) };
}

/// Clears `CR0.TS`.
#[inline]
unsafe fn clts() {
    unsafe { asm!("clts", options(att_syntax, nomem, nostack)) };
}

/// Reinitializes the FPU to a known, empty state.
#[inline]
unsafe fn fninit() {
    unsafe { asm!("fninit", options(att_syntax, nomem, nostack)) };
}

/// Waits for any pending floating-point exception to be reported.
#[inline]
unsafe fn fwait() {
    unsafe { asm!("fwait", options(att_syntax, nomem, nostack)) };
}

/// Clears pending floating-point exception flags without raising #MF.
#[inline]
unsafe fn fnclex() {
    unsafe { asm!("fnclex", options(att_syntax, nomem, nostack)) };
}

/// Saves the FPU register file into `regs` (and reinitializes the FPU).
#[inline]
unsafe fn fnsave(regs: &mut I387) {
    unsafe { asm!("fnsave ({0})", in(reg) regs as *mut I387, options(att_syntax, nostack)) };
}

/// Restores the FPU register file from `regs`.
#[inline]
unsafe fn frstor(regs: &I387) {
    unsafe { asm!("frstor ({0})", in(reg) regs as *const I387, options(att_syntax, nostack)) };
}
