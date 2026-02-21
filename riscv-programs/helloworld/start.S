/* start.S */
.section .text.init
.global _start

_start:
    /* 1. Setup Stack Pointer to 1MB (Adjust if simulator RAM is smaller) */
    li sp, 0x100000

    /* 2. Jump to C main() function */
    call main

    /* 3. Termination Loop */
    /* Only reached if main returns. We use ebreak to tell simulator to quit. */
exit_loop:
    ebreak
    j exit_loop
