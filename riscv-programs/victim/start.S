.section .text.init
.global _start

_start:
    /* Stack at 0x300080 so (addr/64)%64 == 2; target is set 1, avoid evicting on return. */
    li sp, 0x300080
    call main
    ebreak
