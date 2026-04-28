#ifndef BUILTIN_H

#include <stdint.h>

// Needs to not be inlined so the registers are set up corectly to be used by the printf builtin
static void __attribute__((noinline, noclone)) builtin_printf(const char *fmt, ...)
{
	(void)fmt;
	__asm__ volatile (
		"li a7, 1\n"
		"ecall\n"
		:: : "a7"
	);
}

static inline void builtin_exit(void)
{
	__asm__ volatile ("ebreak" :: :);
}

static inline uint64_t builtin_cycle_count(void)
{
	register uint32_t lo __asm__("a0");
	register uint32_t hi __asm__("a1");
	__asm__ volatile (
		"li a7, 2\n"
		"ecall\n"
		: "=r"(lo), "=r"(hi)
		:
		: "a7"
	);
	return ((uint64_t)hi << 32) | lo;
}

static inline uint32_t builtin_random(void)
{
	register uint32_t out __asm__("a0");
	__asm__ volatile (
		"li a7, 3\n"
		"ecall\n"
		: "=r"(out)
		:
		: "a7"
	);
	return out;
}

static inline void builtin_yield(void)
{
	__asm__ volatile (
		"li a7, 4\n"
		"ecall\n"
		:
		:
		: "a7"
	);
}

static inline uint32_t builtin_modulo(uint32_t dividend, uint32_t divisor)
{
	register uint32_t dividend_reg __asm__("a0") = dividend;
	register uint32_t divisor_reg __asm__("a1") = divisor;
	__asm__ volatile (
		"li a7, 5\n"
		"ecall\n"
		: "+r"(dividend_reg)
		: "r"(divisor_reg)
		: "a7"
	);
	return dividend_reg;
}

#define EXP_PHASE_HALT 0U
#define EXP_PHASE_PRIME 1U
#define EXP_PHASE_PROBE 2U
#define EXP_PHASE_VICTIM_ACCESS 3U
#define EXP_PHASE_WARM_VICTIM 4U
#define EXP_PHASE_EVICT_TARGET 5U

static inline uint32_t exp_get_phase(void)
{
	register uint32_t out __asm__("a0");
	__asm__ volatile (
		"li a7, 16\n"
		"ecall\n"
		: "=r"(out)
		:
		: "a7"
	);
	return out;
}

static inline uint32_t exp_get_secret_set(void)
{
	register uint32_t out __asm__("a0");
	__asm__ volatile (
		"li a7, 17\n"
		"ecall\n"
		: "=r"(out)
		:
		: "a7"
	);
	return out;
}

static inline uint32_t exp_get_secret_bit(void)
{
	register uint32_t out __asm__("a0");
	__asm__ volatile (
		"li a7, 18\n"
		"ecall\n"
		: "=r"(out)
		:
		: "a7"
	);
	return out;
}

static inline uint32_t exp_get_target_set(void)
{
	register uint32_t out __asm__("a0");
	__asm__ volatile (
		"li a7, 19\n"
		"ecall\n"
		: "=r"(out)
		:
		: "a7"
	);
	return out;
}

static inline void exp_submit_scalar(uint32_t index, uint64_t value)
{
	register uint32_t index_reg __asm__("a0") = index;
	register uint32_t lo __asm__("a1") = (uint32_t)value;
	register uint32_t hi __asm__("a2") = (uint32_t)(value >> 32);
	__asm__ volatile (
		"li a7, 21\n"
		"ecall\n"
		:
		: "r"(index_reg), "r"(lo), "r"(hi)
		: "a7"
	);
}

static inline void exp_done(void)
{
	__asm__ volatile (
		"li a7, 22\n"
		"ecall\n"
		:
		:
		: "a7"
	);
}

#define BUILTIN_H
#endif
