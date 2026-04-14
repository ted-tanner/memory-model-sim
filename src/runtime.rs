use std::fs;
use std::path::Path;

use crate::device::secdcp_memory::SecurityClass;
use crate::machine::{Machine, MemorySegment, Registers, RiscV32IntegerMachine, StepResult};

#[derive(Clone)]
struct ExecutionContext {
    registers: Registers,
    memory_segment: MemorySegment,
    security_class: SecurityClass,
    exited: bool,
    exit_code: i32,
}

impl ExecutionContext {
    fn new(entry_pc: u32, memory_segment: MemorySegment, security_class: SecurityClass) -> Self {
        let mut registers = Registers::new();
        registers.pc = entry_pc;
        Self {
            registers,
            memory_segment,
            security_class,
            exited: false,
            exit_code: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProgramLayout {
    pub load_base: u32,
    pub entry_pc: u32,
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
    let total_memory = machine.memory_size_bytes();
    let midpoint = total_memory / 2;
    let segment_a = MemorySegment {
        start: 0,
        end_exclusive: midpoint,
    };
    let segment_b = MemorySegment {
        start: midpoint as u32,
        end_exclusive: total_memory,
    };

    assert!(
        segment_a
            .as_range()
            .contains(&(layout.program_a.load_base as u64))
            && segment_a
                .as_range()
                .contains(&(layout.program_a.entry_pc as u64)),
        "program A layout must stay within the first half of memory"
    );
    assert!(
        segment_b
            .as_range()
            .contains(&(layout.program_b.load_base as u64))
            && segment_b
                .as_range()
                .contains(&(layout.program_b.entry_pc as u64)),
        "program B layout must stay within the second half of memory"
    );
    assert!(
        (layout.program_a.load_base as u64) + program_a_bytes.len() as u64
            <= segment_a.end_exclusive,
        "program A binary does not fit within the first half of memory"
    );
    assert!(
        (layout.program_b.load_base as u64) + program_b_bytes.len() as u64
            <= segment_b.end_exclusive,
        "program B binary does not fit within the second half of memory"
    );

    machine.load_binary_at(program_a_bytes, layout.program_a.load_base);
    machine.load_binary_at(program_b_bytes, layout.program_b.load_base);

    let mut contexts = [
        ExecutionContext::new(layout.program_a.entry_pc, segment_a, SecurityClass::High),
        ExecutionContext::new(layout.program_b.entry_pc, segment_b, SecurityClass::Low),
    ];
    let mut current = 0usize;
    machine.set_memory_segment(contexts[current].memory_segment);
    machine.set_security_class(contexts[current].security_class);
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
                        machine.set_memory_segment(contexts[current].memory_segment);
                        machine.set_security_class(contexts[current].security_class);
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
                    machine.set_memory_segment(contexts[current].memory_segment);
                    machine.set_security_class(contexts[current].security_class);
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
                    machine.set_memory_segment(contexts[current].memory_segment);
                    machine.set_security_class(contexts[current].security_class);
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
            },
            program_b: ProgramLayout {
                load_base: 0x8001_0000,
                entry_pc: 0x8001_0000,
            },
        };

        let mut machine = RiscV32IntegerMachine::new();
        let (victim_exit, aggressor_exit) =
            run_dual_flat_binary_bytes(&mut machine, &victim, &aggressor, layout);

        assert_eq!(victim_exit, 0);
        assert_eq!(aggressor_exit, 0);
    }

    #[test]
    fn dual_runner_enforces_memory_segments() {
        let victim = assemble(&[0x0010_0073]);
        let aggressor = assemble(&[
            encode_i(0, 0, 0x2, 10, 0x03), // lw a0, 0(x0) -> outside program B segment
            0x0010_0073,
        ]);
        let layout = DualProgramLayout {
            program_a: ProgramLayout {
                load_base: 0,
                entry_pc: 0,
            },
            program_b: ProgramLayout {
                load_base: 0x8001_0000,
                entry_pc: 0x8001_0000,
            },
        };

        let mut machine = RiscV32IntegerMachine::new();
        let (victim_exit, aggressor_exit) =
            run_dual_flat_binary_bytes(&mut machine, &victim, &aggressor, layout);

        assert_eq!(victim_exit, 0);
        assert_eq!(aggressor_exit, -11);
    }
}
