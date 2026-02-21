use std::collections::BTreeMap;

use crate::device::{Clock, memory::MemoryDevice};
use crate::machine::{Machine, StepResult};

mod builtins;

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

pub struct RiscV32IntegerMachine {
    clock: Clock,
    memory: Box<dyn MemoryDevice>,
    registers: Registers,
    builtins: BTreeMap<u32, Box<dyn FnMut(&mut RiscV32IntegerMachine)>>,
}

impl RiscV32IntegerMachine {
    pub fn new(clock: Clock, memory: Box<dyn MemoryDevice>) -> Self {
        Self {
            clock,
            memory,
            registers: Registers::new(),
            builtins: BTreeMap::new(),
        }
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

    pub fn register_builtin(&mut self, number: u32, f: impl FnMut(&mut Self) + 'static) {
        self.builtins.insert(number, Box::new(f));
    }

    fn fetch_u32(&self, addr: u32) -> u32 {
        self.memory.load_u32(addr as usize)
    }

    fn load_u8(&self, addr: u32) -> u8 {
        self.memory.load_u8(addr as usize)
    }
    fn load_i8(&self, addr: u32) -> i8 {
        self.memory.load_i8(addr as usize)
    }
    fn load_u16(&self, addr: u32) -> u16 {
        self.memory.load_u16(addr as usize)
    }
    fn load_i16(&self, addr: u32) -> i16 {
        self.memory.load_i16(addr as usize)
    }
    fn load_u32(&self, addr: u32) -> u32 {
        self.memory.load_u32(addr as usize)
    }
    fn store_u8(&mut self, addr: u32, v: u8) {
        self.memory.store_u8(addr as usize, v);
    }
    fn store_u16(&mut self, addr: u32, v: u16) {
        self.memory.store_u16(addr as usize, v);
    }
    fn store_u32(&mut self, addr: u32, v: u32) {
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
        let imm13 = ((instruction >> 31) as u32).wrapping_mul(0x1000)
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
        let imm20 = ((instruction >> 31) as u32).wrapping_mul(0x100000)
            | ((instruction >> 12) & 0xff).wrapping_mul(0x1000)
            | ((instruction >> 20) & 1).wrapping_mul(0x800)
            | ((instruction >> 21) & 0x3ff).wrapping_mul(2);
        if (imm20 & 0x100000) != 0 {
            imm20 | 0xffe0_0000
        } else {
            imm20
        }
    }
}

impl Machine for RiscV32IntegerMachine {
    fn load_binary(&mut self, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            self.memory.store_u8(i, b);
        }
        self.registers.pc = 0;
    }

    fn current_tick(&self) -> u64 {
        self.clock.curr_tick()
    }

    fn step(&mut self) -> StepResult {
        self.clock.tick();

        let pc = self.registers.pc;
        let instruction = self.fetch_u32(pc);

        let result = match instruction & 0x7f {
            0x73 => self.op_system(instruction),
            0x03 => self.op_load(instruction),
            0x23 => self.op_store(instruction),
            0x33 => self.op_reg(instruction),
            0x13 => self.op_imm(instruction),
            0x37 => self.op_lui(instruction),
            0x17 => self.op_auipc(instruction),
            0x6f => self.op_jal(instruction),
            0x67 => self.op_jalr(instruction),
            0x63 => self.op_branch(instruction),
            0x0f => self.op_misc_mem(instruction),
            _ => return StepResult::Unimplemented("unknown opcode"),
        };

        match result {
            StepResult::Continue => self.registers.pc = pc.wrapping_add(4),
            StepResult::PcUpdated => {}
            _ => {}
        }
        result
    }
}

impl RiscV32IntegerMachine {
    fn op_system(&mut self, instruction: u32) -> StepResult {
        let imm12 = (instruction >> 20) & 0xfff;
        if imm12 == 0 {
            let a7 = self.x(17);
            if let Some(mut handler) = self.builtins.remove(&a7) {
                handler(self);
                self.builtins.insert(a7, handler);
                StepResult::Continue
            } else {
                StepResult::Unimplemented("ECALL")
            }
        } else if imm12 == 1 {
            StepResult::Halt(0)
        } else {
            StepResult::Unimplemented("SYSTEM")
        }
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
        let shamt = (b & 0x1f) as u32;
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
mod tests {
    use super::*;
    use crate::device::Clock;
    use crate::device::memory::MainMemory;
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

    fn assemble(instructions: &[u32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(instructions.len() * 4);
        for insn in instructions {
            bytes.extend_from_slice(&insn.to_le_bytes());
        }
        bytes
    }

    fn machine_with_program(instructions: &[u32]) -> RiscV32IntegerMachine {
        let clock = Clock::new();
        let memory = Box::new(MainMemory::new(4096));
        let mut machine = RiscV32IntegerMachine::new(clock, memory);
        machine.load_binary(&assemble(instructions));
        machine
    }

    fn run_until_halt(machine: &mut RiscV32IntegerMachine) {
        loop {
            match machine.step() {
                StepResult::Continue | StepResult::PcUpdated => {}
                StepResult::Halt(_) => break,
                StepResult::Unimplemented(op) => panic!("unexpected unimplemented: {op}"),
            }
        }
    }

    #[test]
    fn step_ebreak_returns_halt() {
        let clock = Clock::new();
        let memory = Box::new(MainMemory::new(4096));
        let mut machine = RiscV32IntegerMachine::new(clock, memory);
        machine.load_binary(&EBREAK_BYTES);
        assert_eq!(machine.current_tick(), 0);
        let result = machine.step();
        assert!(matches!(result, StepResult::Halt(0)));
        assert_eq!(machine.current_tick(), 1);
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
}
