#include <builtin.h>
#include <stdint.h>

#define STACK_SIZE (1024 * 1024) // 1mb
#define PW_LEN 16

char pw_available_chars[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

int main(void) {
	builtin_printf("victim: start");
	
	// Take most of the stack so we can put pw at a random offset (but leave some stack space actual work)
	char stack_buf[STACK_SIZE - 1024];
	uint32_t offset = builtin_modulo(builtin_random(), STACK_SIZE - 1024 - PW_LEN); // 16 bytes for pw
	volatile char *pw = stack_buf + offset;

	builtin_printf("victim: chosen password offset: %d", offset);

	for (int i = 0; i < PW_LEN; ++i) {
		uint32_t random = builtin_random();
		pw[i] = pw_available_chars[builtin_modulo(random, sizeof(pw_available_chars) - 1)];
	}

	builtin_printf("victim: chosen password: %s", pw);
	builtin_yield();

	for (int i = 0; i < 8; ++i) {
		for (int j = 0; j < sizeof(pw); ++j) {
			pw[j] = pw[j];
		}
		builtin_printf("victim: touched password memory");
		builtin_yield();
	}

	builtin_printf("victim: done");

	return 0;
}