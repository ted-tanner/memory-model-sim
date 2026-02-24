use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::rc::Rc;

use crate::device::Clock;
use crate::device::memory::{
    CacheTiming, MainMemory, MainMemoryTiming, MemoryDevice, SetAssociativeCache,
};
use crate::machine::{Machine, StepResult};

mod builtins;

#[derive(Clone)]
pub struct Registers {
    pub pc: u32,
    pub x: [u32; 32],
}

impl Registers {
    pub fn new() -> Self {
        Self { pc: 0, x: [0; 32] }
    }

    pub fn x(&self, i: u8) -> u32 {
        if i == 0 { 0 } else { self.x[i as usize] }
    }

    pub fn set_x(&mut self, i: u8, v: u32) {
        if i != 0 {
            self.x[i as usize] = v;
        }
    }

    pub fn a(&self, i: u8) -> u32 {
        self.x(10 + i)
    }
    pub fn set_a(&mut self, i: u8, v: u32) {
        self.set_x(10 + i, v);
    }

    pub fn ra(&self) -> u32 {
        self.x(1)
    }
    pub fn set_ra(&mut self, v: u32) {
        self.set_x(1, v);
    }
    pub fn sp(&self) -> u32 {
        self.x(2)
    }
    pub fn set_sp(&mut self, v: u32) {
        self.set_x(2, v);
    }
    pub fn gp(&self) -> u32 {
        self.x(3)
    }
    pub fn set_gp(&mut self, v: u32) {
        self.set_x(3, v);
    }
    pub fn tp(&self) -> u32 {
        self.x(4)
    }
    pub fn set_tp(&mut self, v: u32) {
        self.set_x(4, v);
    }

    fn t_x_index(i: u8) -> u8 {
        const T: [u8; 7] = [5, 6, 7, 28, 29, 30, 31];
        T[i as usize]
    }
    fn s_x_index(i: u8) -> u8 {
        const S: [u8; 12] = [8, 9, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27];
        S[i as usize]
    }

    pub fn t(&self, i: u8) -> u32 {
        self.x(Self::t_x_index(i))
    }
    pub fn set_t(&mut self, i: u8, v: u32) {
        self.set_x(Self::t_x_index(i), v);
    }

    pub fn s(&self, i: u8) -> u32 {
        self.x(Self::s_x_index(i))
    }
    pub fn set_s(&mut self, i: u8, v: u32) {
        self.set_x(Self::s_x_index(i), v);
    }

    pub fn fp(&self) -> u32 {
        self.s(0)
    }
    pub fn set_fp(&mut self, v: u32) {
        self.set_s(0, v);
    }
}

impl Default for Registers {
    fn default() -> Self {
        Self::new()
    }
}

type BuiltinMap = BTreeMap<u32, Box<dyn FnMut(&mut RiscV32IntegerMachine)>>;

pub struct CsrBank {
    store: BTreeMap<u16, u32>,
}

impl CsrBank {
    fn new() -> Self {
        Self {
            store: BTreeMap::new(),
        }
    }
    fn read(&self, csr: u16) -> u32 {
        *self.store.get(&csr).unwrap_or(&0)
    }
    fn write(&mut self, csr: u16, value: u32) {
        self.store.insert(csr, value);
    }
}

pub struct RiscV32IntegerMachine {
    clock: Rc<Clock>,
    memory: &'static dyn MemoryDevice,
    registers: Registers,
    builtins: BuiltinMap,
    csrs: CsrBank,
}

impl RiscV32IntegerMachine {
    const DEFAULT_MEMORY_SIZE: usize = 4 * 1024 * 1024 * 1024; // 4GB
    const LINE_SIZE: usize = 64;

    const L1_NUM_SETS: usize = 64;
    const L1_WAYS: usize = 8;

    const L2_NUM_SETS: usize = 1024;
    const L2_WAYS: usize = 16;

    const L3_NUM_SETS: usize = 8192;
    const L3_WAYS: usize = 16;

    const MAIN_MEMORY_TIMING: MainMemoryTiming = MainMemoryTiming {
        load: 300,
        store: 300,
    };
    const L3_TIMING: CacheTiming = CacheTiming {
        load_hit: 45,
        load_miss: 90,
        store_hit: 45,
        store_miss: 90,
        write_back: 70,
        invalidation_send: 1,
        invalidation_apply: 1,
    };
    const L2_TIMING: CacheTiming = CacheTiming {
        load_hit: 12,
        load_miss: 35,
        store_hit: 12,
        store_miss: 35,
        write_back: 20,
        invalidation_send: 1,
        invalidation_apply: 1,
    };
    const L1_TIMING: CacheTiming = CacheTiming {
        load_hit: 4,
        load_miss: 12,
        store_hit: 4,
        store_miss: 12,
        write_back: 8,
        invalidation_send: 1,
        invalidation_apply: 1,
    };

    const OP_OP: u32 = 0x33;
    const OP_OP_IMM: u32 = 0x13;
    const OP_LUI: u32 = 0x37;
    const OP_AUIPC: u32 = 0x17;
    const OP_BRANCH: u32 = 0x63;
    const OP_JAL: u32 = 0x6f;
    const OP_JALR: u32 = 0x67;
    const OP_LOAD: u32 = 0x03;
    const OP_STORE: u32 = 0x23;
    const OP_MISC_MEM: u32 = 0x0f;
    const OP_SYSTEM: u32 = 0x73;

