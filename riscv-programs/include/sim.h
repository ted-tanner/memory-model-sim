#ifndef SIM_H

static void __attribute__((noinline, noclone)) sim_printf(const char *fmt, ...)
{
	(void)fmt;
	__asm__ volatile (
		"li a7, 1\n"
		"ecall\n"
		:: : "a7"
	);
}

static inline void sim_exit(void)
{
	__asm__ volatile ("ebreak" :: :);
}

#define SIM_H
#endif
