#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepResult {
    Continue,
    Yield,
    ForceYield,
    PcUpdated,
    Halt(i32),
    Unimplemented(&'static str),
}

pub trait Machine {
    fn load_binary(&mut self, bytes: &[u8]);
    fn load_binary_at(&mut self, bytes: &[u8], _base: u32);
    fn step(&mut self) -> StepResult;
    fn current_tick(&self) -> u64;

    fn run_until_halt(&mut self) -> i32 {
        loop {
            match self.step() {
                StepResult::Continue
                | StepResult::Yield
                | StepResult::ForceYield
                | StepResult::PcUpdated => {}
                StepResult::Halt(code) => return code,
                StepResult::Unimplemented(_) => panic!("unimplemented instruction"),
            }
        }
    }
}

pub(crate) mod riscv32_integer;
pub use riscv32_integer::{MemoryModel, MemorySegment, Registers, RiscV32IntegerMachine};
