use std::path::Path;

use memory_model_sim::machine::{Machine, RiscV32IntegerMachine};
use memory_model_sim::runtime::run_flat_binary_file;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args();
    let bin = args
        .next()
        .unwrap_or_else(|| "memory-model-sim".to_string());

    let Some(path) = args.next() else {
        eprintln!("Usage: {bin} <flat-binary-path> [memory-bytes]");
        return Err("missing flat-binary-path".into());
    };

    if args.next().is_some() {
        eprintln!("Usage: {bin} <flat-binary-path>");
        return Err("too many arguments".into());
    }

    let mut machine = RiscV32IntegerMachine::new();
    let exit_code = match run_flat_binary_file(&mut machine, Path::new(&path)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Failed to run binary: {err}");
            return Err(err.into());
        }
    };

    println!("Program returned exit code: {exit_code}");
    println!("Program ran for {} ticks", machine.current_tick());

    if exit_code < 0 {
        return Err(format!("program returned negative exit code: {exit_code}").into());
    }
    Ok(())
}