    const F3_CSRRW: u32 = 0x1;
    const F3_CSRRS: u32 = 0x2;
    const F3_CSRRC: u32 = 0x3;
    const F3_CSRRWI: u32 = 0x5;
    const F3_CSRRSI: u32 = 0x6;
    const F3_CSRRCI: u32 = 0x7;

    const SYS_ECALL: u32 = 0;
    const SYS_EBREAK: u32 = 1;

    pub fn new() -> Self {
        let memory_size = Self::DEFAULT_MEMORY_SIZE;
        let clock = Rc::new(Clock::new());

        let mem: &'static MainMemory = Box::leak(Box::new(
            MainMemory::new(memory_size)
                .with_clock(clock.clone())
                .with_timing(Self::MAIN_MEMORY_TIMING),
        ));
        let l3: &'static SetAssociativeCache<'static> = Box::leak(Box::new(
            SetAssociativeCache::new(Self::LINE_SIZE, Self::L3_NUM_SETS, Self::L3_WAYS, mem)
                .with_clock(clock.clone())
                .with_timing(Self::L3_TIMING),
        ));
        let l2: &'static SetAssociativeCache<'static> = Box::leak(Box::new(
            SetAssociativeCache::new(Self::LINE_SIZE, Self::L2_NUM_SETS, Self::L2_WAYS, l3)
                .with_clock(clock.clone())
                .with_timing(Self::L2_TIMING),
        ));
        let l1: &'static SetAssociativeCache<'static> = Box::leak(Box::new(
            SetAssociativeCache::new(Self::LINE_SIZE, Self::L1_NUM_SETS, Self::L1_WAYS, l2)
                .with_clock(clock.clone())
                .with_timing(Self::L1_TIMING),
        ));

        l2.set_invalidation_listener(l1);

        let mut machine = Self {
            clock,
            memory: l1,
            registers: Registers::new(),
            builtins: BTreeMap::new(),
            csrs: CsrBank::new(),
        };
        builtins::register_common_builtins(&mut machine);
        machine
    }

    pub fn x(&self, i: u8) -> u32 {
        self.registers.x(i)
    }

    pub fn set_x(&mut self, i: u8, v: u32) {
        self.registers.set_x(i, v);
    }

    pub fn set_pc(&mut self, pc: u32) {
        self.registers.pc = pc;
    }

    pub fn snapshot_registers(&self) -> Registers {
        self.registers.clone()
    }

    pub fn restore_registers(&mut self, registers: &Registers) {
        self.registers = registers.clone();
    }

    pub fn register_builtin(&mut self, number: u32, f: impl FnMut(&mut Self) + 'static) {
        self.builtins.insert(number, Box::new(f));
    }

    pub fn cycle_count(&self) -> u64 {
        self.clock.curr_tick()
    }

    pub fn random_state(&mut self) -> u32 {
        let mut f = File::open("/dev/urandom").expect("unable to open /dev/urandom");
        let mut buf = [0u8; 4];
        f.read_exact(&mut buf)
            .expect("unable to read from /dev/urandom");
        u32::from_ne_bytes(buf)
    }

    pub fn load_u8(&self, addr: u32) -> u8 {
        self.memory.load_u8(addr as usize)
    }
    pub fn load_i8(&self, addr: u32) -> i8 {
        self.memory.load_i8(addr as usize)
    }
    pub fn load_u16(&self, addr: u32) -> u16 {
        self.memory.load_u16(addr as usize)
    }
    pub fn load_i16(&self, addr: u32) -> i16 {
        self.memory.load_i16(addr as usize)
    }
    pub fn load_u32(&self, addr: u32) -> u32 {
        self.memory.load_u32(addr as usize)
    }
    pub fn store_u8(&mut self, addr: u32, v: u8) {
        self.memory.store_u8(addr as usize, v);
    }
    pub fn store_u16(&mut self, addr: u32, v: u16) {
        self.memory.store_u16(addr as usize, v);
    }
    pub fn store_u32(&mut self, addr: u32, v: u32) {
        self.memory.store_u32(addr as usize, v);
    }

    fn rd(instruction: u32) -> u8 {
        ((instruction >> 7) & 0x1f) as u8
    }
    fn rs1(instruction: u32) -> u8 {
        ((instruction >> 15) & 0x1f) as u8
    }
    fn rs2(instruction: u32) -> u8 {
        ((instruction >> 20) & 0x1f) as u8
    }
    fn funct3(instruction: u32) -> u32 {
        (instruction >> 12) & 0x7
    }
    fn funct7(instruction: u32) -> u32 {
        (instruction >> 25) & 0x7f
    }
    fn imm_i(instruction: u32) -> u32 {
        (instruction as i32 >> 20) as u32
    }
    fn imm_s(instruction: u32) -> u32 {
        let raw = ((instruction >> 25) << 5) | ((instruction >> 7) & 0x1f);
        let extended = (raw as i32) << 20 >> 20;
        extended as u32
    }
    fn imm_b(instruction: u32) -> u32 {
        let imm13 = (instruction >> 31).wrapping_mul(0x1000)
            | ((instruction >> 7) & 1).wrapping_mul(0x800)
            | ((instruction >> 25) & 0x3f).wrapping_mul(32)
            | ((instruction >> 8) & 0xf).wrapping_mul(2);
        if (imm13 & 0x1000) != 0 {
            imm13 | 0xffff_e000
        } else {
            imm13
        }
    }
    fn imm_u(instruction: u32) -> u32 {
        instruction & 0xffff_f000
    }
    fn imm_j(instruction: u32) -> u32 {
        let imm20 = (instruction >> 31).wrapping_mul(0x100000)
            | ((instruction >> 12) & 0xff).wrapping_mul(0x1000)
            | ((instruction >> 20) & 1).wrapping_mul(0x800)
            | ((instruction >> 21) & 0x3ff).wrapping_mul(2);
        if (imm20 & 0x100000) != 0 {
            imm20 | 0xffe0_0000
        } else {
            imm20
        }
    }

    fn instruction_cycles(instruction: u32, result: &StepResult) -> u64 {
        match instruction & 0x7f {
            Self::OP_OP => {
                if ((instruction >> 25) & 0x7f) == 0x01 {
                    if ((instruction >> 12) & 0x7) < 4 {
                        4
                    } else {
                        20
                    }
                } else {
                    1
                }
            }
            Self::OP_OP_IMM | Self::OP_LUI | Self::OP_AUIPC => 1,
            Self::OP_BRANCH => {
                if matches!(result, StepResult::PcUpdated) {
                    3
                } else {
                    1
                }
            }
            Self::OP_JAL | Self::OP_JALR => 3,
            Self::OP_LOAD | Self::OP_STORE => 3,
            Self::OP_MISC_MEM => 1,
            Self::OP_SYSTEM => {
                let funct3 = (instruction >> 12) & 0x7;
                match funct3 {
                    0 => match (instruction >> 20) & 0xfff {
                        Self::SYS_ECALL => 8,
                        Self::SYS_EBREAK => 1,
                        _ => 2,
                    },
                    Self::F3_CSRRW
                    | Self::F3_CSRRS
                    | Self::F3_CSRRC
                    | Self::F3_CSRRWI
                    | Self::F3_CSRRSI
                    | Self::F3_CSRRCI => 5,
                    _ => 1,
                }
            }
            _ => 1,
        }
    }
}

