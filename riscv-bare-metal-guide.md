# RISC-V Bare Metal & Simulator Implementation Guide

**Project:** Secure Cache "Clean Slate" Architecture
**Target:** RV32I Base Integer Instruction Set (No FPU, No OS)
**Format:** Flat Binary (Raw Machine Code) loaded at `0x00000000`

---

## 1. Simulator Instruction Support
Your simulator needs to interpret the **RV32I Base Integer Set**. Below is the exhaustive list of instructions you will encounter when compiling with `-march=rv32i`.

See the (RISC-V reference card for opcodes and more information)[https://github.com/jameslzhu/riscv-card/releases/download/latest/riscv-card.pdf]

### **Legend**
* **✅ Implement**: Must implement logic for correctness.
* **🛑 Exit**: Terminates the simulator loop.
* **⚠️ Log/Trap**: Should log "Unimplemented opcode: {NAME}" and continue (treated as NOP) or trap, depending on your strictness.

### **Computational (ALU)**
| Mnemonic | Description | Action |
| :--- | :--- | :--- |
| `ADD` / `ADDI` | Add / Add Immediate | ✅ Implement |
| `SUB` | Subtract | ✅ Implement |
| `AND` / `ANDI` | Bitwise AND | ✅ Implement |
| `OR` / `ORI` | Bitwise OR | ✅ Implement |
| `XOR` / `XORI` | Bitwise XOR | ✅ Implement |
| `SLL` / `SLLI` | Shift Left Logical | ✅ Implement |
| `SRL` / `SRLI` | Shift Right Logical | ✅ Implement |
| `SRA` / `SRAI` | Shift Right Arithmetic (Keep Sign) | ✅ Implement |
| `SLT` / `SLTI` | Set Less Than (Signed) | ✅ Implement |
| `SLTU` / `SLTIU` | Set Less Than (Unsigned) | ✅ Implement |
| `LUI` | Load Upper Immediate (Bits 12-31) | ✅ Implement |
| `AUIPC` | Add Upper Immediate to PC | ✅ Implement |

### **Memory Access**
| Mnemonic | Description | Action |
| :--- | :--- | :--- |
| `LB` / `LBU` | Load Byte (Signed/Unsigned) | ✅ Implement (Trigger Cache Logic) |
| `LH` / `LHU` | Load Halfword (Signed/Unsigned) | ✅ Implement (Trigger Cache Logic) |
| `LW` | Load Word | ✅ Implement (Trigger Cache Logic) |
| `SB` | Store Byte | ✅ Implement (Trigger Cache Logic) |
| `SH` | Store Halfword | ✅ Implement (Trigger Cache Logic) |
| `SW` | Store Word | ✅ Implement (Trigger Cache Logic) |

### **Control Flow**
| Mnemonic | Description | Action |
| :--- | :--- | :--- |
| `JAL` | Jump and Link (Function Call) | ✅ Implement |
| `JALR` | Jump and Link Register (Return/Ptr) | ✅ Implement |
| `BEQ` | Branch if Equal | ✅ Implement |
| `BNE` | Branch if Not Equal | ✅ Implement |
| `BLT` / `BLTU` | Branch Less Than (Signed/Unsigned) | ✅ Implement |
| `BGE` / `BGEU` | Branch Greater/Equal (Signed/Unsigned) | ✅ Implement |

### **System & Control**
| Mnemonic | Description | Action |
| :--- | :--- | :--- |
| `ECALL` | Environment Call | ✅ **Implement** (Use for `print()` hook) |
| `EBREAK` | Environment Break | 🛑 **EXIT SIMULATOR** |
| `FENCE` | Memory Ordering Barrier | ⚠️ Log "Unimplemented: FENCE" (No-op) |
| `FENCE.I` | Instruction Cache Sync | ⚠️ Log "Unimplemented: FENCE.I" (No-op) |
| `CSRRW` | Atomic Read/Write CSR | ⚠️ Log "Unimplemented: CSRRW" (Trap/No-op) |
| `CSRRS` | Atomic Read/Set CSR | ⚠️ Log "Unimplemented: CSRRS" (Trap/No-op) |
| `CSRRC` | Atomic Read/Clear CSR | ⚠️ Log "Unimplemented: CSRRC" (Trap/No-op) |
| `CSRRWI` | Read/Write CSR Immediate | ⚠️ Log "Unimplemented: CSRRWI" (Trap/No-op) |
| `CSRRSI` | Read/Set CSR Immediate | ⚠️ Log "Unimplemented: CSRRSI" (Trap/No-op) |
| `CSRRCI` | Read/Clear CSR Immediate | ⚠️ Log "Unimplemented: CSRRCI" (Trap/No-op) |

---

## 2. Build System Files

### **A. Startup Shim (`start.S`)**
* **Purpose:** Sets the stack pointer (`sp`) and handles program termination.
* **Logic:** Starts at `0x0`, jumps to C code, then hits `ebreak` to kill the simulator.

```assembly
/* start.S */
.section .text.init
.global _start

_start:
    /* 1. Setup Stack Pointer to 1MB (Adjust if simulator RAM is smaller) */
    li sp, 0x100000

    /* 2. Jump to C main() function */
    call main

    /* 3. Termination Loop */
    /* Only reached if main returns. We use ebreak to tell simulator to quit. */
exit_loop:
    ebreak
    j exit_loop
```

### **B. Linker Script (`link.ld`)**
* **Purpose:** Ensures the binary is "Flat" (contiguous) starting at address 0.
* **Logic:** Places `.text.init` first, then `.text`, then data.

```ld
/* link.ld */
OUTPUT_ARCH( "riscv" )
ENTRY( _start )

SECTIONS
{
  /* Start at address 0 */
  . = 0x00000000;

  /* Code Section */
  .text : {
    *(.text.init)   /* Put start.S here first */
    *(.text)        /* Put main.c code here */
  }

  /* Read-only Data (Strings) */
  .rodata : {
    *(.rodata)
  }

  /* Read-Write Data */
  .data : {
    *(.data)
  }

  /* Zero-initialized Data (BSS) */
  /* NOTE: Since we don't have an OS loader, your C code must zero this manually if needed. */
  .bss : {
    *(.bss)
  }

  /* Mark end of memory for stack calculation if needed */
  _end = .;
}
```

### **C. Makefile (Clang / LLVM)**
* **Purpose:** Compiles C/ASM to ELF, then flattens to `.bin`.
* **Toolchain:** Uses `clang` as the cross-compiler and `llvm-objcopy`.

```makefile
# Makefile

# --- Toolchain Configuration ---
# Ensure you have 'llvm' installed. 
# On Mac: brew install llvm
# On Linux: sudo apt install clang llvm lld

# If clang is not in your PATH, set the absolute path here
CC       = clang
LD       = ld.lld
OBJCOPY  = llvm-objcopy
OBJDUMP  = llvm-objdump

# --- Target Architecture ---
# Target 32-bit RISC-V Bare Metal (unknown-elf)
TARGET   = --target=riscv32-unknown-elf

# --- Compilation Flags ---
# -march=rv32i   : Use Base Integer instructions only (No FPU, No Mul/Div)
# -mabi=ilp32    : Use 32-bit integer ABI
# -ffreestanding : No standard library (libc) available
# -nostdlib      : Do not link startup files
# -O2            : Optimize for realistic performance simulation
CFLAGS   = $(TARGET) -march=rv32i -mabi=ilp32 -ffreestanding -nostdlib -O2 -Wall

# --- Linker Flags ---
# Use our custom linker script
LDFLAGS  = -T link.ld

# --- Source Management ---
SRCS     = $(wildcard *.c) $(wildcard *.S)
OBJS     = $(SRCS:.c=.o)
OBJS    := $(OBJS:.S=.o)

# --- Targets ---

all: firmware.bin dump

# 1. Compile C/ASM to Object Files
%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

%.o: %.S
	$(CC) $(CFLAGS) -c $< -o $@

# 2. Link Object Files into ELF
firmware.elf: $(OBJS) link.ld
	$(LD) $(LDFLAGS) $(OBJS) -o $@

# 3. Flatten ELF to Binary (For Simulator)
firmware.bin: firmware.elf
	$(OBJCOPY) -O binary $< $@

# 4. Create Disassembly (For Debugging)
dump: firmware.elf
	$(OBJDUMP) -d -S firmware.elf > firmware.asm

clean:
	rm -f *.o *.elf *.bin *.asm

.PHONY: all clean dump
```

---

## 3. How to Use
1.  Place `start.S`, `link.ld`, `Makefile`, and your `main.c` in one folder.
2.  Run `make`.
3.  **Load `firmware.bin`** into your simulator's byte array (`ram[]`).
4.  **Set PC = 0**.
5.  **Execute Loop**:
    * Fetch 4 bytes.
    * Decode.
    * Execute.
    * Stop when you hit `EBREAK`.


#### TODO:

* May need to support an instruction for flushing cache to test flush+reload
