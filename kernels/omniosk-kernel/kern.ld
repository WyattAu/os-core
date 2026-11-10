/* omniosk-kernel link script (ADR-0013): load at 1 MiB, multiboot header
   first (GRUB-compatible), boot trampoline before general text. */
ENTRY(_start)

SECTIONS
{
    . = 1M;

    .multiboot : {
        KEEP(*(.multiboot))
    }

    .text : {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    .data : {
        *(.data .data.*)
    }

    .bss : {
        __bss_start = .;
        *(COMMON)
        *(.bss .bss.*)
        __bss_end = .;
        __bss_size = (__bss_end - __bss_start + 3) & ~3;
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
    }
}
