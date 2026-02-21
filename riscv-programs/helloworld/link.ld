/* link.ld */
OUTPUT_ARCH( "riscv" )
ENTRY( _start )

SECTIONS
{
  /* Start at address 0 */
  . = 0x00000000;

  /* Code Section */
  .text : {
    *(.text.init)   /* Put start.S here first */
    *(.text)        /* Put main.c code here */
  }

  /* Read-only Data (Strings) */
  .rodata : {
    *(.rodata)
  }

  /* Read-Write Data */
  .data : {
    *(.data)
  }

  /* Zero-initialized Data (BSS) */
  /* NOTE: Since we don't have an OS loader, your C code must zero this manually if needed. */
  .bss : {
    *(.bss)
  }

  /* Mark end of memory for stack calculation if needed */
  _end = .;
}