impl Machine for RiscV32IntegerMachine {
    fn load_binary(&mut self, bytes: &[u8]) {
        self.load_binary_at(bytes, 0);
        self.registers.pc = 0;
        self.clock.reset();
    }

    fn load_binary_at(&mut self, bytes: &[u8], base: u32) {
        let mut backmost_memory = self.memory;
        while let Some(backing) = backmost_memory.backing_memory() {
            backmost_memory = backing;
        }
        for (i, &b) in bytes.iter().enumerate() {
            backmost_memory.store_u8(base as usize + i, b);
        }
    }

    fn current_tick(&self) -> u64 {
        self.cycle_count()
    }

    fn step(&mut self) -> StepResult {
        let pc = self.registers.pc;
        let instruction = self.load_u32(pc);

        let result = match instruction & 0x7f {
            Self::OP_SYSTEM => self.op_system(instruction),
            Self::OP_LOAD => self.op_load(instruction),
            Self::OP_STORE => self.op_store(instruction),
            Self::OP_OP => self.op_reg(instruction),
            Self::OP_OP_IMM => self.op_imm(instruction),
            Self::OP_LUI => self.op_lui(instruction),
            Self::OP_AUIPC => self.op_auipc(instruction),
            Self::OP_JAL => self.op_jal(instruction),
            Self::OP_JALR => self.op_jalr(instruction),
            Self::OP_BRANCH => self.op_branch(instruction),
            Self::OP_MISC_MEM => self.op_misc_mem(instruction),
            _ => StepResult::Unimplemented("unknown opcode"),
        };

        self.clock
            .advance(Self::instruction_cycles(instruction, &result));

        if let StepResult::Unimplemented(msg) = &result {
            panic!(
                "unimplemented instruction: {} at PC=0x{:08x} insn=0x{:08x}",
                msg, pc, instruction
            );
        }
        match result {
            StepResult::Continue | StepResult::Yield => self.registers.pc = pc.wrapping_add(4),
            StepResult::ForceYield => self.registers.pc = pc.wrapping_add(4),
            StepResult::PcUpdated => {}
            _ => {}
        }
        result
    }
}

impl Default for RiscV32IntegerMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl RiscV32IntegerMachine {
    fn op_system(&mut self, instruction: u32) -> StepResult {
        let imm12 = (instruction >> 20) & 0xfff;
        let f3 = Self::funct3(instruction);
        if f3 == 0 {
            if imm12 == 0 {
                let a7 = self.x(17);
                if let Some(mut handler) = self.builtins.remove(&a7) {
                    handler(self);
                    self.builtins.insert(a7, handler);
                    if a7 == builtins::BUILTIN_YIELD {
                        StepResult::ForceYield
                    } else {
                        StepResult::Yield
                    }
                } else {
                    StepResult::Unimplemented("ECALL")
                }
            } else if imm12 == 1 {
                StepResult::Halt(0)
            } else {
                StepResult::Unimplemented("SYSTEM")
            }
        } else {
            self.op_csr(instruction, imm12 as u16, f3)
        }
    }

