use std::path::Path;

use memory_model_sim::experiment::{
    AttackKind, ControlMode, DEFAULT_DECOY_SET, DEFAULT_TARGET_SET, DomainMode, ExperimentConfig,
    ScheduleMode, parse_attack_kind, parse_control_mode, parse_domain_mode, parse_schedule_mode,
    run_experiment,
};
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
        "smtcache" => Ok(MemoryModel::SmtCache),
        "backcache" => Ok(MemoryModel::BackCache),
        "newcache" => Ok(MemoryModel::NewCache),
        "secdcp" => Ok(MemoryModel::SecDcp),
        _ => Err(format!("unsupported memory model: {value}")),
    }
}

fn default_experiment_path(name: &str) -> String {
    format!("riscv-programs/{name}/firmware.bin")
}

fn parse_experiment_args(
    args: impl IntoIterator<Item = String>,
) -> Result<ExperimentConfig, String> {
    let mut attack = AttackKind::BinaryPrimeProbe;
    let mut schedule_mode = ScheduleMode::TimeSliced;
    let mut memory_model = MemoryModel::Default;
    let mut domain_mode = DomainMode::Different;
    let mut control = ControlMode::None;
    let mut trials = 16_384usize;
    let mut seed = 1u64;
    let mut target_set = DEFAULT_TARGET_SET;
    let mut decoy_set = DEFAULT_DECOY_SET;
    let mut attacker_path = default_experiment_path("l1d-attacker");
    let mut victim_path = default_experiment_path("l1d-victim");
    let mut out_dir = "results/l1d-experiment".to_string();

    for arg in args {
        if let Some(value) = arg.strip_prefix("--attack=") {
            attack = parse_attack_kind(value)?;
        } else if let Some(value) = arg.strip_prefix("--mode=") {
            schedule_mode = parse_schedule_mode(value)?;
        } else if let Some(value) = arg.strip_prefix("--memory-model=") {
            memory_model = parse_memory_model(value)?;
        } else if let Some(value) = arg.strip_prefix("--domains=") {
            domain_mode = parse_domain_mode(value)?;
        } else if let Some(value) = arg.strip_prefix("--control=") {
            control = parse_control_mode(value)?;
        } else if let Some(value) = arg.strip_prefix("--trials=") {
            trials = value
                .parse()
                .map_err(|_| format!("invalid --trials value: {value}"))?;
        } else if let Some(value) = arg.strip_prefix("--seed=") {
            seed = value
                .parse()
                .map_err(|_| format!("invalid --seed value: {value}"))?;
        } else if let Some(value) = arg.strip_prefix("--target-set=") {
            target_set = value
                .parse()
                .map_err(|_| format!("invalid --target-set value: {value}"))?;
        } else if let Some(value) = arg.strip_prefix("--decoy-set=") {
            decoy_set = value
                .parse()
                .map_err(|_| format!("invalid --decoy-set value: {value}"))?;
        } else if let Some(value) = arg.strip_prefix("--attacker=") {
            attacker_path = value.to_string();
        } else if let Some(value) = arg.strip_prefix("--victim=") {
            victim_path = value.to_string();
        } else if let Some(value) = arg.strip_prefix("--out=") {
            out_dir = value.to_string();
        } else {
            return Err(format!("unsupported experiment argument: {arg}"));
        }
    }

    if target_set >= 64 {
        return Err("--target-set must be in 0..64".to_string());
    }
    if decoy_set >= 64 {
        return Err("--decoy-set must be in 0..64".to_string());
    }

    Ok(ExperimentConfig {
        attack,
        schedule_mode,
        memory_model,
        domain_mode,
        control,
        trials,
        seed,
        target_set,
        decoy_set,
        attacker_path: attacker_path.into(),
        victim_path: victim_path.into(),
        out_dir: out_dir.into(),
    })
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

    let rest = args.collect::<Vec<_>>();
    if rest.first().is_some_and(|arg| arg == "experiment") {
        let config = match parse_experiment_args(rest.into_iter().skip(1)) {
            Ok(config) => config,
            Err(err) => {
                eprintln!(
                    "Usage: {bin} experiment [--attack=binary-pp|prime-probe|evict-time] [--mode=time-sliced|smt] [--memory-model=default|smtcache|backcache|newcache|secdcp] [--domains=different|same] [--trials=N] [--seed=N] [--out=DIR]"
                );
                return Err(err.into());
            }
        };
        run_experiment(&config)?;
        println!(
            "emulator: Experiment results written to {}",
            config.out_dir.display()
        );
        return Ok(());
    }

    let config = match parse_cli_args(rest) {
        Ok(config) => config,
        Err(err) => {
            eprintln!(
                "Usage: {bin} [--memory-model=default|smtcache|backcache|newcache|secdcp] <flat-binary-path> [flat-binary-path-2]"
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
    fn parse_cli_accepts_smtcache_memory_model() {
        let config = parse_cli_args([
            "--memory-model=smtcache".to_string(),
            "a.bin".to_string(),
            "b.bin".to_string(),
        ])
        .unwrap();
        assert_eq!(config.memory_model, MemoryModel::SmtCache);
    }

    #[test]
    fn parse_cli_accepts_newcache_memory_model() {
        let config = parse_cli_args([
            "--memory-model=newcache".to_string(),
            "a.bin".to_string(),
            "b.bin".to_string(),
        ])
        .unwrap();
        assert_eq!(config.memory_model, MemoryModel::NewCache);
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
