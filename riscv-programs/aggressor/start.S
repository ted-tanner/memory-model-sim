.section .text.init
.global _start

_start:
    li sp, 0x400080
    call main
    ebreak
