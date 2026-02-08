/// System call context — the register frame built on the kernel stack
/// by `syscall_entry.s` before calling into Rust.
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
/// | EAX (syscall nr)   |  0x00
/// | EBX (arg1)         |  0x04
/// | ECX (arg2)         |  0x08
/// | EDX (arg3)         |  0x0C
/// | FS                 |  0x10
/// | ES                 |  0x14
/// | DS                 |  0x18
/// +--------------------+
/// | EIP                |  0x1C  ← pushed by CPU
/// | CS                 |  0x20
/// | EFLAGS             |  0x24
/// | ESP (user)         |  0x28  ← pushed by CPU (privilege change)
/// | SS  (user)         |  0x2C
/// +--------------------+ <- High address
/// ```
#[repr(C)]
#[derive(Debug)]
pub struct SyscallContext {
    // --- pushed by our assembly stub (low address first) ---
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
