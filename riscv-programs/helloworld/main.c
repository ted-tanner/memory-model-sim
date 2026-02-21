#include <sim.h>

static const char msg[] = "Hello, world!\n";

int main(void)
{
	sim_printf(msg);
	return 0;
}
