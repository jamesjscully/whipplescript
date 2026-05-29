use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use serde_json::{json, Value};
use whippletree_kernel::{
    coerce::{BamlCoerceRequest, FakeBamlClient},
    harness::{CommandAgentHarness, CommandLaunchPlan},
    idempotency_key,
    loft::{FakeLoftClient, LoftAction, LoftEffectRequest},
    trace::{
        check_trace, DependencyEdge, DependencyPredicate, EffectStatus, TraceEvent, TraceRecord,
    },
    AgentTurnExecution, BamlCoerceExecution, HumanAskExecution, LoftEffectExecution,
    ProgramVersionInput, RuntimeKernel,
};
use whippletree_parser::{
    parse_expression, BinaryOp, DependencyPredicate as IrDependencyPredicate, Diagnostic, Expr,
    ExprLiteral, IrEffectNode, IrPrimitiveType, IrProgram, IrRule, IrType, QueryKind, SourceSpan,
    UnaryOp,
};
use whippletree_store::{
    ClaimableEffect, EffectCompletion, EffectView, EventView, EvidenceLinkView, EvidenceView,
    FactView, HumanAnswer, InboxItemView, InstanceView, NewEffect, NewEffectDependency, NewFact,
    RetryEffect, RuleCommit, RunStart, RunView, SqliteStore, StatusView, StoreError,
};

fn main() -> ExitCode {
    let raw_args = env::args().skip(1).collect::<Vec<_>>();
    if matches!(
        raw_args.first().map(String::as_str),
        Some("--version" | "-V")
    ) {
        println!("whippletree {}", whippletree_core::version());
        return ExitCode::SUCCESS;
    }

    let options = match CliOptions::parse(raw_args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    match options.command.as_deref() {
        Some("doctor") => doctor(&options),
        Some("check") => check(&options),
        Some("compile") => compile(&options),
        Some("run") => run(&options),
        Some("step") => step(&options),
        Some("worker") => worker(&options),
        Some("dev") => dev(&options),
        Some("instances") => instances(&options),
        Some("status") => status(&options),
        Some("log") => log(&options),
        Some("facts") => facts(&options),
        Some("effects") => effects(&options),
        Some("runs") => runs(&options),
        Some("inbox") => inbox(&options),
        Some("evidence") => evidence(&options),
        Some("trace") => trace(&options),
        Some("pause") => pause(&options),
        Some("resume") => resume(&options),
        Some("cancel") => cancel(&options),
        Some("retry") => retry(&options),
        Some("help") | Some("--help") | Some("-h") | None => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some(command) => {
            eprintln!("unknown command `{command}`");
            eprintln!("try `whip help` to see available commands");
            ExitCode::from(2)
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct CliOptions {
    command: Option<String>,
    args: Vec<String>,
    store_path: PathBuf,
    json: bool,
    input_json: Option<String>,
}

impl CliOptions {
    fn parse(raw_args: Vec<String>) -> Result<Self, String> {
        let mut command = None;
        let mut args = Vec::new();
        let mut store_path = env::var("WHIPPLETREE_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(".whippletree/store.sqlite"));
        let mut json = false;
        let mut input_json = None;
        let mut iter = raw_args.into_iter();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--store" => {
                    let Some(path) = iter.next() else {
                        return Err("expected a path after `--store`".to_owned());
                    };
                    store_path = PathBuf::from(path);
                }
                "--json" => json = true,
                "--input" => {
                    let Some(input) = iter.next() else {
                        return Err("expected JSON after `--input`".to_owned());
                    };
                    input_json = Some(input);
                }
                "--" => {
                    args.extend(iter);
                    break;
                }
                _ if command.is_none() => command = Some(arg),
                _ => args.push(arg),
            }
        }

        Ok(Self {
            command,
            args,
            store_path,
            json,
            input_json,
        })
    }
}

fn print_usage() {
    println!("whippletree {}", whippletree_core::IMPLEMENTATION_STAGE);
    println!("usage: whip [--store path] [--json] <command> [args]");
    println!("commands: check, compile, run, step, worker, dev, instances, status, log, facts, effects, runs");
    println!("          inbox, evidence, trace, pause, resume, cancel, retry, doctor");
}

fn doctor(options: &CliOptions) -> ExitCode {
    let store_status = match open_store(&options.store_path) {
        Ok(store) => match store.schema_version() {
            Ok(version) => json!({
                "ok": true,
                "path": options.store_path.display().to_string(),
                "schema_version": version,
            }),
            Err(error) => json!({
                "ok": false,
                "path": options.store_path.display().to_string(),
                "error": store_error(error),
            }),
        },
        Err(message) => json!({
            "ok": false,
            "path": options.store_path.display().to_string(),
            "error": message,
        }),
    };
    let tools = doctor_tool_checks();
    if options.json {
        emit_json(json!({
            "stage": whippletree_kernel::kernel_stage(),
            "store": store_status,
            "tools": tools.iter().map(tool_check_to_json).collect::<Vec<_>>(),
        }))
    } else {
        println!("whip doctor: {}", whippletree_kernel::kernel_stage());
        println!("store: {}", options.store_path.display());
        if store_status.get("ok").and_then(Value::as_bool) == Some(true) {
            if let Some(version) = store_status.get("schema_version").and_then(Value::as_i64) {
                println!("sqlite schema: {version}");
            }
        } else if let Some(error) = store_status.get("error").and_then(Value::as_str) {
            eprintln!("sqlite store check failed: {error}");
        }
        println!("tools:");
        for tool in tools {
            let status = if tool.available { "ok" } else { "missing" };
            let required = if tool.required {
                "required"
            } else {
                "optional"
            };
            println!(
                "  {:<16} {:<7} {} ({})",
                tool.id,
                status,
                tool.path.as_deref().unwrap_or(tool.command),
                required
            );
            if !tool.available {
                println!("    {}", tool.note);
            }
        }
        ExitCode::SUCCESS
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ToolCheck {
    id: &'static str,
    category: &'static str,
    command: &'static str,
    required: bool,
    available: bool,
    path: Option<String>,
    note: &'static str,
}

fn doctor_tool_checks() -> Vec<ToolCheck> {
    let path = path_value();
    [
        (
            "maude",
            "formal",
            &["maude"][..],
            false,
            "needed for formal model checks and generated Maude searches",
        ),
        (
            "java",
            "formal",
            &["java"][..],
            false,
            "needed for Apalache/TLA+ checks",
        ),
        (
            "apalache",
            "formal",
            &["apalache-mc", "apalache"][..],
            false,
            "needed for TLA+ lifecycle checks",
        ),
        (
            "baml",
            "integration",
            &["baml-cli", "baml"][..],
            false,
            "needed for no-mock BAML coerce integration tests",
        ),
        (
            "codex",
            "provider",
            &["codex"][..],
            false,
            "needed for Codex agent harness provider runs",
        ),
        (
            "claude",
            "provider",
            &["claude"][..],
            false,
            "needed for Claude Code agent harness provider runs",
        ),
        (
            "loft",
            "provider",
            &["loft"][..],
            false,
            "needed for no-mock Loft claim/release/note/status effects",
        ),
    ]
    .into_iter()
    .map(|(id, category, candidates, required, note)| {
        let path = find_executable_in_path(candidates, &path);
        ToolCheck {
            id,
            category,
            command: candidates[0],
            required,
            available: path.is_some(),
            path,
            note,
        }
    })
    .collect()
}

fn path_value() -> String {
    env::var_os("PATH")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn find_executable_in_path(candidates: &[&str], path_value: &str) -> Option<String> {
    for directory in env::split_paths(path_value) {
        for candidate in candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path.display().to_string());
            }
        }
    }
    None
}

fn check(options: &CliOptions) -> ExitCode {
    let check_options = match CheckOptions::parse(&options.args) {
        Ok(check_options) => check_options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    if check_options.paths.is_empty() {
        eprintln!("usage: whip check [--model-search] <workflow.whip>...");
        return ExitCode::from(2);
    }

    let mut failed = false;
    for path in check_options.paths {
        match compile_source_path(&path) {
            Ok((source, ir)) => {
                println!("== {}", display_path(&path));
                print!("{}", ir.to_snapshot());
                if check_options.model_search {
                    match run_model_search(&path, &source, &ir) {
                        Ok(report) if report.searches == 0 => {
                            println!("model search: no effect dependency checks generated");
                        }
                        Ok(report) => {
                            println!(
                                "model search: {} checks passed (solutions={}, no_solutions={})",
                                report.searches, report.solutions, report.no_solutions
                            );
                        }
                        Err(message) => {
                            eprintln!("{path}: model search failed: {message}");
                            failed = true;
                        }
                    }
                }
            }
            Err(CompileFailure::Io(error)) => {
                eprintln!("{path}: failed to read: {error}");
                failed = true;
            }
            Err(CompileFailure::Diagnostics {
                source,
                diagnostics,
            }) => {
                failed = true;
                for diagnostic in diagnostics {
                    eprint!("{}", render_diagnostic(&path, &source, &diagnostic));
                }
            }
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CheckOptions {
    model_search: bool,
    paths: Vec<String>,
}

impl CheckOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut model_search = false;
        let mut paths = Vec::new();
        for arg in args {
            match arg.as_str() {
                "--model-search" => model_search = true,
                "--" => {}
                _ if arg.starts_with('-') => {
                    return Err(format!("unknown check option `{arg}`"));
                }
                _ => paths.push(arg.clone()),
            }
        }
        Ok(Self {
            model_search,
            paths,
        })
    }
}

fn compile(options: &CliOptions) -> ExitCode {
    let Some(path) = single_arg(options, "usage: whip compile <workflow.whip>") else {
        return ExitCode::from(2);
    };
    let (source, ir) = match compile_source_path(path) {
        Ok(compiled) => compiled,
        Err(error) => return report_compile_failure(path, error),
    };
    let snapshot = ir.to_snapshot();

    if options.json {
        emit_json(json!({
            "path": display_path(path),
            "workflow": ir.workflow,
            "source_hash": stable_hash_hex(&source),
            "ir_hash": stable_hash_hex(&snapshot),
            "snapshot": snapshot,
        }))
    } else {
        print!("{snapshot}");
        ExitCode::SUCCESS
    }
}

fn run(options: &CliOptions) -> ExitCode {
    let Some(path) = single_arg(options, "usage: whip run <workflow.whip> [--input JSON]") else {
        return ExitCode::from(2);
    };
    let started = match start_workflow_instance(path, options.input_json.as_deref(), options) {
        Ok(started) => started,
        Err(code) => return code,
    };

    if options.json {
        emit_json(json!({
            "instance_id": started.instance_id,
            "program_id": started.program_id,
            "version_id": started.version_id,
            "workflow": started.workflow,
            "store": options.store_path.display().to_string(),
        }))
    } else {
        println!("started {}", started.instance_id);
        println!("workflow {}", started.workflow);
        println!("store {}", options.store_path.display());
        ExitCode::SUCCESS
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StartedWorkflow {
    instance_id: String,
    program_id: String,
    version_id: String,
    workflow: String,
}

fn start_workflow_instance(
    path: &str,
    input_json: Option<&str>,
    options: &CliOptions,
) -> Result<StartedWorkflow, ExitCode> {
    let input_json = input_json.unwrap_or("{}");
    let input_value = match serde_json::from_str::<Value>(input_json) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("invalid `--input` JSON: {error}");
            return Err(ExitCode::from(2));
        }
    };
    let input_json = input_value.to_string();
    let (source, ir) = match compile_source_path(path) {
        Ok(compiled) => compiled,
        Err(error) => return Err(report_compile_failure(path, error)),
    };
    let snapshot = ir.to_snapshot();
    let store = match open_store(&options.store_path) {
        Ok(store) => store,
        Err(message) => {
            eprintln!("{message}");
            return Err(ExitCode::FAILURE);
        }
    };
    let mut kernel = RuntimeKernel::new(store);
    let version = match kernel.create_program_version(ProgramVersionInput {
        program_name: &ir.workflow,
        source_hash: &stable_hash_hex(&source),
        ir_hash: &stable_hash_hex(&snapshot),
        compiler_version: whippletree_core::version(),
    }) {
        Ok(version) => version,
        Err(error) => {
            eprintln!("failed to create program version: {}", store_error(error));
            return Err(ExitCode::FAILURE);
        }
    };
    let instance_id = match kernel.create_instance(&version, &input_json) {
        Ok(instance_id) => instance_id,
        Err(error) => {
            eprintln!("failed to create instance: {}", store_error(error));
            return Err(ExitCode::FAILURE);
        }
    };
    if let Err(error) = kernel.ingest_external_event(
        &instance_id,
        "external.started",
        &input_json,
        Some(&idempotency_key(&[&instance_id, "external.started"])),
    ) {
        eprintln!("failed to write start event: {}", store_error(error));
        return Err(ExitCode::FAILURE);
    }
    Ok(StartedWorkflow {
        instance_id,
        program_id: version.program_id,
        version_id: version.version_id,
        workflow: ir.workflow,
    })
}

fn step(options: &CliOptions) -> ExitCode {
    let step_options = match StepOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let (_source, ir) = match compile_source_path(&step_options.program_path) {
        Ok(compiled) => compiled,
        Err(error) => return report_compile_failure(&step_options.program_path, error),
    };
    match step_instance(&options.store_path, &step_options.instance_id, &ir) {
        Ok(report) if options.json => emit_json(step_report_to_json(&report)),
        Ok(report) => {
            println!(
                "step {} committed_rules={} facts={} effects={}",
                report.instance_id,
                report.committed_rules,
                report.facts_created,
                report.effects_created
            );
            for guard in report
                .guard_reports
                .iter()
                .filter(|guard| guard.status == GuardStatus::Error)
            {
                eprintln!(
                    "guard error in rule {}: {}{}",
                    guard.rule,
                    guard.expr,
                    guard
                        .error
                        .as_deref()
                        .map(|error| format!(" ({error})"))
                        .unwrap_or_default()
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => report_store_error("failed to step instance", error),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StepOptions {
    instance_id: String,
    program_path: String,
}

impl StepOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut instance_id = None;
        let mut program_path = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--program" => {
                    index += 1;
                    let Some(path) = args.get(index) else {
                        return Err("expected path after `--program`".to_owned());
                    };
                    program_path = Some(path.clone());
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown step option `{other}`"));
                }
                value if instance_id.is_none() => instance_id = Some(value.to_owned()),
                value if program_path.is_none() => program_path = Some(value.to_owned()),
                _ => return Err("usage: whip step <instance> --program <workflow.whip>".to_owned()),
            }
            index += 1;
        }
        let Some(instance_id) = instance_id else {
            return Err("usage: whip step <instance> --program <workflow.whip>".to_owned());
        };
        let Some(program_path) = program_path else {
            return Err("usage: whip step <instance> --program <workflow.whip>".to_owned());
        };
        Ok(Self {
            instance_id,
            program_path,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct StepReport {
    instance_id: String,
    committed_rules: usize,
    facts_created: usize,
    effects_created: usize,
    guard_reports: Vec<GuardReport>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GuardReport {
    rule: String,
    when: String,
    expr: String,
    status: GuardStatus,
    matched: bool,
    actual: Value,
    error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum GuardStatus {
    Matched,
    False,
    Error,
}

impl GuardStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::False => "false",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AssertionReport {
    expr: String,
    status: AssertionStatus,
    passed: bool,
    actual: Value,
    error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AssertionStatus {
    Passed,
    Failed,
    Error,
}

impl AssertionStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Error => "error",
        }
    }
}

fn step_instance(
    store_path: &Path,
    instance_id: &str,
    ir: &IrProgram,
) -> Result<StepReport, StoreError> {
    let mut report = StepReport {
        instance_id: instance_id.to_owned(),
        ..StepReport::default()
    };
    let mut made_progress = true;
    while made_progress {
        made_progress = false;
        let store = SqliteStore::open(store_path)?;
        let events = store.list_events(instance_id)?;
        let facts = store.list_facts(instance_id)?;
        let effects = store.list_effects(instance_id)?;
        let started_event_id = events
            .iter()
            .find(|event| event.event_type == "external.started")
            .map(|event| event.event_id.clone());

        for rule in &ir.rules {
            let ready = ready_contexts(rule, &facts, &effects, started_event_id.as_deref());
            report.guard_reports.extend(ready.guard_reports);
            for context in ready.contexts {
                let lowering = lower_rule(ir, rule, &context, &facts, &effects);
                if lowering.facts.is_empty() && lowering.effects.is_empty() {
                    continue;
                }
                let new_facts = lowering
                    .facts
                    .iter()
                    .map(OwnedFact::as_new_fact)
                    .collect::<Vec<_>>();
                let new_effects = lowering
                    .effects
                    .iter()
                    .map(OwnedEffect::as_new_effect)
                    .collect::<Vec<_>>();
                let new_dependencies = lowering
                    .dependencies
                    .iter()
                    .map(OwnedDependency::as_new_dependency)
                    .collect::<Vec<_>>();
                let mut store = SqliteStore::open(store_path)?;
                let mut kernel = RuntimeKernel::new(store);
                let lowering_key = lowering_idempotency_key(&lowering);
                let commit_key = idempotency_key(&[
                    instance_id,
                    &rule.name,
                    context.identity.as_deref().unwrap_or("started"),
                    &lowering_key,
                ]);
                let event = kernel.commit_rule(RuleCommit {
                    instance_id,
                    rule: &rule.name,
                    trigger_event_id: context.trigger_event_id.as_deref(),
                    facts: &new_facts,
                    effects: &new_effects,
                    dependencies: &new_dependencies,
                    idempotency_key: Some(&commit_key),
                });
                store = kernel.into_store();
                drop(store);
                match event {
                    Ok(_) => {
                        report.committed_rules += 1;
                        report.facts_created += new_facts.len();
                        report.effects_created += new_effects.len();
                        made_progress = true;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(report)
}

fn worker(options: &CliOptions) -> ExitCode {
    let worker_options = match WorkerOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run_worker_once(&options.store_path, &worker_options) {
        Ok(report) if options.json => emit_json(worker_report_to_json(&report)),
        Ok(report) => {
            println!(
                "worker {} ran={} provider={}",
                report.instance_id, report.ran_effects, report.provider
            );
            ExitCode::SUCCESS
        }
        Err(error) => report_store_error("failed to run worker", error),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkerOptions {
    instance_id: String,
    provider: String,
    fail: bool,
}

impl WorkerOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut instance_id = None;
        let mut provider = "fixture".to_owned();
        let mut fail = false;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--provider" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected provider after `--provider`".to_owned());
                    };
                    provider = value.clone();
                }
                "--fail" => fail = true,
                "--once" => {}
                other if other.starts_with('-') => {
                    return Err(format!("unknown worker option `{other}`"));
                }
                value if instance_id.is_none() => instance_id = Some(value.to_owned()),
                _ => {
                    return Err(
                        "usage: whip worker <instance> [--provider fixture] [--once]".to_owned(),
                    )
                }
            }
            index += 1;
        }
        let Some(instance_id) = instance_id else {
            return Err("usage: whip worker <instance> [--provider fixture] [--once]".to_owned());
        };
        Ok(Self {
            instance_id,
            provider,
            fail,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct WorkerReport {
    instance_id: String,
    provider: String,
    ran_effects: usize,
    terminal_events: Vec<String>,
}

fn run_worker_once(store_path: &Path, options: &WorkerOptions) -> Result<WorkerReport, StoreError> {
    let store = SqliteStore::open(store_path)?;
    let claimable = store.claimable_effects(&options.instance_id)?;
    let mut report = WorkerReport {
        instance_id: options.instance_id.clone(),
        provider: options.provider.clone(),
        ..WorkerReport::default()
    };
    for effect in claimable {
        let terminal = match effect.kind.as_str() {
            "agent.tell" => run_agent_effect(store_path, &options.instance_id, &effect, options)?,
            "baml.coerce" => run_baml_effect(store_path, &options.instance_id, &effect, options)?,
            "loft.claim" => run_loft_effect(store_path, &options.instance_id, &effect, options)?,
            "human.ask" => run_human_effect(store_path, &options.instance_id, &effect, options)?,
            "capability.call" => {
                run_capability_effect(store_path, &options.instance_id, &effect, options)?
            }
            _ => continue,
        };
        report.ran_effects += 1;
        report.terminal_events.push(terminal.event_id);
    }
    Ok(report)
}

fn run_agent_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whippletree_store::StoredEvent, StoreError> {
    let input_json = resolve_effect_input_after_bindings(store_path, instance_id, effect)?;
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "lease"]);
    let harness = fixture_harness(options.fail);
    kernel.run_agent_turn(
        AgentTurnExecution {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            lease_id: &lease_id,
            lease_expires_at: "2030-01-01T00:00:00Z",
            agent: effect.target.as_deref().unwrap_or("agent"),
            profile: effect.profile.as_deref(),
            input_json: &input_json,
            skill_names: &[],
        },
        &harness,
    )
}

fn run_baml_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whippletree_store::StoredEvent, StoreError> {
    let input_json = resolve_effect_input_after_bindings(store_path, instance_id, effect)?;
    let input = json_from_str(&input_json);
    let function_name = input
        .get("function_name")
        .and_then(Value::as_str)
        .unwrap_or("coerce")
        .to_owned();
    let arguments_json = input
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}))
        .to_string();
    let output_type = input
        .get("output_type")
        .and_then(Value::as_str)
        .unwrap_or("json")
        .to_owned();
    let request = BamlCoerceRequest {
        function_name,
        arguments_json,
        output_type: output_type.clone(),
        generated_baml_source_hash: "fixture".to_owned(),
        input_schema_hash: "fixture".to_owned(),
        output_schema_hash: "fixture".to_owned(),
    };
    let value = fixture_baml_value(&output_type);
    let client = if options.fail {
        FakeBamlClient::fails("fixture coerce failure")
    } else {
        FakeBamlClient::succeeds(value)
    };
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "baml-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "baml-lease"]);
    kernel.run_baml_coerce(
        BamlCoerceExecution {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            lease_id: &lease_id,
            lease_expires_at: "2030-01-01T00:00:00Z",
            request: &request,
        },
        &client,
    )
}

fn run_loft_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whippletree_store::StoredEvent, StoreError> {
    let input = json_from_str(&effect.input_json);
    let issue_id = input
        .pointer("/issue/issue/id")
        .and_then(Value::as_str)
        .or_else(|| input.pointer("/issue/issue_id").and_then(Value::as_str))
        .unwrap_or("issue-fixture")
        .to_owned();
    let request = LoftEffectRequest {
        action: LoftAction::Claim,
        issue_id: issue_id.clone(),
        lease_id: None,
        claim_ready: true,
        issue_version: None,
        actor: Some("whip-worker".to_owned()),
        lease_duration_seconds: Some(3600),
        command_id: idempotency_key(&[instance_id, &effect.effect_id, "loft-command"]),
        note: None,
        target_status: None,
        evidence_json: None,
        evidence_kind: None,
        evidence_artifact: None,
        evidence_data_path: None,
        resource_intent_json: None,
        release_after_failure: false,
        expect_heads: Vec::new(),
        metadata_json: effect.input_json.clone(),
    };
    let client = if options.fail {
        FakeLoftClient::fails("fixture loft failure")
    } else {
        FakeLoftClient::succeeds(
            json!({
                "issue": {
                    "id": issue_id,
                    "title": "Fixture Loft issue",
                    "body": "Fixture body"
                },
                "lease": {
                    "id": idempotency_key(&[instance_id, &effect.effect_id, "loft-lease-value"])
                }
            })
            .to_string(),
        )
    };
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "loft-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "loft-lease"]);
    kernel.run_loft_effect(
        LoftEffectExecution {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            lease_id: &lease_id,
            lease_expires_at: "2030-01-01T00:00:00Z",
            request: &request,
        },
        &client,
    )
}

fn run_human_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whippletree_store::StoredEvent, StoreError> {
    let input_json = resolve_effect_input_after_bindings(store_path, instance_id, effect)?;
    let input = json_from_str(&input_json);
    let prompt = input
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("Human review requested");
    let choices_json = input
        .get("choices")
        .cloned()
        .unwrap_or_else(|| json!(["accept", "revise", "block"]))
        .to_string();
    let severity = input
        .get("severity")
        .and_then(Value::as_str)
        .unwrap_or("normal");
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "human-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "human-lease"]);
    let inbox_item_id = idempotency_key(&[instance_id, &effect.effect_id, "inbox"]);
    kernel.run_human_ask(HumanAskExecution {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: &options.provider,
        worker_id: "whip-worker",
        lease_id: &lease_id,
        lease_expires_at: "2030-01-01T00:00:00Z",
        inbox_item_id: &inbox_item_id,
        prompt,
        choices_json: &choices_json,
        freeform_allowed: true,
        severity,
        related_effects_json: &json!([effect.effect_id]).to_string(),
        related_artifacts_json: "[]",
    })
}

fn resolve_effect_input_after_bindings(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
) -> Result<String, StoreError> {
    let mut input = json_from_str(&effect.input_json);
    let Some(after) = input.get("after").cloned() else {
        return Ok(effect.input_json.clone());
    };
    let Some(binding) = after.get("binding").and_then(Value::as_str) else {
        return Ok(effect.input_json.clone());
    };
    let Some(predicate) = after.get("predicate").and_then(Value::as_str) else {
        return Ok(effect.input_json.clone());
    };
    let Some(upstream_effect_id) = after.get("upstream_effect_id").and_then(Value::as_str) else {
        return Ok(effect.input_json.clone());
    };
    let store = SqliteStore::open(store_path)?;
    let facts = store.list_facts(instance_id)?;
    let Some(binding_value) = effect_binding_value(&facts, upstream_effect_id, predicate) else {
        return Ok(effect.input_json.clone());
    };
    if let Some(bindings) = input.get_mut("bindings").and_then(Value::as_object_mut) {
        bindings.insert(binding.to_owned(), binding_value.clone());
    }
    let mut context = context_from_input_bindings(&input);
    context.bindings.push((
        binding.to_owned(),
        FactView {
            fact_id: upstream_effect_id.to_owned(),
            name: binding.to_owned(),
            key: upstream_effect_id.to_owned(),
            value_json: binding_value.to_string(),
            provenance_class: "effect".to_owned(),
        },
    ));
    if let Some(argument_exprs) = input.get("argument_exprs").and_then(Value::as_array) {
        let mut arguments = serde_json::Map::new();
        for (index, expr) in argument_exprs.iter().filter_map(Value::as_str).enumerate() {
            arguments.insert(format!("arg{index}"), parse_field_value(expr, &context));
        }
        if let Some(object) = input.as_object_mut() {
            object.insert("arguments".to_owned(), Value::Object(arguments));
        }
    }
    if let Some(prompt) = input
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        if let Some(object) = input.as_object_mut() {
            object.insert(
                "prompt".to_owned(),
                Value::String(interpolate_prompt(&prompt, &context)),
            );
        }
    }
    Ok(input.to_string())
}

fn context_from_input_bindings(input: &Value) -> RuleContext {
    let mut context = RuleContext {
        trigger_event_id: None,
        identity: None,
        bindings: Vec::new(),
    };
    let Some(bindings) = input.get("bindings").and_then(Value::as_object) else {
        return context;
    };
    for (binding, value) in bindings {
        context.bindings.push((
            binding.clone(),
            FactView {
                fact_id: binding.clone(),
                name: binding.clone(),
                key: binding.clone(),
                value_json: value.to_string(),
                provenance_class: "input".to_owned(),
            },
        ));
    }
    context
}

fn run_capability_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whippletree_store::StoredEvent, StoreError> {
    let input = json_from_str(&effect.input_json);
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "capability-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "capability-lease"]);
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    kernel.start_run(RunStart {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: &options.provider,
        worker_id: "whip-worker",
        lease_id: &lease_id,
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: &json!({
            "target": effect.target,
            "input": input,
        })
        .to_string(),
    })?;

    let terminal = if options.fail {
        let metadata_json = json!({
            "failure": {
                "phase": "provider.capability.failed",
                "error_kind": "fixture_failure",
                "message": "fixture capability failure"
            },
            "target": effect.target,
            "input": input,
        })
        .to_string();
        let terminal = kernel.fail_run(EffectCompletion {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            status: "failed",
            exit_code: Some(1),
            summary: Some("fixture capability failure"),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "terminal",
            ])),
        })?;
        let value_json = json!({
            "effect_id": effect.effect_id,
            "run_id": run_id,
            "target": effect.target,
            "status": "failed",
            "value": null,
            "error": {
                "kind": "fixture_failure",
                "message": "fixture capability failure"
            },
            "summary": "fixture capability failure"
        })
        .to_string();
        kernel.derive_fact(
            instance_id,
            "capability.call.failed",
            &effect.effect_id,
            &value_json,
            Some(&terminal.event_id),
            Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "capability.call.failed",
            ])),
        )?;
        terminal
    } else {
        let value = json!({
            "summary": "Fixture capability context",
            "target": effect.target,
        });
        let metadata_json = json!({
            "target": effect.target,
            "input": input,
            "value": value,
        })
        .to_string();
        let terminal = kernel.complete_run(EffectCompletion {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            status: "completed",
            exit_code: Some(0),
            summary: Some("fixture capability completed"),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "terminal",
            ])),
        })?;
        let value_json = json!({
            "effect_id": effect.effect_id,
            "run_id": run_id,
            "target": effect.target,
            "status": "completed",
            "value": value,
            "error": null,
            "summary": "fixture capability completed"
        })
        .to_string();
        kernel.derive_fact(
            instance_id,
            "capability.call.succeeded",
            &effect.effect_id,
            &value_json,
            Some(&terminal.event_id),
            Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "capability.call.succeeded",
            ])),
        )?;
        terminal
    };
    Ok(terminal)
}

