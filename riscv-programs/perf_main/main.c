#include <builtin.h>
#include <perf_bench.h>
#include <stdint.h>

static volatile unsigned char hot_buf[HOT_BUF_BYTES] __attribute__((aligned(CACHE_LINE_BYTES)));
static volatile unsigned char set_buf[MAIN_SET_BUF_BYTES] __attribute__((aligned(SET_STRIDE)));
static volatile unsigned char stream_buf[STREAM_BUF_BYTES] __attribute__((aligned(CACHE_LINE_BYTES)));
static volatile unsigned char sink;

static void prepare_scenario(enum bench_scenario scenario)
{
	switch (scenario) {
	case SCENARIO_HOT_REUSE_BASELINE:
		touch_stream_lines(hot_buf, 4U, &sink);
		break;
	case SCENARIO_L1_CONFLICT_PRESSURE:
	case SCENARIO_BACKCACHE_RESCUE_WINDOW:
	case SCENARIO_BACKCACHE_OVERFLOW:
	case SCENARIO_NEWCACHE_TARGET_BREAKDOWN:
		touch_set_lines(set_buf, TARGET_SET, L1_NUM_WAYS, &sink);
		break;
	case SCENARIO_SECDCP_ISOLATION_PRESSURE:
		touch_stream_lines(stream_buf, 32U, &sink);
		break;
	default:
		break;
	}
}

static int measure_scenario(enum bench_scenario scenario)
{
	switch (scenario) {
	case SCENARIO_HOT_REUSE_BASELINE:
		return time_hot_reuse(hot_buf, HOT_REPS, &sink);
	case SCENARIO_L1_CONFLICT_PRESSURE:
	case SCENARIO_BACKCACHE_RESCUE_WINDOW:
	case SCENARIO_BACKCACHE_OVERFLOW:
	case SCENARIO_NEWCACHE_TARGET_BREAKDOWN:
		return time_set_probe(set_buf, TARGET_SET, L1_NUM_WAYS, PROBE_REPS, &sink);
	case SCENARIO_SECDCP_ISOLATION_PRESSURE:
		return time_hot_reuse(stream_buf, HOT_REPS, &sink);
	default:
		return 0;
	}
}

static void run_and_report(enum bench_scenario scenario)
{
	uint32_t round;
	uint64_t baseline_sum = 0;
	uint64_t stressed_sum = 0;
	int baseline_avg;
	int stressed_avg;
	int delta;

	for (round = 0; round < BENCH_ROUNDS; ++round) {
		prepare_scenario(scenario);
		builtin_yield();
		baseline_sum += (uint64_t)measure_scenario(scenario);
	}

	for (round = 0; round < BENCH_ROUNDS; ++round) {
		prepare_scenario(scenario);
		builtin_yield();
		stressed_sum += (uint64_t)measure_scenario(scenario);
	}

	baseline_avg = (int)(baseline_sum / BENCH_ROUNDS);
	stressed_avg = (int)(stressed_sum / BENCH_ROUNDS);
	delta = stressed_avg - baseline_avg;

	builtin_printf(
		"scenario=%s rounds=%d baseline=%d stressed=%d delta=%d",
		scenario_name(scenario),
		(int)BENCH_ROUNDS,
		baseline_avg,
		stressed_avg,
		delta
	);
}

int main(void)
{
	uint32_t scenario;

	init_line_buffer(hot_buf, HOT_BUF_BYTES, 3U);
	init_line_buffer(set_buf, MAIN_SET_BUF_BYTES, 19U);
	init_line_buffer(stream_buf, STREAM_BUF_BYTES, 67U);

	builtin_printf("perf_main: start");
	builtin_yield();

	for (scenario = 0; scenario < SCENARIO_COUNT; ++scenario) {
		run_and_report((enum bench_scenario)scenario);
	}

	builtin_printf("perf_main: done");
	return 0;
}
