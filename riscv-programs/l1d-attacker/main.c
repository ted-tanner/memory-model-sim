#include <attack_layout.h>
#include <builtin.h>
#include <stdint.h>

#define PROBE_BUFFER_BYTES ((NUM_WAYS * SET_STRIDE) + (NUM_SETS * LINE_SIZE))
#define PRIME_ROUNDS 1U

static volatile uint8_t evict_buf[PROBE_BUFFER_BYTES] __attribute__((aligned(SET_STRIDE)));

static inline void consume_u32(uint32_t value)
{
	__asm__ volatile ("" : "+r"(value));
}

static inline volatile uint8_t *evset_addr(uint32_t set, uint32_t way)
{
	return evict_buf + (set * LINE_SIZE) + (way * SET_STRIDE);
}

static void init_evset(void)
{
	for (uint32_t set = 0; set < NUM_SETS; set++) {
		for (uint32_t way = 0; way < NUM_WAYS; way++) {
			*evset_addr(set, way) = (uint8_t)(set ^ way);
		}
	}
}

static void prime_one_set(uint32_t set)
{
	uint32_t acc = 0;
	for (uint32_t r = 0; r < PRIME_ROUNDS; r++) {
		for (uint32_t way = 0; way < NUM_WAYS; way++) {
			acc ^= *evset_addr(set, way);
		}
	}
	consume_u32(acc);
}

static uint64_t probe_one_set(uint32_t set)
{
	uint32_t acc = 0;
	uint64_t t0 = builtin_cycle_count();
	for (uint32_t way = 0; way < NUM_WAYS; way++) {
		acc ^= *evset_addr(set, way);
	}
	uint64_t t1 = builtin_cycle_count();
	consume_u32(acc);
	return t1 - t0;
}

static uint32_t permuted_set(uint32_t start, uint32_t idx)
{
	return (start + (idx * 17U)) & (NUM_SETS - 1U);
}

static void prime_all_sets(void)
{
	uint32_t start = exp_get_secret_set() & (NUM_SETS - 1U);
	for (uint32_t i = 0; i < NUM_SETS; i++) {
		prime_one_set(permuted_set(start, i));
	}
}

static void probe_all_sets(void)
{
	uint32_t start = (exp_get_secret_set() * 7U + exp_get_target_set()) & (NUM_SETS - 1U);
	for (uint32_t i = 0; i < NUM_SETS; i++) {
		uint32_t set = permuted_set(start, i);
		exp_submit_scalar(set, probe_one_set(set));
	}
}

int main(void)
{
	init_evset();
	exp_done();

	for (;;) {
		uint32_t phase = exp_get_phase();
		if (phase == EXP_PHASE_HALT) {
			exp_done();
			return 0;
		}
		if (phase == EXP_PHASE_PRIME) {
			prime_all_sets();
			exp_done();
			continue;
		}
		if (phase == EXP_PHASE_PROBE) {
			probe_all_sets();
			exp_done();
			continue;
		}
		if (phase == EXP_PHASE_EVICT_TARGET) {
			prime_one_set(exp_get_target_set());
			exp_done();
			continue;
		}
		exp_done();
	}
}
