use std::path::Path;

use memory_model_sim::machine::{Machine, RiscV32IntegerMachine};
use memory_model_sim::runtime::{
    DualProgramLayout, ProgramLayout, run_dual_flat_binary_files, run_flat_binary_file,
};

const DEFAULT_DUAL_LAYOUT: DualProgramLayout = DualProgramLayout {
    program_a: ProgramLayout {
        load_base: 0x0000_0000,
        entry_pc: 0x0000_0000,
    },
    program_b: ProgramLayout {
        load_base: 0x0001_0000,
        entry_pc: 0x0001_0000,
    },
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args();
    let bin = args
        .next()
        .unwrap_or_else(|| "memory-model-sim".to_string());

    let Some(path_a) = args.next() else {
        eprintln!("Usage: {bin} <flat-binary-path> [flat-binary-path-2]");
        return Err("missing flat-binary-path".into());
    };
    let path_b = args.next();

    if args.next().is_some() {
        eprintln!("Usage: {bin} <flat-binary-path> [flat-binary-path-2]");
        return Err("too many arguments".into());
    }

    let mut machine = RiscV32IntegerMachine::new();
    if let Some(path_b) = path_b {
        let (a_exit, b_exit) = match run_dual_flat_binary_files(
            &mut machine,
            Path::new(&path_a),
            Path::new(&path_b),
            DEFAULT_DUAL_LAYOUT,
        ) {
            Ok(codes) => codes,
            Err(err) => {
                eprintln!("Failed to run binaries: {err}");
                return Err(err.into());
            }
        };
        println!("emulator: Program A returned exit code: {a_exit}");
        println!("emulator: Program B returned exit code: {b_exit}");
        if a_exit < 0 || b_exit < 0 {
            return Err(format!(
                "program returned negative exit code(s): victim={a_exit}, aggressor={b_exit}"
            )
            .into());
        }
    } else {
        let exit_code = match run_flat_binary_file(&mut machine, Path::new(&path_a)) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("Failed to run binary: {err}");
                return Err(err.into());
            }
        };
        println!("emulator: Program returned exit code: {exit_code}");
        if exit_code < 0 {
            return Err(format!("program returned negative exit code: {exit_code}").into());
        }
    }

    println!(
        "emulator: Program ran for {} clock cycles",
        machine.current_tick()
    );
    Ok(())
}
