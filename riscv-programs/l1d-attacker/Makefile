# Makefile - L1D experiment attacker program.
TOPDIR   = $(dir $(abspath $(lastword $(MAKEFILE_LIST))))..

ifeq ($(CLANG),1)
CC       = clang
LD       = ld.lld
OBJCOPY  = llvm-objcopy
OBJDUMP  = llvm-objdump
CFLAGS   = --target=riscv32-unknown-elf -march=rv32i -mabi=ilp32
else
CROSS    ?= riscv64-elf-
CC       = $(CROSS)gcc
LD       = $(CROSS)ld
OBJCOPY  = $(CROSS)objcopy
OBJDUMP  = $(CROSS)objdump
CFLAGS   =
LDFLAGS  = -m elf32lriscv
endif

CFLAGS  += -march=rv32i_zicsr_zifencei -mabi=ilp32 -ffreestanding -nostdlib -O2 -Wall
CFLAGS  += -I$(TOPDIR)/include
LDFLAGS += -T link.ld

SRCS     = $(wildcard *.c) $(wildcard *.S)
OBJS     = $(SRCS:.c=.o)
OBJS    := $(OBJS:.S=.o)

all: firmware.bin dump

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

%.o: %.S
	$(CC) $(CFLAGS) -c $< -o $@

firmware.elf: $(OBJS) link.ld
	$(LD) $(LDFLAGS) $(OBJS) -o $@

firmware.bin: firmware.elf
	$(OBJCOPY) -O binary $< $@

dump: firmware.elf
	$(OBJDUMP) -d -S firmware.elf > firmware.asm

clean:
	rm -f *.o *.elf *.bin *.asm

.PHONY: all clean dump
