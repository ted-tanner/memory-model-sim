.section .text.init
.global _start

_start:
    li sp, 0x80030080
    call main
    ebreak