fn fixture_baml_value(output_type: &str) -> String {
    match output_type {
        "MessageClassification" => json!({
            "priority": "Normal",
            "summary": "Fixture classification",
            "confidence": 0.75,
        }),
        "WorkReview" => json!({
            "status": "Accept",
            "reason": "Fixture review",
            "followups": [],
            "confidence": 0.75,
        }),
        "FrenchPoemReview" => json!({
            "isFrench": true,
            "isPoem": true,
            "reason": "Fixture review saw a completed poem turn",
            "confidence": 0.75,
        }),
        "LanguageQualityReview" => json!({
            "isTargetLanguage": true,
            "usesExpectedScript": true,
            "isWellFormed": true,
            "reason": "Fixture review accepted the provider language task",
            "confidence": 0.75,
        }),
        _ => json!({
            "summary": "Fixture value",
            "confidence": 0.75,
        }),
    }
    .to_string()
}

fn fixture_harness(fail: bool) -> CommandAgentHarness {
    let script = if fail {
        "cat >/dev/null; echo fixture failure >&2; exit 42"
    } else {
        "cat >/dev/null; echo fixture completed"
    };
    CommandAgentHarness::new(
        CommandLaunchPlan::new("fixture", "sh")
            .arg("-c")
            .arg(script),
    )
}

