#include <attack_layout.h>
#include <builtin.h>
#include <stdint.h>

static char pw_available_chars[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
static volatile unsigned char password_arena[NUM_SETS * LINE_SIZE]
	__attribute__((aligned(SET_STRIDE)));
static volatile uint32_t password_sink;

int main(void) {
	builtin_printf("victim: start");

	uint32_t password_set =
		ATTACKABLE_SET_START + builtin_modulo(builtin_random(), ATTACKABLE_SET_COUNT);
	volatile unsigned char *pw = password_arena + (password_set * LINE_SIZE);

	for (uint32_t i = 0; i < PW_LEN; ++i) {
		uint32_t random = builtin_random();
		pw[i] = pw_available_chars[builtin_modulo(random, sizeof(pw_available_chars) - 1)];
	}
	pw[PW_LEN] = '\0';

	builtin_printf("victim: chosen password set: %d", (int)password_set);
	builtin_printf("victim: chosen password: %s", pw);
	builtin_yield();

	for (uint32_t round = 0; round < CALIBRATION_ROUNDS; ++round) {
		builtin_yield();
	}

	for (uint32_t round = 0; round < ATTACK_ROUNDS; ++round) {
		uint32_t acc = 0;
		for (uint32_t i = 0; i < PW_LEN; ++i) {
			acc ^= pw[i];
		}
		password_sink = acc;
		builtin_yield();
	}

	builtin_printf("victim: done");

	return 0;
}
