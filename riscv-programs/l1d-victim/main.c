#include <attack_layout.h>
#include <builtin.h>
#include <stdint.h>

#define VICTIM_LINES_PER_SECRET 1U
#define VICTIM_BUFFER_BYTES ((VICTIM_LINES_PER_SECRET * SET_STRIDE) + (NUM_SETS * LINE_SIZE))

static volatile uint8_t victim_buf[VICTIM_BUFFER_BYTES] __attribute__((aligned(SET_STRIDE)));

static inline void consume_u32(uint32_t value)
{
	__asm__ volatile ("" : "+r"(value));
}

static inline volatile uint8_t *victim_line(uint32_t set, uint32_t index)
{
	return victim_buf + (set * LINE_SIZE) + (index * SET_STRIDE);
}

static void init_secret_lines(void)
{
	for (uint32_t set = 0; set < NUM_SETS; set++) {
		for (uint32_t i = 0; i < VICTIM_LINES_PER_SECRET; i++) {
			*victim_line(set, i) = (uint8_t)(set + i);
		}
	}
}

static void victim_access_secret_set(uint32_t secret_set)
{
	uint32_t acc = 0;
	for (uint32_t i = 0; i < VICTIM_LINES_PER_SECRET; i++) {
		acc ^= *victim_line(secret_set, i);
	}
	consume_u32(acc);
}

static void victim_warm_table(void)
{
	for (uint32_t set = 0; set < NUM_SETS; set++) {
		victim_access_secret_set(set);
	}
}

int main(void)
{
	init_secret_lines();
	exp_done();

	for (;;) {
		uint32_t phase = exp_get_phase();
		if (phase == EXP_PHASE_HALT) {
			exp_done();
			return 0;
		}
		if (phase == EXP_PHASE_WARM_VICTIM) {
			victim_warm_table();
			exp_done();
			continue;
		}
		if (phase == EXP_PHASE_VICTIM_ACCESS) {
			victim_access_secret_set(exp_get_secret_set());
			exp_done();
			continue;
		}
		exp_done();
	}
}
