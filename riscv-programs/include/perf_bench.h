#ifndef PERF_BENCH_H

#include <builtin.h>
#include <stdint.h>

#define CACHE_LINE_BYTES 64U
#define L1_NUM_SETS 64U
#define L1_NUM_WAYS 8U
#define SET_STRIDE (CACHE_LINE_BYTES * L1_NUM_SETS)

#define BENCH_ROUNDS 8U
#define HOT_REPS 64U
#define PROBE_REPS 8U

#define TARGET_SET 17U

#define RESCUE_PEER_LINES 16U
#define OVERFLOW_MARGIN 32U
#define OVERFLOW_PEER_LINES (L1_NUM_WAYS + (L1_NUM_SETS * L1_NUM_WAYS) + OVERFLOW_MARGIN)
#define LARGE_STREAM_LINES 2048U

#define HOT_BUF_BYTES (4U * CACHE_LINE_BYTES)
#define MAIN_SET_BUF_BYTES (((L1_NUM_WAYS + 2U) * SET_STRIDE) + (L1_NUM_SETS * CACHE_LINE_BYTES))
#define PEER_SET_BUF_BYTES (((OVERFLOW_PEER_LINES + 2U) * SET_STRIDE) + (L1_NUM_SETS * CACHE_LINE_BYTES))
#define STREAM_BUF_BYTES (LARGE_STREAM_LINES * CACHE_LINE_BYTES)

enum bench_scenario {
	SCENARIO_HOT_REUSE_BASELINE = 0,
	SCENARIO_L1_CONFLICT_PRESSURE = 1,
	SCENARIO_BACKCACHE_RESCUE_WINDOW = 2,
	SCENARIO_BACKCACHE_OVERFLOW = 3,
	SCENARIO_SECDCP_ISOLATION_PRESSURE = 4,
	SCENARIO_NEWCACHE_TARGET_BREAKDOWN = 5,
	SCENARIO_COUNT = 6,
};

static inline const char *scenario_name(enum bench_scenario scenario)
{
	switch (scenario) {
	case SCENARIO_HOT_REUSE_BASELINE:
		return "hot_reuse_baseline";
	case SCENARIO_L1_CONFLICT_PRESSURE:
		return "l1_conflict_pressure";
	case SCENARIO_BACKCACHE_RESCUE_WINDOW:
		return "backcache_rescue_window";
	case SCENARIO_BACKCACHE_OVERFLOW:
		return "backcache_overflow";
	case SCENARIO_SECDCP_ISOLATION_PRESSURE:
		return "secdcp_isolation_pressure";
	case SCENARIO_NEWCACHE_TARGET_BREAKDOWN:
		return "newcache_target_breakdown";
	default:
		return "unknown";
	}
}

static inline volatile unsigned char *same_set_addr(
	volatile unsigned char *base,
	uint32_t set,
	uint32_t index
)
{
	return base + (index * SET_STRIDE) + (set * CACHE_LINE_BYTES);
}

static inline void init_line_buffer(volatile unsigned char *buf, uint32_t bytes, uint8_t seed)
{
	uint32_t i;
	for (i = 0; i < bytes; i += CACHE_LINE_BYTES) {
		buf[i] = (unsigned char)(seed + (uint8_t)(i / CACHE_LINE_BYTES));
	}
}

static inline void touch_set_lines(
	volatile unsigned char *buf,
	uint32_t set,
	uint32_t count,
	volatile unsigned char *sink
)
{
	uint32_t i;
	unsigned char acc = *sink;
	for (i = 0; i < count; ++i) {
		acc ^= *same_set_addr(buf, set, i);
	}
	*sink = acc;
}

static inline void touch_stream_lines(
	volatile unsigned char *buf,
	uint32_t line_count,
	volatile unsigned char *sink
)
{
	uint32_t i;
	unsigned char acc = *sink;
	for (i = 0; i < line_count; ++i) {
		acc ^= buf[i * CACHE_LINE_BYTES];
	}
	*sink = acc;
}

static inline int time_set_probe(
	volatile unsigned char *buf,
	uint32_t set,
	uint32_t count,
	uint32_t reps,
	volatile unsigned char *sink
)
{
	uint32_t rep;
	uint64_t t0 = builtin_cycle_count();
	for (rep = 0; rep < reps; ++rep) {
		touch_set_lines(buf, set, count, sink);
	}
	uint64_t t1 = builtin_cycle_count();
	return (int)(t1 - t0);
}

static inline int time_hot_reuse(
	volatile unsigned char *buf,
	uint32_t reps,
	volatile unsigned char *sink
)
{
	uint32_t rep;
	uint64_t t0 = builtin_cycle_count();
	for (rep = 0; rep < reps; ++rep) {
		*sink ^= buf[0];
		*sink ^= buf[CACHE_LINE_BYTES];
		*sink ^= buf[CACHE_LINE_BYTES * 2U];
		*sink ^= buf[CACHE_LINE_BYTES * 3U];
	}
	uint64_t t1 = builtin_cycle_count();
	return (int)(t1 - t0);
}

#define PERF_BENCH_H
#endif
