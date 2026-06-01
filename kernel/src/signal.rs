//! Signal delivery for returning to user mode.
//!
//! When a system call or timer interrupt is about to return to Ring 3,
//! [`handle_pending_signal`] checks for pending unblocked signals and, if
//! one is found, pushes a signal frame onto the user stack so the handler
//! runs before the interrupted code resumes.
//!
//! ```text
//! User stack after delivery (growing downward):
//!
//!   restorer | signal_number | [blocked] | eax | ecx | edx | eflags | old_eip
//! ```

use user_lib::syscall::signal::{NSIG, SigFlags, SigHandler, Signal};

use crate::{mm, segment::uaccess, task};

/// Checks for one pending unblocked signal and delivers it before returning
/// to user mode.
pub fn handle_pending_signal(frame: &mut dyn SignalDeliveryFrame) {
    if !frame.is_returning_to_user() {
        return;
    }

    let action = task::with_current(|inner| {
        let pending = inner.signal_info.signal & !inner.signal_info.blocked;
        if pending == 0 {
            return PendingSignalAction::None;
        }

        let bit = pending.trailing_zeros() as usize;
        if bit >= NSIG {
            return PendingSignalAction::None;
        }
        inner.signal_info.clear(bit as u32 + 1);
        let signal_number = (bit + 1) as u32;
        let sa = inner.signal_info.sigaction[bit];

        match sa.sa_handler {
            x if x == SigHandler::Ignore as u32 => PendingSignalAction::None,
            x if x == SigHandler::Default as u32 => {
                if signal_number == Signal::Chld as u32 {
                    PendingSignalAction::None
                } else if signal_number == Signal::Stop as u32
                    || signal_number == Signal::Tstp as u32
                {
                    PendingSignalAction::Stop
                } else {
                    PendingSignalAction::Exit { signal_number }
                }
            }
            handler => {
                if (sa.sa_flags & SigFlags::ONE_SHOT.bits()) != 0 {
                    inner.signal_info.sigaction[bit].sa_handler = 0;
                }
                PendingSignalAction::Deliver(DeliverAction {
                    handler,
                    restorer: sa.sa_restorer,
                    signal_number,
                    blocked: inner.signal_info.blocked,
                    sa_flags: sa.sa_flags,
                    sa_mask: sa.sa_mask,
                })
            }
        }
    });

    match action {
        PendingSignalAction::None => {}
        PendingSignalAction::Stop => {
            task::with_current(|inner| inner.sched.state = task::TaskState::Stopped);
            task::schedule();
        }
        PendingSignalAction::Exit { signal_number } => {
            task::exit_process(1 << (signal_number - 1));
        }
        PendingSignalAction::Deliver(deliver) => {
            if frame.deliver_signal(deliver) {
                task::with_current(|inner| {
                    inner.signal_info.blocked |= deliver.sa_mask;
                });
            }
        }
    }
}

/// Builds the user-space signal frame on the user stack via the FS segment.
///
/// Returns the updated ESP pointing to the start of the frame. The frame
/// layout (top to bottom) is:
///
/// ```text
/// restorer | signal_number | [blocked] | eax | ecx | edx | eflags | old_eip
/// ```
///
/// The `blocked` slot is omitted when `SA_NOMASK` is set.
pub fn push_user_signal_frame(
    user_esp: u32,
    restorer: u32,
    signal_number: u32,
    blocked: u32,
    sa_flags: u32,
    regs: SignalSavedRegisters,
) -> u32 {
    let has_nomask = (sa_flags & SigFlags::NO_MASK.bits()) != 0;
    let frame_words = if has_nomask { 7u32 } else { 8u32 };
    let new_esp = user_esp.wrapping_sub(frame_words * 4);

    mm::ensure_user_area_writable(new_esp, (frame_words * 4) as usize);

    let mut sp = new_esp as *mut u32;
    let mut push = |val: u32| {
        uaccess::write_u32(val, sp);
        sp = sp.wrapping_add(1);
    };

    push(restorer);
    push(signal_number);
    if !has_nomask {
        push(blocked);
    }
    push(regs.eax);
    push(regs.ecx);
    push(regs.edx);
    push(regs.eflags);
    // Last word — no advance needed.
    uaccess::write_u32(regs.old_eip, sp);

    new_esp
}

/// Caller-saved registers included in the user-space signal frame.
///
/// Pushed onto the user stack before the handler runs and restored by the
/// sigreturn path so the interrupted code resumes with correct register state.
#[derive(Clone, Copy)]
pub struct SignalSavedRegisters {
    /// Saved `EAX`.
    pub eax: u32,
    /// Saved `ECX`.
    pub ecx: u32,
    /// Saved `EDX`.
    pub edx: u32,
    /// Saved `EFLAGS`.
    pub eflags: u32,
    /// Instruction pointer the handler returns to.
    pub old_eip: u32,
}

/// Parameters for delivering a single signal to user space.
#[derive(Clone, Copy)]
pub struct DeliverAction {
    /// User-space address of the signal handler.
    pub handler: u32,
    /// User-space address of the sigreturn trampoline.
    pub restorer: u32,
    /// One-based signal number being delivered.
    pub signal_number: u32,
    /// Signal mask in effect before delivery.
    pub blocked: u32,
    /// `sa_flags` from the installed `sigaction`.
    pub sa_flags: u32,
    /// Additional signals to block while the handler runs.
    pub sa_mask: u32,
}

/// Implemented by interrupt/syscall return frames that support signal delivery.
///
/// Both `SyscallContext` and `TimerFrame` implement this trait so that
/// [`handle_pending_signal`] can inject a signal frame regardless of the
/// return path.
pub trait SignalDeliveryFrame {
    /// Returns `true` if this frame is about to return to ring 3.
    fn is_returning_to_user(&self) -> bool;
    /// Injects a signal frame for `action`, returning `true` on success.
    fn deliver_signal(&mut self, action: DeliverAction) -> bool;
}

/// Outcome of inspecting the current task's pending signals.
enum PendingSignalAction {
    /// Nothing to deliver.
    None,
    /// Run the user-supplied handler described by the action.
    Deliver(DeliverAction),
    /// Stop the task and reschedule.
    Stop,
    /// Terminate the task with the given fatal signal number.
    Exit { signal_number: u32 },
}
