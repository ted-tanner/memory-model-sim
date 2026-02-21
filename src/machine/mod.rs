#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepResult {
    Continue,
    PcUpdated,
    Halt(i32),
    Unimplemented(&'static str),
}

pub trait Machine {
    fn load_binary(&mut self, bytes: &[u8]);
    fn step(&mut self) -> StepResult;
    fn current_tick(&self) -> u64;

    fn run_until_halt(&mut self) -> i32 {
        loop {
            match self.step() {
                StepResult::Continue | StepResult::PcUpdated => {}
                StepResult::Halt(code) => return code,
                StepResult::Unimplemented(_) => panic!("unimplemented instruction"),
            }
        }
    }
}

mod riscv32_integer;
pub use riscv32_integer::{Registers, RiscV32IntegerMachine};
