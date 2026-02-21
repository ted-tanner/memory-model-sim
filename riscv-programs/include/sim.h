#ifndef SIM_H

/*
 * memory-model-sim host ABI (ecall with a7 = builtin number):
 *
 * a7 = 1 (printf): a0 = format string pointer, a1..a7 = variadic args.
 *                  Supports %d (signed int) and %s (char*). No libc.
 */

static inline void sim_printf(const char *fmt)
{
	__asm__ volatile (
		"mv a0, %0\n"
		"li a7, 1\n"
		"ecall\n"
		: : "r"(fmt) : "a0", "a7"
	);
}

#define SIM_H
#endif
