use std::cell::RefCell;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;

use crate::device::cache_trace::{CacheAccessEvent, CacheAccessSource, CacheTrace};
use crate::device::secdcp_memory::SecurityClass;
use crate::machine::riscv32_integer::builtins;
use crate::machine::{
    Machine, MemoryModel, MemorySegment, Registers, RiscV32IntegerMachine, StepResult,
};

const SETS: usize = 64;
const RESERVED_STACK_SET: usize = 1;
pub const DEFAULT_TARGET_SET: usize = 17;
pub const DEFAULT_DECOY_SET: usize = 23;

pub const PHASE_HALT: u32 = 0;
pub const PHASE_PRIME: u32 = 1;
pub const PHASE_PROBE: u32 = 2;
pub const PHASE_VICTIM_ACCESS: u32 = 3;
pub const PHASE_WARM_VICTIM: u32 = 4;
pub const PHASE_EVICT_TARGET: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackKind {
    BinaryPrimeProbe,
    PrimeProbe,
    EvictTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleMode {
    TimeSliced,
    Smt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainMode {
    Different,
    Same,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlMode {
    None,
    NoVictim,
    ForcedEviction,
}

#[derive(Debug)]
pub struct ExperimentConfig {
    pub attack: AttackKind,
    pub schedule_mode: ScheduleMode,
    pub memory_model: MemoryModel,
    pub domain_mode: DomainMode,
    pub control: ControlMode,
    pub trials: usize,
    pub seed: u64,
    pub target_set: usize,
    pub decoy_set: usize,
    pub attacker_path: PathBuf,
    pub victim_path: PathBuf,
    pub out_dir: PathBuf,
}

#[derive(Default)]
struct ExperimentBuiltinState {
    phase: u32,
    secret_set: u32,
    secret_bit: u32,
    target_set: u32,
    done: bool,
    submitted: Vec<u64>,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
struct ProcessMetadata {
    pid: u32,
    asid: u32,
    security_domain_id: u32,
    current_core: u32,
    current_hardware_thread: u32,
}

struct ExecutionContext {
    registers: Registers,
    memory_segment: MemorySegment,
    security_class: SecurityClass,
    #[allow(dead_code)]
    metadata: ProcessMetadata,
    exited: bool,
    exit_code: i32,
}

struct SummaryStats {
    trials: usize,
    correct: usize,
    binary_tp: usize,
    binary_tn: usize,
    binary_fp: usize,
    binary_fn: usize,
    sum_touched: f64,
    sum_untouched: f64,
    sumsq_touched: f64,
    sumsq_untouched: f64,
    count_touched: usize,
    count_untouched: usize,
    total_trial_cycles: u64,
    victim_evicted_attacker_lines: u64,
    victim_memory_accesses: u64,
    binary_samples: Vec<(f64, bool)>,
    binary_joint: [[usize; 2]; 2],
    multi_joint: [[usize; SETS]; SETS],
}

impl Default for SummaryStats {
    fn default() -> Self {
        Self {
            trials: 0,
            correct: 0,
            binary_tp: 0,
            binary_tn: 0,
            binary_fp: 0,
            binary_fn: 0,
            sum_touched: 0.0,
            sum_untouched: 0.0,
            sumsq_touched: 0.0,
            sumsq_untouched: 0.0,
            count_touched: 0,
            count_untouched: 0,
            total_trial_cycles: 0,
            victim_evicted_attacker_lines: 0,
            victim_memory_accesses: 0,
            binary_samples: Vec::new(),
            binary_joint: [[0; 2]; 2],
            multi_joint: [[0; SETS]; SETS],
        }
    }
}

struct TrialRecord {
    target_set: Option<usize>,
    secret_set: usize,
    secret_bit: Option<u32>,
    probe_time: Vec<u64>,
    victim_elapsed_cycles: Option<u64>,
    total_trial_cycles: u64,
    attacker_l1d_evictions_by_victim: u64,
}

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    fn next_usize(&mut self, modulus: usize) -> usize {
        ((self.next() >> 32) as usize) % modulus
    }
}

impl ExecutionContext {
    fn new(
        entry_pc: u32,
        memory_segment: MemorySegment,
        security_class: SecurityClass,
        metadata: ProcessMetadata,
    ) -> Self {
        let mut registers = Registers::new();
        registers.pc = entry_pc;
        Self {
            registers,
            memory_segment,
            security_class,
            metadata,
            exited: false,
            exit_code: 0,
        }
    }
}

pub fn parse_attack_kind(value: &str) -> Result<AttackKind, String> {
    match value {
        "binary-pp" | "binary-prime-probe" => Ok(AttackKind::BinaryPrimeProbe),
        "prime-probe" => Ok(AttackKind::PrimeProbe),
        "evict-time" => Ok(AttackKind::EvictTime),
        _ => Err(format!("unsupported attack: {value}")),
    }
}

pub fn parse_schedule_mode(value: &str) -> Result<ScheduleMode, String> {
    match value {
        "time-sliced" => Ok(ScheduleMode::TimeSliced),
        "smt" => Ok(ScheduleMode::Smt),
        _ => Err(format!("unsupported experiment mode: {value}")),
    }
}

pub fn parse_domain_mode(value: &str) -> Result<DomainMode, String> {
    match value {
        "different" => Ok(DomainMode::Different),
        "same" => Ok(DomainMode::Same),
        _ => Err(format!("unsupported domains mode: {value}")),
    }
}

pub fn parse_control_mode(value: &str) -> Result<ControlMode, String> {
    match value {
        "none" => Ok(ControlMode::None),
        "no-victim" => Ok(ControlMode::NoVictim),
        "forced-eviction" => Ok(ControlMode::ForcedEviction),
        _ => Err(format!("unsupported control: {value}")),
    }
}

pub fn run_experiment(config: &ExperimentConfig) -> io::Result<()> {
    fs::create_dir_all(&config.out_dir)?;

    let trace = CacheTrace::new_shared();
    let mut machine = RiscV32IntegerMachine::with_memory_model_trace_and_seed(
        config.memory_model,
        Some(trace.clone()),
        config.seed,
    );
    machine.set_model_instruction_fetch(false);
    let state = Rc::new(RefCell::new(ExperimentBuiltinState::default()));
    register_experiment_builtins(&mut machine, state.clone());
    let mut contexts = load_experiment_programs(&mut machine, config)?;
    initialize_contexts(&mut machine, &mut contexts, &state, config);

    write_config(config)?;

    let mut rng = XorShift64::new(config.seed);
    let mut summary = SummaryStats::default();

    match config.attack {
        AttackKind::BinaryPrimeProbe => {
            for _ in 0..config.trials {
                let secret_bit = if config.control == ControlMode::ForcedEviction {
                    1
                } else {
                    rng.next_usize(2) as u32
                };
                let secret_set = if secret_bit == 1 {
                    config.target_set
                } else {
                    config.decoy_set
                };
                let record = run_prime_probe_trial(
                    &mut machine,
                    &mut contexts,
                    &state,
                    &trace,
                    config,
                    PrimeProbeTrial {
                        secret_set,
                        secret_bit: Some(secret_bit),
                    },
                );
                update_summary(config, &record, &mut summary);
            }
        }
        AttackKind::PrimeProbe => {
            for _ in 0..config.trials {
                let secret_set = if config.control == ControlMode::ForcedEviction {
                    config.target_set
                } else {
                    random_measured_set(&mut rng)
                };
                let record = run_prime_probe_trial(
                    &mut machine,
                    &mut contexts,
                    &state,
                    &trace,
                    config,
                    PrimeProbeTrial {
                        secret_set,
                        secret_bit: None,
                    },
                );
                update_summary(config, &record, &mut summary);
            }
        }
        AttackKind::EvictTime => {
            let trials_per_set = config.trials.max(SETS) / SETS;
            for target_set in 0..SETS {
                for _ in 0..trials_per_set {
                    let secret_set = if config.control == ControlMode::ForcedEviction {
                        target_set
                    } else {
                        random_measured_set(&mut rng)
                    };
                    let record = run_evict_time_trial(
                        &mut machine,
                        &mut contexts,
                        &state,
                        &trace,
                        config,
                        EvictTimeTrial {
                            target_set,
                            secret_set,
                        },
                    );
                    update_summary(config, &record, &mut summary);
                }
            }
        }
    }

    write_summary(config, &summary)?;
    halt_contexts(&mut machine, &mut contexts, &state, config);
    Ok(())
}

fn random_measured_set(rng: &mut XorShift64) -> usize {
    let mut set = rng.next_usize(SETS - 1);
    if set >= RESERVED_STACK_SET {
        set += 1;
    }
    set
}

fn write_config(config: &ExperimentConfig) -> io::Result<()> {
    let mut f = File::create(config.out_dir.join("config.json"))?;
    writeln!(f, "{{")?;
    writeln!(f, "  \"attack\": \"{}\",", attack_name(config.attack))?;
    writeln!(
        f,
        "  \"mode\": \"{}\",",
        schedule_name(config.schedule_mode)
    )?;
    writeln!(
        f,
        "  \"memory_model\": \"{}\",",
        memory_model_name(config.memory_model)
    )?;
    writeln!(f, "  \"domains\": \"{}\",", domain_name(config.domain_mode))?;
    writeln!(f, "  \"control\": \"{}\",", control_name(config.control))?;
    writeln!(f, "  \"trials\": {},", config.trials)?;
    writeln!(f, "  \"seed\": {},", config.seed)?;
    writeln!(f, "  \"target_set\": {},", config.target_set)?;
    writeln!(f, "  \"decoy_set\": {},", config.decoy_set)?;
    writeln!(f, "  \"reserved_sets\": [{}]", RESERVED_STACK_SET)?;
    writeln!(f, "}}")?;
    Ok(())
}

fn register_experiment_builtins(
    machine: &mut RiscV32IntegerMachine,
    state: Rc<RefCell<ExperimentBuiltinState>>,
) {
    let s = state.clone();
    machine.register_builtin(builtins::BUILTIN_EXP_GET_PHASE, move |machine| {
        machine.set_x(10, s.borrow().phase);
    });
    let s = state.clone();
    machine.register_builtin(builtins::BUILTIN_EXP_GET_SECRET_SET, move |machine| {
        machine.set_x(10, s.borrow().secret_set);
    });
    let s = state.clone();
    machine.register_builtin(builtins::BUILTIN_EXP_GET_SECRET_BIT, move |machine| {
        machine.set_x(10, s.borrow().secret_bit);
    });
    let s = state.clone();
    machine.register_builtin(builtins::BUILTIN_EXP_GET_TARGET_SET, move |machine| {
        machine.set_x(10, s.borrow().target_set);
    });
    let s = state.clone();
    machine.register_builtin(builtins::BUILTIN_EXP_SUBMIT_SCALAR, move |machine| {
        let idx = machine.x(10) as usize;
        let value = machine.x(11) as u64 | ((machine.x(12) as u64) << 32);
        let mut state = s.borrow_mut();
        if state.submitted.len() <= idx {
            state.submitted.resize(idx + 1, 0);
        }
        state.submitted[idx] = value;
    });
    let s = state.clone();
    machine.register_builtin(builtins::BUILTIN_EXP_SUBMIT_VECTOR, move |machine| {
        let ptr = machine.x(10);
        let len = machine.x(11) as usize;
        let mut values = Vec::with_capacity(len);
        for idx in 0..len {
            let addr = ptr.wrapping_add((idx * 8) as u32);
            let lo = machine.load_u32(addr) as u64;
            let hi = machine.load_u32(addr.wrapping_add(4)) as u64;
            values.push(lo | (hi << 32));
        }
        s.borrow_mut().submitted = values;
    });
    let s = state;
    machine.register_builtin(builtins::BUILTIN_EXP_DONE, move |_machine| {
        s.borrow_mut().done = true;
    });
}

fn load_experiment_programs(
    machine: &mut RiscV32IntegerMachine,
    config: &ExperimentConfig,
) -> io::Result<[ExecutionContext; 2]> {
    let attacker = fs::read(&config.attacker_path)?;
    let victim = fs::read(&config.victim_path)?;
    let total_memory = machine.memory_size_bytes();
    let midpoint = total_memory / 2;
    let segment_a = MemorySegment {
        start: 0,
        end_exclusive: midpoint,
    };
    let segment_v = MemorySegment {
        start: midpoint as u32,
        end_exclusive: total_memory,
    };
    let attacker_base = 0;
    let victim_base = 0x8001_0000;

    machine.load_binary_at(&attacker, attacker_base);
    machine.load_binary_at(&victim, victim_base);

    let same = config.domain_mode == DomainMode::Same;
    Ok([
        ExecutionContext::new(
            attacker_base,
            segment_a,
            SecurityClass::Low,
            ProcessMetadata {
                pid: 1,
                asid: 1,
                security_domain_id: 1,
                current_core: 0,
                current_hardware_thread: 0,
            },
        ),
        ExecutionContext::new(
            victim_base,
            segment_v,
            if same {
                SecurityClass::Low
            } else {
                SecurityClass::High
            },
            ProcessMetadata {
                pid: 2,
                asid: if same { 1 } else { 2 },
                security_domain_id: if same { 1 } else { 2 },
                current_core: 0,
                current_hardware_thread: if config.schedule_mode == ScheduleMode::Smt {
                    1
                } else {
                    0
                },
            },
        ),
    ])
}

struct PrimeProbeTrial {
    secret_set: usize,
    secret_bit: Option<u32>,
}

fn run_prime_probe_trial(
    machine: &mut RiscV32IntegerMachine,
    contexts: &mut [ExecutionContext; 2],
    state: &Rc<RefCell<ExperimentBuiltinState>>,
    trace: &Rc<CacheTrace>,
    config: &ExperimentConfig,
    trial: PrimeProbeTrial,
) -> TrialRecord {
    let trial_start = machine.cycle_count();
    trace.drain();
    begin_phase(
        state,
        PHASE_PRIME,
        trial.secret_set,
        trial.secret_bit.unwrap_or(0),
        config.target_set,
        true,
    );
    run_context_until_done(machine, contexts, 0, state, config);

    if config.control != ControlMode::NoVictim {
        begin_phase(
            state,
            PHASE_VICTIM_ACCESS,
            trial.secret_set,
            trial.secret_bit.unwrap_or(0),
            config.target_set,
            true,
        );
        run_context_until_done(machine, contexts, 1, state, config);
    }

    begin_phase(
        state,
        PHASE_PROBE,
        trial.secret_set,
        trial.secret_bit.unwrap_or(0),
        config.target_set,
        true,
    );
    run_context_until_done(machine, contexts, 0, state, config);
    let submitted = state.borrow().submitted.clone();
    let events = trace.drain();

    record_from_events(
        RecordEventInput {
            target_set: Some(config.target_set),
            secret_set: trial.secret_set,
            secret_bit: trial.secret_bit,
            probe_time: submitted,
            victim_elapsed_cycles: None,
            total_trial_cycles: machine.cycle_count().saturating_sub(trial_start),
        },
        &events,
    )
}

struct EvictTimeTrial {
    target_set: usize,
    secret_set: usize,
}

fn run_evict_time_trial(
    machine: &mut RiscV32IntegerMachine,
    contexts: &mut [ExecutionContext; 2],
    state: &Rc<RefCell<ExperimentBuiltinState>>,
    trace: &Rc<CacheTrace>,
    config: &ExperimentConfig,
    trial: EvictTimeTrial,
) -> TrialRecord {
    let trial_start = machine.cycle_count();
    trace.drain();
    begin_phase(
        state,
        PHASE_WARM_VICTIM,
        trial.secret_set,
        0,
        trial.target_set,
        true,
    );
    run_context_until_done(machine, contexts, 1, state, config);

    begin_phase(
        state,
        PHASE_EVICT_TARGET,
        trial.secret_set,
        0,
        trial.target_set,
        true,
    );
    run_context_until_done(machine, contexts, 0, state, config);

    let victim_start = machine.cycle_count();
    if config.control != ControlMode::NoVictim {
        begin_phase(
            state,
            PHASE_VICTIM_ACCESS,
            trial.secret_set,
            0,
            trial.target_set,
            true,
        );
        run_context_until_done(machine, contexts, 1, state, config);
    }
    let victim_elapsed = machine.cycle_count().saturating_sub(victim_start);
    let events = trace.drain();

    record_from_events(
        RecordEventInput {
            target_set: Some(trial.target_set),
            secret_set: trial.secret_set,
            secret_bit: Some((trial.secret_set == trial.target_set) as u32),
            probe_time: Vec::new(),
            victim_elapsed_cycles: Some(victim_elapsed),
            total_trial_cycles: machine.cycle_count().saturating_sub(trial_start),
        },
        &events,
    )
}

fn begin_phase(
    state: &Rc<RefCell<ExperimentBuiltinState>>,
    phase: u32,
    secret_set: usize,
    secret_bit: u32,
    target_set: usize,
    clear_submission: bool,
) {
    let mut state = state.borrow_mut();
    state.phase = phase;
    state.secret_set = secret_set as u32;
    state.secret_bit = secret_bit;
    state.target_set = target_set as u32;
    state.done = false;
    if clear_submission {
        state.submitted.clear();
    }
}

fn run_context_until_done(
    machine: &mut RiscV32IntegerMachine,
    contexts: &mut [ExecutionContext; 2],
    idx: usize,
    state: &Rc<RefCell<ExperimentBuiltinState>>,
    config: &ExperimentConfig,
) {
    machine.set_memory_segment(contexts[idx].memory_segment);
    machine.set_requester_identity(
        contexts[idx].security_class,
        contexts[idx].metadata.pid,
        contexts[idx].metadata.security_domain_id,
    );
    if config.schedule_mode == ScheduleMode::TimeSliced {
        machine.notify_context_switch();
    }
    machine.restore_registers(&contexts[idx].registers);

    loop {
        if state.borrow().done {
            break;
        }
        match machine.step() {
            StepResult::Continue
            | StepResult::Yield
            | StepResult::ForceYield
            | StepResult::PcUpdated => {}
            StepResult::Halt(code) => {
                contexts[idx].exited = true;
                contexts[idx].exit_code = code;
                break;
            }
            StepResult::Unimplemented(op) => {
                panic!("unimplemented instruction in experiment: {op}")
            }
        }
    }

    contexts[idx].registers = machine.snapshot_registers();
}

fn initialize_contexts(
    machine: &mut RiscV32IntegerMachine,
    contexts: &mut [ExecutionContext; 2],
    state: &Rc<RefCell<ExperimentBuiltinState>>,
    config: &ExperimentConfig,
) {
    for idx in 0..contexts.len() {
        begin_phase(state, PHASE_HALT, 0, 0, 0, true);
        run_context_until_done(machine, contexts, idx, state, config);
    }
}

fn halt_contexts(
    machine: &mut RiscV32IntegerMachine,
    contexts: &mut [ExecutionContext; 2],
    state: &Rc<RefCell<ExperimentBuiltinState>>,
    config: &ExperimentConfig,
) {
    for idx in 0..contexts.len() {
        if !contexts[idx].exited {
            begin_phase(state, PHASE_HALT, 0, 0, 0, true);
            run_context_until_done(machine, contexts, idx, state, config);
        }
    }
}

struct RecordEventInput {
    target_set: Option<usize>,
    secret_set: usize,
    secret_bit: Option<u32>,
    probe_time: Vec<u64>,
    victim_elapsed_cycles: Option<u64>,
    total_trial_cycles: u64,
}

fn record_from_events(input: RecordEventInput, events: &[CacheAccessEvent]) -> TrialRecord {
    let mut attacker_l1d_evictions_by_victim = 0;

    for event in events {
        if event.requester_pid == 2
            && event.evicted_pid == Some(1)
            && event.source == CacheAccessSource::Lower
        {
            attacker_l1d_evictions_by_victim += 1;
        }
    }

    TrialRecord {
        target_set: input.target_set,
        secret_set: input.secret_set,
        secret_bit: input.secret_bit,
        probe_time: input.probe_time,
        victim_elapsed_cycles: input.victim_elapsed_cycles,
        total_trial_cycles: input.total_trial_cycles,
        attacker_l1d_evictions_by_victim,
    }
}

fn update_summary(config: &ExperimentConfig, record: &TrialRecord, summary: &mut SummaryStats) {
    summary.trials += 1;
    summary.total_trial_cycles += record.total_trial_cycles;
    summary.victim_evicted_attacker_lines += record.attacker_l1d_evictions_by_victim;
    summary.victim_memory_accesses += 1;

    match config.attack {
        AttackKind::PrimeProbe => {
            if let Some((predicted, _)) = record
                .probe_time
                .iter()
                .enumerate()
                .filter(|(set, _)| *set != RESERVED_STACK_SET)
                .max_by_key(|(_, value)| *value)
            {
                if predicted == record.secret_set {
                    summary.correct += 1;
                }
                summary.multi_joint[record.secret_set][predicted] += 1;
            }
        }
        AttackKind::BinaryPrimeProbe => {
            let t = record
                .probe_time
                .get(record.target_set.unwrap_or(config.target_set))
                .copied()
                .unwrap_or(0);
            let threshold = 700;
            let predicted = (t >= threshold) as u32;
            let bit = record.secret_bit.unwrap_or(0);
            if predicted == bit {
                summary.correct += 1;
            }
            summary.binary_samples.push((t as f64, bit == 1));
            summary.binary_joint[bit as usize][predicted as usize] += 1;
            match (bit, predicted) {
                (1, 1) => summary.binary_tp += 1,
                (0, 0) => summary.binary_tn += 1,
                (0, 1) => summary.binary_fp += 1,
                (1, 0) => summary.binary_fn += 1,
                _ => {}
            }
            add_timing(summary, bit == 1, t as f64);
        }
        AttackKind::EvictTime => {
            let label = record.secret_set == record.target_set.unwrap_or(usize::MAX);
            let t = record.victim_elapsed_cycles.unwrap_or(0) as f64;
            add_timing(summary, label, t);
        }
    }
}

fn add_timing(summary: &mut SummaryStats, touched: bool, value: f64) {
    if touched {
        summary.count_touched += 1;
        summary.sum_touched += value;
        summary.sumsq_touched += value * value;
    } else {
        summary.count_untouched += 1;
        summary.sum_untouched += value;
        summary.sumsq_untouched += value * value;
    }
}

fn write_summary(config: &ExperimentConfig, summary: &SummaryStats) -> io::Result<()> {
    let mut f = File::create(config.out_dir.join("summary.csv"))?;
    let accuracy = if summary.trials == 0 {
        0.0
    } else {
        summary.correct as f64 / summary.trials as f64
    };
    let mean_touched = mean(summary.sum_touched, summary.count_touched);
    let mean_untouched = mean(summary.sum_untouched, summary.count_untouched);
    let delta = mean_touched - mean_untouched;
    let d = cohens_d(summary);
    let auc = binary_auc(&summary.binary_samples);
    let mutual_information_bits = match config.attack {
        AttackKind::PrimeProbe => mutual_information(&summary.multi_joint),
        AttackKind::BinaryPrimeProbe => mutual_information(&summary.binary_joint),
        AttackKind::EvictTime => 0.0,
    };
    let mean_cycles = if summary.trials == 0 {
        0.0
    } else {
        summary.total_trial_cycles as f64 / summary.trials as f64
    };
    let interference = if summary.victim_memory_accesses == 0 {
        0.0
    } else {
        summary.victim_evicted_attacker_lines as f64 / summary.victim_memory_accesses as f64
    };
    let leakage_bits_per_cycle = if mean_cycles == 0.0 {
        0.0
    } else {
        mutual_information_bits / mean_cycles
    };

    writeln!(
        f,
        "attack,architecture,mode,domains,trials,accuracy,false_positive_rate,false_negative_rate,auc,timing_delta,cohens_d,mutual_information_bits,mean_trial_cycles,leakage_bits_per_cycle,causal_l1d_interference"
    )?;
    writeln!(
        f,
        "{},{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.3},{:.12},{:.6}",
        attack_name(config.attack),
        memory_model_name(config.memory_model),
        schedule_name(config.schedule_mode),
        domain_name(config.domain_mode),
        summary.trials,
        accuracy,
        rate(summary.binary_fp, summary.binary_fp + summary.binary_tn),
        rate(summary.binary_fn, summary.binary_fn + summary.binary_tp),
        auc,
        delta,
        d,
        mutual_information_bits,
        mean_cycles,
        leakage_bits_per_cycle,
        interference
    )
}

fn mean(sum: f64, count: usize) -> f64 {
    if count == 0 { 0.0 } else { sum / count as f64 }
}

fn variance(sum: f64, sumsq: f64, count: usize) -> f64 {
    if count < 2 {
        0.0
    } else {
        (sumsq - (sum * sum / count as f64)) / (count as f64 - 1.0)
    }
}

fn cohens_d(summary: &SummaryStats) -> f64 {
    if summary.count_touched < 2 || summary.count_untouched < 2 {
        return 0.0;
    }
    let var_touched = variance(
        summary.sum_touched,
        summary.sumsq_touched,
        summary.count_touched,
    );
    let var_untouched = variance(
        summary.sum_untouched,
        summary.sumsq_untouched,
        summary.count_untouched,
    );
    let denom = (((summary.count_touched - 1) as f64 * var_touched
        + (summary.count_untouched - 1) as f64 * var_untouched)
        / (summary.count_touched + summary.count_untouched - 2) as f64)
        .sqrt();
    if denom == 0.0 {
        0.0
    } else {
        (mean(summary.sum_touched, summary.count_touched)
            - mean(summary.sum_untouched, summary.count_untouched))
            / denom
    }
}

fn rate(num: usize, denom: usize) -> f64 {
    if denom == 0 {
        0.0
    } else {
        num as f64 / denom as f64
    }
}

fn binary_auc(samples: &[(f64, bool)]) -> f64 {
    let positives = samples.iter().filter(|(_, label)| *label).count();
    let negatives = samples.len().saturating_sub(positives);
    if positives == 0 || negatives == 0 {
        return 0.0;
    }

    let mut wins = 0.0;
    for (pos_score, pos_label) in samples {
        if !*pos_label {
            continue;
        }
        for (neg_score, neg_label) in samples {
            if *neg_label {
                continue;
            }
            if pos_score > neg_score {
                wins += 1.0;
            } else if pos_score == neg_score {
                wins += 0.5;
            }
        }
    }
    wins / (positives * negatives) as f64
}

fn mutual_information<const R: usize, const C: usize>(joint: &[[usize; C]; R]) -> f64 {
    let total: usize = joint.iter().flat_map(|row| row.iter()).sum();
    if total == 0 {
        return 0.0;
    }
    let total_f = total as f64;
    let mut row_totals = [0usize; R];
    let mut col_totals = [0usize; C];
    for r in 0..R {
        for (c, value) in joint[r].iter().enumerate() {
            row_totals[r] += *value;
            col_totals[c] += *value;
        }
    }

    let mut mi = 0.0;
    for r in 0..R {
        for (c, count) in joint[r].iter().enumerate() {
            if *count == 0 {
                continue;
            }
            let pxy = *count as f64 / total_f;
            let px = row_totals[r] as f64 / total_f;
            let py = col_totals[c] as f64 / total_f;
            mi += pxy * (pxy / (px * py)).log2();
        }
    }
    mi
}

pub fn memory_model_name(model: MemoryModel) -> &'static str {
    match model {
        MemoryModel::Default => "default",
        MemoryModel::SmtCache => "smtcache",
        MemoryModel::BackCache => "backcache",
        MemoryModel::NewCache => "newcache",
        MemoryModel::SecDcp => "secdcp",
    }
}

pub fn attack_name(attack: AttackKind) -> &'static str {
    match attack {
        AttackKind::BinaryPrimeProbe => "binary-pp",
        AttackKind::PrimeProbe => "prime-probe",
        AttackKind::EvictTime => "evict-time",
    }
}

fn schedule_name(mode: ScheduleMode) -> &'static str {
    match mode {
        ScheduleMode::TimeSliced => "time-sliced",
        ScheduleMode::Smt => "smt",
    }
}

fn domain_name(mode: DomainMode) -> &'static str {
    match mode {
        DomainMode::Different => "different",
        DomainMode::Same => "same",
    }
}

fn control_name(mode: ControlMode) -> &'static str {
    match mode {
        ControlMode::None => "none",
        ControlMode::NoVictim => "no-victim",
        ControlMode::ForcedEviction => "forced-eviction",
    }
}
