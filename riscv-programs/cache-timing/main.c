#include <builtin.h>

#define CACHE_LINE_BYTES 64
#define PROBE_BYTES 256
/*
 * Keep this comfortably larger than LLC so cold probes are forced to main memory.
 * Current machine config is 8 MiB L3, so use 32 MiB.
 */
#define THRASH_BYTES (32 * 1024 * 1024)

static volatile unsigned char probe[PROBE_BYTES] __attribute__((aligned(CACHE_LINE_BYTES)));
static volatile unsigned char thrash_buf[THRASH_BYTES] __attribute__((aligned(CACHE_LINE_BYTES)));
static volatile unsigned char sink; // Just here to prevent compiler optimizations

static void debug_assert(int cond, const char *msg)
{
	if (!cond) {
		builtin_printf("Assert failed: %s\n", msg);
		builtin_exit();
	}
}

static void thrash_caches(void)
{
	unsigned i;
	unsigned char acc = 0;
	for (i = 0; i < THRASH_BYTES; i += CACHE_LINE_BYTES)
		acc ^= thrash_buf[i];
	sink = acc;
}

static int timed_load(volatile unsigned char *p)
{
	uint64_t t0 = builtin_cycle_count();
	unsigned char v = *p;
	uint64_t t1 = builtin_cycle_count();
	sink ^= v;
	return (int)(t1 - t0);
}

int main(void)
{
	int i;
	for (i = 0; i < PROBE_BYTES; i++)
		probe[i] = (unsigned char)i;
	for (i = 0; i < THRASH_BYTES; i += CACHE_LINE_BYTES)
		thrash_buf[i] = (unsigned char)(i & 0xff);

	builtin_printf("Cache timing test\n");

	thrash_caches();
	int cold = timed_load(&probe[0]);
	int hot = timed_load(&probe[0]);
	builtin_printf("cold probe[0]: %d cycles\n", cold);
	builtin_printf("hot  probe[0]: %d cycles\n", hot);
	debug_assert(cold > hot, "cold access should be slower than hot L1 hit");

	thrash_caches();
	int first_line_miss = timed_load(&probe[0]);
	int same_line_hit = timed_load(&probe[CACHE_LINE_BYTES - 4]);
	int next_line = timed_load(&probe[CACHE_LINE_BYTES]);
	builtin_printf("first line load probe[0]: %d cycles\n", first_line_miss);
	builtin_printf("same line probe[60]:      %d cycles\n", same_line_hit);
	builtin_printf("next line probe[64]:      %d cycles\n", next_line);

	debug_assert(first_line_miss > same_line_hit, "same-line access should hit after fill");
	debug_assert(next_line > same_line_hit, "next cache line should be slower than same-line hit");

	builtin_printf("Cache timing checks passed.\n");
	return 0;
}