    // Machine will always be single-threaded, so we don't need to worry about ordering
    fn op_csr(&mut self, instruction: u32, csr: u16, f3: u32) -> StepResult {
        let rd = Self::rd(instruction);
        let rs1 = Self::rs1(instruction);
        let uimm = rs1 as u32;
        let old = self.csrs.read(csr);
        let write_value = match f3 {
            Self::F3_CSRRW | Self::F3_CSRRWI => {
                if f3 == Self::F3_CSRRWI {
                    uimm
                } else {
                    self.x(rs1)
                }
            }
            Self::F3_CSRRS | Self::F3_CSRRSI => {
                old | if f3 == Self::F3_CSRRSI {
                    uimm
                } else {
                    self.x(rs1)
                }
            }
            Self::F3_CSRRC | Self::F3_CSRRCI => {
                old & !(if f3 == Self::F3_CSRRCI {
                    uimm
                } else {
                    self.x(rs1)
                })
            }
            _ => return StepResult::Unimplemented("SYSTEM"),
        };
        let should_write_csr = match f3 {
            Self::F3_CSRRW | Self::F3_CSRRWI => true,
            Self::F3_CSRRS | Self::F3_CSRRSI => {
                (if f3 == Self::F3_CSRRSI {
                    uimm
                } else {
                    self.x(rs1)
                }) != 0
            }
            Self::F3_CSRRC | Self::F3_CSRRCI => {
                (if f3 == Self::F3_CSRRCI {
                    uimm
                } else {
                    self.x(rs1)
                }) != 0
            }
            _ => false,
        };
        if should_write_csr {
            self.csrs.write(csr, write_value);
        }
        if rd != 0 {
            self.set_x(rd, old);
        }
        StepResult::Continue
    }

    fn op_lui(&mut self, instruction: u32) -> StepResult {
        let rd = Self::rd(instruction);
        let imm = Self::imm_u(instruction);
        self.set_x(rd, imm);
        StepResult::Continue
    }

    fn op_auipc(&mut self, instruction: u32) -> StepResult {
        let rd = Self::rd(instruction);
        let imm = Self::imm_u(instruction);
        let pc = self.registers.pc;
        self.set_x(rd, pc.wrapping_add(imm));
        StepResult::Continue
    }

    fn op_reg(&mut self, instruction: u32) -> StepResult {
        let rd = Self::rd(instruction);
        let rs1 = Self::rs1(instruction);
        let rs2 = Self::rs2(instruction);
        let f3 = Self::funct3(instruction);
        let f7 = Self::funct7(instruction);
        let a = self.x(rs1);
        let b = self.x(rs2);
        let shamt = b & 0x1f;
        let result = match (f7, f3) {
            (0x00, 0x0) => a.wrapping_add(b),
            (0x20, 0x0) => a.wrapping_sub(b),
            (0x00, 0x1) => a << shamt,
            (0x00, 0x2) => ((a as i32) < (b as i32)) as u32,
            (0x00, 0x3) => (a < b) as u32,
            (0x00, 0x4) => a ^ b,
            (0x00, 0x5) => a >> shamt,
            (0x20, 0x5) => ((a as i32) >> (shamt as i32)) as u32,
            (0x00, 0x6) => a | b,
            (0x00, 0x7) => a & b,
            _ => return StepResult::Unimplemented("OP"),
        };
        self.set_x(rd, result);
        StepResult::Continue
    }

    fn op_imm(&mut self, instruction: u32) -> StepResult {
        let rd = Self::rd(instruction);
        let rs1 = Self::rs1(instruction);
        let f3 = Self::funct3(instruction);
        let f7 = Self::funct7(instruction);
        let a = self.x(rs1);
        let imm = Self::imm_i(instruction);
        let shamt = imm & 0x1f;
        let result = match (f7, f3) {
            (_, 0x0) => a.wrapping_add(imm),
            (_, 0x2) => ((a as i32) < (imm as i32)) as u32,
            (_, 0x3) => (a < imm) as u32,
            (_, 0x4) => a ^ imm,
            (0x00, 0x5) => a >> shamt,
            (0x20, 0x5) => ((a as i32) >> (shamt as i32)) as u32,
            (_, 0x6) => a | imm,
            (_, 0x7) => a & imm,
            (0x00, 0x1) => a << shamt,
            _ => return StepResult::Unimplemented("OP-IMM"),
        };
        self.set_x(rd, result);
        StepResult::Continue
    }

    fn op_load(&mut self, instruction: u32) -> StepResult {
        let rd = Self::rd(instruction);
        let rs1 = Self::rs1(instruction);
        let f3 = Self::funct3(instruction);
        let base = self.x(rs1);
        let offset = Self::imm_i(instruction);
        let addr = base.wrapping_add(offset);
        let value = match f3 {
            0 => self.load_i8(addr) as i32 as u32,
            1 => self.load_i16(addr) as i32 as u32,
            2 => self.load_u32(addr),
            4 => self.load_u8(addr) as u32,
            5 => self.load_u16(addr) as u32,
            _ => return StepResult::Unimplemented("LOAD"),
        };
        self.set_x(rd, value);
        StepResult::Continue
    }