fn dev(options: &CliOptions) -> ExitCode {
    let dev_options = match DevOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let started = match start_workflow_instance(
        &dev_options.program_path,
        options.input_json.as_deref(),
        options,
    ) {
        Ok(started) => started,
        Err(code) => return code,
    };
    let (_source, ir) = match compile_source_path(&dev_options.program_path) {
        Ok(compiled) => compiled,
        Err(error) => return report_compile_failure(&dev_options.program_path, error),
    };
    let mut steps = Vec::new();
    let mut workers = Vec::new();
    for _ in 0..dev_options.max_iterations {
        let step_report = match step_instance(&options.store_path, &started.instance_id, &ir) {
            Ok(report) => report,
            Err(error) => return report_store_error("failed to step instance", error),
        };
        let worker_report = match run_worker_once(
            &options.store_path,
            &WorkerOptions {
                instance_id: started.instance_id.clone(),
                provider: dev_options.provider.clone(),
                fail: dev_options.fail,
            },
        ) {
            Ok(report) => report,
            Err(error) => return report_store_error("failed to run worker", error),
        };
        let idle = step_report.committed_rules == 0 && worker_report.ran_effects == 0;
        steps.push(step_report);
        workers.push(worker_report);
        if idle {
            break;
        }
    }
    let store = match SqliteStore::open(&options.store_path) {
        Ok(store) => store,
        Err(error) => return report_store_error("failed to open store", error),
    };
    let facts = match store.list_facts(&started.instance_id) {
        Ok(facts) => facts,
        Err(error) => return report_store_error("failed to list facts", error),
    };
    let effects = match store.list_effects(&started.instance_id) {
        Ok(effects) => effects,
        Err(error) => return report_store_error("failed to list effects", error),
    };
    let assertions = eval_assertions(&ir, &facts, &effects);
    let failed_assertions = assertions
        .iter()
        .filter(|assertion| !assertion.passed)
        .collect::<Vec<_>>();
    let guard_errors = steps
        .iter()
        .flat_map(|step| &step.guard_reports)
        .filter(|guard| guard.status == GuardStatus::Error)
        .collect::<Vec<_>>();
    if options.json {
        let _ = emit_json(json!({
            "instance_id": started.instance_id,
            "workflow": started.workflow,
            "steps": steps.iter().map(step_report_to_json).collect::<Vec<_>>(),
            "workers": workers.iter().map(worker_report_to_json).collect::<Vec<_>>(),
            "assertions": assertions.iter().map(assertion_report_to_json).collect::<Vec<_>>(),
        }));
        if failed_assertions.is_empty() && guard_errors.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        }
    } else if !guard_errors.is_empty() {
        for guard in guard_errors {
            eprintln!(
                "guard error in rule {}: {}{}",
                guard.rule,
                guard.expr,
                guard
                    .error
                    .as_deref()
                    .map(|error| format!(" ({error})"))
                    .unwrap_or_default()
            );
        }
        ExitCode::from(1)
    } else if !failed_assertions.is_empty() {
        for assertion in failed_assertions {
            match assertion.status {
                AssertionStatus::Failed => eprintln!("assertion failed: {}", assertion.expr),
                AssertionStatus::Error => eprintln!(
                    "assertion error: {}{}",
                    assertion.expr,
                    assertion
                        .error
                        .as_deref()
                        .map(|error| format!(" ({error})"))
                        .unwrap_or_default()
                ),
                AssertionStatus::Passed => {}
            }
        }
        ExitCode::from(1)
    } else {
        println!("dev {}", started.instance_id);
        println!("workflow {}", started.workflow);
        println!("iterations {}", steps.len());
        ExitCode::SUCCESS
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DevOptions {
    program_path: String,
    provider: String,
    fail: bool,
    max_iterations: usize,
}

impl DevOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut program_path = None;
        let mut provider = "fixture".to_owned();
        let mut fail = false;
        let mut max_iterations = 8usize;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--provider" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected provider after `--provider`".to_owned());
                    };
                    provider = value.clone();
                }
                "--fail" => fail = true,
                "--max-iterations" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected number after `--max-iterations`".to_owned());
                    };
                    max_iterations = value
                        .parse::<usize>()
                        .map_err(|_| "expected integer after `--max-iterations`".to_owned())?;
                }
                "--until" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected value after `--until`".to_owned());
                    };
                    if value != "idle" {
                        return Err("only `--until idle` is supported".to_owned());
                    }
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown dev option `{other}`"))
                }
                value if program_path.is_none() => program_path = Some(value.to_owned()),
                _ => {
                    return Err(
                        "usage: whip dev <workflow.whip> [--provider fixture] [--until idle]"
                            .to_owned(),
                    )
                }
            }
            index += 1;
        }
        let Some(program_path) = program_path else {
            return Err(
                "usage: whip dev <workflow.whip> [--provider fixture] [--until idle]".to_owned(),
            );
        };
        Ok(Self {
            program_path,
            provider,
            fail,
            max_iterations,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RuleContext {
    trigger_event_id: Option<String>,
    identity: Option<String>,
    bindings: Vec<(String, FactView)>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct OwnedLowering {
    facts: Vec<OwnedFact>,
    effects: Vec<OwnedEffect>,
    dependencies: Vec<OwnedDependency>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedFact {
    fact_id: String,
    name: String,
    key: String,
    value_json: String,
    schema_id: Option<String>,
    provenance_class: String,
    correlation_id: Option<String>,
}

impl OwnedFact {
    fn as_new_fact(&self) -> NewFact<'_> {
        NewFact {
            fact_id: &self.fact_id,
            name: &self.name,
            key: &self.key,
            value_json: &self.value_json,
            schema_id: self.schema_id.as_deref(),
            provenance_class: &self.provenance_class,
            correlation_id: self.correlation_id.as_deref(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedEffect {
    effect_id: String,
    kind: String,
    target: Option<String>,
    input_json: String,
    status: String,
    idempotency_key: String,
    required_capabilities_json: String,
    profile: Option<String>,
    correlation_id: Option<String>,
}

impl OwnedEffect {
    fn as_new_effect(&self) -> NewEffect<'_> {
        NewEffect {
            effect_id: &self.effect_id,
            kind: &self.kind,
            target: self.target.as_deref(),
            input_json: &self.input_json,
            status: &self.status,
            idempotency_key: &self.idempotency_key,
            required_capabilities_json: &self.required_capabilities_json,
            profile: self.profile.as_deref(),
            correlation_id: self.correlation_id.as_deref(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedDependency {
    dependency_id: String,
    upstream_effect_id: String,
    downstream_effect_id: String,
    predicate: String,
}

impl OwnedDependency {
    fn as_new_dependency(&self) -> NewEffectDependency<'_> {
        NewEffectDependency {
            dependency_id: &self.dependency_id,
            upstream_effect_id: &self.upstream_effect_id,
            downstream_effect_id: &self.downstream_effect_id,
            predicate: &self.predicate,
        }
    }
}

fn ready_contexts(
    rule: &IrRule,
    facts: &[FactView],
    effects: &[EffectView],
    started_event_id: Option<&str>,
) -> ReadyContexts {
    let mut contexts = vec![RuleContext {
        trigger_event_id: started_event_id.map(str::to_owned),
        identity: None,
        bindings: Vec::new(),
    }];
    let mut guard_reports = Vec::new();
    for when in &rule.whens {
        let pattern = when.pattern.as_str();
        if pattern == "started" {
            if started_event_id.is_none() {
                return ReadyContexts::empty(guard_reports);
            }
            continue;
        }
        if pattern.ends_with(" is available") {
            continue;
        }
        if let Some((schema, binding)) = pattern.split_once(" as ") {
            let schema = schema.trim();
            let binding = binding.trim();
            let matching = facts
                .iter()
                .filter(|fact| fact.name == schema || fact.name == normalize_pattern_name(schema))
                .cloned()
                .collect::<Vec<_>>();
            if matching.is_empty() {
                return ReadyContexts::empty(guard_reports);
            }
            let mut expanded = Vec::new();
            for context in contexts {
                for fact in &matching {
                    let mut context = context.clone();
                    context.identity = Some(format!("{binding}:{}", fact.key));
                    context.bindings.push((binding.to_owned(), fact.clone()));
                    match &when.guard {
                        Some(guard) => {
                            let report = eval_guard(
                                &rule.name,
                                &when.source,
                                &guard.source,
                                &guard.expr,
                                &context,
                                facts,
                                effects,
                            );
                            let matched = report.matched;
                            guard_reports.push(report);
                            if matched {
                                expanded.push(context);
                            }
                        }
                        None => expanded.push(context),
                    }
                }
            }
            contexts = expanded;
            continue;
        }
        return ReadyContexts::empty(guard_reports);
    }
    ReadyContexts {
        contexts,
        guard_reports,
    }
}

struct ReadyContexts {
    contexts: Vec<RuleContext>,
    guard_reports: Vec<GuardReport>,
}

impl ReadyContexts {
    fn empty(guard_reports: Vec<GuardReport>) -> Self {
        Self {
            contexts: Vec::new(),
            guard_reports,
        }
    }
}

fn eval_guard(
    rule: &str,
    when: &str,
    source: &str,
    guard: &Expr,
    context: &RuleContext,
    facts: &[FactView],
    effects: &[EffectView],
) -> GuardReport {
    let (status, actual, error) = guard_result(eval_expr_value(
        guard,
        &EvalScope::rule(context, facts, effects),
    ));
    let matched = status == GuardStatus::Matched;
    GuardReport {
        rule: rule.to_owned(),
        when: when.to_owned(),
        expr: source.to_owned(),
        status,
        matched,
        actual,
        error,
    }
}

fn eval_guard_source(guard: &str, context: &RuleContext) -> bool {
    parse_expression(guard).ok().is_some_and(|expr| {
        guard_result(eval_expr_value(&expr, &EvalScope::rule(context, &[], &[]))).0
            == GuardStatus::Matched
    })
}

fn guard_result(value: EvalValue) -> (GuardStatus, Value, Option<String>) {
    match value {
        EvalValue::Json(Value::Bool(true)) => (GuardStatus::Matched, Value::Bool(true), None),
        EvalValue::Json(Value::Bool(false)) => (GuardStatus::False, Value::Bool(false), None),
        EvalValue::Json(value) => (
            GuardStatus::Error,
            value,
            Some("guard expression did not evaluate to bool".to_owned()),
        ),
        EvalValue::Missing => (
            GuardStatus::Error,
            json!({"internal": "Missing"}),
            Some("guard expression evaluated to Missing".to_owned()),
        ),
        EvalValue::Error => (
            GuardStatus::Error,
            json!({"internal": "Error"}),
            Some("guard expression evaluation failed".to_owned()),
        ),
    }
}

fn parse_guard_literal(expr: &str) -> Value {
    let expr = expr.trim();
    if let Some(unquoted) = expr
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return Value::String(unquoted.to_owned());
    }
    if let Ok(number) = expr.parse::<i64>() {
        return Value::Number(number.into());
    }
    match expr {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        value => Value::String(value.to_owned()),
    }
}

fn eval_assertions(
    ir: &IrProgram,
    facts: &[FactView],
    effects: &[EffectView],
) -> Vec<AssertionReport> {
    ir.assertions
        .iter()
        .map(|assertion| {
            eval_assertion(
                assertion.expr.source.as_str(),
                &assertion.expr.expr,
                facts,
                effects,
            )
        })
        .collect()
}

fn eval_assertion(
    source: &str,
    expr: &Expr,
    facts: &[FactView],
    effects: &[EffectView],
) -> AssertionReport {
    let source = source.trim();
    let (status, actual, error) = assertion_result(eval_expr_value(
        expr,
        &EvalScope::assertions(facts, effects),
    ));
    let passed = status == AssertionStatus::Passed;
    AssertionReport {
        expr: source.to_owned(),
        status,
        passed,
        actual,
        error,
    }
}

fn assertion_result(value: EvalValue) -> (AssertionStatus, Value, Option<String>) {
    match value {
        EvalValue::Json(Value::Bool(true)) => (AssertionStatus::Passed, Value::Bool(true), None),
        EvalValue::Json(Value::Bool(false)) => (AssertionStatus::Failed, Value::Bool(false), None),
        EvalValue::Json(value) => (
            AssertionStatus::Error,
            value,
            Some("assertion expression did not evaluate to bool".to_owned()),
        ),
        EvalValue::Missing => (
            AssertionStatus::Error,
            json!({"internal": "Missing"}),
            Some("assertion expression evaluated to Missing".to_owned()),
        ),
        EvalValue::Error => (
            AssertionStatus::Error,
            json!({"internal": "Error"}),
            Some("assertion expression evaluation failed".to_owned()),
        ),
    }
}

struct EvalScope<'a> {
    context: Option<&'a RuleContext>,
    facts: &'a [FactView],
    effects: &'a [EffectView],
    projection: Option<&'a Value>,
}

impl<'a> EvalScope<'a> {
    fn rule(context: &'a RuleContext, facts: &'a [FactView], effects: &'a [EffectView]) -> Self {
        Self {
            context: Some(context),
            facts,
            effects,
            projection: None,
        }
    }

    fn assertions(facts: &'a [FactView], effects: &'a [EffectView]) -> Self {
        Self {
            context: None,
            facts,
            effects,
            projection: None,
        }
    }

    fn projection(&self, projection: &'a Value) -> Self {
        Self {
            context: self.context,
            facts: self.facts,
            effects: self.effects,
            projection: Some(projection),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum EvalValue {
    Json(Value),
    Missing,
    Error,
}

impl EvalValue {
    fn into_json(self) -> Value {
        match self {
            Self::Json(value) => value,
            Self::Missing => json!({"internal": "Missing"}),
            Self::Error => json!({"internal": "Error"}),
        }
    }

    fn is_missing_or_null(&self) -> bool {
        matches!(self, Self::Missing | Self::Json(Value::Null))
    }
}

fn eval_expr_value(expr: &Expr, scope: &EvalScope<'_>) -> EvalValue {
    match expr {
        Expr::Literal(ExprLiteral::Ident(value)) => eval_ident_literal(value, scope),
        Expr::Literal(literal) => EvalValue::Json(eval_expr_literal(literal)),
        Expr::Path(path) => eval_path(path, scope),
        Expr::Index { target, key } => eval_index(target, key, scope),
        Expr::Array(items) => EvalValue::Json(Value::Array(
            items
                .iter()
                .map(|item| eval_expr_value(item, scope).into_json())
                .collect(),
        )),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Not => EvalValue::Json(Value::Bool(!truthy(&eval_expr_value(expr, scope)))),
        },
        Expr::Binary { op, left, right } => eval_binary(*op, left, right, scope),
        Expr::Call { name, args } => eval_call(name, args, scope),
        Expr::Query { .. } => eval_query_count(expr, scope)
            .map(|count| EvalValue::Json(Value::Number(count.into())))
            .unwrap_or(EvalValue::Error),
    }
}

fn eval_index(target: &Expr, key: &Expr, scope: &EvalScope<'_>) -> EvalValue {
    let target = eval_expr_value(target, scope);
    let key = eval_expr_value(key, scope);
    let key = match key {
        EvalValue::Json(Value::String(value)) => value,
        EvalValue::Missing => return EvalValue::Missing,
        _ => return EvalValue::Error,
    };
    match target {
        EvalValue::Json(Value::Object(object)) => object
            .get(&key)
            .cloned()
            .map(EvalValue::Json)
            .unwrap_or(EvalValue::Missing),
        EvalValue::Missing => EvalValue::Missing,
        _ => EvalValue::Error,
    }
}

fn eval_ident_literal(value: &str, scope: &EvalScope<'_>) -> EvalValue {
    if let Some(projection) = scope.projection {
        if let Some(value) = projection.get(value) {
            return EvalValue::Json(value.clone());
        }
    }
    EvalValue::Json(eval_expr_literal(&ExprLiteral::Ident(value.to_owned())))
}

fn eval_expr_literal(literal: &ExprLiteral) -> Value {
    match literal {
        ExprLiteral::String(value) | ExprLiteral::Ident(value) => Value::String(value.clone()),
        ExprLiteral::Number(value) => value
            .parse::<i64>()
            .map(|number| Value::Number(number.into()))
            .or_else(|_| {
                value
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(Value::Number)
                    .ok_or(())
            })
            .unwrap_or_else(|_| Value::String(value.clone())),
        ExprLiteral::Bool(value) => Value::Bool(*value),
        ExprLiteral::Null => Value::Null,
    }
}

fn eval_binary(op: BinaryOp, left: &Expr, right: &Expr, scope: &EvalScope<'_>) -> EvalValue {
    match op {
        BinaryOp::Or => {
            let left = eval_expr_value(left, scope);
            if truthy(&left) {
                EvalValue::Json(Value::Bool(true))
            } else {
                EvalValue::Json(Value::Bool(truthy(&eval_expr_value(right, scope))))
            }
        }
        BinaryOp::And => {
            let left = eval_expr_value(left, scope);
            if !truthy(&left) {
                EvalValue::Json(Value::Bool(false))
            } else {
                EvalValue::Json(Value::Bool(truthy(&eval_expr_value(right, scope))))
            }
        }
        BinaryOp::Eq => EvalValue::Json(Value::Bool(compare_eq(
            &eval_expr_value(left, scope),
            &eval_expr_value(right, scope),
        ))),
        BinaryOp::Ne => EvalValue::Json(Value::Bool(!compare_eq(
            &eval_expr_value(left, scope),
            &eval_expr_value(right, scope),
        ))),
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            let left = eval_expr_value(left, scope);
            let right = eval_expr_value(right, scope);
            let result = match (number_value(&left), number_value(&right)) {
                (Some(left), Some(right)) => match op {
                    BinaryOp::Lt => left < right,
                    BinaryOp::Le => left <= right,
                    BinaryOp::Gt => left > right,
                    BinaryOp::Ge => left >= right,
                    _ => false,
                },
                _ => false,
            };
            EvalValue::Json(Value::Bool(result))
        }
        BinaryOp::In | BinaryOp::NotIn => {
            let needle = eval_expr_value(left, scope).into_json();
            let haystack = eval_expr_value(right, scope).into_json();
            let contains = match &haystack {
                Value::Array(items) => items.iter().any(|item| item == &needle),
                Value::Object(object) => {
                    needle.as_str().is_some_and(|key| object.contains_key(key))
                }
                _ => false,
            };
            EvalValue::Json(Value::Bool(if op == BinaryOp::In {
                contains
            } else {
                !contains
            }))
        }
    }
}

fn eval_call(name: &str, args: &[Expr], scope: &EvalScope<'_>) -> EvalValue {
    match (name, args) {
        ("count", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Number(count.into())))
            .unwrap_or(EvalValue::Error),
        ("exists", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Bool(count > 0)))
            .unwrap_or(EvalValue::Error),
        ("exists", [expr]) => EvalValue::Json(Value::Bool(
            !eval_expr_value(expr, scope).is_missing_or_null(),
        )),
        ("empty", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Bool(count == 0)))
            .unwrap_or(EvalValue::Error),
        ("empty", [expr]) => {
            EvalValue::Json(Value::Bool(is_empty_value(&eval_expr_value(expr, scope))))
        }
        _ => EvalValue::Error,
    }
}

fn eval_query_count(query: &Expr, scope: &EvalScope<'_>) -> Result<i64, EvalValue> {
    let Expr::Query { kind, head, guard } = query else {
        return Ok(0);
    };
    match kind {
        QueryKind::Fact => {
            let mut count = 0;
            for fact in scope.facts.iter().filter(|fact| fact.name == head.trim()) {
                if let Some(guard) = guard {
                    let value = json_from_str(&fact.value_json);
                    if guard_filter_matches(eval_expr_value(guard, &scope.projection(&value)))? {
                        count += 1;
                    }
                } else {
                    count += 1;
                }
            }
            Ok(count)
        }
        QueryKind::Effect => {
            let kind = head
                .trim()
                .strip_prefix("kind ")
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let mut count = 0;
            for effect in scope
                .effects
                .iter()
                .filter(|effect| kind.is_none_or(|kind| effect.kind == kind))
            {
                if let Some(guard) = guard {
                    let value = json!({
                        "kind": effect.kind,
                        "target": effect.target,
                        "status": effect.status,
                        "profile": effect.profile,
                    });
                    if guard_filter_matches(eval_expr_value(guard, &scope.projection(&value)))? {
                        count += 1;
                    }
                } else {
                    count += 1;
                }
            }
            Ok(count)
        }
    }
}

fn guard_filter_matches(value: EvalValue) -> Result<bool, EvalValue> {
    match value {
        EvalValue::Json(Value::Bool(value)) => Ok(value),
        value => Err(value),
    }
}

fn eval_path(path: &[String], scope: &EvalScope<'_>) -> EvalValue {
    if path.is_empty() {
        return EvalValue::Error;
    }
    if let Some(context) = scope.context {
        if let Some(first) = path.first() {
            if let Some(rest) = path.get(1..) {
                if rest.is_empty() {
                    return context
                        .bindings
                        .iter()
                        .find_map(|(binding, fact)| {
                            (binding == first)
                                .then(|| EvalValue::Json(json_from_str(&fact.value_json)))
                        })
                        .unwrap_or(EvalValue::Missing);
                }
                if let Some(value) = context_path_value(context, first, &rest.join(".")) {
                    return EvalValue::Json(value);
                }
                return EvalValue::Missing;
            }
        }
    }
    if let Some(projection) = scope.projection {
        let mut current = projection;
        for field in path {
            let Some(next) = current.get(field) else {
                return EvalValue::Missing;
            };
            current = next;
        }
        return EvalValue::Json(current.clone());
    }
    EvalValue::Missing
}

fn compare_eq(left: &EvalValue, right: &EvalValue) -> bool {
    match (left, right) {
        (EvalValue::Json(left), EvalValue::Json(right)) => left == right,
        (EvalValue::Missing, EvalValue::Missing) => true,
        _ => false,
    }
}

fn truthy(value: &EvalValue) -> bool {
    match value {
        EvalValue::Json(Value::Bool(value)) => *value,
        EvalValue::Json(Value::Null) | EvalValue::Missing | EvalValue::Error => false,
        EvalValue::Json(Value::Array(values)) => !values.is_empty(),
        EvalValue::Json(Value::String(value)) => !value.is_empty(),
        EvalValue::Json(Value::Number(value)) => value.as_i64().unwrap_or(0) != 0,
        EvalValue::Json(Value::Object(value)) => !value.is_empty(),
    }
}

fn number_value(value: &EvalValue) -> Option<f64> {
    match value {
        EvalValue::Json(value) => value.as_f64(),
        _ => None,
    }
}

fn is_empty_value(value: &EvalValue) -> bool {
    match value {
        EvalValue::Missing => true,
        EvalValue::Json(Value::Null) => true,
        EvalValue::Json(Value::String(value)) => value.is_empty(),
        EvalValue::Json(Value::Array(value)) => value.is_empty(),
        EvalValue::Json(Value::Object(value)) => value.is_empty(),
        _ => false,
    }
}

fn lower_rule(
    ir: &IrProgram,
    rule: &IrRule,
    context: &RuleContext,
    facts: &[FactView],
    effects: &[EffectView],
) -> OwnedLowering {
    let (body, context) = selected_rule_body(&rule.body, context);
    let existing_fact_ids = facts
        .iter()
        .map(|fact| fact.fact_id.as_str())
        .collect::<Vec<_>>();
    let existing_effect_ids = effects
        .iter()
        .map(|effect| effect.effect_id.as_str())
        .collect::<Vec<_>>();
    let mut lowering = OwnedLowering::default();

    for block in top_level_record_blocks(&body) {
        let value = parse_record_fields(&block.body, &context);
        let value_json = Value::Object(value).to_string();
        let fact_key = record_fact_key(&block.schema, &value_json);
        let fact_id = idempotency_key(&[&rule.name, &block.schema, &fact_key, &value_json]);
        if existing_fact_ids
            .iter()
            .any(|existing| *existing == fact_id)
        {
            continue;
        }
        lowering.facts.push(OwnedFact {
            fact_id,
            name: block.schema.clone(),
            key: fact_key,
            value_json,
            schema_id: Some(block.schema),
            provenance_class: "rule".to_owned(),
            correlation_id: context.identity.clone(),
        });
    }

    let parsed_effects = parse_effect_statements(&body, &context);
    let mut node_to_effect_id = std::collections::BTreeMap::new();
    let mut binding_to_effect_id = std::collections::BTreeMap::new();
    for (index, parsed) in parsed_effects.iter().enumerate() {
        let effect_node = effect_node_for_parsed(rule, parsed, index);
        let node_id = effect_node
            .map(|effect| effect.id.as_str())
            .unwrap_or(parsed.kind.as_str());
        let effect_id = idempotency_key(&[
            &rule.name,
            node_id,
            context.identity.as_deref().unwrap_or("started"),
        ]);
        node_to_effect_id.insert(node_id.to_owned(), effect_id.clone());
        if let Some(binding) = effect_node
            .and_then(|effect| effect.binding.as_ref())
            .or(parsed.binding.as_ref())
        {
            binding_to_effect_id.insert(binding.clone(), effect_id);
        }
    }
    for (index, parsed) in parsed_effects.iter().enumerate() {
        let effect_node = effect_node_for_parsed(rule, parsed, index);
        let node_id = effect_node
            .map(|effect| effect.id.as_str())
            .unwrap_or(parsed.kind.as_str());
        let Some(effect_id) = node_to_effect_id.get(node_id).cloned() else {
            continue;
        };
        if existing_effect_ids
            .iter()
            .any(|existing| *existing == effect_id)
        {
            continue;
        }
        let input_json =
            parsed_effect_input_json(ir, rule, parsed, &context, &binding_to_effect_id);
        let profile = parsed
            .target
            .as_deref()
            .and_then(|target| ir.agents.iter().find(|agent| agent.name == target))
            .and_then(|agent| agent.profile.clone());
        let effect_idempotency_key = idempotency_key(&[&effect_id, "effect"]);
        lowering.effects.push(OwnedEffect {
            effect_id,
            kind: parsed.kind.clone(),
            target: parsed.target.clone(),
            input_json,
            status: "queued".to_owned(),
            idempotency_key: effect_idempotency_key,
            required_capabilities_json: parsed.required_capabilities_json(),
            profile,
            correlation_id: context.identity.clone(),
        });
    }

    for dependency in &rule.metadata.dependencies {
        let Some(upstream_effect_id) = node_to_effect_id.get(&dependency.upstream) else {
            continue;
        };
        let Some(downstream_effect_id) = node_to_effect_id.get(&dependency.downstream) else {
            continue;
        };
        if !lowering.effects.iter().any(|effect| {
            effect.effect_id == *upstream_effect_id || effect.effect_id == *downstream_effect_id
        }) {
            continue;
        }
        lowering.dependencies.push(OwnedDependency {
            dependency_id: idempotency_key(&[
                &rule.name,
                upstream_effect_id,
                dependency_predicate_str(&dependency.predicate),
                downstream_effect_id,
            ]),
            upstream_effect_id: upstream_effect_id.clone(),
            downstream_effect_id: downstream_effect_id.clone(),
            predicate: dependency_predicate_str(&dependency.predicate).to_owned(),
        });
    }

    for after in after_record_blocks(&body) {
        let Some(upstream_effect_id) = binding_to_effect_id.get(&after.binding) else {
            continue;
        };
        let Some(binding_value) = effect_binding_value(facts, upstream_effect_id, &after.predicate)
        else {
            continue;
        };
        let mut after_context = context.clone();
        push_effect_binding(
            &mut after_context,
            &after.binding,
            upstream_effect_id,
            binding_value,
        );
        for (binding, effect_id) in &binding_to_effect_id {
            if binding == &after.binding {
                continue;
            }
            if let Some(value) = effect_binding_value(facts, effect_id, "succeeds") {
                push_effect_binding(&mut after_context, binding, effect_id, value);
            }
        }
        let value = parse_record_fields(&after.record.body, &after_context);
        let value_json = Value::Object(value).to_string();
        let fact_key = record_fact_key(&after.record.schema, &value_json);
        let fact_id = idempotency_key(&[
            &rule.name,
            &after.binding,
            &after.predicate,
            &after.record.schema,
            &fact_key,
            &value_json,
        ]);
        if existing_fact_ids
            .iter()
            .any(|existing| *existing == fact_id)
            || lowering.facts.iter().any(|fact| fact.fact_id == fact_id)
        {
            continue;
        }
        lowering.facts.push(OwnedFact {
            fact_id,
            name: after.record.schema.clone(),
            key: fact_key,
            value_json,
            schema_id: Some(after.record.schema),
            provenance_class: "rule".to_owned(),
            correlation_id: context.identity.clone(),
        });
    }

    lowering
}

fn effect_node_for_parsed<'a>(
    rule: &'a IrRule,
    parsed: &ParsedEffect,
    index: usize,
) -> Option<&'a IrEffectNode> {
    parsed
        .binding
        .as_ref()
        .and_then(|binding| {
            rule.metadata
                .effects
                .iter()
                .find(|effect| effect.binding.as_ref() == Some(binding))
        })
        .or_else(|| rule.metadata.effects.get(index))
}

fn push_effect_binding(context: &mut RuleContext, binding: &str, effect_id: &str, value: Value) {
    context
        .bindings
        .retain(|(candidate, _)| candidate != binding);
    context.bindings.push((
        binding.to_owned(),
        FactView {
            fact_id: effect_id.to_owned(),
            name: binding.to_owned(),
            key: effect_id.to_owned(),
            value_json: value.to_string(),
            provenance_class: "effect".to_owned(),
        },
    ));
}

fn selected_rule_body(body: &str, context: &RuleContext) -> (String, RuleContext) {
    let lines = body.lines().collect::<Vec<_>>();
    let mut selected = Vec::new();
    let mut context = context.clone();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if !trimmed.starts_with("case ") {
            selected.push(lines[index].to_owned());
            index += 1;
            continue;
        }
        let Some((case, next_index)) = parse_case_block(&lines, index) else {
            selected.push(lines[index].to_owned());
            index += 1;
            continue;
        };
        if let Some(branch) = select_case_branch(&case, &mut context) {
            let (branch_body, branch_context) =
                selected_rule_body(&branch.body.join("\n"), &context);
            context = branch_context;
            selected.extend(branch_body.lines().map(str::to_owned));
        }
        index = next_index;
    }
    (selected.join("\n"), context)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CaseBlock {
    scrutinee: String,
    branches: Vec<CaseBranch>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CaseBranch {
    pattern: String,
    guard: Option<String>,
    body: Vec<String>,
}

fn parse_case_block(lines: &[&str], start: usize) -> Option<(CaseBlock, usize)> {
    let header = lines.get(start)?.trim();
    let scrutinee = header
        .strip_prefix("case ")?
        .strip_suffix('{')
        .unwrap_or_else(|| header.strip_prefix("case ").expect("case prefix"))
        .trim()
        .to_owned();
    let mut branches = Vec::new();
    let mut index = start + 1;
    let mut case_depth = brace_delta(header).max(1);
    while index < lines.len() && case_depth > 0 {
        let trimmed = lines[index].trim();
        if let Some((pattern, guard, before_body)) = case_branch_header(trimmed) {
            let mut body = Vec::new();
            let mut branch_depth = brace_delta(before_body).max(1);
            index += 1;
            while index < lines.len() && branch_depth > 0 {
                let line = lines[index];
                let next_depth = branch_depth + brace_delta(line);
                if next_depth >= 1 {
                    body.push(line.to_owned());
                }
                branch_depth = next_depth;
                index += 1;
            }
            branches.push(CaseBranch {
                pattern,
                guard,
                body,
            });
            continue;
        }
        case_depth += brace_delta(trimmed);
        index += 1;
    }
    Some((
        CaseBlock {
            scrutinee,
            branches,
        },
        index,
    ))
}

fn case_branch_header(line: &str) -> Option<(String, Option<String>, &str)> {
    let (head, body_start) = line.split_once("=>")?;
    let body_start = body_start.trim();
    if !body_start.starts_with('{') {
        return None;
    }
    let head = head.trim();
    let (pattern, guard) = match head.split_once(" where ") {
        Some((pattern, guard)) => (pattern.trim(), Some(guard.trim().to_owned())),
        None => (head, None),
    };
    Some((pattern.to_owned(), guard, body_start))
}

fn select_case_branch(case: &CaseBlock, context: &mut RuleContext) -> Option<CaseBranch> {
    let value = parse_field_value(&case.scrutinee, context);
    let mut fallback = None;
    for branch in &case.branches {
        if matches!(branch.pattern.as_str(), "_" | "default") {
            fallback = Some(branch.clone());
            continue;
        }
        if case_pattern_matches(&branch.pattern, &value, context)
            && branch
                .guard
                .as_deref()
                .is_none_or(|guard| eval_guard_source(guard, context))
        {
            return Some(branch.clone());
        }
    }
    fallback
}

fn case_pattern_matches(pattern: &str, value: &Value, context: &mut RuleContext) -> bool {
    if pattern == "None" {
        return value.is_null();
    }
    if let Some(binding) = pattern.strip_prefix("Some ").map(str::trim) {
        if value.is_null() || binding.is_empty() {
            return false;
        }
        context.bindings.push((
            binding.to_owned(),
            FactView {
                fact_id: format!("case:{binding}"),
                name: binding.to_owned(),
                key: binding.to_owned(),
                value_json: value.to_string(),
                provenance_class: "case".to_owned(),
            },
        ));
        return true;
    }
    parse_guard_literal(pattern) == *value
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordBlock {
    schema: String,
    body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AfterRecordBlock {
    binding: String,
    predicate: String,
    record: RecordBlock,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AfterScope {
    binding: String,
    predicate: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveAfterScope {
    scope: AfterScope,
    depth: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedEffect {
    kind: String,
    target: Option<String>,
    name: Option<String>,
    binding: Option<String>,
    args: Vec<String>,
    prompt: Option<String>,
    after: Option<AfterScope>,
}

impl ParsedEffect {
    fn required_capabilities_json(&self) -> String {
        match self.kind.as_str() {
            "baml.coerce" => r#"["baml.coerce"]"#,
            "loft.claim" => r#"["loft.claim"]"#,
            "human.ask" => r#"["human.ask"]"#,
            "capability.call" => r#"["capability.call"]"#,
            _ => "[]",
        }
        .to_owned()
    }
}

fn parse_effect_statements(body: &str, context: &RuleContext) -> Vec<ParsedEffect> {
    let mut effects = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut after_scopes: Vec<ActiveAfterScope> = Vec::new();
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if let Some(scope) = parse_after_scope(trimmed) {
            after_scopes.push(ActiveAfterScope {
                scope,
                depth: brace_delta(trimmed).max(1),
            });
            index += 1;
            continue;
        }
        let current_after = after_scopes.last().map(|scope| scope.scope.clone());
        if let Some(rest) = trimmed.strip_prefix("tell ") {
            let target_expr = rest.split_whitespace().next().unwrap_or("agent");
            let (prompt, next_index) = parse_prompt_from_lines(&lines, index, trimmed);
            effects.push(ParsedEffect {
                kind: "agent.tell".to_owned(),
                target: Some(resolve_tell_target(target_expr, context)),
                name: None,
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: Some(interpolate_prompt(&prompt, context)),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("coerce ") {
            let call = rest.split(" as ").next().unwrap_or(rest).trim();
            let name = call.split_once('(').map(|(name, _)| name).unwrap_or(call);
            let args = call
                .split_once('(')
                .and_then(|(_, tail)| tail.rsplit_once(')').map(|(args, _)| args))
                .map(split_args)
                .unwrap_or_default();
            effects.push(ParsedEffect {
                kind: "baml.coerce".to_owned(),
                target: Some(name.to_owned()),
                name: Some(name.to_owned()),
                binding: binding_after_as(trimmed),
                args,
                prompt: None,
                after: current_after,
            });
        } else if trimmed.starts_with("claim ") && trimmed.contains(" with loft") {
            effects.push(ParsedEffect {
                kind: "loft.claim".to_owned(),
                target: Some("loft".to_owned()),
                name: Some("claim".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                after: current_after,
            });
        } else if trimmed.starts_with("askHuman ") {
            let (prompt, next_index) = parse_prompt_from_lines(&lines, index, trimmed);
            effects.push(ParsedEffect {
                kind: "human.ask".to_owned(),
                target: Some("human".to_owned()),
                name: Some("askHuman".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: Some(interpolate_prompt(&prompt, context)),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("call ") {
            let target = rest
                .split_whitespace()
                .next()
                .unwrap_or("plugin")
                .to_owned();
            effects.push(ParsedEffect {
                kind: "capability.call".to_owned(),
                target: Some(target),
                name: Some("call".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                after: current_after,
            });
        }
        let delta = brace_delta(trimmed);
        for scope in &mut after_scopes {
            scope.depth += delta;
        }
        after_scopes.retain(|scope| scope.depth > 0);
        index += 1;
    }
    effects
}

fn resolve_tell_target(target_expr: &str, context: &RuleContext) -> String {
    parse_field_value(target_expr, context)
        .as_str()
        .unwrap_or(target_expr)
        .to_owned()
}

fn parsed_effect_input_json(
    ir: &IrProgram,
    rule: &IrRule,
    effect: &ParsedEffect,
    context: &RuleContext,
    effect_bindings: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut input = match effect.kind.as_str() {
        "agent.tell" => json!({
            "prompt": effect.prompt.as_deref().unwrap_or_default(),
            "rule": rule.name,
            "bindings": context_bindings_json(context),
        }),
        "baml.coerce" => {
            let function_name = effect.name.as_deref().unwrap_or("coerce");
            let output_type = ir
                .coerces
                .iter()
                .find(|coerce| coerce.name == function_name)
                .map(|coerce| ir_type_name(&coerce.output))
                .unwrap_or_else(|| "json".to_owned());
            let mut arguments = serde_json::Map::new();
            for (index, arg) in effect.args.iter().enumerate() {
                arguments.insert(format!("arg{index}"), parse_field_value(arg, context));
            }
            json!({
                "function_name": function_name,
                "arguments": Value::Object(arguments),
                "argument_exprs": effect.args,
                "output_type": output_type,
                "bindings": context_bindings_json(context),
                "rule": rule.name,
            })
        }
        "loft.claim" => json!({
            "action": "claim",
            "issue": context_bindings_json(context),
            "rule": rule.name,
        }),
        "human.ask" => json!({
            "prompt": effect.prompt.as_deref().unwrap_or_default(),
            "choices": ["accept", "revise", "block"],
            "severity": "normal",
            "rule": rule.name,
        }),
        "capability.call" => json!({
            "target": effect.target,
            "bindings": context_bindings_json(context),
            "rule": rule.name,
        }),
        _ => json!({"rule": rule.name}),
    };
    if let Some(after) = &effect.after {
        if let Some(upstream_effect_id) = effect_bindings.get(&after.binding) {
            if let Some(object) = input.as_object_mut() {
                object.insert(
                    "after".to_owned(),
                    json!({
                        "binding": after.binding,
                        "predicate": after.predicate,
                        "upstream_effect_id": upstream_effect_id,
                    }),
                );
            }
        }
    }
    input.to_string()
}

fn parse_after_scope(trimmed: &str) -> Option<AfterScope> {
    let rest = trimmed.strip_prefix("after ")?;
    let mut parts = rest.split('{').next().unwrap_or(rest).split_whitespace();
    let binding = parts.next()?;
    let predicate = parts.next()?;
    Some(AfterScope {
        binding: binding.to_owned(),
        predicate: predicate.to_owned(),
    })
}

fn binding_after_as(line: &str) -> Option<String> {
    let mut tokens = line.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "as" {
            return tokens
                .next()
                .map(|binding| binding.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_'))
                .filter(|binding| !binding.is_empty())
                .map(str::to_owned);
        }
    }
    None
}

fn parse_prompt_from_lines(lines: &[&str], index: usize, trimmed: &str) -> (String, usize) {
    if trimmed.contains("\"\"\"") {
        let mut prompt_lines = Vec::new();
        let after_open = trimmed
            .split_once("\"\"\"")
            .map(|(_, tail)| tail)
            .unwrap_or("");
        if !after_open.is_empty() {
            prompt_lines.push(after_open.to_owned());
        }
        let mut cursor = index + 1;
        while cursor < lines.len() {
            let line = lines[cursor];
            if let Some((head, _tail)) = line.split_once("\"\"\"") {
                prompt_lines.push(head.to_owned());
                return (prompt_lines.join("\n").trim().to_owned(), cursor);
            }
            prompt_lines.push(line.to_owned());
            cursor += 1;
        }
        return (prompt_lines.join("\n").trim().to_owned(), cursor);
    }
    let prompt = trimmed
        .split_once('"')
        .and_then(|(_, tail)| tail.rsplit_once('"').map(|(prompt, _)| prompt))
        .unwrap_or("")
        .to_owned();
    (prompt, index)
}

fn split_args(args: &str) -> Vec<String> {
    args.split(',')
        .map(str::trim)
        .filter(|arg| !arg.is_empty())
        .map(str::to_owned)
        .collect()
}

fn dependency_predicate_str(predicate: &IrDependencyPredicate) -> &'static str {
    match predicate {
        IrDependencyPredicate::Succeeds => "succeeds",
        IrDependencyPredicate::Fails => "fails",
        IrDependencyPredicate::Completes => "completes",
    }
}

fn normalize_pattern_name(pattern: &str) -> String {
    pattern.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ir_type_name(ty: &IrType) -> String {
    match ty {
        IrType::Primitive(primitive) => match primitive {
            IrPrimitiveType::String => "string",
            IrPrimitiveType::Int => "int",
            IrPrimitiveType::Float => "float",
            IrPrimitiveType::Bool => "bool",
            IrPrimitiveType::Null => "null",
            IrPrimitiveType::Duration => "duration",
            IrPrimitiveType::Time => "time",
            IrPrimitiveType::Image => "image",
            IrPrimitiveType::Audio => "audio",
            IrPrimitiveType::Pdf => "pdf",
            IrPrimitiveType::Video => "video",
        }
        .to_owned(),
        IrType::LiteralString(value) | IrType::Ref(value) => value.clone(),
        IrType::AgentRef(agents) => format!("AgentRef<{}>", agents.join(" | ")),
        IrType::Optional(inner) => ir_type_name(inner),
        IrType::Array(inner) => format!("{}[]", ir_type_name(inner)),
        IrType::Map(inner) => format!("map<{}>", ir_type_name(inner)),
        IrType::Union(types) => types
            .iter()
            .map(ir_type_name)
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

fn top_level_record_blocks(body: &str) -> Vec<RecordBlock> {
    let mut blocks = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut skip_depth = 0i32;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if skip_depth > 0 {
            skip_depth += brace_delta(trimmed);
            index += 1;
            continue;
        }
        if trimmed.starts_with("after ") {
            skip_depth += brace_delta(trimmed).max(1);
            index += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("record ") {
            let schema = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches('{')
                .to_owned();
            let mut record_lines = Vec::new();
            let mut depth = brace_delta(trimmed);
            index += 1;
            while index < lines.len() && depth > 0 {
                let line = lines[index];
                depth += brace_delta(line);
                if depth >= 1 && line.trim() != "}" {
                    record_lines.push(line.to_owned());
                }
                index += 1;
            }
            blocks.push(RecordBlock {
                schema,
                body: record_lines.join("\n"),
            });
            continue;
        }
        index += 1;
    }
    blocks
}

fn after_record_blocks(body: &str) -> Vec<AfterRecordBlock> {
    let mut blocks = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        let Some(rest) = trimmed.strip_prefix("after ") else {
            index += 1;
            continue;
        };
        let mut parts = rest.split('{').next().unwrap_or(rest).split_whitespace();
        let Some(binding) = parts.next() else {
            index += 1;
            continue;
        };
        let Some(predicate) = parts.next() else {
            index += 1;
            continue;
        };
        let mut depth = brace_delta(trimmed).max(1);
        let mut inner = Vec::new();
        index += 1;
        while index < lines.len() && depth > 0 {
            let line = lines[index];
            depth += brace_delta(line);
            if depth >= 1 && line.trim() != "}" {
                inner.push(line.to_owned());
            }
            index += 1;
        }
        for record in top_level_record_blocks(&inner.join("\n")) {
            blocks.push(AfterRecordBlock {
                binding: binding.to_owned(),
                predicate: predicate.to_owned(),
                record,
            });
        }
    }
    blocks
}

fn effect_binding_value(
    facts: &[FactView],
    upstream_effect_id: &str,
    predicate: &str,
) -> Option<Value> {
    facts.iter().find_map(|fact| {
        let payload = json_from_str(&fact.value_json);
        if payload.get("effect_id").and_then(Value::as_str) != Some(upstream_effect_id) {
            return None;
        }
        if !fact_matches_after_predicate(&fact.name, &payload, predicate) {
            return None;
        }
        Some(
            payload
                .get("value")
                .cloned()
                .or_else(|| payload.get("output").cloned())
                .or_else(|| payload.get("error").cloned())
                .unwrap_or(payload),
        )
    })
}

fn fact_matches_after_predicate(name: &str, payload: &Value, predicate: &str) -> bool {
    let status = payload.get("status").and_then(Value::as_str);
    match predicate {
        "succeeds" => {
            name.ends_with(".succeeded")
                || name.ends_with(".completed")
                || status == Some("completed")
        }
        "fails" => name.ends_with(".failed") || matches!(status, Some("failed" | "timed_out")),
        "completes" => {
            name.ends_with(".succeeded")
                || name.ends_with(".failed")
                || name.ends_with(".completed")
                || matches!(
                    status,
                    Some("completed" | "failed" | "timed_out" | "cancelled")
                )
        }
        _ => false,
    }
}

fn brace_delta(line: &str) -> i32 {
    line.chars().fold(0, |depth, ch| match ch {
        '{' => depth + 1,
        '}' => depth - 1,
        _ => depth,
    })
}

fn parse_record_fields(body: &str, context: &RuleContext) -> serde_json::Map<String, Value> {
    let mut object = serde_json::Map::new();
    for line in body.lines() {
        let trimmed = line.trim().trim_end_matches(',');
        if trimmed.is_empty() {
            continue;
        }
        let Some((name, value)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        object.insert(name.to_owned(), parse_field_value(value.trim(), context));
    }
    object
}

fn parse_field_value(value: &str, context: &RuleContext) -> Value {
    let value = value.trim();
    if let Some(unquoted) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return Value::String(interpolate_prompt(unquoted, context));
    }
    if matches!(value.as_bytes().first(), Some(b'{' | b'[')) {
        if let Ok(parsed) = serde_json::from_str(value) {
            return parsed;
        }
    }
    if value == "true" {
        return Value::Bool(true);
    }
    if value == "false" {
        return Value::Bool(false);
    }
    if value == "null" {
        return Value::Null;
    }
    if let Ok(number) = value.parse::<i64>() {
        return Value::Number(number.into());
    }
    if let Some((binding, field)) = value.split_once('.') {
        return context_field_value(context, binding, field).unwrap_or(Value::Null);
    }
    context
        .bindings
        .iter()
        .find(|(binding, _)| binding == value)
        .map(|(_, fact)| json_from_str(&fact.value_json))
        .unwrap_or_else(|| Value::String(value.to_owned()))
}

fn interpolate_prompt(prompt: &str, context: &RuleContext) -> String {
    let mut rendered = prompt.to_owned();
    for (binding, fact) in &context.bindings {
        let value = json_from_str(&fact.value_json);
        if let Some(object) = value.as_object() {
            for (field, field_value) in object {
                let needle = format!("{{{{ {binding}.{field} }}}}");
                rendered = rendered.replace(&needle, &render_interpolation_value(field_value));
            }
        }
    }
    rendered
}

fn render_interpolation_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn context_field_value(context: &RuleContext, binding: &str, field: &str) -> Option<Value> {
    context_path_value(context, binding, field)
}

fn context_path_value(context: &RuleContext, binding: &str, path: &str) -> Option<Value> {
    let fact = context
        .bindings
        .iter()
        .find(|(candidate, _)| candidate == binding)?
        .1
        .value_json
        .clone();
    let mut value = json_from_str(&fact);
    for field in path.split('.') {
        value = value.get(field)?.clone();
    }
    Some(value)
}

fn context_bindings_json(context: &RuleContext) -> Value {
    let mut object = serde_json::Map::new();
    for (binding, fact) in &context.bindings {
        object.insert(binding.clone(), json_from_str(&fact.value_json));
    }
    Value::Object(object)
}

fn record_fact_key(schema: &str, value_json: &str) -> String {
    let value = json_from_str(value_json);
    if let Some(number) = value.get("number").and_then(Value::as_i64) {
        return format!("{schema}:{number}");
    }
    if let Some(status) = value.get("status").and_then(Value::as_str) {
        return format!("{schema}:{status}:{}", stable_hash_hex(value_json));
    }
    format!("{schema}:{}", stable_hash_hex(value_json))
}

fn step_report_to_json(report: &StepReport) -> Value {
    json!({
        "instance_id": report.instance_id,
        "committed_rules": report.committed_rules,
        "facts_created": report.facts_created,
        "effects_created": report.effects_created,
        "guards": report.guard_reports.iter().map(guard_report_to_json).collect::<Vec<_>>(),
    })
}

fn guard_report_to_json(report: &GuardReport) -> Value {
    let mut value = json!({
        "rule": report.rule,
        "when": report.when,
        "expr": report.expr,
        "status": report.status.as_str(),
        "matched": report.matched,
        "actual": report.actual,
    });
    if let Some(error) = &report.error {
        if let Some(object) = value.as_object_mut() {
            object.insert("error".to_owned(), Value::String(error.clone()));
        }
    }
    value
}

fn assertion_report_to_json(report: &AssertionReport) -> Value {
    let mut value = json!({
        "expr": report.expr,
        "status": report.status.as_str(),
        "passed": report.passed,
        "actual": report.actual,
    });
    if let Some(error) = &report.error {
        if let Some(object) = value.as_object_mut() {
            object.insert("error".to_owned(), Value::String(error.clone()));
        }
    }
    value
}

fn worker_report_to_json(report: &WorkerReport) -> Value {
    json!({
        "instance_id": report.instance_id,
        "provider": report.provider,
        "ran_effects": report.ran_effects,
        "terminal_events": report.terminal_events,
    })
}

fn lowering_idempotency_key(lowering: &OwnedLowering) -> String {
    let mut ids = Vec::new();
    ids.extend(lowering.facts.iter().map(|fact| fact.fact_id.as_str()));
    ids.extend(
        lowering
            .effects
            .iter()
            .map(|effect| effect.effect_id.as_str()),
    );
    ids.extend(
        lowering
            .dependencies
            .iter()
            .map(|dependency| dependency.dependency_id.as_str()),
    );
    idempotency_key(&ids)
}

fn instances(options: &CliOptions) -> ExitCode {
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let instances = match store.list_instances() {
        Ok(instances) => instances,
        Err(error) => return report_store_error("failed to list instances", error),
    };

    if options.json {
        emit_json(Value::Array(
            instances.iter().map(instance_to_json).collect::<Vec<_>>(),
        ))
    } else {
        if instances.is_empty() {
            println!("no instances");
        }
        for instance in instances {
            println!(
                "{} {} workflow_version={} updated={}",
                instance.instance_id, instance.status, instance.version_id, instance.updated_at
            );
        }
        ExitCode::SUCCESS
    }
}

fn status(options: &CliOptions) -> ExitCode {
    let Some(instance_id) = single_arg(options, "usage: whip status <instance>") else {
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let Some(status) = (match store.status(instance_id) {
        Ok(status) => status,
        Err(error) => return report_store_error("failed to load status", error),
    }) else {
        eprintln!("instance `{instance_id}` was not found");
        eprintln!(
            "try `whip instances --store {}`",
            options.store_path.display()
        );
        return ExitCode::FAILURE;
    };

    if options.json {
        emit_json(status_to_json(&status))
    } else {
        println!(
            "instance {} {}",
            status.instance.instance_id, status.instance.status
        );
        println!(
            "facts={} queued_effects={} blocked_effects={} active_runs={} failures={}",
            status.fact_count,
            status.queued_effect_count,
            status.blocked_effect_count,
            status.active_run_count,
            status.failure_count
        );
        if status.recent_events.is_empty() {
            println!("recent events: none");
        } else {
            println!("recent events:");
            for event in status.recent_events {
                println!(
                    "  #{} {} source={}",
                    event.sequence, event.event_type, event.source
                );
            }
        }
        ExitCode::SUCCESS
    }
}

fn log(options: &CliOptions) -> ExitCode {
    let Some(instance_id) = single_arg(options, "usage: whip log <instance>") else {
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let events = match store.list_events(instance_id) {
        Ok(events) => events,
        Err(error) => return report_store_error("failed to load event log", error),
    };

    if options.json {
        emit_json(Value::Array(
            events.iter().map(event_to_json).collect::<Vec<_>>(),
        ))
    } else {
        for event in events {
            println!(
                "#{:04} {} {} source={}",
                event.sequence, event.occurred_at, event.event_type, event.source
            );
        }
        ExitCode::SUCCESS
    }
}

fn facts(options: &CliOptions) -> ExitCode {
    let Some(instance_id) = single_arg(options, "usage: whip facts <instance>") else {
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let facts = match store.list_facts(instance_id) {
        Ok(facts) => facts,
        Err(error) => return report_store_error("failed to load facts", error),
    };

    if options.json {
        emit_json(Value::Array(
            facts.iter().map(fact_to_json).collect::<Vec<_>>(),
        ))
    } else {
        for fact in facts {
            println!("{} {} {}", fact.name, fact.key, fact.value_json);
        }
        ExitCode::SUCCESS
    }
}

fn effects(options: &CliOptions) -> ExitCode {
    let Some(instance_id) = single_arg(options, "usage: whip effects <instance>") else {
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let effects = match store.list_effects(instance_id) {
        Ok(effects) => effects,
        Err(error) => return report_store_error("failed to load effects", error),
    };

    if options.json {
        emit_json(Value::Array(
            effects.iter().map(effect_to_json).collect::<Vec<_>>(),
        ))
    } else {
        for effect in effects {
            println!(
                "{} {} status={} target={} profile={} reason={}",
                effect.effect_id,
                effect.kind,
                effect.status,
                effect.target.as_deref().unwrap_or("-"),
                effect.profile.as_deref().unwrap_or("-"),
                effect.policy_block_reason.as_deref().unwrap_or("-")
            );
        }
        ExitCode::SUCCESS
    }
}

fn runs(options: &CliOptions) -> ExitCode {
    let Some(instance_id) = single_arg(options, "usage: whip runs <instance>") else {
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let runs = match store.list_runs(instance_id) {
        Ok(runs) => runs,
        Err(error) => return report_store_error("failed to load runs", error),
    };

    if options.json {
        emit_json(Value::Array(
            runs.iter().map(run_to_json).collect::<Vec<_>>(),
        ))
    } else {
        for run in runs {
            println!(
                "{} effect={} status={} worker={} started={}",
                run.run_id, run.effect_id, run.status, run.worker_id, run.started_at
            );
        }
        ExitCode::SUCCESS
    }
}

fn inbox(options: &CliOptions) -> ExitCode {
    match options.args.first().map(String::as_str) {
        None => inbox_list(options),
        Some("show") => inbox_show(options),
        Some("answer") => inbox_answer(options),
        _ => {
            eprintln!("usage: whip inbox [show <item>|answer <item> (--choice X|--text X)]");
            ExitCode::from(2)
        }
    }
}

fn inbox_list(options: &CliOptions) -> ExitCode {
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let items = match store.list_inbox_items(Some("pending")) {
        Ok(items) => items,
        Err(error) => return report_store_error("failed to list inbox items", error),
    };

    if options.json {
        emit_json(Value::Array(
            items.iter().map(inbox_item_to_json).collect::<Vec<_>>(),
        ))
    } else {
        if items.is_empty() {
            println!("inbox empty");
        }
        for item in items {
            println!(
                "{} instance={} severity={} created={}",
                item.inbox_item_id, item.instance_id, item.severity, item.created_at
            );
            println!("  {}", one_line(&item.prompt));
        }
        ExitCode::SUCCESS
    }
}

fn inbox_show(options: &CliOptions) -> ExitCode {
    if options.args.len() != 2 {
        eprintln!("usage: whip inbox show <item>");
        return ExitCode::from(2);
    }
    let item_id = &options.args[1];
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let Some(item) = (match store.get_inbox_item(item_id) {
        Ok(item) => item,
        Err(error) => return report_store_error("failed to load inbox item", error),
    }) else {
        eprintln!("inbox item `{item_id}` was not found");
        return ExitCode::FAILURE;
    };

    if options.json {
        emit_json(inbox_item_to_json(&item))
    } else {
        println!("{} status={}", item.inbox_item_id, item.status);
        println!("instance {}", item.instance_id);
        println!("severity {}", item.severity);
        println!("freeform_allowed {}", item.freeform_allowed);
        println!("choices {}", item.choices_json);
        println!();
        println!("{}", item.prompt);
        if let Some(answer) = item.answer_json {
            println!();
            println!("answer {answer}");
        }
        ExitCode::SUCCESS
    }
}

fn inbox_answer(options: &CliOptions) -> ExitCode {
    if options.args.len() < 4 {
        eprintln!("usage: whip inbox answer <item> (--choice X|--text X) [--by NAME]");
        return ExitCode::from(2);
    }
    let item_id = &options.args[1];
    let mut choice = None;
    let mut text = None;
    let mut answered_by = env::var("USER").unwrap_or_else(|_| "operator".to_owned());
    let mut index = 2;
    while index < options.args.len() {
        match options.args[index].as_str() {
            "--choice" => {
                index += 1;
                let Some(value) = options.args.get(index) else {
                    eprintln!("expected a value after `--choice`");
                    return ExitCode::from(2);
                };
                choice = Some(value.clone());
            }
            "--text" => {
                index += 1;
                let Some(value) = options.args.get(index) else {
                    eprintln!("expected a value after `--text`");
                    return ExitCode::from(2);
                };
                text = Some(value.clone());
            }
            "--by" => {
                index += 1;
                let Some(value) = options.args.get(index) else {
                    eprintln!("expected a value after `--by`");
                    return ExitCode::from(2);
                };
                answered_by = value.clone();
            }
            other => {
                eprintln!("unknown inbox answer option `{other}`");
                return ExitCode::from(2);
            }
        }
        index += 1;
    }
    if choice.is_some() == text.is_some() {
        eprintln!("provide exactly one of `--choice` or `--text`");
        return ExitCode::from(2);
    }
    let answer_json = if let Some(choice) = choice {
        json!({"kind": "choice", "choice": choice, "answered_by": answered_by}).to_string()
    } else {
        json!({"kind": "text", "text": text.expect("text present"), "answered_by": answered_by})
            .to_string()
    };
    let mut store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let event = match store.answer_inbox_item(HumanAnswer {
        inbox_item_id: item_id,
        answer_json: &answer_json,
        answered_by: &answered_by,
        idempotency_key: Some(&idempotency_key(&[item_id, "human-answer", &answer_json])),
    }) {
        Ok(event) => event,
        Err(error) => return report_store_error("failed to answer inbox item", error),
    };

    if options.json {
        emit_json(json!({
            "inbox_item_id": item_id,
            "event_id": event.event_id,
            "sequence": event.sequence,
            "answer": json_from_str(&answer_json),
        }))
    } else {
        println!("{item_id} answered at event #{}", event.sequence);
        ExitCode::SUCCESS
    }
}

fn evidence(options: &CliOptions) -> ExitCode {
    let Some(instance_id) = single_arg(options, "usage: whip evidence <instance>") else {
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let evidence = match store.list_evidence(instance_id) {
        Ok(evidence) => evidence,
        Err(error) => return report_store_error("failed to list evidence", error),
    };
    let links = match store.list_evidence_links(instance_id) {
        Ok(links) => links,
        Err(error) => return report_store_error("failed to list evidence links", error),
    };

    if options.json {
        emit_json(json!({
            "instance_id": instance_id,
            "evidence": evidence.iter().map(evidence_to_json).collect::<Vec<_>>(),
            "links": links.iter().map(evidence_link_to_json).collect::<Vec<_>>(),
        }))
    } else {
        for evidence in evidence {
            println!(
                "{} {} subject={}:{} created={}",
                evidence.evidence_id,
                evidence.kind,
                evidence.subject_type,
                evidence.subject_id,
                evidence.created_at
            );
            if let Some(summary) = evidence.summary {
                println!("  {summary}");
            }
        }
        ExitCode::SUCCESS
    }
}

fn trace(options: &CliOptions) -> ExitCode {
    let trace_options = match TraceOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let events = match store.list_events(&trace_options.instance_id) {
        Ok(events) => events,
        Err(error) => return report_store_error("failed to list events", error),
    };
    let facts = match store.list_facts(&trace_options.instance_id) {
        Ok(facts) => facts,
        Err(error) => return report_store_error("failed to list facts", error),
    };
    let effects = match store.list_effects(&trace_options.instance_id) {
        Ok(effects) => effects,
        Err(error) => return report_store_error("failed to list effects", error),
    };
    let runs = match store.list_runs(&trace_options.instance_id) {
        Ok(runs) => runs,
        Err(error) => return report_store_error("failed to list runs", error),
    };
    let evidence = match store.list_evidence(&trace_options.instance_id) {
        Ok(evidence) => evidence,
        Err(error) => return report_store_error("failed to list evidence", error),
    };
    let links = match store.list_evidence_links(&trace_options.instance_id) {
        Ok(links) => links,
        Err(error) => return report_store_error("failed to list evidence links", error),
    };
    let abstract_records = reconstruct_trace_records(&events);
    let conformance = if trace_options.check {
        Some(check_trace(&abstract_records))
    } else {
        None
    };
    let mut trace_json = json!({
        "schema": "whippletree.local_trace.v0",
        "instance_id": trace_options.instance_id,
        "events": events.iter().map(event_to_json).collect::<Vec<_>>(),
        "facts": facts.iter().map(fact_to_json).collect::<Vec<_>>(),
        "effects": effects.iter().map(effect_to_json).collect::<Vec<_>>(),
        "runs": runs.iter().map(run_to_json).collect::<Vec<_>>(),
        "evidence": evidence.iter().map(evidence_to_json).collect::<Vec<_>>(),
        "evidence_links": links.iter().map(evidence_link_to_json).collect::<Vec<_>>(),
    });
    if trace_options.check {
        if let Some(object) = trace_json.as_object_mut() {
            object.insert(
                "abstract_trace".to_owned(),
                Value::Array(
                    abstract_records
                        .iter()
                        .map(trace_record_to_json)
                        .collect::<Vec<_>>(),
                ),
            );
            object.insert(
                "conformance".to_owned(),
                match &conformance {
                    Some(Ok(())) => json!({"ok": true}),
                    Some(Err(violation)) => json!({
                        "ok": false,
                        "sequence": violation.sequence,
                        "message": violation.message,
                    }),
                    None => json!({"ok": true}),
                },
            );
        }
    }

    if options.json {
        let code = emit_json(trace_json);
        if matches!(conformance, Some(Err(_))) {
            ExitCode::FAILURE
        } else {
            code
        }
    } else {
        println!("trace {}", trace_options.instance_id);
        println!(
            "events={} facts={} effects={} runs={} evidence={} links={}",
            events.len(),
            facts.len(),
            effects.len(),
            runs.len(),
            evidence.len(),
            links.len()
        );
        match conformance {
            Some(Ok(())) => {
                println!("conformance=ok abstract_events={}", abstract_records.len());
                ExitCode::SUCCESS
            }
            Some(Err(violation)) => {
                eprintln!(
                    "conformance violation at abstract event #{}: {}",
                    violation.sequence, violation.message
                );
                ExitCode::FAILURE
            }
            None => ExitCode::SUCCESS,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct TraceOptions {
    instance_id: String,
    check: bool,
}

impl TraceOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut instance_id = None;
        let mut check = false;

        for arg in args {
            match arg.as_str() {
                "--check" => check = true,
                _ if arg.starts_with('-') => {
                    return Err(format!(
                        "unknown trace option `{arg}`\nusage: whip trace <instance> [--check]"
                    ));
                }
                _ if instance_id.is_none() => instance_id = Some(arg.clone()),
                _ => return Err("usage: whip trace <instance> [--check]".to_owned()),
            }
        }

        let Some(instance_id) = instance_id else {
            return Err("usage: whip trace <instance> [--check]".to_owned());
        };

        Ok(Self { instance_id, check })
    }
}

fn reconstruct_trace_records(events: &[EventView]) -> Vec<TraceRecord> {
    let mut records = Vec::new();

    for event in events {
        let payload = json_from_str(&event.payload_json);
        match event.event_type.as_str() {
            "rule.committed" => {
                if let Some(effects) = payload.get("effects").and_then(Value::as_array) {
                    for effect in effects {
                        let Some(effect_id) = effect.get("effect_id").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        push_trace_record(
                            &mut records,
                            TraceEvent::EffectCreated {
                                effect_id: effect_id.to_owned(),
                                status: trace_effect_status(
                                    effect
                                        .get("status")
                                        .and_then(Value::as_str)
                                        .unwrap_or("queued"),
                                ),
                            },
                        );
                    }
                }
                if let Some(dependencies) = payload.get("dependencies").and_then(Value::as_array) {
                    for dependency in dependencies {
                        let Some(upstream_effect_id) =
                            dependency.get("upstream_effect_id").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        let Some(downstream_effect_id) = dependency
                            .get("downstream_effect_id")
                            .and_then(Value::as_str)
                        else {
                            continue;
                        };
                        push_trace_record(
                            &mut records,
                            TraceEvent::DependencyCreated(DependencyEdge {
                                upstream_effect_id: upstream_effect_id.to_owned(),
                                downstream_effect_id: downstream_effect_id.to_owned(),
                                predicate: trace_dependency_predicate(
                                    dependency
                                        .get("predicate")
                                        .and_then(Value::as_str)
                                        .unwrap_or("succeeds"),
                                ),
                            }),
                        );
                    }
                }
            }
            "effect.run_started" => {
                let Some(effect_id) = payload.get("effect_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(run_id) = payload.get("run_id").and_then(Value::as_str) else {
                    continue;
                };
                push_trace_record(
                    &mut records,
                    TraceEvent::EffectClaimed {
                        effect_id: effect_id.to_owned(),
                    },
                );
                push_trace_record(
                    &mut records,
                    TraceEvent::RunStarted {
                        run_id: run_id.to_owned(),
                        effect_id: effect_id.to_owned(),
                    },
                );
            }
            "effect.terminal" => {
                let Some(effect_id) = payload.get("effect_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(run_id) = payload.get("run_id").and_then(Value::as_str) else {
                    continue;
                };
                let status = trace_effect_status(
                    payload
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("failed"),
                );
                push_trace_record(
                    &mut records,
                    TraceEvent::EffectTerminal {
                        run_id: run_id.to_owned(),
                        effect_id: effect_id.to_owned(),
                        status,
                    },
                );
            }
            "effect.blocked" => {
                let Some(effect_id) = payload.get("effect_id").and_then(Value::as_str) else {
                    continue;
                };
                push_trace_record(
                    &mut records,
                    TraceEvent::EffectBlocked {
                        effect_id: effect_id.to_owned(),
                        reason: payload
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("blocked")
                            .to_owned(),
                    },
                );
            }
            "effect.cancelled" => {
                let Some(effect_id) = payload.get("effect_id").and_then(Value::as_str) else {
                    continue;
                };
                push_trace_record(
                    &mut records,
                    TraceEvent::EffectCancelled {
                        effect_id: effect_id.to_owned(),
                    },
                );
            }
            "lease.expired" => {
                let Some(effect_id) = payload.get("effect_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(run_id) = payload.get("run_id").and_then(Value::as_str) else {
                    continue;
                };
                push_trace_record(
                    &mut records,
                    TraceEvent::LeaseExpired {
                        run_id: run_id.to_owned(),
                        effect_id: effect_id.to_owned(),
                    },
                );
            }
            "instance.transitioned" => match payload.get("status").and_then(Value::as_str) {
                Some("paused") => push_trace_record(&mut records, TraceEvent::InstancePaused),
                Some("running") => push_trace_record(&mut records, TraceEvent::InstanceResumed),
                Some("cancelled") => push_trace_record(&mut records, TraceEvent::InstanceCancelled),
                _ => {}
            },
            _ => {}
        }
    }

    records
}

fn push_trace_record(records: &mut Vec<TraceRecord>, event: TraceEvent) {
    records.push(TraceRecord {
        sequence: records.len() as u64 + 1,
        event,
    });
}

fn trace_effect_status(status: &str) -> EffectStatus {
    match status {
        "blocked_by_dependency" | "blocked_by_capability" | "blocked_by_profile" => {
            EffectStatus::Blocked
        }
        "claimed" => EffectStatus::Claimed,
        "running" => EffectStatus::Running,
        "completed" => EffectStatus::Completed,
        "failed" => EffectStatus::Failed,
        "timed_out" => EffectStatus::TimedOut,
        "cancelled" => EffectStatus::Cancelled,
        _ => EffectStatus::Queued,
    }
}

fn trace_dependency_predicate(predicate: &str) -> DependencyPredicate {
    match predicate {
        "fails" => DependencyPredicate::Fails,
        "completes" => DependencyPredicate::Completes,
        _ => DependencyPredicate::Succeeds,
    }
}

fn pause(options: &CliOptions) -> ExitCode {
    transition_instance(
        options,
        "usage: whip pause <instance>",
        |kernel, instance_id| {
            kernel.pause_instance(
                instance_id,
                Some("operator pause"),
                Some(&idempotency_key(&[instance_id, "pause"])),
            )
        },
    )
}

fn resume(options: &CliOptions) -> ExitCode {
    transition_instance(
        options,
        "usage: whip resume <instance>",
        |kernel, instance_id| {
            kernel.resume_instance(
                instance_id,
                Some(&idempotency_key(&[instance_id, "resume"])),
            )
        },
    )
}

fn cancel(options: &CliOptions) -> ExitCode {
    transition_instance(
        options,
        "usage: whip cancel <instance>",
        |kernel, instance_id| {
            kernel.cancel_instance(
                instance_id,
                Some("operator cancel"),
                Some(&idempotency_key(&[instance_id, "cancel"])),
            )
        },
    )
}

fn transition_instance(
    options: &CliOptions,
    usage: &str,
    transition: impl FnOnce(
        &mut RuntimeKernel,
        &str,
    ) -> Result<whippletree_store::StoredEvent, StoreError>,
) -> ExitCode {
    let Some(instance_id) = single_arg(options, usage) else {
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let mut kernel = RuntimeKernel::new(store);
    match transition(&mut kernel, instance_id) {
        Ok(event) if options.json => emit_json(json!({
            "instance_id": instance_id,
            "event_id": event.event_id,
            "sequence": event.sequence,
        })),
        Ok(event) => {
            println!("{instance_id} updated at event #{}", event.sequence);
            ExitCode::SUCCESS
        }
        Err(error) => report_store_error("failed to transition instance", error),
    }
}

fn retry(options: &CliOptions) -> ExitCode {
    if options.args.len() != 2 {
        eprintln!("usage: whip retry <instance> <effect>");
        return ExitCode::from(2);
    }
    let instance_id = &options.args[0];
    let effect_id = &options.args[1];
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let mut kernel = RuntimeKernel::new(store);
    match kernel.retry_effect(RetryEffect {
        instance_id,
        effect_id,
        retry_after: None,
        idempotency_key: Some(&idempotency_key(&[instance_id, "retry", effect_id])),
    }) {
        Ok(event) if options.json => emit_json(json!({
            "instance_id": instance_id,
            "effect_id": effect_id,
            "event_id": event.event_id,
            "sequence": event.sequence,
        })),
        Ok(event) => {
            println!("{effect_id} retried at event #{}", event.sequence);
            ExitCode::SUCCESS
        }
        Err(error) => report_store_error("failed to retry effect", error),
    }
}

fn single_arg<'a>(options: &'a CliOptions, usage: &str) -> Option<&'a str> {
    if options.args.len() == 1 {
        Some(&options.args[0])
    } else {
        eprintln!("{usage}");
        None
    }
}

fn open_store_or_exit(options: &CliOptions) -> Result<SqliteStore, ExitCode> {
    open_store(&options.store_path).map_err(|message| {
        eprintln!("{message}");
        ExitCode::FAILURE
    })
}

fn open_store(path: &Path) -> Result<SqliteStore, String> {
    if path.to_string_lossy() == ":memory:" {
        return SqliteStore::open_in_memory().map_err(store_error);
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create store directory `{}`: {error}",
                    parent.display()
                )
            })?;
        }
    }

    SqliteStore::open(path).map_err(|error| {
        format!(
            "failed to open store `{}`: {}",
            path.display(),
            store_error(error)
        )
    })
}

enum CompileFailure {
    Io(std::io::Error),
    Diagnostics {
        source: String,
        diagnostics: Vec<Diagnostic>,
    },
}

fn compile_source_path(path: &str) -> Result<(String, IrProgram), CompileFailure> {
    let source = fs::read_to_string(path).map_err(CompileFailure::Io)?;
    let compiled = whippletree_parser::compile_program(&source);
    if let Some(ir) = compiled.ir {
        Ok((source, ir))
    } else {
        Err(CompileFailure::Diagnostics {
            source,
            diagnostics: compiled.diagnostics,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ModelSearchReport {
    searches: usize,
    solutions: usize,
    no_solutions: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MaudeRunOutput {
    stdout: String,
    stderr: String,
}

fn run_model_search(path: &str, source: &str, ir: &IrProgram) -> Result<ModelSearchReport, String> {
    let kernel_path = Path::new("models/maude/kernel.maude");
    if !kernel_path.exists() {
        return Err("models/maude/kernel.maude was not found".to_owned());
    }
    let kernel_path = fs::canonicalize(kernel_path)
        .map_err(|error| format!("failed to resolve Maude kernel path: {error}"))?;
    let (maude_source, expected) = generate_maude_model_search(source, ir, &kernel_path);
    if expected.is_empty() {
        return Ok(ModelSearchReport {
            searches: 0,
            solutions: 0,
            no_solutions: 0,
        });
    }
    let output = run_maude_source(path, &maude_source)?;
    if !output.stderr.is_empty() && output.stdout.is_empty() {
        return Err(format!("Maude produced no stdout\n{}", output.stderr));
    }
    let actual = extract_maude_search_results(&output.stdout);
    let solutions = actual
        .iter()
        .filter(|actual| **actual == ExpectedSearchResult::Solution)
        .count();
    let no_solutions = actual.len() - solutions;
    let expected_solutions = expected
        .iter()
        .filter(|expected| expected.outcome == ExpectedSearchResult::Solution)
        .count();
    let expected_no_solutions = expected.len() - expected_solutions;
    if actual.len() != expected.len() {
        let diagnostic = expected.first().map(|first_expected| Diagnostic {
            span: first_expected.span,
            message: format!(
                "model search produced {} result(s), expected {}",
                actual.len(),
                expected.len()
            ),
            suggestion: Some(
                "inspect the generated effect dependency checks or rerun with Maude available"
                    .to_owned(),
            ),
        });
        return Err(format_model_search_error(
            path,
            source,
            diagnostic.as_ref(),
            &format!(
                "expected {expected_solutions} solution(s) and {expected_no_solutions} no-solution result(s), got {solutions} solution(s) and {no_solutions} no-solution result(s)\n{}{}",
                output.stdout,
                output.stderr
            ),
        ));
    }
    for (index, (expected, actual)) in expected.iter().zip(actual.iter()).enumerate() {
        if expected.outcome != *actual {
            let diagnostic = Diagnostic {
                span: expected.span,
                message: format!("model-search counterexample for {}", expected.description),
                suggestion: Some(format!(
                    "expected {}, got {}; inspect dependency {} --{}--> {}",
                    expected.outcome.label(),
                    actual.label(),
                    expected.upstream,
                    expected.predicate,
                    expected.downstream
                )),
            };
            return Err(format_model_search_error(
                path,
                source,
                Some(&diagnostic),
                &format!(
                    "search #{} failed: expected {}, got {}\n{}{}",
                    index + 1,
                    expected.outcome.label(),
                    actual.label(),
                    output.stdout,
                    output.stderr
                ),
            ));
        }
    }
    Ok(ModelSearchReport {
        searches: expected.len(),
        solutions,
        no_solutions,
    })
}

fn run_maude_source(label: &str, source: &str) -> Result<MaudeRunOutput, String> {
    let path_hash = stable_hash_hex(label);
    let model_path = env::temp_dir().join(format!(
        "whippletree-model-search-{}-{path_hash}.maude",
        std::process::id()
    ));
    fs::write(&model_path, source)
        .map_err(|error| format!("failed to write generated Maude file: {error}"))?;
    let maude = match find_executable_in_path(&["maude"], &path_value()) {
        Some(maude) => maude,
        None => {
            let _ = fs::remove_file(&model_path);
            return Err("Maude executable `maude` was not found on PATH".to_owned());
        }
    };
    let output = Command::new(&maude)
        .arg(&model_path)
        .output()
        .map_err(|error| format!("failed to run `{maude}`: {error}"))?;
    let _ = fs::remove_file(&model_path);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "Maude exited with status {:?}\n{}{}",
            output.status.code(),
            stdout,
            stderr
        ));
    }
    Ok(MaudeRunOutput { stdout, stderr })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpectedSearchResult {
    Solution,
    NoSolution,
}

impl ExpectedSearchResult {
    fn label(&self) -> &'static str {
        match self {
            Self::Solution => "solution",
            Self::NoSolution => "no solution",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExpectedSearch {
    outcome: ExpectedSearchResult,
    span: SourceSpan,
    description: String,
    upstream: String,
    predicate: &'static str,
    downstream: String,
}

fn generate_maude_model_search(
    source: &str,
    ir: &IrProgram,
    kernel_path: &Path,
) -> (String, Vec<ExpectedSearch>) {
    let mut effect_symbols = std::collections::BTreeMap::<String, String>::new();
    for rule in &ir.rules {
        for effect in &rule.metadata.effects {
            let key = effect_key(&rule.name, &effect.id);
            effect_symbols
                .entry(key.clone())
                .or_insert_with(|| maude_symbol("eff", &key));
        }
    }

    let mut output = String::new();
    let mut expected = Vec::new();
    output.push_str(&format!("load {}\n\n", kernel_path.display()));
    output.push_str("mod WHIPPLETREE-GENERATED-CHECK is\n");
    output.push_str("  including WHIPPLETREE-KERNEL .\n");
    if !effect_symbols.is_empty() {
        output.push_str("  ops\n");
        for symbol in effect_symbols.values() {
            output.push_str("    ");
            output.push_str(symbol);
            output.push('\n');
        }
        output.push_str("    : -> EffectId .\n");
    }
    output.push_str("endm\n\n");

    for rule in &ir.rules {
        for dependency in &rule.metadata.dependencies {
            let upstream_key = effect_key(&rule.name, &dependency.upstream);
            let downstream_key = effect_key(&rule.name, &dependency.downstream);
            let Some(upstream) = effect_symbols.get(&upstream_key) else {
                continue;
            };
            let Some(downstream) = effect_symbols.get(&downstream_key) else {
                continue;
            };
            let predicate = maude_predicate(&dependency.predicate);
            let terminal = satisfying_terminal(&dependency.predicate);
            let span = dependency_source_span(source, &dependency.upstream, predicate);
            output.push_str(&format!(
                "--- {}: {} --{}--> {} cannot run before upstream terminal.\n",
                rule.name, dependency.upstream, predicate, dependency.downstream
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLETREE-GENERATED-CHECK :\n  effect({upstream}, queued) dep({upstream}, {predicate}, {downstream}) effect({downstream}, blocked)\n  =>*\n  effect({upstream}, queued) dep({upstream}, {predicate}, {downstream}) effect({downstream}, running) .\n\n"
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::NoSolution,
                span,
                description: format!(
                    "{} --{}--> {} cannot run before upstream terminal",
                    dependency.upstream, predicate, dependency.downstream
                ),
                upstream: dependency.upstream.clone(),
                predicate,
                downstream: dependency.downstream.clone(),
            });

            output.push_str(&format!(
                "--- {}: satisfying terminal releases downstream.\n",
                rule.name
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLETREE-GENERATED-CHECK :\n  effect({upstream}, queued) dep({upstream}, {predicate}, {downstream}) effect({downstream}, blocked)\n  =>*\n  effect({upstream}, {terminal}) dep({upstream}, {predicate}, {downstream}) effect({downstream}, running) .\n\n"
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::Solution,
                span,
                description: format!(
                    "{} --{}--> {} releases after satisfying terminal",
                    dependency.upstream, predicate, dependency.downstream
                ),
                upstream: dependency.upstream.clone(),
                predicate,
                downstream: dependency.downstream.clone(),
            });

            if let Some(non_terminal) = non_satisfying_terminal(&dependency.predicate) {
                output.push_str(&format!(
                    "--- {}: non-satisfying terminal does not release downstream.\n",
                    rule.name
                ));
                output.push_str(&format!(
                    "search [1] in WHIPPLETREE-GENERATED-CHECK :\n  effect({upstream}, queued) dep({upstream}, {predicate}, {downstream}) effect({downstream}, blocked)\n  =>*\n  effect({upstream}, {non_terminal}) dep({upstream}, {predicate}, {downstream}) effect({downstream}, running) .\n\n"
                ));
                expected.push(ExpectedSearch {
                    outcome: ExpectedSearchResult::NoSolution,
                    span,
                    description: format!(
                        "{} --{}--> {} does not release after non-satisfying terminal",
                        dependency.upstream, predicate, dependency.downstream
                    ),
                    upstream: dependency.upstream.clone(),
                    predicate,
                    downstream: dependency.downstream.clone(),
                });
            }
        }
    }

    (output, expected)
}

fn extract_maude_search_results(output: &str) -> Vec<ExpectedSearchResult> {
    let mut matches = Vec::new();
    for (index, _) in output.match_indices("Solution 1") {
        matches.push((index, ExpectedSearchResult::Solution));
    }
    for (index, _) in output.match_indices("No solution.") {
        matches.push((index, ExpectedSearchResult::NoSolution));
    }
    matches.sort_by_key(|(index, _)| *index);
    matches.into_iter().map(|(_, result)| result).collect()
}

fn format_model_search_error(
    path: &str,
    source: &str,
    diagnostic: Option<&Diagnostic>,
    details: &str,
) -> String {
    let mut rendered = String::new();
    if let Some(diagnostic) = diagnostic {
        rendered.push_str(&render_diagnostic(path, source, diagnostic));
    }
    rendered.push_str(details);
    rendered
}

fn dependency_source_span(source: &str, upstream: &str, predicate: &str) -> SourceSpan {
    let pattern = format!("after {upstream} {predicate}");
    source
        .find(&pattern)
        .map(|start| SourceSpan {
            start,
            end: start + pattern.len(),
        })
        .unwrap_or(SourceSpan { start: 0, end: 0 })
}

fn effect_key(rule_name: &str, effect_id: &str) -> String {
    format!("{rule_name}:{effect_id}")
}

fn maude_symbol(prefix: &str, value: &str) -> String {
    format!("{prefix}{:016x}", stable_hash(value))
}

fn maude_predicate(predicate: &whippletree_parser::DependencyPredicate) -> &'static str {
    match predicate {
        whippletree_parser::DependencyPredicate::Succeeds => "succeeds",
        whippletree_parser::DependencyPredicate::Fails => "fails",
        whippletree_parser::DependencyPredicate::Completes => "completes",
    }
}

fn satisfying_terminal(predicate: &whippletree_parser::DependencyPredicate) -> &'static str {
    match predicate {
        whippletree_parser::DependencyPredicate::Succeeds => "completed",
        whippletree_parser::DependencyPredicate::Fails => "failed",
        whippletree_parser::DependencyPredicate::Completes => "completed",
    }
}

fn non_satisfying_terminal(
    predicate: &whippletree_parser::DependencyPredicate,
) -> Option<&'static str> {
    match predicate {
        whippletree_parser::DependencyPredicate::Succeeds => Some("failed"),
        whippletree_parser::DependencyPredicate::Fails => Some("completed"),
        whippletree_parser::DependencyPredicate::Completes => None,
    }
}

fn report_compile_failure(path: &str, error: CompileFailure) -> ExitCode {
    match error {
        CompileFailure::Io(error) => {
            eprintln!("{path}: failed to read: {error}");
        }
        CompileFailure::Diagnostics {
            source,
            diagnostics,
        } => {
            for diagnostic in diagnostics {
                eprint!("{}", render_diagnostic(path, &source, &diagnostic));
            }
        }
    }
    ExitCode::FAILURE
}

fn report_store_error(context: &str, error: StoreError) -> ExitCode {
    eprintln!("{context}: {}", store_error(error));
    ExitCode::FAILURE
}

fn store_error(error: StoreError) -> String {
    match error {
        StoreError::Io(error) => error.to_string(),
        StoreError::Sqlite(error) => error.to_string(),
        StoreError::Json(error) => error.to_string(),
        StoreError::Conflict(message) => message,
        StoreError::PolicyBlocked { reason, .. } => reason,
    }
}

fn emit_json(value: Value) -> ExitCode {
    match serde_json::to_string_pretty(&value) {
        Ok(rendered) => {
            println!("{rendered}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to render JSON: {error}");
            ExitCode::FAILURE
        }
    }
}

fn instance_to_json(instance: &InstanceView) -> Value {
    json!({
        "instance_id": instance.instance_id,
        "program_id": instance.program_id,
        "version_id": instance.version_id,
        "status": instance.status,
        "input": json_from_str(&instance.input_json),
        "created_at": instance.created_at,
        "updated_at": instance.updated_at,
    })
}

fn event_to_json(event: &EventView) -> Value {
    json!({
        "event_id": event.event_id,
        "sequence": event.sequence,
        "event_type": event.event_type,
        "payload": json_from_str(&event.payload_json),
        "source": event.source,
        "occurred_at": event.occurred_at,
    })
}

fn trace_record_to_json(record: &TraceRecord) -> Value {
    json!({
        "sequence": record.sequence,
        "event": trace_event_to_json(&record.event),
    })
}

fn trace_event_to_json(event: &TraceEvent) -> Value {
    match event {
        TraceEvent::EffectCreated { effect_id, status } => json!({
            "type": "effect_created",
            "effect_id": effect_id,
            "status": trace_status_name(status),
        }),
        TraceEvent::DependencyCreated(edge) => json!({
            "type": "dependency_created",
            "upstream_effect_id": edge.upstream_effect_id,
            "downstream_effect_id": edge.downstream_effect_id,
            "predicate": trace_predicate_name(&edge.predicate),
        }),
        TraceEvent::EffectClaimed { effect_id } => json!({
            "type": "effect_claimed",
            "effect_id": effect_id,
        }),
        TraceEvent::RunStarted { run_id, effect_id } => json!({
            "type": "run_started",
            "run_id": run_id,
            "effect_id": effect_id,
        }),
        TraceEvent::LeaseExpired { run_id, effect_id } => json!({
            "type": "lease_expired",
            "run_id": run_id,
            "effect_id": effect_id,
        }),
        TraceEvent::EffectTerminal {
            run_id,
            effect_id,
            status,
        } => json!({
            "type": "effect_terminal",
            "run_id": run_id,
            "effect_id": effect_id,
            "status": trace_status_name(status),
        }),
        TraceEvent::EffectBlocked { effect_id, reason } => json!({
            "type": "effect_blocked",
            "effect_id": effect_id,
            "reason": reason,
        }),
        TraceEvent::EffectCancelled { effect_id } => json!({
            "type": "effect_cancelled",
            "effect_id": effect_id,
        }),
        TraceEvent::InstancePaused => json!({"type": "instance_paused"}),
        TraceEvent::InstanceResumed => json!({"type": "instance_resumed"}),
        TraceEvent::InstanceCancelled => json!({"type": "instance_cancelled"}),
    }
}

fn trace_status_name(status: &EffectStatus) -> &'static str {
    match status {
        EffectStatus::Queued => "queued",
        EffectStatus::Blocked => "blocked",
        EffectStatus::Claimed => "claimed",
        EffectStatus::Running => "running",
        EffectStatus::Completed => "completed",
        EffectStatus::Failed => "failed",
        EffectStatus::TimedOut => "timed_out",
        EffectStatus::Cancelled => "cancelled",
    }
}

fn trace_predicate_name(predicate: &DependencyPredicate) -> &'static str {
    match predicate {
        DependencyPredicate::Succeeds => "succeeds",
        DependencyPredicate::Fails => "fails",
        DependencyPredicate::Completes => "completes",
    }
}

fn fact_to_json(fact: &FactView) -> Value {
    json!({
        "fact_id": fact.fact_id,
        "name": fact.name,
        "key": fact.key,
        "value": json_from_str(&fact.value_json),
        "provenance_class": fact.provenance_class,
    })
}

fn effect_to_json(effect: &EffectView) -> Value {
    json!({
        "effect_id": effect.effect_id,
        "kind": effect.kind,
        "target": effect.target,
        "input": json_from_str(&effect.input_json),
        "status": effect.status,
        "created_by_rule": effect.created_by_rule,
        "profile": effect.profile,
        "required_capabilities": json_from_str(&effect.required_capabilities_json),
        "policy_block_reason": effect.policy_block_reason,
    })
}

fn run_to_json(run: &RunView) -> Value {
    json!({
        "run_id": run.run_id,
        "effect_id": run.effect_id,
        "provider": run.provider,
        "worker_id": run.worker_id,
        "status": run.status,
        "started_at": run.started_at,
        "completed_at": run.completed_at,
    })
}

fn inbox_item_to_json(item: &InboxItemView) -> Value {
    json!({
        "inbox_item_id": item.inbox_item_id,
        "instance_id": item.instance_id,
        "effect_id": item.effect_id,
        "status": item.status,
        "prompt": item.prompt,
        "choices": json_from_str(&item.choices_json),
        "freeform_allowed": item.freeform_allowed,
        "severity": item.severity,
        "related_effects": json_from_str(&item.related_effects_json),
        "related_artifacts": json_from_str(&item.related_artifacts_json),
        "answer": item.answer_json.as_deref().map(json_from_str),
        "answered_by": item.answered_by,
        "created_at": item.created_at,
        "answered_at": item.answered_at,
    })
}

fn evidence_to_json(evidence: &EvidenceView) -> Value {
    json!({
        "evidence_id": evidence.evidence_id,
        "instance_id": evidence.instance_id,
        "kind": evidence.kind,
        "subject_type": evidence.subject_type,
        "subject_id": evidence.subject_id,
        "causation_id": evidence.causation_id,
        "correlation_id": evidence.correlation_id,
        "summary": evidence.summary,
        "metadata": json_from_str(&evidence.metadata_json),
        "created_at": evidence.created_at,
    })
}

fn evidence_link_to_json(link: &EvidenceLinkView) -> Value {
    json!({
        "evidence_id": link.evidence_id,
        "target_type": link.target_type,
        "target_id": link.target_id,
        "relation": link.relation,
        "created_at": link.created_at,
    })
}

fn tool_check_to_json(tool: &ToolCheck) -> Value {
    json!({
        "id": tool.id,
        "category": tool.category,
        "command": tool.command,
        "required": tool.required,
        "available": tool.available,
        "path": tool.path,
        "note": tool.note,
    })
}

fn status_to_json(status: &StatusView) -> Value {
    json!({
        "instance": instance_to_json(&status.instance),
        "fact_count": status.fact_count,
        "queued_effect_count": status.queued_effect_count,
        "blocked_effect_count": status.blocked_effect_count,
        "active_run_count": status.active_run_count,
        "failure_count": status.failure_count,
        "recent_events": status.recent_events.iter().map(event_to_json).collect::<Vec<_>>(),
    })
}

fn json_from_str(source: &str) -> Value {
    serde_json::from_str(source).unwrap_or_else(|_| Value::String(source.to_owned()))
}

fn stable_hash_hex(value: &str) -> String {
    format!("{:016x}", stable_hash(value))
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn display_path(path: &str) -> String {
    Path::new(path).display().to_string()
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_diagnostic(path: &str, source: &str, diagnostic: &Diagnostic) -> String {
    let location = locate_span(source, diagnostic.span);
    let gutter_width = location.line.to_string().len();
    let underline = underline_for_span(&location, diagnostic.span);
    let mut rendered = String::new();

    rendered.push_str("error: ");
    rendered.push_str(&diagnostic.message);
    rendered.push('\n');
    rendered.push_str(&format!(
        "{:>width$}--> {}:{}:{}\n",
        "",
        display_path(path),
        location.line,
        location.column,
        width = gutter_width + 1
    ));
    rendered.push_str(&format!("{:>width$} |\n", "", width = gutter_width));
    rendered.push_str(&format!(
        "{:>width$} | {}\n",
        location.line,
        location.line_text,
        width = gutter_width
    ));
    rendered.push_str(&format!(
        "{:>width$} | {}{}\n",
        "",
        " ".repeat(location.column.saturating_sub(1)),
        underline,
        width = gutter_width
    ));

    if let Some(suggestion) = &diagnostic.suggestion {
        rendered.push_str(&format!(
            "{:>width$} = help: {suggestion}\n",
            "",
            width = gutter_width
        ));
    }

    rendered
}

#[derive(Debug, Eq, PartialEq)]
struct SourceLocation {
    line: usize,
    column: usize,
    line_start: usize,
    line_end: usize,
    line_text: String,
}

fn locate_span(source: &str, span: SourceSpan) -> SourceLocation {
    let (line, column) = line_column(source, span.start);
    let line_start = source[..span.start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[span.start..]
        .find('\n')
        .map(|offset| span.start + offset)
        .unwrap_or(source.len());
    let line_text = source[line_start..line_end].to_owned();

    SourceLocation {
        line,
        column,
        line_start,
        line_end,
        line_text,
    }
}

fn underline_for_span(location: &SourceLocation, span: SourceSpan) -> String {
    let underline_start = span.start.max(location.line_start);
    let underline_end = span.end.min(location.line_end).max(underline_start);
    let width = if underline_end == underline_start {
        1
    } else {
        location.line_text
            [underline_start - location.line_start..underline_end - location.line_start]
            .chars()
            .count()
            .max(1)
    };

    "^".repeat(width)
}

fn line_column(source: &str, byte_index: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;

    for (index, ch) in source.char_indices() {
        if index >= byte_index {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_source_span_diagnostic() {
        let source = "agent worker {\n  profile 42\n}\n";
        let diagnostic = Diagnostic {
            span: SourceSpan { start: 25, end: 27 },
            message: "expected profile string, found number literal".to_owned(),
            suggestion: Some("write `profile \"profile-name\"`".to_owned()),
        };

        let expected = concat!(
            "error: expected profile string, found number literal\n",
            "  --> example.whip:2:11\n",
            "  |\n",
            "2 |   profile 42\n",
            "  |           ^^\n",
            "  = help: write `profile \"profile-name\"`\n",
        );

        assert_eq!(
            render_diagnostic("example.whip", source, &diagnostic),
            expected
        );
    }

    #[test]
    fn parses_global_cli_options() {
        let options = CliOptions::parse(vec![
            "--store".to_owned(),
            "state.sqlite".to_owned(),
            "--json".to_owned(),
            "status".to_owned(),
            "ins_1".to_owned(),
        ])
        .expect("options parse");

        assert_eq!(options.command.as_deref(), Some("status"));
        assert_eq!(options.args, vec!["ins_1"]);
        assert_eq!(options.store_path, PathBuf::from("state.sqlite"));
        assert!(options.json);
    }

    #[test]
    fn parses_check_model_search_option() {
        let options = CheckOptions::parse(&[
            "--model-search".to_owned(),
            "examples/ralph.whip".to_owned(),
        ])
        .expect("check options parse");

        assert!(options.model_search);
        assert_eq!(options.paths, vec!["examples/ralph.whip"]);
    }

    #[test]
    fn parses_trace_check_option() {
        let options =
            TraceOptions::parse(&["ins_123".to_owned(), "--check".to_owned()]).expect("parse");

        assert_eq!(options.instance_id, "ins_123");
        assert!(options.check);
    }

    #[test]
    fn reconstructs_trace_records_from_store_events() {
        let events = vec![
            event_view(
                1,
                "rule.committed",
                json!({
                    "rule": "dispatch",
                    "facts": [],
                    "effects": [
                        {"effect_id": "prepare", "status": "queued"},
                        {"effect_id": "send", "status": "queued"}
                    ],
                    "dependencies": [
                        {
                            "dependency_id": "dep_1",
                            "upstream_effect_id": "prepare",
                            "downstream_effect_id": "send",
                            "predicate": "succeeds"
                        }
                    ]
                }),
            ),
            event_view(
                2,
                "effect.run_started",
                json!({"effect_id": "prepare", "run_id": "run_prepare"}),
            ),
            event_view(
                3,
                "effect.terminal",
                json!({
                    "effect_id": "prepare",
                    "run_id": "run_prepare",
                    "status": "completed"
                }),
            ),
            event_view(
                4,
                "effect.run_started",
                json!({"effect_id": "send", "run_id": "run_send"}),
            ),
            event_view(
                5,
                "effect.terminal",
                json!({
                    "effect_id": "send",
                    "run_id": "run_send",
                    "status": "completed"
                }),
            ),
        ];

        let records = reconstruct_trace_records(&events);

        assert_eq!(records.len(), 9);
        check_trace(&records).expect("reconstructed trace conforms");
    }

    #[test]
    fn generates_model_searches_for_effect_dependencies() {
        let source = include_str!("../../../examples/loft-worker-with-review.whip");
        let compiled = whippletree_parser::compile_program(source);
        let ir = compiled.ir.expect("example compiles");
        let (_maude, expected) =
            generate_maude_model_search(source, &ir, Path::new("/tmp/kernel.maude"));

        assert_eq!(expected.len(), 6);
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.outcome == ExpectedSearchResult::Solution)
                .count(),
            2
        );
    }

    #[test]
    fn generated_model_search_detects_unsafe_dependency_release_fixture() {
        if find_executable_in_path(&["maude"], &path_value()).is_none() {
            return;
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let kernel_path =
            fs::canonicalize(root.join("models/maude/kernel.maude")).expect("kernel path resolves");
        let source = include_str!("../../../examples/loft-worker-with-review.whip");
        let compiled = whippletree_parser::compile_program(source);
        let ir = compiled.ir.expect("example compiles");
        let (maude, expected) = generate_maude_model_search(source, &ir, &kernel_path);
        assert!(!expected.is_empty());

        let module_end = maude
            .find("endm\n\n")
            .expect("generated module has an end marker");
        let unsafe_rule = concat!(
            "  vars U D : EffectId .\n",
            "  rl [unsafe-generated-fixture-release] :\n",
            "    effect(U, queued) dep(U, succeeds, D) effect(D, blocked)\n",
            "    => effect(U, queued) dep(U, succeeds, D) effect(D, queued) .\n",
        );
        let unsafe_maude = format!(
            "{}{}{}",
            &maude[..module_end],
            unsafe_rule,
            &maude[module_end..]
        );

        let output = run_maude_source("unsafe-generated-check-fixture", &unsafe_maude)
            .expect("unsafe generated Maude fixture runs");
        let actual = extract_maude_search_results(&output.stdout);
        assert_eq!(actual.len(), expected.len(), "{}", output.stdout);
        assert!(
            expected
                .iter()
                .zip(actual.iter())
                .any(|(expected, actual)| {
                    expected.description.contains("cannot run before")
                        && expected.outcome == ExpectedSearchResult::NoSolution
                        && *actual == ExpectedSearchResult::Solution
                }),
            "unsafe fixture did not produce a generated-check counterexample\n{}",
            output.stdout
        );
    }

    #[test]
    fn extracts_maude_search_results_in_order() {
        let output = concat!(
            "search 1\nNo solution.\n",
            "search 2\nSolution 1 (state 1)\n",
            "search 3\nNo solution.\n",
        );

        assert_eq!(
            extract_maude_search_results(output),
            vec![
                ExpectedSearchResult::NoSolution,
                ExpectedSearchResult::Solution,
                ExpectedSearchResult::NoSolution,
            ]
        );
    }

    #[test]
    fn locates_dependency_source_span() {
        let source =
            "rule work {\n  after prepare succeeds {\n    agent.tell \"send\" as notify\n  }\n}\n";
        let span = dependency_source_span(source, "prepare", "succeeds");

        assert_eq!(&source[span.start..span.end], "after prepare succeeds");
    }

    #[test]
    fn finds_first_matching_tool_on_path() {
        let directory =
            std::env::temp_dir().join(format!("whippletree-doctor-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("temp directory creates");
        let executable = directory.join("tool-b");
        fs::write(&executable, "").expect("tool file creates");

        let found = find_executable_in_path(
            &["tool-a", "tool-b"],
            directory.to_str().expect("path is utf-8"),
        );

        assert_eq!(found, Some(executable.display().to_string()));
        let _ = fs::remove_dir_all(directory);
    }

    fn event_view(sequence: i64, event_type: &str, payload: Value) -> EventView {
        EventView {
            event_id: format!("evt_{sequence}"),
            sequence,
            event_type: event_type.to_owned(),
            payload_json: payload.to_string(),
            source: "kernel".to_owned(),
            occurred_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }
}
