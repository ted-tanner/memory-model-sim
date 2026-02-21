.section .text.init
.global _start

_start:
    li sp, 0x300000
    call main
    ebreak
