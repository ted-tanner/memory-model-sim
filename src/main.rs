use std::path::Path;

use memory_model_sim::machine::{Machine, MemoryModel, RiscV32IntegerMachine};
use memory_model_sim::runtime::{
    DualProgramLayout, ProgramLayout, run_dual_flat_binary_files, run_flat_binary_file,
};

const DEFAULT_DUAL_LAYOUT: DualProgramLayout = DualProgramLayout {
    program_a: ProgramLayout {
        load_base: 0x0000_0000,
        entry_pc: 0x0000_0000,
    },
    program_b: ProgramLayout {
        load_base: 0x8001_0000,
        entry_pc: 0x8001_0000,
    },
};

#[derive(Debug)]
struct CliConfig {
    memory_model: MemoryModel,
    path_a: String,
    path_b: Option<String>,
}

fn parse_memory_model(value: &str) -> Result<MemoryModel, String> {
    match value {
        "default" => Ok(MemoryModel::Default),
        "backcache" => Ok(MemoryModel::BackCache),
        "secdcp" => Ok(MemoryModel::SecDcp),
        _ => Err(format!("unsupported memory model: {value}")),
    }
}

fn parse_cli_args(args: impl IntoIterator<Item = String>) -> Result<CliConfig, String> {
    let mut memory_model = MemoryModel::Default;
    let mut positional = Vec::new();

    for arg in args {
        if let Some(value) = arg.strip_prefix("--memory-model=") {
            memory_model = parse_memory_model(value)?;
        } else {
            positional.push(arg);
        }
    }

    let Some(path_a) = positional.first().cloned() else {
        return Err("missing flat-binary-path".to_string());
    };
    let path_b = positional.get(1).cloned();
    if positional.len() > 2 {
        return Err("too many arguments".to_string());
    }

    Ok(CliConfig {
        memory_model,
        path_a,
        path_b,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args();
    let bin = args
        .next()
        .unwrap_or_else(|| "memory-model-sim".to_string());

    let config = match parse_cli_args(args) {
        Ok(config) => config,
        Err(err) => {
            eprintln!(
                "Usage: {bin} [--memory-model=default|backcache|secdcp] <flat-binary-path> [flat-binary-path-2]"
            );
            return Err(err.into());
        }
    };

    let mut machine = RiscV32IntegerMachine::with_memory_model(config.memory_model);
    if let Some(path_b) = config.path_b {
        let (a_exit, b_exit) = match run_dual_flat_binary_files(
            &mut machine,
            Path::new(&config.path_a),
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
        let exit_code = match run_flat_binary_file(&mut machine, Path::new(&config.path_a)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_defaults_to_default_memory_model() {
        let config = parse_cli_args(["prog.bin".to_string()]).unwrap();
        assert_eq!(config.memory_model, MemoryModel::Default);
        assert_eq!(config.path_a, "prog.bin");
        assert_eq!(config.path_b, None);
    }

    #[test]
    fn parse_cli_accepts_secdcp_memory_model() {
        let config = parse_cli_args([
            "--memory-model=secdcp".to_string(),
            "a.bin".to_string(),
            "b.bin".to_string(),
        ])
        .unwrap();
        assert_eq!(config.memory_model, MemoryModel::SecDcp);
        assert_eq!(config.path_a, "a.bin");
        assert_eq!(config.path_b.as_deref(), Some("b.bin"));
    }

    #[test]
    fn parse_cli_accepts_backcache_memory_model() {
        let config = parse_cli_args([
            "--memory-model=backcache".to_string(),
            "a.bin".to_string(),
            "b.bin".to_string(),
        ])
        .unwrap();
        assert_eq!(config.memory_model, MemoryModel::BackCache);
        assert_eq!(config.path_a, "a.bin");
        assert_eq!(config.path_b.as_deref(), Some("b.bin"));
    }

    #[test]
    fn parse_cli_rejects_unknown_memory_model() {
        let err =
            parse_cli_args(["--memory-model=bogus".to_string(), "a.bin".to_string()]).unwrap_err();
        assert!(err.contains("unsupported memory model"));
    }
}
