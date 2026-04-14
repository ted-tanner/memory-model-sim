#ifndef ATTACK_LAYOUT_H

#include <stdint.h>

#define LINE_SIZE 64U
#define NUM_SETS 64U
#define NUM_WAYS 8U
#define SET_STRIDE (LINE_SIZE * NUM_SETS)
#define CALIBRATION_ROUNDS 16U
#define ATTACK_ROUNDS 16U
#define PW_LEN 16U
#define ATTACKABLE_SET_START 16U
#define ATTACKABLE_SET_COUNT (NUM_SETS - ATTACKABLE_SET_START)

static inline uint32_t cache_set_for_addr(uint32_t addr)
{
	return (addr / LINE_SIZE) % NUM_SETS;
}

#define ATTACK_LAYOUT_H
#endif
