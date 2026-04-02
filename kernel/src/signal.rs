//! Signal support and delivery.
//!
//! This module contains the shared signal handling flow and the return-frame
//! abstraction used by syscall/timer return paths.

use crate::{mm, segment::uaccess, task, task::task_struct::NSIG};

/// SA_NOMASK flag bit.
pub const SA_NOMASK: u32 = 0x4000_0000;
/// SA_ONESHOT flag bit.
pub const SA_ONESHOT: u32 = 0x8000_0000;

const SIG_DFL: u32 = 0;
const SIG_IGN: u32 = 1;
/// Hangup signal (sent to session when leader exits).
pub const SIGHUP: u32 = 1;
/// Cannot be caught or ignored.
pub const SIGKILL: u32 = 9;
/// Invalid memory reference signal number.
pub const SIGSEGV: u32 = 11;
/// Alarm clock signal number.
pub const SIGALRM: u32 = 14;
/// Child status changed signal number.
pub const SIGCHLD: u32 = 17;
/// Stop signal (cannot be caught or ignored).
pub const SIGSTOP: u32 = 19;

/// Saved register subset required by the user signal frame layout.
#[derive(Clone, Copy)]
pub struct SignalSavedRegisters {
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub eflags: u32,
    pub old_eip: u32,
}

/// A complete signal delivery request.
#[derive(Clone, Copy)]
pub struct DeliverAction {
    pub handler: u32,
    pub restorer: u32,
    pub signr: u32,
    pub blocked: u32,
    pub sa_flags: u32,
    pub sa_mask: u32,
}

/// Behavior required by return frames that can receive signal delivery.
pub trait SignalDeliveryFrame {
    /// Returns true when this frame will return to user mode.
    fn is_returning_to_user(&self) -> bool;

    /// Pushes one signal frame to user stack and updates return state.
    ///
    /// Returns `true` when the frame is successfully updated.
    fn deliver_signal(&mut self, action: DeliverAction) -> bool;
}

enum PendingSignalAction {
    None,
    Deliver(DeliverAction),
    Exit { signr: u32 },
}

/// Handles one pending unblocked signal before returning to user mode.
pub fn handle_pending_signal(frame: &mut dyn SignalDeliveryFrame) {
    if !frame.is_returning_to_user() {
        return;
    }

    let action = task::current_task().pcb.inner.exclusive(|inner| {
        let pending = inner.signal_info.signal & !inner.signal_info.blocked;
        if pending == 0 {
            return PendingSignalAction::None;
        }

        let bit = pending.trailing_zeros() as usize;
        if bit >= NSIG {
            return PendingSignalAction::None;
        }
        inner.signal_info.signal &= !(1u32 << bit);
        let signr = (bit + 1) as u32;
        let sa = inner.signal_info.sigaction[bit].clone();

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
        PendingSignalAction::Exit { signr } => {
            crate::println!("[signal] handle signr={} action=exit", signr);
            task::do_exit(1 << (signr - 1))
        }
        PendingSignalAction::Deliver(deliver) => {
            crate::println!("[signal] handle signr={} action=deliver", deliver.signr);
            if frame.deliver_signal(deliver) {
                task::current_task().pcb.inner.exclusive(|inner| {
                    inner.signal_info.blocked |= deliver.sa_mask;
                });
            }
        }
    }
}

/// Pushes the user-space signal frame via FS segment and returns updated ESP.
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
    let frame_bytes = (frame_words * 4) as usize;
    let new_esp = user_esp.wrapping_sub(frame_words * 4);

    mm::ensure_user_area_writable(new_esp, frame_bytes);

    let mut sp = new_esp as *mut u32;
    uaccess::write_u32(restorer, sp);
    sp = sp.wrapping_add(1);
    uaccess::write_u32(signr, sp);
    sp = sp.wrapping_add(1);

    if !has_nomask {
        uaccess::write_u32(blocked, sp);
        sp = sp.wrapping_add(1);
    }

    uaccess::write_u32(regs.eax, sp);
    sp = sp.wrapping_add(1);
    uaccess::write_u32(regs.ecx, sp);
    sp = sp.wrapping_add(1);
    uaccess::write_u32(regs.edx, sp);
    sp = sp.wrapping_add(1);
    uaccess::write_u32(regs.eflags, sp);
    sp = sp.wrapping_add(1);
    uaccess::write_u32(regs.old_eip, sp);

    new_esp
}
