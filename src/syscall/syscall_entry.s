# System call entry point — invoked by `int 0x80` from user mode.
#
# Saves user-mode registers onto the kernel stack to form a `SyscallContext`
# frame (see context.rs), then calls `syscall_rust_entry(ctx: &SyscallContext)`
# which handles syscall-number validation, dispatch, and returns the result
# in %eax (cdecl).
#
# The return value is written back into the saved-EAX slot on the stack so
# that `popl %eax` restores the syscall result to user mode.

.globl system_call
system_call:
    # Save user segment registers & GPRs to build a SyscallContext frame.
    push %ds
    push %es
    push %fs
    pushl %edx
    pushl %ecx
    pushl %ebx
    pushl %eax              # syscall number — completes the SyscallContext frame

    # Switch to kernel data segments.
    movl $0x10, %edx        # 0x10 = kernel data segment selector
    mov %dx, %ds
    mov %dx, %es
    movl $0x17, %edx        # 0x17 = user data segment (for fs — accessing user space)
    mov %dx, %fs

    # Call into Rust with a pointer to the SyscallContext (%esp) as first arg.
    movl %esp, %eax
    pushl %eax
    call syscall_rust_entry
    addl $4, %esp           # clean up the pushed argument

    # Overwrite saved EAX with the return value so popl %eax picks it up.
    movl %eax, 0(%esp)

    # Restore registers & return to user mode.
    popl %eax               # syscall return value
    popl %ebx
    popl %ecx
    popl %edx
    pop %fs
    pop %es
    pop %ds
    iret