    fn op_store(&mut self, instruction: u32) -> StepResult {
        let rs1 = Self::rs1(instruction);
        let rs2 = Self::rs2(instruction);
        let f3 = Self::funct3(instruction);
        let base = self.x(rs1);
        let offset = Self::imm_s(instruction);
        let addr = base.wrapping_add(offset);
        let value = self.x(rs2);
        match f3 {
            0 => self.store_u8(addr, value as u8),
            1 => self.store_u16(addr, value as u16),
            2 => self.store_u32(addr, value),
            _ => return StepResult::Unimplemented("STORE"),
        };
        StepResult::Continue
    }

    fn op_jal(&mut self, instruction: u32) -> StepResult {
        let rd = Self::rd(instruction);
        let pc = self.registers.pc;
        let offset = Self::imm_j(instruction);
        self.set_x(rd, pc.wrapping_add(4));
        self.registers.pc = pc.wrapping_add(offset);
        StepResult::PcUpdated
    }

    fn op_jalr(&mut self, instruction: u32) -> StepResult {
        let rd = Self::rd(instruction);
        let rs1 = Self::rs1(instruction);
        let pc = self.registers.pc;
        let target = self.x(rs1).wrapping_add(Self::imm_i(instruction));
        self.set_x(rd, pc.wrapping_add(4));
        self.registers.pc = target & !1u32;
        StepResult::PcUpdated
    }

    fn op_branch(&mut self, instruction: u32) -> StepResult {
        let rs1 = Self::rs1(instruction);
        let rs2 = Self::rs2(instruction);
        let f3 = Self::funct3(instruction);
        let pc = self.registers.pc;
        let offset = Self::imm_b(instruction);
        let a = self.x(rs1) as i32;
        let b = self.x(rs2) as i32;
        let take = match f3 {
            0x0 => a == b,
            0x1 => a != b,
            0x4 => a < b,
            0x5 => a >= b,
            0x6 => (a as u32) < (b as u32),
            0x7 => (a as u32) >= (b as u32),
            _ => return StepResult::Unimplemented("BRANCH"),
        };
        if take {
            self.registers.pc = pc.wrapping_add(offset);
            StepResult::PcUpdated
        } else {
            StepResult::Continue
        }
    }

    fn op_misc_mem(&mut self, _instruction: u32) -> StepResult {
        StepResult::Continue
    }
}

#[cfg(test)]
#[allow(clippy::int_plus_one)]
mod tests {
    use super::*;
    use crate::machine::{Machine, StepResult};

    const EBREAK_BYTES: [u8; 4] = [0x73, 0x00, 0x10, 0x00];

    fn encode_r(funct7: u32, rs2: u8, rs1: u8, funct3: u32, rd: u8, opcode: u32) -> u32 {
        (funct7 << 25)
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | ((rd as u32) << 7)
            | opcode
    }

