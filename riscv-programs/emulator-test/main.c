/*
 * Emulator capability test: exercises RV32I instructions used by the sim.
 * No libc, no multiply/divide (rv32i). Build with -march=rv32i -mabi=ilp32.
 */
#include <sim.h>

static void debug_assert(int cond, const char *msg)
{
	if (!(cond)) {
		sim_printf("Assert failed: %s\n", msg);
		sim_exit();
	}
}

/* Global data: exercises LUI/AUIPC and load/store of globals */
static unsigned g_word = 0xdeadbeef;
static unsigned g_array[4] = { 10, 20, 30, 40 };
static const char *const msg = "emulator-test";

/* --- ALU (ADD, SUB, AND, OR, XOR, SLL, SRL, SRA, SLT, SLTU) --- */
static int add(int a, int b) { return a + b; }
static int sub(int a, int b) { return a - b; }
static unsigned and(unsigned a, unsigned b) { return a & b; }
static unsigned or(unsigned a, unsigned b) { return a | b; }
static unsigned xor(unsigned a, unsigned b) { return a ^ b; }
static unsigned sll(unsigned a, int sh) { return a << sh; }
static unsigned srl(unsigned a, int sh) { return (unsigned)((unsigned)a >> sh); }
static int sra(int a, int sh) { return a >> sh; }
static int slt(int a, int b) { return (a < b) ? 1 : 0; }
static unsigned sltu(unsigned a, unsigned b) { return (a < b) ? 1u : 0u; }

static void test_alu(void)
{
	sim_printf("=== ALU (ADD/SUB/AND/OR/XOR/SLL/SRL/SRA/SLT/SLTU) ===\n");
	int v;
	v = add(3, 7);
	debug_assert(v == 10, "add(3,7)==10");
	sim_printf("  add(3,7)=%d\n", v);
	v = sub(10, 4);
	debug_assert(v == 6, "sub(10,4)==6");
	sim_printf("  sub(10,4)=%d\n", v);
	debug_assert((int)and(0xffu, 0x0fu) == 15, "and(0xff,0x0f)==15");
	sim_printf("  and(0xff,0x0f)=%d\n", (int)and(0xffu, 0x0fu));
	debug_assert((int)or(0x0fu, 0xf0u) == 255, "or(0x0f,0xf0)==255");
	sim_printf("  or(0x0f,0xf0)=%d\n", (int)or(0x0fu, 0xf0u));
	debug_assert((int)xor(0xffu, 0x0fu) == 240, "xor(0xff,0x0f)==240");
	sim_printf("  xor(0xff,0x0f)=%d\n", (int)xor(0xffu, 0x0fu));
	debug_assert((int)sll(1u, 5) == 32, "sll(1,5)==32");
	sim_printf("  sll(1,5)=%d\n", (int)sll(1u, 5));
	debug_assert((int)srl(32u, 3) == 4, "srl(32,3)==4");
	sim_printf("  srl(32,3)=%d\n", (int)srl(32u, 3));
	debug_assert(sra(-8, 2) == -2, "sra(-8,2)==-2");
	sim_printf("  sra(-8,2)=%d\n", sra(-8, 2));
	debug_assert(slt(2, 5) == 1 && slt(5, 2) == 0, "slt");
	sim_printf("  slt(2,5)=%d slt(5,2)=%d\n", slt(2, 5), slt(5, 2));
	debug_assert(sltu(-1u, 1u) == 0, "sltu(-1u,1u)==0");
	sim_printf("  sltu(-1u,1u)=%d\n", (int)sltu(-1u, 1u));
}

/* --- Memory (LW/SW, LB/LBU/SB, LH/LHU/SH) --- */
static void test_memory(void)
{
	sim_printf("=== Memory (LW/SW, LB/LBU/SB, LH/LHU/SH) ===\n");
	debug_assert(g_word == 0xdeadbeef, "g_word");
	sim_printf("  g_word=%d\n", (int)g_word);

	g_array[0] = 100;
	g_array[1] = 200;
	debug_assert(g_array[0] == 100 && g_array[1] == 200, "g_array store");
	sim_printf("  g_array[0]=%d g_array[1]=%d\n", (int)g_array[0], (int)g_array[1]);

	/* Byte and halfword access so compiler emits LB/LBU/SB, LH/LHU/SH */
	unsigned char bytes[8];
	bytes[0] = 0x11;
	bytes[1] = 0x22;
	bytes[2] = 0x33;
	bytes[3] = 0x44;
	unsigned v = (unsigned)bytes[0] | ((unsigned)bytes[1] << 8) |
	    ((unsigned)bytes[2] << 16) | ((unsigned)bytes[3] << 24);
	debug_assert(v == 0x44332211u, "bytes->word");
	sim_printf("  bytes->word %d\n", (int)v);

	unsigned short half[2];
	half[0] = 0x1234u;
	half[1] = 0x5678u;
	debug_assert(half[0] == 0x1234u && half[1] == 0x5678u, "half");
	sim_printf("  half[0]=%d half[1]=%d\n", (int)half[0], (int)half[1]);
}

/* --- Branches and loops (BEQ/BNE/BLT/BGE/BLTU/BGEU) --- */
static void test_branches(void)
{
	sim_printf("=== Branches & loops (BEQ/BNE/BLT/BGE) ===\n");
	int n = 0;
	for (int i = 0; i < 4; i++)
		n = add(n, (int)g_array[i]);
	debug_assert(n == 370, "sum(g_array)==370");
	sim_printf("  sum(g_array)=%d\n", n);

	int a = 7, b = 3;
	if (a > b)
		sim_printf("  if a>b: ok\n");
	if (a != b)
		sim_printf("  if a!=b: ok\n");
	if ((unsigned)a > (unsigned)b)
		sim_printf("  if (unsigned)a>(unsigned)b: ok\n");

	int j = 0;
	while (j < 3) {
		sim_printf("  while j=%d\n", j);
		j = add(j, 1);
	}
}

/* --- Printf variadic (ECALL builtin) --- */
static void test_printf(void)
{
	sim_printf("=== Printf (ECALL a7=1) ===\n");
	sim_printf("  plain string\n");
	sim_printf("  %%d: %d\n", 42);
	sim_printf("  two %%d: %d %d\n", 10, 20);
	sim_printf("  %%s: %s\n", msg);
}

/* --- Entry: all tests then exit via ebreak in start.S --- */
int main(void)
{
	sim_printf("Emulator capability test (RV32I)\n");
	test_printf();
	test_alu();
	test_memory();
	test_branches();
	sim_printf("Done.\n");
	return 0;
}
