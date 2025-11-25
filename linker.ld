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

    .data :
    {
        *(.data .data.*)
    }

    .bss :
    {
        *(.bss .bss.* COMMON)
    }
}
