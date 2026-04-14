.section .text.init
.global _start

_start:
    li sp, 0x80040080
    call main
    ebreak
