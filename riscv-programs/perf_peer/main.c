#include <builtin.h>
#include <perf_bench.h>
#include <stdint.h>

static volatile unsigned char set_buf[PEER_SET_BUF_BYTES] __attribute__((aligned(SET_STRIDE)));
static volatile unsigned char stream_buf[STREAM_BUF_BYTES] __attribute__((aligned(CACHE_LINE_BYTES)));
static volatile unsigned char sink;

static void idle_round(void)
{
	sink ^= 1U;
}

static void peer_round(enum bench_scenario scenario, int stressed_phase)
{
	switch (scenario) {
	case SCENARIO_HOT_REUSE_BASELINE:
		if (stressed_phase) {
			touch_stream_lines(stream_buf, LARGE_STREAM_LINES, &sink);
		} else {
			idle_round();
		}
		break;
	case SCENARIO_L1_CONFLICT_PRESSURE:
		if (stressed_phase) {
			touch_set_lines(set_buf, TARGET_SET, L1_NUM_WAYS, &sink);
		} else {
			idle_round();
		}
		break;
	case SCENARIO_BACKCACHE_RESCUE_WINDOW:
		if (stressed_phase) {
			touch_set_lines(set_buf, TARGET_SET, RESCUE_PEER_LINES, &sink);
		} else {
			idle_round();
		}
		break;
	case SCENARIO_BACKCACHE_OVERFLOW:
		if (stressed_phase) {
			touch_set_lines(set_buf, TARGET_SET, OVERFLOW_PEER_LINES, &sink);
		} else {
			idle_round();
		}
		break;
	case SCENARIO_SECDCP_ISOLATION_PRESSURE:
		if (stressed_phase) {
			touch_stream_lines(stream_buf, LARGE_STREAM_LINES, &sink);
		} else {
			idle_round();
		}
		break;
	case SCENARIO_NEWCACHE_TARGET_BREAKDOWN:
		if (stressed_phase) {
			touch_set_lines(set_buf, TARGET_SET, L1_NUM_WAYS, &sink);
		} else {
			touch_stream_lines(stream_buf, L1_NUM_WAYS, &sink);
		}
		break;
	default:
		idle_round();
		break;
	}
}

int main(void)
{
	uint32_t scenario;
	uint32_t round;

	init_line_buffer(set_buf, PEER_SET_BUF_BYTES, 41U);
	init_line_buffer(stream_buf, STREAM_BUF_BYTES, 99U);

	builtin_yield();

	for (scenario = 0; scenario < SCENARIO_COUNT; ++scenario) {
		for (round = 0; round < BENCH_ROUNDS; ++round) {
			peer_round((enum bench_scenario)scenario, 0);
			builtin_yield();
		}
		for (round = 0; round < BENCH_ROUNDS; ++round) {
			peer_round((enum bench_scenario)scenario, 1);
			builtin_yield();
		}
	}

	return 0;
}
