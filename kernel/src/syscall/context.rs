use crate::{
    segment::{USER_CS, USER_DS},
    signal::{self, DeliverAction, SignalDeliveryFrame, SignalSavedRegisters},
};

/// System call context — the complete register frame built on the kernel
/// stack by `syscall_entry.s` before calling into Rust.
///
/// Since system calls are always issued from user mode (`int 0x80` with
/// DPL = 3), a privilege-level transition (ring 3 → ring 0) is guaranteed.
/// The CPU therefore **always** pushes `SS` and `ESP` before `EFLAGS`,
/// `CS`, and `EIP`.
///
/// # Stack layout (low address at top)
///
/// ```text
/// +--------------------+ <- ESP when Rust is called (= &SyscallContext)
/// | GS                 |  0x00
/// | ESI                |  0x04
/// | EDI                |  0x08
/// | EBP                |  0x0C
/// | EAX (syscall nr)   |  0x10
/// | EBX (arg1)         |  0x14
/// | ECX (arg2)         |  0x18
/// | EDX (arg3)         |  0x1C
/// | FS                 |  0x20
/// | ES                 |  0x24
/// | DS                 |  0x28
/// +--------------------+
/// | EIP                |  0x2C  ← pushed by CPU
/// | CS                 |  0x30
/// | EFLAGS             |  0x34
/// | ESP (user)         |  0x38  ← pushed by CPU (privilege change)
/// | SS  (user)         |  0x3C
/// +--------------------+ <- High address
/// ```
#[repr(C)]
#[derive(Debug)]
pub struct SyscallContext {
    // --- callee-saved registers, captured for fork/exec child TSS setup ---
    /// GS segment selector (user mode value).
    pub gs: u32,
    /// Source index register.
    pub esi: u32,
    /// Destination index register.
    pub edi: u32,
    /// Base pointer register.
    pub ebp: u32,

    // --- syscall number and arguments ---
    /// Syscall number on entry; overwritten with the return value before `iret`.
    pub eax: u32,
    /// First argument (Linux 0.11 convention: arg1 in EBX).
    pub ebx: u32,
    /// Second argument.
    pub ecx: u32,
    /// Third argument.
    pub edx: u32,
    /// User-data segment selector (`0x17` after setup in asm stub).
    pub fs: u32,
    /// Extra segment selector.
    pub es: u32,
    /// Data segment selector.
    pub ds: u32,

    // --- pushed by CPU on `int 0x80` (always a privilege transition) ---
    /// Instruction pointer to return to.
    pub eip: u32,
    /// Code segment of the caller.
    pub cs: u32,
    /// Flags register.
    pub eflags: u32,
    /// User-mode stack pointer (always present — syscalls come from ring 3).
    pub user_esp: u32,
    /// User-mode stack segment (always present — syscalls come from ring 3).
    pub user_ss: u32,
}

impl SyscallContext {
    /// Returns the syscall number (value of EAX when `int 0x80` was executed).
    #[inline]
    pub fn syscall_nr(&self) -> u32 {
        self.eax
    }

    /// Returns the syscall arguments `(arg1, arg2, arg3)` — i.e. `(EBX, ECX, EDX)`.
    #[inline]
    pub fn args(&self) -> (u32, u32, u32) {
        (self.ebx, self.ecx, self.edx)
    }
}

impl SignalDeliveryFrame for SyscallContext {
    #[inline]
    fn is_returning_to_user(&self) -> bool {
        (self.cs & 0xffff) == USER_CS.as_u32() && (self.user_ss & 0xffff) == USER_DS.as_u32()
    }

    fn deliver_signal(&mut self, action: DeliverAction) -> bool {
        if !self.is_returning_to_user() {
            return false;
        }

        let regs = SignalSavedRegisters {
            eax: self.eax,
            ecx: self.ecx,
            edx: self.edx,
            eflags: self.eflags,
            old_eip: self.eip,
        };
        let new_esp = signal::push_user_signal_frame(
            self.user_esp,
            action.restorer,
            action.signr,
            action.blocked,
            action.sa_flags,
            regs,
        );

        self.user_esp = new_esp;
        self.eip = action.handler;
        true
    }
}
