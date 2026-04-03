OUTPUT_ARCH(i386)
ENTRY(_start)

SECTIONS
{
    . = 0x00000000;

    /* Text segment — code + read-only data (a.out a_text). */
    .text :
    {
        *(.text .text.*)
        *(.rodata .rodata.*)
    }

    /* Page-align the text/data boundary for demand paging. */
    . = ALIGN(4096);
    _etext = .;

    /* Data segment — initialized writable data (a.out a_data). */
    .data :
    {
        *(.data .data.*)
    }
    _edata = .;

    /* BSS — uninitialized data, zero-filled at load time (a.out a_bss). */
    .bss :
    {
        *(.bss .bss.* COMMON)
    }
    . = ALIGN(4);
    _end = .;

    /DISCARD/ :
    {
        *(.eh_frame*)
        *(.note*)
        *(.comment*)
    }
}
