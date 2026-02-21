use std::fs;
use std::path::Path;

use crate::machine::Machine;

pub fn run_flat_binary_bytes<M: Machine>(machine: &mut M, bytes: &[u8]) -> i32 {
    machine.load_binary(bytes);
    machine.run_until_halt()
}

pub fn run_flat_binary_file<M: Machine>(machine: &mut M, path: &Path) -> std::io::Result<i32> {
    let bytes = fs::read(path)?;
    Ok(run_flat_binary_bytes(machine, &bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::RiscV32IntegerMachine;

    /// EBREAK (halt with code 0) as raw bytes — same as step_ebreak_returns_halt.
    const EBREAK_BYTES: [u8; 4] = 0x0010_0073u32.to_le_bytes();

    #[test]
    fn run_flat_binary_bytes_smoke_test() {
        let mut machine = RiscV32IntegerMachine::new();
        let exit = run_flat_binary_bytes(&mut machine, &EBREAK_BYTES);
        assert_eq!(exit, 0);
    }
}
