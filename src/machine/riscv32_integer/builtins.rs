use std::io::{self, Write};

use super::RiscV32IntegerMachine;

pub const BUILTIN_PRINTF: u32 = 1;

pub fn register_common_builtins(machine: &mut RiscV32IntegerMachine) {
    machine.register_builtin(BUILTIN_PRINTF, printf);
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
