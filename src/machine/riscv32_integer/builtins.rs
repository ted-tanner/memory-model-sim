use std::io::{self, Write};

use super::RiscV32IntegerMachine;

pub const BUILTIN_PRINTF: u32 = 1;
pub const BUILTIN_CYCLE_COUNT: u32 = 2;
pub const BUILTIN_RANDOM: u32 = 3;
pub const BUILTIN_YIELD: u32 = 4;
pub const BUILTIN_MODULO: u32 = 5;

pub fn register_common_builtins(machine: &mut RiscV32IntegerMachine) {
    machine.register_builtin(BUILTIN_PRINTF, printf);
    machine.register_builtin(BUILTIN_CYCLE_COUNT, cycle_count);
    machine.register_builtin(BUILTIN_RANDOM, random);
    machine.register_builtin(BUILTIN_YIELD, yield_execution);
    machine.register_builtin(BUILTIN_MODULO, modulo);
}

fn yield_execution(_machine: &mut RiscV32IntegerMachine) {
    // No-op; scheduler forces context switch on ForceYield
}

fn random(machine: &mut RiscV32IntegerMachine) {
    let state = machine.random_state();
    machine.set_x(10, state as u32);
}

fn read_c_str(machine: &RiscV32IntegerMachine, ptr: u32) -> String {
    let mut s = Vec::new();
    let mut addr = ptr;
    loop {
        let b = (machine.load_u32(addr & !3) >> ((addr & 3) * 8)) as u8;
        if b == 0 {
            break;
        }
        s.push(b);
        addr = addr.wrapping_add(1);
    }
    String::from_utf8_lossy(&s).into_owned()
}

pub fn printf(machine: &mut RiscV32IntegerMachine) {
    let fmt_ptr = machine.x(10); // a0 = format string
    let mut arg_index: u8 = 1; // next variadic arg: a1, a2, ...

    let mut addr = fmt_ptr;
    let stdout = io::stdout();
    let mut out = stdout.lock();

    loop {
        let b = (machine.load_u32(addr & !3) >> ((addr & 3) * 8)) as u8;
        addr = addr.wrapping_add(1);
        if b == 0 {
            break;
        }
        if b != b'%' {
            let _ = out.write_all(&[b]);
            continue;
        }
        let spec = (machine.load_u32(addr & !3) >> ((addr & 3) * 8)) as u8;
        addr = addr.wrapping_add(1);
        match spec {
            b'd' => {
                let val = machine.x(10 + arg_index) as i32;
                let _ = write!(out, "{val}");
                arg_index = arg_index.saturating_add(1);
            }
            b's' => {
                let str_ptr = machine.x(10 + arg_index);
                let s = read_c_str(machine, str_ptr);
                let _ = write!(out, "{s}");
                arg_index = arg_index.saturating_add(1);
            }
            _ => {
                let _ = out.write_all(&[b'%', spec]);
            }
        }
    }
    let _ = writeln!(out);
}

pub fn cycle_count(machine: &mut RiscV32IntegerMachine) {
    let ticks = machine.cycle_count();
    machine.set_x(10, ticks as u32);
    machine.set_x(11, (ticks >> 32) as u32);
}

// Base RV32I doesn't have a modulo and a naive implementation with subtraction
// in a loop can take a LONG time for big numerators with small divisors
pub fn modulo(machine: &mut RiscV32IntegerMachine) {
    let dividend = machine.x(10);
    let divisor = machine.x(11);
    machine.set_x(10, dividend % divisor);
}
