/* Licensed under the Apache-2.0 license */

OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    RAM (rwx) : ORIGIN = 0x00000000, LENGTH = 8K
}

SECTIONS
{
    .text : {
        KEEP(*(.text.init))
        *(.text .text.*)
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    } > RAM
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    } > RAM
    .bss : {
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
    } > RAM
    _stack_top = ORIGIN(RAM) + LENGTH(RAM);
}