    fn encode_i(imm: i32, rs1: u8, funct3: u32, rd: u8, opcode: u32) -> u32 {
        (((imm as u32) & 0xfff) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | ((rd as u32) << 7)
            | opcode
    }

    fn encode_s(imm: i32, rs2: u8, rs1: u8, funct3: u32, opcode: u32) -> u32 {
        let imm_u = (imm as u32) & 0xfff;
        ((imm_u >> 5) << 25)
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | ((imm_u & 0x1f) << 7)
            | opcode
    }

    fn encode_b(imm: i32, rs2: u8, rs1: u8, funct3: u32, opcode: u32) -> u32 {
        let imm_u = (imm as u32) & 0x1fff;
        let bit12 = (imm_u >> 12) & 0x1;
        let bit11 = (imm_u >> 11) & 0x1;
        let bits10_5 = (imm_u >> 5) & 0x3f;
        let bits4_1 = (imm_u >> 1) & 0xf;
        (bit12 << 31)
            | (bits10_5 << 25)
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | (bits4_1 << 8)
            | (bit11 << 7)
            | opcode
    }

    fn encode_u(imm: u32, rd: u8, opcode: u32) -> u32 {
        (imm & 0xffff_f000) | ((rd as u32) << 7) | opcode
    }

    fn encode_j(imm: i32, rd: u8, opcode: u32) -> u32 {
        let imm_u = (imm as u32) & 0x1f_ffff;
        let bit20 = (imm_u >> 20) & 0x1;
        let bits10_1 = (imm_u >> 1) & 0x3ff;
        let bit11 = (imm_u >> 11) & 0x1;
        let bits19_12 = (imm_u >> 12) & 0xff;
        (bit20 << 31)
            | (bits19_12 << 12)
            | (bit11 << 20)
            | (bits10_1 << 21)
            | ((rd as u32) << 7)
            | opcode
    }

    fn encode_csr(csr: u32, rs1: u8, funct3: u32, rd: u8) -> u32 {
        (csr << 20) | ((rs1 as u32) << 15) | (funct3 << 12) | ((rd as u32) << 7) | 0x73
    }

    fn assemble(instructions: &[u32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(instructions.len() * 4);
        for insn in instructions {
            bytes.extend_from_slice(&insn.to_le_bytes());
        }
        bytes
    }

    fn machine_with_program(instructions: &[u32]) -> RiscV32IntegerMachine {
        let mut machine = RiscV32IntegerMachine::new();
        machine.load_binary(&assemble(instructions));
        machine
    }

    fn run_until_halt(machine: &mut RiscV32IntegerMachine) {
        loop {
            match machine.step() {
                StepResult::Continue
                | StepResult::Yield
                | StepResult::ForceYield
                | StepResult::PcUpdated => {}
                StepResult::Halt(_) => break,
                StepResult::Unimplemented(op) => panic!("unexpected unimplemented: {op}"),
            }
        }
    }

    #[test]
    fn step_ebreak_returns_halt() {
        let mut machine = RiscV32IntegerMachine::new();
        machine.load_binary(&EBREAK_BYTES);
        assert_eq!(machine.current_tick(), 0);
        let result = machine.step();
        assert!(matches!(result, StepResult::Halt(0)));
        assert!(machine.current_tick() >= 1);
    }

    #[test]
    fn csr_read_write_and_set_clear() {
        // CSRRW: write a0 to CSR 0x300 (mstatus), copy old to t0
        // CSRRS: set bits in CSR from t0, copy old to t1
        // CSRRC: clear bits in CSR from t1, copy old to t2
        // Then ebreak
        let program = vec![
            encode_i(0x123, 0, 0x0, 10, 0x13), // addi a0, x0, 0x123
            encode_csr(0x300, 10, 0x1, 5), // csrrw t0, mstatus, a0  (rd=t0 gets old, csr gets 0x123)
            encode_i(0x004, 0, 0x0, 6, 0x13), // addi t1, x0, 4
            encode_csr(0x300, 6, 0x2, 7),  // csrrs t2, mstatus, t1   (set bit 2; t2 gets 0x123)
            encode_csr(0x300, 7, 0x3, 28), // csrrc t3, mstatus, t2   (clear t2's bits; t3 gets new csr)
            encode_i(1, 0, 0x0, 17, 0x13), // addi a7, x0, 1
            0x0010_0073,                   // ebreak
        ];
        let mut m = machine_with_program(&program);
        run_until_halt(&mut m);
        // After csrrw: csr=0x123, t0=0
        // After csrrs 4: csr=0x123|4=0x127, t2=0x123
        // After csrrc 0x123: csr=0x127&!0x123=4, t3=0x127
        assert_eq!(m.x(5), 0, "t0 (old mstatus)");
        assert_eq!(m.x(7), 0x123, "t2 (old before set)");
        assert_eq!(m.x(28), 0x127, "t3 (old before clear)");
    }

    #[test]
    fn csr_immediate_variants_no_op_on_rd_or_rs1_zero() {
        // CSRRWI x0, 0xC00, 0: rd=0 so no GPR write; uimm=0 so write 0 to CSR (no-op for CSRRSI/CSRRCI)
        // CSRRWI t0, 0xC00, 7: t0 = old, CSR 0xC00 = 7
        let program = vec![
            encode_csr(0xC00, 0, 0x5, 0), // csrrwi x0, 0xC00, 0
            encode_csr(0xC00, 7, 0x5, 5), // csrrwi t0, 0xC00, 7
            encode_csr(0xC00, 0, 0x5, 6), // csrrwi t1, 0xC00, 0  -> t1 = 7, csr stays 7
            0x0010_0073,                  // ebreak
        ];
        let mut m = machine_with_program(&program);
        run_until_halt(&mut m);
        assert_eq!(m.x(5), 0, "t0 gets previous CSR value (0)");
        assert_eq!(m.x(6), 7, "t1 gets 7 from CSR before write of 0");
    }

    #[test]
    fn new_load_and_run_uses_common_builtins() {
        let program = vec![
            encode_i(42, 0, 0x0, 10, 0x13), // addi a0, x0, 42
            encode_i(1, 0, 0x0, 17, 0x13),  // addi a7, x0, 1 (printf)
            0x0000_0073,                    // ecall
            0x0010_0073,                    // ebreak
        ];
        let mut machine = RiscV32IntegerMachine::new();
        machine.load_binary(&assemble(&program));
        let exit = machine.run_until_halt();

        assert_eq!(exit, 0);
    }

    #[test]
    fn reg_and_imm_alu_behaviors() {
        let program = vec![
            encode_i(10, 0, 0x0, 1, 0x13),      // addi x1, x0, 10
            encode_i(3, 0, 0x0, 2, 0x13),       // addi x2, x0, 3
            encode_r(0x00, 2, 1, 0x0, 3, 0x33), // add x3, x1, x2
            encode_r(0x20, 2, 1, 0x0, 4, 0x33), // sub x4, x1, x2
            encode_r(0x00, 2, 1, 0x7, 5, 0x33), // and x5, x1, x2
            encode_r(0x00, 2, 1, 0x6, 6, 0x33), // or x6, x1, x2
            encode_r(0x00, 2, 1, 0x4, 7, 0x33), // xor x7, x1, x2
            encode_i(1, 0, 0x0, 0, 0x13),       // addi x0, x0, 1 (must be ignored)
            0x0010_0073,                        // ebreak
        ];
        let mut machine = machine_with_program(&program);
        run_until_halt(&mut machine);

        assert_eq!(machine.x(1), 10);
        assert_eq!(machine.x(2), 3);
        assert_eq!(machine.x(3), 13);
        assert_eq!(machine.x(4), 7);
        assert_eq!(machine.x(5), 10 & 3);
        assert_eq!(machine.x(6), 10 | 3);
        assert_eq!(machine.x(7), 10 ^ 3);
        assert_eq!(machine.x(0), 0);
    }

    #[test]
    fn lui_auipc_behaviors() {
        let program = vec![
            encode_u(0x1234_5000, 1, 0x37), // lui x1, 0x12345
            encode_u(0x0000_1000, 2, 0x17), // auipc x2, 0x1
            0x0010_0073,                    // ebreak
        ];
        let mut machine = machine_with_program(&program);
        run_until_halt(&mut machine);

        assert_eq!(machine.x(1), 0x1234_5000);
        assert_eq!(machine.x(2), 0x1004);
    }

    #[test]
    fn load_store_signed_unsigned_behaviors() {
        let program = vec![
            encode_i(100, 0, 0x0, 1, 0x13), // addi x1, x0, 100
            encode_i(-1, 0, 0x0, 2, 0x13),  // addi x2, x0, -1
            encode_s(0, 2, 1, 0x0, 0x23),   // sb x2, 0(x1)
            encode_i(0, 1, 0x0, 3, 0x03),   // lb x3, 0(x1)
            encode_i(0, 1, 0x4, 4, 0x03),   // lbu x4, 0(x1)
            encode_s(2, 2, 1, 0x1, 0x23),   // sh x2, 2(x1)
            encode_i(2, 1, 0x1, 5, 0x03),   // lh x5, 2(x1)
            encode_i(2, 1, 0x5, 6, 0x03),   // lhu x6, 2(x1)
            encode_s(4, 2, 1, 0x2, 0x23),   // sw x2, 4(x1)
            encode_i(4, 1, 0x2, 7, 0x03),   // lw x7, 4(x1)
            0x0010_0073,                    // ebreak
        ];
        let mut machine = machine_with_program(&program);
        run_until_halt(&mut machine);

        assert_eq!(machine.x(3), 0xffff_ffff);
        assert_eq!(machine.x(4), 0x0000_00ff);
        assert_eq!(machine.x(5), 0xffff_ffff);
        assert_eq!(machine.x(6), 0x0000_ffff);
        assert_eq!(machine.x(7), 0xffff_ffff);
    }

    #[test]
    fn branch_and_jal_behaviors() {
        let program = vec![
            encode_i(1, 0, 0x0, 1, 0x13),  // addi x1, x0, 1
            encode_i(1, 0, 0x0, 2, 0x13),  // addi x2, x0, 1
            encode_b(8, 2, 1, 0x0, 0x63),  // beq x1, x2, +8 (skip next)
            encode_i(7, 0, 0x0, 3, 0x13),  // addi x3, x0, 7 (skipped)
            encode_i(42, 0, 0x0, 3, 0x13), // addi x3, x0, 42
            encode_j(8, 4, 0x6f),          // jal x4, +8 (skip next)
            encode_i(9, 0, 0x0, 5, 0x13),  // addi x5, x0, 9 (skipped)
            encode_i(11, 0, 0x0, 5, 0x13), // addi x5, x0, 11
            0x0010_0073,                   // ebreak
        ];
        let mut machine = machine_with_program(&program);
        run_until_halt(&mut machine);

        assert_eq!(machine.x(3), 42);
        assert_eq!(machine.x(4), 24); // jal at pc=20 stores pc+4
        assert_eq!(machine.x(5), 11);
    }

    #[test]
    fn jalr_behavior() {
        let program = vec![
            encode_i(12, 0, 0x0, 5, 0x13), // addi x5, x0, 12
            encode_i(0, 5, 0x0, 1, 0x67),  // jalr x1, x5, 0 (to pc=12)
            encode_i(1, 0, 0x0, 2, 0x13),  // addi x2, x0, 1 (skipped)
            encode_i(2, 0, 0x0, 2, 0x13),  // addi x2, x0, 2
            0x0010_0073,                   // ebreak
        ];
        let mut machine = machine_with_program(&program);
        run_until_halt(&mut machine);

        assert_eq!(machine.x(1), 8); // jalr at pc=4 stores pc+4
        assert_eq!(machine.x(2), 2);
    }

    #[test]
    fn ecall_dispatches_registered_builtin() {
        let program = vec![
            encode_i(7, 0, 0x0, 10, 0x13), // addi x10 (a0), x0, 7
            encode_i(1, 0, 0x0, 17, 0x13), // addi x17 (a7), x0, 1 (builtin number)
            0x0000_0073,                   // ecall
            0x0010_0073,                   // ebreak
        ];

        let mut machine = machine_with_program(&program);
        machine.register_builtin(1, |m| {
            let current = m.x(10);
            m.set_x(10, current.wrapping_add(35));
        });

        run_until_halt(&mut machine);
        assert_eq!(machine.x(10), 42);
    }

    #[test]
    fn ecall_builtin_returns_yield_and_advances_pc() {
        let program = vec![
            encode_i(1, 0, 0x0, 17, 0x13), // addi a7, x0, 1
            0x0000_0073,                   // ecall
            0x0010_0073,                   // ebreak
        ];
        let mut machine = machine_with_program(&program);
        machine.register_builtin(1, |_| {});

        assert!(matches!(machine.step(), StepResult::Continue));
        let result = machine.step();
        assert!(matches!(result, StepResult::Yield));
        assert!(matches!(machine.step(), StepResult::Halt(0)));
    }

    #[test]
    fn ecall_builtin_cycle_count_returns_monotonic_ticks() {
        let program = vec![
            encode_i(2, 0, 0x0, 17, 0x13), // addi a7, x0, 2 (cycle_count)
            0x0000_0073,                   // ecall -> a0/a1 = t0
            encode_i(0, 10, 0x0, 5, 0x13), // addi x5, a0, 0 (save low)
            encode_i(0, 11, 0x0, 6, 0x13), // addi x6, a1, 0 (save high)
            0x0000_0073,                   // ecall -> a0/a1 = t1
            0x0010_0073,                   // ebreak
        ];
        let mut machine = machine_with_program(&program);
        run_until_halt(&mut machine);

        let first = ((machine.x(6) as u64) << 32) | (machine.x(5) as u64);
        let second = ((machine.x(11) as u64) << 32) | (machine.x(10) as u64);
        assert!(first > 0);
        assert!(second > first);
    }

    #[test]
    fn instruction_cycles_add_up_for_simple_program() {
        let program = vec![
            encode_i(1, 0, 0x0, 1, 0x13), // addi x1, x0, 1  => 1 cycle
            0x0010_0073,                  // ebreak          => 1 cycle
        ];
        let mut machine = machine_with_program(&program);
        let exit = machine.run_until_halt();
        assert_eq!(exit, 0);
        assert!(machine.current_tick() >= 2);
    }

    #[test]
    fn taken_branch_costs_more_cycles() {
        let program = vec![
            encode_i(1, 0, 0x0, 1, 0x13),  // addi x1, x0, 1  => 1
            encode_i(1, 0, 0x0, 2, 0x13),  // addi x2, x0, 1  => 1
            encode_b(8, 2, 1, 0x0, 0x63),  // beq taken        => 2
            encode_i(99, 0, 0x0, 3, 0x13), // skipped
            0x0010_0073,                   // ebreak          => 1
        ];
        let mut machine = machine_with_program(&program);
        let exit = machine.run_until_halt();
        assert_eq!(exit, 0);
        assert!(machine.current_tick() >= 5);
        assert_eq!(machine.x(3), 0);
    }

    #[test]
    fn clock_advances_one_per_alu_instruction() {
        let mut machine = machine_with_program(&[encode_i(1, 0, 0x0, 1, 0x13), 0x0010_0073]);
        assert_eq!(machine.current_tick(), 0);
        let start = machine.current_tick();
        let r = machine.step();
        assert!(matches!(r, StepResult::Continue));
        let after_first = machine.current_tick();
        assert!(after_first - start >= 1);
        let r = machine.step();
        assert!(matches!(r, StepResult::Halt(0)));
        let after_second = machine.current_tick();
        assert!(after_second - after_first >= 1);
    }

    #[test]
    fn clock_advances_two_per_load_store() {
        let program = vec![
            encode_i(64, 0, 0x0, 1, 0x13), // addi x1, x0, 64 (base)
            encode_i(0, 1, 0x2, 2, 0x03),  // lw x2, 0(x1) => 2 cycles
            encode_s(0, 2, 1, 0x2, 0x23),  // sw x2, 0(x1) => 2 cycles
            0x0010_0073,                   // ebreak => 1
        ];
        let mut machine = machine_with_program(&program);
        machine.run_until_halt();
        assert!(machine.current_tick() >= 1 + 3 + 3 + 1);
    }

    #[test]
    fn clock_advances_two_per_jump() {
        let program = vec![
            encode_j(4, 0, 0x6f),         // jal x0, +4 => 2 cycles, land on next
            encode_i(0, 0, 0x0, 1, 0x13), // addi (1 cycle)
            0x0010_0073,                  // ebreak => 1
        ];
        let mut machine = machine_with_program(&program);
        machine.run_until_halt();
        assert!(machine.current_tick() >= 3 + 1 + 1);
    }

    #[test]
    fn clock_advances_eight_for_ecall() {
        let mut machine = machine_with_program(&[
            encode_i(1, 0, 0x0, 17, 0x13), // addi a7, x0, 1 => 1
            0x0000_0073,                   // ecall => 8
            0x0010_0073,                   // ebreak => 1
        ]);
        machine.register_builtin(1, |_| {});
        assert_eq!(machine.current_tick(), 0);
        let t0 = machine.current_tick();
        let _ = machine.step();
        let t1 = machine.current_tick();
        assert!(t1 - t0 >= 1);
        let _ = machine.step();
        let t2 = machine.current_tick();
        assert!(t2 - t1 >= 8);
        let _ = machine.step();
        let t3 = machine.current_tick();
        assert!(t3 - t2 >= 1);
    }

    #[test]
    fn not_taken_branch_costs_one_cycle() {
        let program = vec![
            encode_i(0, 0, 0x0, 1, 0x13),  // addi x1, x0, 0  => 1
            encode_i(1, 0, 0x0, 2, 0x13),  // addi x2, x0, 1  => 1
            encode_b(8, 2, 1, 0x0, 0x63),  // beq not taken (x1!=x2) => 1
            encode_i(42, 0, 0x0, 3, 0x13), // addi x3, x0, 42 => 1
            0x0010_0073,                   // ebreak => 1
        ];
        let mut machine = machine_with_program(&program);
        machine.run_until_halt();
        assert!(machine.current_tick() >= 1 + 1 + 1 + 1 + 1);
        assert_eq!(machine.x(3), 42);
    }
}
