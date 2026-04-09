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
//!   restorer | signr | [blocked] | eax | ecx | edx | eflags | old_eip
//! ```

pub use user_lib::process::*;

use crate::{mm, segment::uaccess, task};

/// Caller-saved registers included in the user-space signal frame.
///
/// Pushed onto the user stack before the handler runs and restored by the
/// sigreturn path so the interrupted code resumes with correct register state.
#[derive(Clone, Copy)]
pub struct SignalSavedRegisters {
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub eflags: u32,
    pub old_eip: u32,
}

/// Parameters for delivering a single signal to user space.
#[derive(Clone, Copy)]
pub struct DeliverAction {
    pub handler: u32,
    pub restorer: u32,
    pub signr: u32,
    pub blocked: u32,
    pub sa_flags: u32,
    pub sa_mask: u32,
}

/// Implemented by interrupt/syscall return frames that support signal delivery.
///
/// Both `SyscallContext` and `TimerFrame` implement this trait so that
/// [`handle_pending_signal`] can inject a signal frame regardless of the
/// return path.
pub trait SignalDeliveryFrame {
    fn is_returning_to_user(&self) -> bool;
    fn deliver_signal(&mut self, action: DeliverAction) -> bool;
}

enum PendingSignalAction {
    None,
    Deliver(DeliverAction),
    Exit { signr: u32 },
}

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
        let signr = (bit + 1) as u32;
        let sa = inner.signal_info.sigaction[bit];

        match sa.sa_handler {
            SIG_IGN => PendingSignalAction::None,
            SIG_DFL => {
                if signr == SIGCHLD {
                    PendingSignalAction::None
                } else {
                    PendingSignalAction::Exit { signr }
                }
            }
            handler => {
                if (sa.sa_flags & SA_ONESHOT) != 0 {
                    inner.signal_info.sigaction[bit].sa_handler = 0;
                }
                PendingSignalAction::Deliver(DeliverAction {
                    handler,
                    restorer: sa.sa_restorer,
                    signr,
                    blocked: inner.signal_info.blocked,
                    sa_flags: sa.sa_flags,
                    sa_mask: sa.sa_mask,
                })
            }
        }
    });

    match action {
        PendingSignalAction::None => {}
        PendingSignalAction::Exit { signr } => task::do_exit(1 << (signr - 1)),
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
/// restorer | signr | [blocked] | eax | ecx | edx | eflags | old_eip
/// ```
///
/// The `blocked` slot is omitted when `SA_NOMASK` is set.
pub fn push_user_signal_frame(
    user_esp: u32,
    restorer: u32,
    signr: u32,
    blocked: u32,
    sa_flags: u32,
    regs: SignalSavedRegisters,
) -> u32 {
    let has_nomask = (sa_flags & SA_NOMASK) != 0;
    let frame_words = if has_nomask { 7u32 } else { 8u32 };
    let new_esp = user_esp.wrapping_sub(frame_words * 4);

    mm::ensure_user_area_writable(new_esp, (frame_words * 4) as usize);

    let mut sp = new_esp as *mut u32;
    let mut push = |val: u32| {
        uaccess::write_u32(val, sp);
        sp = sp.wrapping_add(1);
    };

    push(restorer);
    push(signr);
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
