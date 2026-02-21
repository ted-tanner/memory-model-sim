use super::RiscV32IntegerMachine;

pub const BUILTIN_PRINTF: u32 = 1;

pub fn register_common_builtins(machine: &mut RiscV32IntegerMachine) {
    machine.register_builtin(BUILTIN_PRINTF, printf);
}

pub fn printf(machine: &mut RiscV32IntegerMachine) {
    let value = machine.x(10);
    println!("{value}");
}
