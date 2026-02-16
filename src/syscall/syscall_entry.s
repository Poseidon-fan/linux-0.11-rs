# System call entry point — invoked by `int 0x80` from user mode.
#
# Saves all user-mode registers onto the kernel stack to form a
# `SyscallContext` frame (see context.rs), then calls
# `syscall_rust_entry(ctx: &SyscallContext)` which dispatches to the
# appropriate handler and returns the result in %eax (cdecl).
#
# The return value is written back into the saved-EAX slot on the stack
# so that `popl %eax` restores the syscall result to user mode.

.globl system_call
system_call:
    # Save user segment registers & GPRs to build a SyscallContext frame.
    # The push order (high → low address) must mirror the struct field order.
    push %ds
    push %es
    push %fs
    pushl %edx
    pushl %ecx
    pushl %ebx
    pushl %eax              # syscall number

    # Also capture callee-saved registers so that fork/exec can read them
    # from SyscallContext to populate the child's TSS.
    pushl %ebp
    pushl %edi
    pushl %esi
    push %gs

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
    # EAX slot is at offset 16 from ESP (skip gs, esi, edi, ebp = 4×4).
    movl %eax, 16(%esp)

    # Restore registers & return to user mode.
    # Skip gs/esi/edi/ebp — they are callee-saved and still hold the
    # original user-mode values after the Rust call returns.
    addl $16, %esp
    popl %eax               # syscall return value
    popl %ebx
    popl %ecx
    popl %edx
    pop %fs
    pop %es
    pop %ds
    iret
