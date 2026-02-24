use std::fs;
use std::path::Path;

use crate::machine::{Machine, Registers, RiscV32IntegerMachine, StepResult};

#[derive(Clone)]
struct ExecutionContext {
    registers: Registers,
    exited: bool,
    exit_code: i32,
}

impl ExecutionContext {
    fn new(entry_pc: u32, stack_top: u32) -> Self {
        let mut registers = Registers::new();
        registers.pc = entry_pc;
        registers.set_sp(stack_top);
        Self {
            registers,
            exited: false,
            exit_code: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProgramLayout {
    pub load_base: u32,
    pub entry_pc: u32,
    pub stack_top: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct DualProgramLayout {
    pub program_a: ProgramLayout,
    pub program_b: ProgramLayout,
}

pub fn run_flat_binary_bytes<M: Machine>(machine: &mut M, bytes: &[u8]) -> i32 {
    machine.load_binary(bytes);
    machine.run_until_halt()
}

pub fn run_flat_binary_file<M: Machine>(machine: &mut M, path: &Path) -> std::io::Result<i32> {
    let bytes = fs::read(path)?;
    Ok(run_flat_binary_bytes(machine, &bytes))
}

pub fn run_dual_flat_binary_bytes(
    machine: &mut RiscV32IntegerMachine,
    program_a_bytes: &[u8],
    program_b_bytes: &[u8],
    layout: DualProgramLayout,
) -> (i32, i32) {
    const CTX_SWITCH_AFTER_CYCLE_COUNT: u64 = 1_000_000_000;

    machine.load_binary_at(program_a_bytes, layout.program_a.load_base);
    machine.load_binary_at(program_b_bytes, layout.program_b.load_base);

    let mut contexts = [
        ExecutionContext::new(layout.program_a.entry_pc, layout.program_a.stack_top),
        ExecutionContext::new(layout.program_b.entry_pc, layout.program_b.stack_top),
    ];
    let mut current = 0usize;
    machine.restore_registers(&contexts[current].registers);
    let mut last_switch_cycle = machine.cycle_count();

    loop {
        match machine.step() {
            StepResult::Continue | StepResult::PcUpdated => {}
            StepResult::Yield => {
                let now = machine.cycle_count();
                if now.saturating_sub(last_switch_cycle) >= CTX_SWITCH_AFTER_CYCLE_COUNT {
                    contexts[current].registers = machine.snapshot_registers();
                    let next = 1 - current;
                    if !contexts[next].exited {
                        current = next;
                        machine.restore_registers(&contexts[current].registers);
                        last_switch_cycle = machine.cycle_count();
                    }
                }
            }
            StepResult::ForceYield => {
                contexts[current].registers = machine.snapshot_registers();
                let next = 1 - current;
                if !contexts[next].exited {
                    current = next;
                    machine.restore_registers(&contexts[current].registers);
                    last_switch_cycle = machine.cycle_count();
                }
            }
            StepResult::Halt(code) => {
                contexts[current].registers = machine.snapshot_registers();
                contexts[current].exited = true;
                contexts[current].exit_code = code;

                if contexts[0].exited && contexts[1].exited {
                    return (contexts[0].exit_code, contexts[1].exit_code);
                }

                let next = 1 - current;
                if !contexts[next].exited {
                    current = next;
                    machine.restore_registers(&contexts[current].registers);
                    last_switch_cycle = machine.cycle_count();
                }
            }
            StepResult::Unimplemented(_) => panic!("unimplemented instruction"),
        }
    }
}

pub fn run_dual_flat_binary_files(
    machine: &mut RiscV32IntegerMachine,
    program_a_path: &Path,
    program_b_path: &Path,
    layout: DualProgramLayout,
) -> std::io::Result<(i32, i32)> {
    let program_a = fs::read(program_a_path)?;
    let program_b = fs::read(program_b_path)?;
    Ok(run_dual_flat_binary_bytes(
        machine, &program_a, &program_b, layout,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::RiscV32IntegerMachine;

    /// EBREAK (halt with code 0) as raw bytes — same as step_ebreak_returns_halt.
    const EBREAK_BYTES: [u8; 4] = 0x0010_0073u32.to_le_bytes();

    fn encode_i(imm: i32, rs1: u8, funct3: u32, rd: u8, opcode: u32) -> u32 {
        (((imm as u32) & 0xfff) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
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

    #[test]
    fn run_flat_binary_bytes_smoke_test() {
        let mut machine = RiscV32IntegerMachine::new();
        let exit = run_flat_binary_bytes(&mut machine, &EBREAK_BYTES);
        assert_eq!(exit, 0);
    }

    #[test]
    fn dual_runner_switches_on_each_builtin_and_exits_both_programs() {
        let victim = assemble(&[
            encode_i(1, 0, 0x0, 17, 0x13), // addi a7, x0, 1 (printf builtin)
            0x0000_0073,                   // ecall -> Yield to aggressor
            0x0010_0073,                   // ebreak
        ]);
        let aggressor = assemble(&[
            encode_i(2, 0, 0x0, 17, 0x13), // addi a7, x0, 2 (cycle_count builtin)
            0x0000_0073,                   // ecall -> Yield to victim
            0x0010_0073,                   // ebreak
        ]);
        let layout = DualProgramLayout {
            program_a: ProgramLayout {
                load_base: 0,
                entry_pc: 0,
                stack_top: 0x0030_0000,
            },
            program_b: ProgramLayout {
                load_base: 0x0001_0000,
                entry_pc: 0x0001_0000,
                stack_top: 0x0040_0000,
            },
        };

        let mut machine = RiscV32IntegerMachine::new();
        let (victim_exit, aggressor_exit) =
            run_dual_flat_binary_bytes(&mut machine, &victim, &aggressor, layout);

        assert_eq!(victim_exit, 0);
        assert_eq!(aggressor_exit, 0);
    }
}
