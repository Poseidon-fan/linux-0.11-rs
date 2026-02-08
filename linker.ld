OUTPUT_ARCH(i386)
ENTRY(startup_32)

SECTIONS
{
    . = 0x00000000;
    .head :
    {
        *(.head)
    }

    .text :
    {
        *(.text .text.*)
    }

    .rodata :
    {
        *(.rodata .rodata.*)
    }

    /*
     * With `#[distributed_slice(SYSCALL_TABLE, N)]`, elements are placed into
     * input sections like `linkme_SYSCALL_TABLE0000`. linkme computes the slice
     * boundaries using `__start_linkme_SYSCALL_TABLE` / `__stop_linkme_SYSCALL_TABLE`,
     * which correspond to an output section named `linkme_SYSCALL_TABLE`.
     *
     * So we must collect `linkme_SYSCALL_TABLE*` into an output section named
     * exactly `linkme_SYSCALL_TABLE`, and KEEP it from GC.
     */
    . = ALIGN(4);
    linkme_SYSCALL_TABLE :
    {
        KEEP(*(linkme_SYSCALL_TABLE*))
    }

    /* Optional but recommended: keep dupcheck section used by linkme. */
    linkm2_SYSCALL_TABLE :
    {
        KEEP(*(linkm2_SYSCALL_TABLE*))
    }

    .data :
    {
        *(.data .data.*)
    }

    .bss :
    {
        *(.bss .bss.* COMMON)
    }

    ekernel = .;
}
