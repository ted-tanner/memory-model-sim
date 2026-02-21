/* start.S */
.section .text.init
.global _start

_start:
    li sp, 0x300000
    call main
exit_loop:
    ebreak
    j exit_loop
