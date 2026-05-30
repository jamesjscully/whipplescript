use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use serde_json::{json, Value};
use whipplescript_kernel::{
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
use whipplescript_parser::{
    parse_duration_seconds, parse_expression, parse_time_epoch_seconds, BinaryOp,
    DependencyPredicate as IrDependencyPredicate, Diagnostic, Expr, ExprLiteral, ExprObjectField,
    IrEffectNode, IrInclude, IrPrimitiveType, IrProgram, IrRule, IrSchema, IrType,
    IrWorkflowContract, IrWorkflowContractKind, Item, QueryKind, SourceSpan, UnaryOp,
};
use whipplescript_store::{
    ClaimableEffect, DiagnosticRecord, DiagnosticView, EffectCancellation, EffectCompletion,
    EffectView, EventView, EvidenceLinkView, EvidenceView, FactView, HumanAnswer, InboxItemView,
    InstanceView, NewEffect, NewEffectDependency, NewFact, NewWorkflowInvocation, RetryEffect,
    RuleCommit, RunStart, RunView, SqliteStore, StatusView, StoreError, WorkflowInvocationView,
    WorkflowRevisionView, WorkflowTerminal, WorkflowTerminalKind,
};

fn main() -> ExitCode {
    let raw_args = env::args().skip(1).collect::<Vec<_>>();
    if matches!(
        raw_args.first().map(String::as_str),
        Some("--version" | "-V")
    ) {
        println!("whipplescript {}", whipplescript_core::version());
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
        Some("diagnostics") => diagnostics(&options),
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
        let mut store_path = env::var("WHIPPLESCRIPT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(".whipplescript/store.sqlite"));
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
    println!("whipplescript {}", whipplescript_core::IMPLEMENTATION_STAGE);
    println!("usage: whip [--store path] [--json] <command> [args]");
    println!("commands: check, compile, run, step, worker, dev, instances, status, log, facts, effects, runs");
    println!("          inbox, evidence, diagnostics, trace, pause, resume, cancel, retry, doctor");
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
            "stage": whipplescript_kernel::kernel_stage(),
            "store": store_status,
            "tools": tools.iter().map(tool_check_to_json).collect::<Vec<_>>(),
        }))
    } else {
        println!("whip doctor: {}", whipplescript_kernel::kernel_stage());
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
        match compile_source_path_with_root(&path, check_options.root.as_deref()) {
            Ok((source, ir)) => {
                println!("== {}", display_path(&path));
                print!("{}", ir.to_snapshot());
                if check_options.model_search {
                    match run_model_search(&path, &source, &ir) {
                        Ok(report) if report.searches == 0 => {
                            println!("model search: no generated checks");
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
    root: Option<String>,
    paths: Vec<String>,
}

impl CheckOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut model_search = false;
        let mut root = None;
        let mut paths = Vec::new();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--model-search" => model_search = true,
                "--root" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected workflow name after `--root`".to_owned());
                    };
                    root = Some(value.clone());
                }
                "--" => {}
                arg if arg.starts_with('-') => {
                    return Err(format!("unknown check option `{arg}`"));
                }
                arg => paths.push(arg.to_owned()),
            }
            index += 1;
        }
        Ok(Self {
            model_search,
            root,
            paths,
        })
    }
}

fn compile(options: &CliOptions) -> ExitCode {
    let compile_options = match CompileOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let (source, ir) = match compile_source_path_with_root(
        &compile_options.program_path,
        compile_options.root.as_deref(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => return report_compile_failure(&compile_options.program_path, error),
    };
    let snapshot = ir.to_snapshot();

    if options.json {
        emit_json(json!({
            "path": display_path(&compile_options.program_path),
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompileOptions {
    program_path: String,
    root: Option<String>,
}

impl CompileOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut program_path = None;
        let mut root = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--root" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected workflow name after `--root`".to_owned());
                    };
                    root = Some(value.clone());
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown compile option `{other}`"));
                }
                value if program_path.is_none() => program_path = Some(value.to_owned()),
                _ => return Err("usage: whip compile <workflow.whip> [--root Workflow]".to_owned()),
            }
            index += 1;
        }
        let Some(program_path) = program_path else {
            return Err("usage: whip compile <workflow.whip> [--root Workflow]".to_owned());
        };
        Ok(Self { program_path, root })
    }
}

fn run(options: &CliOptions) -> ExitCode {
    let run_options = match RunOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let started = match start_workflow_instance(
        &run_options.program_path,
        run_options.root.as_deref(),
        options.input_json.as_deref(),
        options,
    ) {
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
struct RunOptions {
    program_path: String,
    root: Option<String>,
}

impl RunOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut program_path = None;
        let mut root = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--root" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected workflow name after `--root`".to_owned());
                    };
                    root = Some(value.clone());
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown run option `{other}`"));
                }
                value if program_path.is_none() => program_path = Some(value.to_owned()),
                _ => {
                    return Err(
                        "usage: whip run <workflow.whip> [--root Workflow] [--input JSON]"
                            .to_owned(),
                    )
                }
            }
            index += 1;
        }
        let Some(program_path) = program_path else {
            return Err(
                "usage: whip run <workflow.whip> [--root Workflow] [--input JSON]".to_owned(),
            );
        };
        Ok(Self { program_path, root })
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
    root: Option<&str>,
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
    let (source, ir) = match compile_source_path_with_root(path, root) {
        Ok(compiled) => compiled,
        Err(error) => return Err(report_compile_failure(path, error)),
    };
    let input_facts = match validate_workflow_start_input(&ir, &input_value) {
        Ok(facts) => facts,
        Err(message) => {
            eprintln!("{message}");
            return Err(ExitCode::from(2));
        }
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
    let version = match kernel.create_program_version_for_program(
        ProgramVersionInput {
            program_name: &ir.workflow,
            source_hash: &stable_hash_hex(&source),
            ir_hash: &stable_hash_hex(&snapshot),
            compiler_version: whipplescript_core::version(),
        },
        &ir,
    ) {
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
    let started_event = match kernel.ingest_external_event(
        &instance_id,
        "external.started",
        &input_json,
        Some(&idempotency_key(&[&instance_id, "external.started"])),
    ) {
        Ok(event) => event,
        Err(error) => {
            eprintln!("failed to write start event: {}", store_error(error));
            return Err(ExitCode::FAILURE);
        }
    };
    for fact in input_facts {
        if let Err(error) = kernel.derive_fact(
            &instance_id,
            &fact.name,
            &fact.key,
            &fact.value_json,
            Some(&started_event.event_id),
            Some(&idempotency_key(&[
                &instance_id,
                "workflow.input",
                &fact.key,
                &fact.name,
            ])),
        ) {
            eprintln!("failed to seed workflow input: {}", store_error(error));
            return Err(ExitCode::FAILURE);
        }
    }
    Ok(StartedWorkflow {
        instance_id,
        program_id: version.program_id,
        version_id: version.version_id,
        workflow: ir.workflow,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkflowInputFact {
    name: String,
    key: String,
    value_json: String,
}

fn validate_workflow_start_input(
    ir: &IrProgram,
    input: &Value,
) -> Result<Vec<WorkflowInputFact>, String> {
    let contracts = ir
        .workflow_contracts
        .iter()
        .filter(|contract| contract.kind == IrWorkflowContractKind::Input)
        .collect::<Vec<_>>();
    if contracts.is_empty() {
        return Ok(Vec::new());
    }
    let Some(object) = input.as_object() else {
        return Err(format!(
            "workflow `{}` expects input object keyed by declared input names",
            ir.workflow
        ));
    };

    let mut errors = Vec::new();
    let contracts_by_name = contracts
        .iter()
        .map(|contract| (contract.name.as_str(), *contract))
        .collect::<BTreeMap<_, _>>();
    for key in object.keys() {
        if !contracts_by_name.contains_key(key.as_str()) {
            errors.push(format!("unexpected workflow input `{key}`"));
        }
    }

    let mut facts = Vec::new();
    for contract in contracts {
        let Some(value) = object.get(&contract.name) else {
            errors.push(format!("missing workflow input `{}`", contract.name));
            continue;
        };
        validate_json_for_ir_type(ir, value, &contract.ty, &contract.name, &mut errors);
        facts.push(WorkflowInputFact {
            name: workflow_input_fact_name(contract),
            key: contract.name.clone(),
            value_json: value.to_string(),
        });
    }

    if errors.is_empty() {
        Ok(facts)
    } else {
        Err(format!(
            "invalid workflow input for `{}`: {}",
            ir.workflow,
            errors.join("; ")
        ))
    }
}

fn workflow_input_fact_name(contract: &IrWorkflowContract) -> String {
    match &contract.ty {
        IrType::Ref(name) => name.clone(),
        other => ir_type_name(other),
    }
}

fn validate_json_for_ir_type(
    ir: &IrProgram,
    value: &Value,
    ty: &IrType,
    path: &str,
    errors: &mut Vec<String>,
) {
    match ty {
        IrType::Primitive(primitive) => validate_json_for_primitive(value, primitive, path, errors),
        IrType::LiteralString(expected) => {
            if value.as_str() != Some(expected.as_str()) {
                errors.push(format!("{path} must be literal {expected:?}"));
            }
        }
        IrType::Ref(name) => validate_json_for_ref(ir, value, name, path, errors),
        IrType::AgentRef(agents) => match value.as_str() {
            Some(agent) if agents.iter().any(|candidate| candidate == agent) => {}
            Some(_) => errors.push(format!(
                "{path} must name one of these agents: {}",
                agents.join(", ")
            )),
            None => errors.push(format!("{path} must be an agent name string")),
        },
        IrType::Object(fields) => validate_json_for_object(ir, value, fields, path, errors),
        IrType::Optional(inner) => {
            if !value.is_null() {
                validate_json_for_ir_type(ir, value, inner, path, errors);
            }
        }
        IrType::Array(inner) => match value.as_array() {
            Some(items) => {
                for (index, item) in items.iter().enumerate() {
                    validate_json_for_ir_type(ir, item, inner, &format!("{path}[{index}]"), errors);
                }
            }
            None => errors.push(format!("{path} must be an array")),
        },
        IrType::Map(inner) => match value.as_object() {
            Some(map) => {
                for (key, item) in map {
                    validate_json_for_ir_type(ir, item, inner, &format!("{path}.{key}"), errors);
                }
            }
            None => errors.push(format!("{path} must be an object map")),
        },
        IrType::Union(types) => {
            if !types
                .iter()
                .any(|candidate| json_matches_ir_type(ir, value, candidate))
            {
                errors.push(format!(
                    "{path} must match one of: {}",
                    types
                        .iter()
                        .map(ir_type_name)
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
            }
        }
    }
}

fn validate_json_for_primitive(
    value: &Value,
    primitive: &IrPrimitiveType,
    path: &str,
    errors: &mut Vec<String>,
) {
    let valid = match primitive {
        IrPrimitiveType::String
        | IrPrimitiveType::Duration
        | IrPrimitiveType::Time
        | IrPrimitiveType::Image
        | IrPrimitiveType::Audio
        | IrPrimitiveType::Pdf
        | IrPrimitiveType::Video => value.is_string(),
        IrPrimitiveType::Int => value.as_i64().is_some(),
        IrPrimitiveType::Float => value.as_f64().is_some(),
        IrPrimitiveType::Bool => value.is_boolean(),
        IrPrimitiveType::Null => value.is_null(),
    };
    if !valid {
        errors.push(format!(
            "{path} must be {}",
            ir_type_name(&IrType::Primitive(primitive.clone()))
        ));
    }
}

fn validate_json_for_ref(
    ir: &IrProgram,
    value: &Value,
    name: &str,
    path: &str,
    errors: &mut Vec<String>,
) {
    if let Some(class) = ir.schemas.iter().find_map(|schema| match schema {
        IrSchema::Class(class) if class.name == name => Some(class),
        _ => None,
    }) {
        validate_json_for_object(ir, value, &class.fields, path, errors);
        return;
    }
    if let Some(enum_decl) = ir.schemas.iter().find_map(|schema| match schema {
        IrSchema::Enum(enum_decl) if enum_decl.name == name => Some(enum_decl),
        _ => None,
    }) {
        match value.as_str() {
            Some(variant)
                if enum_decl
                    .variants
                    .iter()
                    .any(|candidate| candidate == variant) => {}
            Some(_) => errors.push(format!(
                "{path} must be one of: {}",
                enum_decl.variants.join(", ")
            )),
            None => errors.push(format!("{path} must be a string enum variant")),
        }
        return;
    }
    errors.push(format!("{path} references unknown type `{name}`"));
}

fn validate_json_for_object(
    ir: &IrProgram,
    value: &Value,
    fields: &[whipplescript_parser::IrClassField],
    path: &str,
    errors: &mut Vec<String>,
) {
    let Some(object) = value.as_object() else {
        errors.push(format!("{path} must be an object"));
        return;
    };
    for key in object.keys() {
        if !fields.iter().any(|field| field.name == *key) {
            errors.push(format!("{path}.{key} is not declared"));
        }
    }
    for field in fields {
        let field_path = format!("{path}.{}", field.name);
        match object.get(&field.name) {
            Some(value) => validate_json_for_ir_type(ir, value, &field.ty, &field_path, errors),
            None if matches!(field.ty, IrType::Optional(_)) => {}
            None => errors.push(format!("{field_path} is required")),
        }
    }
}

fn json_matches_ir_type(ir: &IrProgram, value: &Value, ty: &IrType) -> bool {
    let mut errors = Vec::new();
    validate_json_for_ir_type(ir, value, ty, "$", &mut errors);
    errors.is_empty()
}

fn step(options: &CliOptions) -> ExitCode {
    let step_options = match StepOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let (_source, ir) = match compile_source_path_with_root(
        &step_options.program_path,
        step_options.root.as_deref(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => return report_compile_failure(&step_options.program_path, error),
    };
    match step_instance(
        &options.store_path,
        &step_options.instance_id,
        &ir,
        Some(Path::new(&step_options.program_path)),
    ) {
        Ok(report) if options.json => emit_json(step_report_to_json(&report)),
        Ok(report) => {
            println!(
                "step {} committed_rules={} facts={} consumed={} effects={}",
                report.instance_id,
                report.committed_rules,
                report.facts_created,
                report.facts_consumed,
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
    root: Option<String>,
}

impl StepOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut instance_id = None;
        let mut program_path = None;
        let mut root = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--root" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected workflow name after `--root`".to_owned());
                    };
                    root = Some(value.clone());
                }
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
            root,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct StepReport {
    instance_id: String,
    committed_rules: usize,
    facts_created: usize,
    facts_consumed: usize,
    effects_created: usize,
    guard_reports: Vec<GuardReport>,
    branch_reports: Vec<BranchReport>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GuardReport {
    rule: String,
    when: String,
    expr: String,
    source_span_json: Option<String>,
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
struct BranchReport {
    scrutinee: String,
    status: BranchStatus,
    matched: bool,
    tag: Option<String>,
    actual: Value,
    error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BranchStatus {
    Matched,
    NoMatch,
    Error,
}

impl BranchStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::NoMatch => "no_match",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AssertionReport {
    expr: String,
    source_span_json: Option<String>,
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
    source_path: Option<&Path>,
) -> Result<StepReport, StoreError> {
    let mut report = StepReport {
        instance_id: instance_id.to_owned(),
        ..StepReport::default()
    };
    let mut made_progress = true;
    while made_progress {
        made_progress = false;
        let store = SqliteStore::open(store_path)?;
        if let Some(status) = store.status(instance_id)? {
            if status.instance.status != "running" {
                break;
            }
        }
        let events = store.list_events(instance_id)?;
        let facts = store.list_facts(instance_id)?;
        let all_facts = store.list_facts_including_consumed(instance_id)?;
        let effects = store.list_effects(instance_id)?;
        let started_event_id = events
            .iter()
            .find(|event| event.event_type == "external.started")
            .map(|event| event.event_id.clone());

        'rules: for rule in &ir.rules {
            let ready = ready_contexts(ir, rule, &facts, &effects, started_event_id.as_deref());
            report.guard_reports.extend(ready.guard_reports);
            for context in ready.contexts {
                let lowering = lower_rule(ir, rule, &context, &all_facts, &effects, source_path);
                report
                    .branch_reports
                    .extend(lowering.branch_reports.iter().cloned());
                if lowering.facts.is_empty()
                    && lowering.consumed_fact_ids.is_empty()
                    && lowering.effects.is_empty()
                    && lowering.dependencies.is_empty()
                    && lowering.terminal.is_none()
                {
                    continue;
                }
                let consumed_fact_ids = lowering
                    .consumed_fact_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>();
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
                let terminal = lowering
                    .terminal
                    .as_ref()
                    .map(OwnedWorkflowTerminal::as_workflow_terminal);
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
                    consumed_fact_ids: &consumed_fact_ids,
                    effects: &new_effects,
                    dependencies: &new_dependencies,
                    terminal,
                    idempotency_key: Some(&commit_key),
                });
                store = kernel.into_store();
                drop(store);
                match event {
                    Ok(_) => {
                        report.committed_rules += 1;
                        report.facts_created += new_facts.len();
                        report.facts_consumed += consumed_fact_ids.len();
                        report.effects_created += new_effects.len();
                        made_progress = true;
                        break 'rules;
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
    outcome: FixtureOutcome,
    program_path: Option<PathBuf>,
    root: Option<String>,
    max_child_iterations: usize,
}

impl WorkerOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut instance_id = None;
        let mut provider = "fixture".to_owned();
        let mut outcome = FixtureOutcome::Completed;
        let mut program_path = None;
        let mut root = None;
        let mut max_child_iterations = 8usize;
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
                "--program" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected path after `--program`".to_owned());
                    };
                    program_path = Some(PathBuf::from(value));
                }
                "--root" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected workflow name after `--root`".to_owned());
                    };
                    root = Some(value.clone());
                }
                "--max-child-iterations" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err(
                            "expected number after `--max-child-iterations`".to_owned()
                        );
                    };
                    max_child_iterations = value
                        .parse()
                        .map_err(|_| "`--max-child-iterations` must be a number".to_owned())?;
                }
                "--fail" => outcome = FixtureOutcome::Failed,
                "--timeout" => outcome = FixtureOutcome::TimedOut,
                "--cancel" => outcome = FixtureOutcome::Cancelled,
                "--once" => {}
                other if other.starts_with('-') => {
                    return Err(format!("unknown worker option `{other}`"));
                }
                value if instance_id.is_none() => instance_id = Some(value.to_owned()),
                _ => {
                    return Err(
                        "usage: whip worker <instance> [--provider fixture] [--once] [--fail|--timeout|--cancel]".to_owned(),
                    )
                }
            }
            index += 1;
        }
        let Some(instance_id) = instance_id else {
            return Err(
                "usage: whip worker <instance> [--provider fixture] [--once] [--fail|--timeout|--cancel]"
                    .to_owned(),
            );
        };
        Ok(Self {
            instance_id,
            provider,
            outcome,
            program_path,
            root,
            max_child_iterations,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FixtureOutcome {
    #[default]
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

impl FixtureOutcome {
    fn is_failed(self) -> bool {
        self == Self::Failed
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
            "event.emit" => run_event_effect(store_path, &options.instance_id, &effect, options)?,
            "workflow.invoke" => {
                run_workflow_invoke_effect(store_path, &options.instance_id, &effect, options)?
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
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    let input_json = resolve_effect_input_after_bindings(store_path, instance_id, effect)?;
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "lease"]);
    let harness = fixture_harness(options.outcome.is_failed());
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
) -> Result<whipplescript_store::StoredEvent, StoreError> {
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
    if options.outcome == FixtureOutcome::Cancelled {
        return cancel_baml_effect(store_path, instance_id, effect, &input, options);
    }
    let client = match options.outcome {
        FixtureOutcome::Completed => FakeBamlClient::succeeds(value),
        FixtureOutcome::Failed => FakeBamlClient::fails("fixture coerce failure"),
        FixtureOutcome::TimedOut => FakeBamlClient::times_out("fixture coerce timeout"),
        FixtureOutcome::Cancelled => unreachable!("cancelled handled above"),
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

fn cancel_baml_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    input: &Value,
    options: &WorkerOptions,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "baml-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "baml-lease"]);
    kernel.start_run(RunStart {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: &options.provider,
        worker_id: "whip-worker",
        lease_id: &lease_id,
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: "{}",
    })?;
    let terminal = kernel.cancel_effect(EffectCancellation {
        instance_id,
        effect_id: &effect.effect_id,
        reason: Some("fixture coerce cancelled"),
        idempotency_key: Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "baml-cancelled",
        ])),
    })?;
    let value_json = json!({
        "effect_id": effect.effect_id,
        "run_id": run_id,
        "function_name": input.get("function_name").cloned().unwrap_or(Value::Null),
        "status": "cancelled",
        "value": null,
        "error": {
            "reason": "fixture coerce cancelled",
            "recoverable": true
        },
        "summary": "fixture coerce cancelled"
    })
    .to_string();
    kernel.derive_fact(
        instance_id,
        "baml.coerce.cancelled",
        &effect.effect_id,
        &value_json,
        Some(&terminal.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "baml.coerce.cancelled",
        ])),
    )?;
    Ok(terminal)
}

fn run_loft_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
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
    let client = if options.outcome.is_failed() {
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
) -> Result<whipplescript_store::StoredEvent, StoreError> {
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
) -> Result<whipplescript_store::StoredEvent, StoreError> {
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

    let terminal = if options.outcome.is_failed() {
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

fn run_event_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    let input = json_from_str(&effect.input_json);
    let event_type = input
        .get("event_type")
        .and_then(Value::as_str)
        .or(effect.target.as_deref())
        .unwrap_or("event.emitted");
    let payload = input
        .get("payload")
        .cloned()
        .unwrap_or_else(|| json!({"effect_id": effect.effect_id, "event_type": event_type}));
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "event-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "event-lease"]);
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
            "event_type": event_type,
            "input": input,
        })
        .to_string(),
    })?;

    let emitted = kernel.ingest_external_event(
        instance_id,
        event_type,
        &payload.to_string(),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            event_type,
            "event.emit",
        ])),
    )?;
    let metadata_json = json!({
        "event_type": event_type,
        "event_id": emitted.event_id,
        "input": input,
        "value": payload,
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
        summary: Some("fixture event emitted"),
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
        "event_id": emitted.event_id,
        "event_type": event_type,
        "status": "completed",
        "value": payload,
        "summary": "fixture event emitted",
    })
    .to_string();
    kernel.derive_fact(
        instance_id,
        "event.emit.succeeded",
        &effect.effect_id,
        &value_json,
        Some(&terminal.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "event.emit.succeeded",
        ])),
    )?;
    kernel.derive_fact(
        instance_id,
        event_type,
        &effect.effect_id,
        &value_json,
        Some(&emitted.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            event_type,
            "fact",
        ])),
    )?;
    Ok(terminal)
}

fn run_workflow_invoke_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    let input_json = resolve_effect_input_after_bindings(store_path, instance_id, effect)?;
    let input = json_from_str(&input_json);
    let target_workflow = input
        .get("target_workflow")
        .and_then(Value::as_str)
        .or(effect.target.as_deref())
        .unwrap_or("workflow");
    let child_input = input.get("input").cloned().unwrap_or_else(|| json!({}));
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "workflow-invoke-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "workflow-invoke-lease"]);
    let Some(program_path) = options.program_path.as_deref() else {
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
            metadata_json: &json!({"input": input, "error": "missing program path"}).to_string(),
        })?;
        return kernel.fail_run(EffectCompletion {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            status: "failed",
            exit_code: Some(2),
            summary: Some("workflow invocation requires --program"),
            metadata_json: &json!({"input": input}).to_string(),
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "terminal",
            ])),
        });
    };

    let invocation_store = SqliteStore::open(store_path)?;
    let existing_invocation =
        invocation_store.get_workflow_invocation(instance_id, &effect.effect_id)?;
    let (child_instance_id, child_ir, start_event) = match existing_invocation {
        Some(invocation) => {
            let (_source, child_ir) = compile_source_path_with_root(
                program_path.to_str().unwrap_or_default(),
                Some(&invocation.target_workflow),
            )
            .map_err(|error| {
                StoreError::Conflict(child_compile_error(&invocation.target_workflow, error))
            })?;
            (invocation.child_instance_id, child_ir, None)
        }
        None => {
            let (child_started, child_ir) = start_child_workflow_instance(
                store_path,
                program_path,
                target_workflow,
                &child_input.to_string(),
            )?;
            let child_instance_id = child_started.instance_id.clone();
            let invocation_id = idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "invokes",
                &child_instance_id,
            ]);
            let invocation_key =
                idempotency_key(&[instance_id, &effect.effect_id, target_workflow]);
            let source_span_json =
                invocation_store.effect_source_span_json(instance_id, &effect.effect_id)?;
            invocation_store.record_workflow_invocation(NewWorkflowInvocation {
                invocation_id: &invocation_id,
                parent_instance_id: instance_id,
                parent_effect_id: &effect.effect_id,
                child_instance_id: &child_instance_id,
                target_workflow,
                input_json: &child_input.to_string(),
                source_span_json: source_span_json.as_deref(),
                idempotency_key: &invocation_key,
            })?;
            let store = SqliteStore::open(store_path)?;
            let mut kernel = RuntimeKernel::new(store);
            let started_run = kernel.start_run(RunStart {
                instance_id,
                effect_id: &effect.effect_id,
                run_id: &run_id,
                provider: &options.provider,
                worker_id: "whip-worker",
                lease_id: &lease_id,
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: &json!({
                    "input": input,
                    "child_instance_id": child_instance_id,
                })
                .to_string(),
            })?;
            (child_instance_id, child_ir, Some(started_run))
        }
    };

    if options.max_child_iterations == 0 {
        if let Some(event) = start_event {
            return Ok(event);
        }
    }

    for _ in 0..options.max_child_iterations {
        let step_report = step_instance(
            store_path,
            &child_instance_id,
            &child_ir,
            Some(program_path),
        )?;
        let child_worker = WorkerOptions {
            instance_id: child_instance_id.clone(),
            provider: options.provider.clone(),
            outcome: options.outcome,
            program_path: Some(program_path.to_path_buf()),
            root: Some(target_workflow.to_owned()),
            max_child_iterations: options.max_child_iterations,
        };
        let worker_report = run_worker_once(store_path, &child_worker)?;
        let child = SqliteStore::open(store_path)?
            .get_instance(&child_instance_id)?
            .ok_or_else(|| {
                StoreError::Conflict("child workflow instance disappeared".to_owned())
            })?;
        if child.status != "running" {
            break;
        }
        if step_report.committed_rules == 0 && worker_report.ran_effects == 0 {
            break;
        }
    }

    let store = SqliteStore::open(store_path)?;
    let child = store
        .get_instance(&child_instance_id)?
        .ok_or_else(|| StoreError::Conflict("child workflow instance disappeared".to_owned()))?;
    let terminal = workflow_terminal_summary_from_store(&store, &child_instance_id)?;
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);

    let (fact_name, status, value, summary) = match terminal {
        Some(terminal) if terminal.status == "completed" => (
            "workflow.invoke.succeeded",
            "completed",
            terminal.payload,
            "child workflow completed",
        ),
        Some(terminal) if terminal.status == "failed" => (
            "workflow.invoke.failed",
            "failed",
            terminal.payload,
            "child workflow failed",
        ),
        _ if child.status == "cancelled" => (
            "workflow.invoke.cancelled",
            "cancelled",
            json!({
                "reason": "child workflow was cancelled",
            }),
            "child workflow cancelled",
        ),
        _ => (
            "workflow.invoke.failed",
            "timed_out",
            json!({
                "reason": format!("child workflow did not reach terminal state: {}", child.status),
            }),
            "child workflow timed out",
        ),
    };
    let metadata_json = json!({
        "input": input,
        "child_instance_id": child_instance_id,
        "child_status": child.status,
        "value": value,
    })
    .to_string();
    let terminal_event = match status {
        "completed" => kernel.complete_run(EffectCompletion {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            status,
            exit_code: Some(0),
            summary: Some(summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "terminal",
            ])),
        })?,
        "failed" => kernel.fail_run(EffectCompletion {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            status,
            exit_code: Some(0),
            summary: Some(summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "terminal",
            ])),
        })?,
        "timed_out" => kernel.timeout_run(EffectCompletion {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            status,
            exit_code: Some(0),
            summary: Some(summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "terminal",
            ])),
        })?,
        "cancelled" => kernel.cancel_run(EffectCompletion {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            status,
            exit_code: Some(0),
            summary: Some(summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "terminal",
            ])),
        })?,
        _ => kernel.fail_run(EffectCompletion {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &options.provider,
            worker_id: "whip-worker",
            status,
            exit_code: Some(0),
            summary: Some(summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "terminal",
            ])),
        })?,
    };
    let value_json = json!({
        "effect_id": effect.effect_id,
        "run_id": run_id,
        "child_instance_id": child_instance_id,
        "target_workflow": target_workflow,
        "status": status,
        "value": value,
        "output": value,
        "summary": summary,
    })
    .to_string();
    kernel.derive_fact(
        instance_id,
        fact_name,
        &effect.effect_id,
        &value_json,
        Some(&terminal_event.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            fact_name,
        ])),
    )?;
    kernel.derive_fact(
        instance_id,
        "workflow.invoke.completed",
        &effect.effect_id,
        &value_json,
        Some(&terminal_event.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "workflow.invoke.completed",
        ])),
    )?;
    Ok(terminal_event)
}

fn start_child_workflow_instance(
    store_path: &Path,
    program_path: &Path,
    root: &str,
    input_json: &str,
) -> Result<(StartedWorkflow, IrProgram), StoreError> {
    let input_value = serde_json::from_str::<Value>(input_json)?;
    let input_json = input_value.to_string();
    let (source, ir) =
        compile_source_path_with_root(program_path.to_str().unwrap_or_default(), Some(root))
            .map_err(|error| StoreError::Conflict(child_compile_error(root, error)))?;
    let input_facts = validate_workflow_start_input(&ir, &input_value).map_err(|message| {
        StoreError::Conflict(format!("invalid child workflow input: {message}"))
    })?;
    let snapshot = ir.to_snapshot();
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    let version = kernel.create_program_version_for_program(
        ProgramVersionInput {
            program_name: &ir.workflow,
            source_hash: &stable_hash_hex(&source),
            ir_hash: &stable_hash_hex(&snapshot),
            compiler_version: whipplescript_core::version(),
        },
        &ir,
    )?;
    let instance_id = kernel.create_instance(&version, &input_json)?;
    let started_event = kernel.ingest_external_event(
        &instance_id,
        "external.started",
        &input_json,
        Some(&idempotency_key(&[&instance_id, "external.started"])),
    )?;
    for fact in input_facts {
        kernel.derive_fact(
            &instance_id,
            &fact.name,
            &fact.key,
            &fact.value_json,
            Some(&started_event.event_id),
            Some(&idempotency_key(&[
                &instance_id,
                "workflow.input",
                &fact.key,
                &fact.name,
            ])),
        )?;
    }
    Ok((
        StartedWorkflow {
            instance_id,
            program_id: version.program_id,
            version_id: version.version_id,
            workflow: ir.workflow.clone(),
        },
        ir,
    ))
}

fn child_compile_error(root: &str, error: CompileFailure) -> String {
    match error {
        CompileFailure::Io(error) => {
            format!("failed to read child workflow source for `{root}`: {error}")
        }
        CompileFailure::Diagnostics { diagnostics, .. } => {
            let messages = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            format!("failed to compile child workflow `{root}`: {messages}")
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct WorkflowTerminalSummary {
    status: String,
    payload: Value,
}

fn workflow_terminal_summary_from_store(
    store: &SqliteStore,
    instance_id: &str,
) -> Result<Option<WorkflowTerminalSummary>, StoreError> {
    let events = store.list_events(instance_id)?;
    Ok(events.into_iter().rev().find_map(|event| {
        if !matches!(
            event.event_type.as_str(),
            "workflow.completed" | "workflow.failed"
        ) {
            return None;
        }
        let payload = json_from_str(&event.payload_json);
        let status = payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if event.event_type == "workflow.completed" {
                    "completed"
                } else {
                    "failed"
                }
            })
            .to_owned();
        Some(WorkflowTerminalSummary {
            status,
            payload: payload.get("payload").cloned().unwrap_or(Value::Null),
        })
    }))
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
        dev_options.root.as_deref(),
        options.input_json.as_deref(),
        options,
    ) {
        Ok(started) => started,
        Err(code) => return code,
    };
    let (_source, ir) =
        match compile_source_path_with_root(&dev_options.program_path, dev_options.root.as_deref())
        {
            Ok(compiled) => compiled,
            Err(error) => return report_compile_failure(&dev_options.program_path, error),
        };
    let mut steps = Vec::new();
    let mut workers = Vec::new();
    for _ in 0..dev_options.max_iterations {
        let step_report = match step_instance(
            &options.store_path,
            &started.instance_id,
            &ir,
            Some(Path::new(&dev_options.program_path)),
        ) {
            Ok(report) => report,
            Err(error) => return report_store_error("failed to step instance", error),
        };
        let worker_report = match run_worker_once(
            &options.store_path,
            &WorkerOptions {
                instance_id: started.instance_id.clone(),
                provider: dev_options.provider.clone(),
                outcome: dev_options.outcome,
                program_path: Some(PathBuf::from(&dev_options.program_path)),
                root: dev_options.root.clone(),
                max_child_iterations: 8,
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
    let assertions = eval_assertions(
        &ir,
        &facts,
        &effects,
        Some(Path::new(&dev_options.program_path)),
    );
    if let Err(error) = persist_assertion_diagnostics(
        &store,
        &started.instance_id,
        &started.program_id,
        &started.version_id,
        &assertions,
    ) {
        return report_store_error("failed to record assertion diagnostics", error);
    }
    let failed_assertions = assertions
        .iter()
        .filter(|assertion| !assertion.passed)
        .collect::<Vec<_>>();
    let guard_errors = steps
        .iter()
        .flat_map(|step| &step.guard_reports)
        .filter(|guard| guard.status == GuardStatus::Error)
        .collect::<Vec<_>>();
    let branch_errors = steps
        .iter()
        .flat_map(|step| &step.branch_reports)
        .filter(|branch| branch.status != BranchStatus::Matched)
        .collect::<Vec<_>>();
    if options.json {
        let _ = emit_json(json!({
            "instance_id": started.instance_id,
            "workflow": started.workflow,
            "steps": steps.iter().map(step_report_to_json).collect::<Vec<_>>(),
            "workers": workers.iter().map(worker_report_to_json).collect::<Vec<_>>(),
            "assertions": assertions.iter().map(assertion_report_to_json).collect::<Vec<_>>(),
        }));
        if failed_assertions.is_empty() && guard_errors.is_empty() && branch_errors.is_empty() {
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
    } else if !branch_errors.is_empty() {
        for branch in branch_errors {
            eprintln!(
                "case branch {} in rule body for {}{}",
                branch.status.as_str(),
                branch.scrutinee,
                branch
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
    root: Option<String>,
    provider: String,
    outcome: FixtureOutcome,
    max_iterations: usize,
}

impl DevOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut program_path = None;
        let mut root = None;
        let mut provider = "fixture".to_owned();
        let mut outcome = FixtureOutcome::Completed;
        let mut max_iterations = 8usize;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--root" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected workflow name after `--root`".to_owned());
                    };
                    root = Some(value.clone());
                }
                "--provider" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected provider after `--provider`".to_owned());
                    };
                    provider = value.clone();
                }
                "--fail" => outcome = FixtureOutcome::Failed,
                "--timeout" => outcome = FixtureOutcome::TimedOut,
                "--cancel" => outcome = FixtureOutcome::Cancelled,
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
                        "usage: whip dev <workflow.whip> [--provider fixture] [--until idle] [--fail|--timeout|--cancel]"
                            .to_owned(),
                    )
                }
            }
            index += 1;
        }
        let Some(program_path) = program_path else {
            return Err(
                "usage: whip dev <workflow.whip> [--provider fixture] [--until idle] [--fail|--timeout|--cancel]"
                    .to_owned(),
            );
        };
        Ok(Self {
            program_path,
            root,
            provider,
            outcome,
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
    consumed_fact_ids: Vec<String>,
    effects: Vec<OwnedEffect>,
    dependencies: Vec<OwnedDependency>,
    terminal: Option<OwnedWorkflowTerminal>,
    branch_reports: Vec<BranchReport>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedWorkflowTerminal {
    kind: WorkflowTerminalKind,
    name: String,
    payload_json: String,
    idempotency_key: String,
}

impl OwnedWorkflowTerminal {
    fn as_workflow_terminal(&self) -> WorkflowTerminal<'_> {
        WorkflowTerminal {
            kind: self.kind,
            name: &self.name,
            payload_json: &self.payload_json,
            idempotency_key: Some(&self.idempotency_key),
        }
    }
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
    source_span_json: Option<String>,
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
            source_span_json: self.source_span_json.as_deref(),
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
    ir: &IrProgram,
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
                                ir,
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
    ir: &IrProgram,
) -> GuardReport {
    let (status, actual, error) = guard_result(eval_expr_value(
        guard,
        &EvalScope::rule(context, facts, effects, ir),
    ));
    let matched = status == GuardStatus::Matched;
    GuardReport {
        rule: rule.to_owned(),
        when: when.to_owned(),
        expr: source.to_owned(),
        source_span_json: None,
        status,
        matched,
        actual,
        error,
    }
}

fn eval_guard_source_result(
    guard: &str,
    context: &RuleContext,
) -> (GuardStatus, Value, Option<String>) {
    let Ok(expr) = parse_expression(guard) else {
        return (
            GuardStatus::Error,
            json!({"internal": "ParseError"}),
            Some("case guard could not be parsed".to_owned()),
        );
    };
    let empty_ir = empty_ir_program();
    guard_result(eval_expr_value(
        &expr,
        &EvalScope::rule(context, &[], &[], &empty_ir),
    ))
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
        EvalValue::Error(message) => (
            GuardStatus::Error,
            json!({"internal": "Error", "message": message}),
            Some(message),
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
    source_path: Option<&Path>,
) -> Vec<AssertionReport> {
    ir.assertions
        .iter()
        .map(|assertion| {
            eval_assertion(
                assertion.expr.source.as_str(),
                &assertion.expr.expr,
                ir,
                source_path.map(|_| assertion.expr.span),
                source_path,
                facts,
                effects,
            )
        })
        .collect()
}

fn persist_assertion_diagnostics(
    store: &SqliteStore,
    instance_id: &str,
    program_id: &str,
    version_id: &str,
    assertions: &[AssertionReport],
) -> Result<(), StoreError> {
    for assertion in assertions.iter().filter(|assertion| !assertion.passed) {
        let assertion_id = idempotency_key(&[instance_id, "assertion", &assertion.expr]);
        let actual_json = assertion.actual.to_string();
        let message = match assertion.status {
            AssertionStatus::Failed => format!("assertion failed: {}", assertion.expr),
            AssertionStatus::Error => format!(
                "assertion error: {}{}",
                assertion.expr,
                assertion
                    .error
                    .as_deref()
                    .map(|error| format!(" ({error})"))
                    .unwrap_or_default()
            ),
            AssertionStatus::Passed => continue,
        };
        store.record_diagnostic(DiagnosticRecord {
            instance_id: Some(instance_id),
            program_id: Some(program_id),
            program_version_id: Some(version_id),
            severity: "error",
            code: Some(match assertion.status {
                AssertionStatus::Failed => "assertion.failed",
                AssertionStatus::Error => "assertion.error",
                AssertionStatus::Passed => unreachable!("passed assertions filtered out"),
            }),
            message: &message,
            source_span_json: assertion.source_span_json.as_deref(),
            subject_type: Some("assertion"),
            subject_id: Some(&assertion.expr),
            event_id: None,
            effect_id: None,
            run_id: None,
            assertion_id: Some(&assertion_id),
            evidence_ids_json: "[]",
            artifact_ids_json: "[]",
            causation_id: None,
            correlation_id: None,
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                "assertion-diagnostic",
                &assertion.expr,
                assertion.status.as_str(),
                &actual_json,
            ])),
        })?;
    }
    Ok(())
}

fn eval_assertion(
    source: &str,
    expr: &Expr,
    ir: &IrProgram,
    span: Option<SourceSpan>,
    source_path: Option<&Path>,
    facts: &[FactView],
    effects: &[EffectView],
) -> AssertionReport {
    let source = source.trim();
    let (status, actual, error) = assertion_result(eval_expr_value(
        expr,
        &EvalScope::assertions(facts, effects, ir),
    ));
    let passed = status == AssertionStatus::Passed;
    AssertionReport {
        expr: source.to_owned(),
        source_span_json: span.map(|span| source_span_json(source_path, span, "assertion")),
        status,
        passed,
        actual,
        error,
    }
}

fn source_span_json(source_path: Option<&Path>, span: SourceSpan, construct: &str) -> String {
    json!({
        "path": source_path.map(|path| path.display().to_string()),
        "start": span.start,
        "end": span.end,
        "construct": construct,
    })
    .to_string()
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
        EvalValue::Error(message) => (
            AssertionStatus::Error,
            json!({"internal": "Error", "message": message}),
            Some(message),
        ),
    }
}

struct EvalScope<'a> {
    context: Option<&'a RuleContext>,
    facts: &'a [FactView],
    effects: &'a [EffectView],
    ir: &'a IrProgram,
    projection: Option<&'a Value>,
    projection_schema: Option<&'a str>,
}

impl<'a> EvalScope<'a> {
    fn rule(
        context: &'a RuleContext,
        facts: &'a [FactView],
        effects: &'a [EffectView],
        ir: &'a IrProgram,
    ) -> Self {
        Self {
            context: Some(context),
            facts,
            effects,
            ir,
            projection: None,
            projection_schema: None,
        }
    }

    fn assertions(facts: &'a [FactView], effects: &'a [EffectView], ir: &'a IrProgram) -> Self {
        Self {
            context: None,
            facts,
            effects,
            ir,
            projection: None,
            projection_schema: None,
        }
    }

    fn projection(&self, projection: &'a Value, schema: Option<&'a str>) -> Self {
        Self {
            context: self.context,
            facts: self.facts,
            effects: self.effects,
            ir: self.ir,
            projection: Some(projection),
            projection_schema: schema,
        }
    }
}

fn empty_ir_program() -> IrProgram {
    IrProgram {
        workflow: String::new(),
        includes: Vec::new(),
        pattern_applications: Vec::new(),
        workflow_contracts: Vec::new(),
        uses: Vec::new(),
        schemas: Vec::new(),
        agents: Vec::new(),
        coerces: Vec::new(),
        assertions: Vec::new(),
        rules: Vec::new(),
        rule_dependencies: Vec::new(),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum EvalValue {
    Json(Value),
    Missing,
    Error(String),
}

impl EvalValue {
    fn error(message: impl Into<String>) -> Self {
        Self::Error(message.into())
    }

    fn into_json(self) -> Value {
        match self {
            Self::Json(value) => value,
            Self::Missing => json!({"internal": "Missing"}),
            Self::Error(message) => json!({"internal": "Error", "message": message}),
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
        Expr::Object(fields) => {
            let mut object = serde_json::Map::new();
            for field in fields {
                object.insert(
                    field.key.clone(),
                    eval_expr_value(&field.value, scope).into_json(),
                );
            }
            EvalValue::Json(Value::Object(object))
        }
        Expr::Unary { op, expr } => match op {
            UnaryOp::Not => EvalValue::Json(Value::Bool(!truthy(&eval_expr_value(expr, scope)))),
        },
        Expr::Binary { op, left, right } => eval_binary(*op, left, right, scope),
        Expr::Call { name, args } => eval_call(name, args, scope),
        Expr::Query { .. } => eval_query_count(expr, scope)
            .map(|count| EvalValue::Json(Value::Number(count.into())))
            .unwrap_or_else(|value| value),
    }
}

fn eval_index(target: &Expr, key: &Expr, scope: &EvalScope<'_>) -> EvalValue {
    let target = eval_expr_value(target, scope);
    let key = eval_expr_value(key, scope);
    let key = match key {
        EvalValue::Json(Value::String(value)) => value,
        EvalValue::Missing => return EvalValue::Missing,
        _ => return EvalValue::error("index expression key did not evaluate to string"),
    };
    match target {
        EvalValue::Json(Value::Object(object)) => object
            .get(&key)
            .cloned()
            .map(EvalValue::Json)
            .unwrap_or(EvalValue::Missing),
        EvalValue::Missing => EvalValue::Missing,
        _ => EvalValue::error("index expression target did not evaluate to object"),
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
            let left_value = eval_expr_value(left, scope);
            let right_value = eval_expr_value(right, scope);
            let left_ty = expr_runtime_primitive(left, scope);
            let right_ty = expr_runtime_primitive(right, scope);
            match ordered_cmp(
                &left_value,
                &right_value,
                left_ty.as_ref(),
                right_ty.as_ref(),
            ) {
                Ok(ordering) => {
                    let result = ordering
                        .map(|ordering| match op {
                            BinaryOp::Lt => ordering.is_lt(),
                            BinaryOp::Le => ordering.is_le(),
                            BinaryOp::Gt => ordering.is_gt(),
                            BinaryOp::Ge => ordering.is_ge(),
                            _ => false,
                        })
                        .unwrap_or(false);
                    EvalValue::Json(Value::Bool(result))
                }
                Err(message) => EvalValue::error(message),
            }
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
            .unwrap_or_else(|value| value),
        ("one", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Bool(count == 1)))
            .unwrap_or_else(|value| value),
        ("none", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Bool(count == 0)))
            .unwrap_or_else(|value| value),
        ("exists", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Bool(count > 0)))
            .unwrap_or_else(|value| value),
        ("exists", [expr]) => EvalValue::Json(Value::Bool(
            !eval_expr_value(expr, scope).is_missing_or_null(),
        )),
        ("empty", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Bool(count == 0)))
            .unwrap_or_else(|value| value),
        ("empty", [expr]) => {
            EvalValue::Json(Value::Bool(is_empty_value(&eval_expr_value(expr, scope))))
        }
        _ => EvalValue::error("unknown expression function"),
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
                    if guard_filter_matches(eval_expr_value(
                        guard,
                        &scope.projection(&value, Some(head.trim())),
                    ))? {
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
                    if guard_filter_matches(eval_expr_value(
                        guard,
                        &scope.projection(&value, None),
                    ))? {
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
        return EvalValue::error("empty expression path");
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
        EvalValue::Json(Value::Null) | EvalValue::Missing | EvalValue::Error(_) => false,
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

fn ordered_cmp(
    left: &EvalValue,
    right: &EvalValue,
    left_ty: Option<&IrPrimitiveType>,
    right_ty: Option<&IrPrimitiveType>,
) -> Result<Option<std::cmp::Ordering>, String> {
    let typed = left_ty.or(right_ty);
    match typed {
        Some(IrPrimitiveType::Duration) => {
            let left = typed_string_seconds(left, "duration")?;
            let right = typed_string_seconds(right, "duration")?;
            return Ok(left.partial_cmp(&right));
        }
        Some(IrPrimitiveType::Time) => {
            let left = typed_string_seconds(left, "time")?;
            let right = typed_string_seconds(right, "time")?;
            return Ok(left.partial_cmp(&right));
        }
        _ => {}
    }

    match (number_value(left), number_value(right)) {
        (Some(left), Some(right)) => return Ok(left.partial_cmp(&right)),
        (Some(_), None) | (None, Some(_)) => return Ok(None),
        (None, None) => {}
    }
    let (EvalValue::Json(Value::String(left)), EvalValue::Json(Value::String(right))) =
        (left, right)
    else {
        return Ok(None);
    };
    if let (Some(left), Some(right)) = (parse_duration_seconds(left), parse_duration_seconds(right))
    {
        return Ok(left.partial_cmp(&right));
    }
    if let (Some(left), Some(right)) = (
        parse_time_epoch_seconds(left),
        parse_time_epoch_seconds(right),
    ) {
        return Ok(left.partial_cmp(&right));
    }
    Ok(None)
}

fn typed_string_seconds(value: &EvalValue, expected: &str) -> Result<f64, String> {
    let EvalValue::Json(Value::String(value)) = value else {
        return Err(format!("{expected} ordering expected string value"));
    };
    let parsed = match expected {
        "duration" => parse_duration_seconds(value),
        "time" => parse_time_epoch_seconds(value),
        _ => None,
    };
    parsed.ok_or_else(|| format!("invalid {expected} value `{value}`"))
}

fn expr_runtime_primitive(expr: &Expr, scope: &EvalScope<'_>) -> Option<IrPrimitiveType> {
    match expr {
        Expr::Path(path) => path_runtime_primitive(path, scope),
        Expr::Literal(ExprLiteral::Ident(field)) if scope.projection_schema.is_some() => {
            let path = [field.clone()];
            path_runtime_primitive(&path, scope)
        }
        _ => None,
    }
}

fn path_runtime_primitive(path: &[String], scope: &EvalScope<'_>) -> Option<IrPrimitiveType> {
    if let Some(context) = scope.context {
        let (binding, rest) = path.split_first()?;
        let (_, fact) = context
            .bindings
            .iter()
            .find(|(candidate, _)| candidate == binding)?;
        return ir_path_primitive(scope.ir, &fact.name, rest);
    }
    if let Some(schema) = scope.projection_schema {
        return ir_path_primitive(scope.ir, schema, path);
    }
    None
}

fn ir_path_primitive(ir: &IrProgram, schema: &str, path: &[String]) -> Option<IrPrimitiveType> {
    let mut current_schema = schema;
    for (index, field_name) in path.iter().enumerate() {
        let ty = ir.schemas.iter().find_map(|schema| match schema {
            IrSchema::Class(class) if class.name == current_schema => class
                .fields
                .iter()
                .find(|field| field.name == *field_name)
                .map(|field| &field.ty),
            _ => None,
        });
        let next_ty = unwrap_optional_type(ty?)?;
        if index == path.len() - 1 {
            return match next_ty {
                IrType::Primitive(primitive) => Some(primitive.clone()),
                _ => None,
            };
        }
        match next_ty {
            IrType::Ref(next_schema) => current_schema = next_schema,
            _ => return None,
        }
    }
    None
}

fn unwrap_optional_type(ty: &IrType) -> Option<&IrType> {
    match ty {
        IrType::Optional(inner) => unwrap_optional_type(inner),
        other => Some(other),
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
    source_path: Option<&Path>,
) -> OwnedLowering {
    let rule_body = desugar_then_chains(&rule.body);
    let (body, context, branch_reports) = selected_rule_body(&rule_body, context);
    let existing_fact_ids = facts
        .iter()
        .map(|fact| fact.fact_id.as_str())
        .collect::<Vec<_>>();
    let existing_effect_ids = effects
        .iter()
        .map(|effect| effect.effect_id.as_str())
        .collect::<Vec<_>>();
    let mut lowering = OwnedLowering::default();
    lowering.branch_reports.extend(branch_reports);
    let pre_terminal_body = strip_after_blocks(&body);
    append_consumed_fact_ids(&mut lowering, &pre_terminal_body, &context, facts);
    append_workflow_terminal(&mut lowering, ir, rule, &pre_terminal_body, &context, None);

    for block in top_level_record_blocks(&pre_terminal_body) {
        let value = parse_record_fields(&block.body, &context, block.from_binding.as_deref());
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

    let parsed_effects = parse_effect_statements(&pre_terminal_body, &context);
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
            source_span_json: effect_node
                .map(|effect| source_span_json(source_path, effect.span, "effect")),
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

    for after in after_blocks(&body) {
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
            binding_value.clone(),
        );
        if let Some(alias) = &after.alias {
            push_effect_binding(&mut after_context, alias, upstream_effect_id, binding_value);
        }
        for (binding, effect_id) in &binding_to_effect_id {
            if binding == &after.binding {
                continue;
            }
            if let Some(value) = effect_binding_value(facts, effect_id, "succeeds") {
                push_effect_binding(&mut after_context, binding, effect_id, value);
            }
        }
        let (selected_after_body, after_context, branch_reports) =
            selected_rule_body(&after.body, &after_context);
        lowering.branch_reports.extend(branch_reports);
        append_consumed_fact_ids(&mut lowering, &selected_after_body, &after_context, facts);
        append_workflow_terminal(
            &mut lowering,
            ir,
            rule,
            &selected_after_body,
            &after_context,
            Some((&after.binding, &after.predicate)),
        );
        for record in top_level_record_blocks(&selected_after_body) {
            let value =
                parse_record_fields(&record.body, &after_context, record.from_binding.as_deref());
            let value_json = Value::Object(value).to_string();
            let fact_key = record_fact_key(&record.schema, &value_json);
            let fact_id = idempotency_key(&[
                &rule.name,
                &after.binding,
                &after.predicate,
                &record.schema,
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
                name: record.schema.clone(),
                key: fact_key,
                value_json,
                schema_id: Some(record.schema),
                provenance_class: "rule".to_owned(),
                correlation_id: context.identity.clone(),
            });
        }
        let mut selected_effects = parse_effect_statements(&selected_after_body, &after_context);
        for effect in &mut selected_effects {
            effect.after.get_or_insert_with(|| AfterScope {
                binding: after.binding.clone(),
                predicate: after.predicate.clone(),
            });
        }
        let mut selected_binding_to_effect_id = binding_to_effect_id.clone();
        let mut selected_node_to_effect_id = std::collections::BTreeMap::new();
        for (index, parsed) in selected_effects.iter().enumerate() {
            let effect_node = effect_node_for_parsed(rule, parsed, index);
            let node_id = effect_node
                .map(|effect| effect.id.as_str())
                .unwrap_or(parsed.kind.as_str());
            let effect_id = idempotency_key(&[
                &rule.name,
                &after.binding,
                &after.predicate,
                node_id,
                after_context.identity.as_deref().unwrap_or("started"),
            ]);
            selected_node_to_effect_id.insert(node_id.to_owned(), effect_id.clone());
            if let Some(binding) = effect_node
                .and_then(|effect| effect.binding.as_ref())
                .or(parsed.binding.as_ref())
            {
                selected_binding_to_effect_id.insert(binding.clone(), effect_id);
            }
        }
        for (binding, effect_id) in &selected_binding_to_effect_id {
            binding_to_effect_id
                .entry(binding.clone())
                .or_insert_with(|| effect_id.clone());
        }
        for (index, parsed) in selected_effects.iter().enumerate() {
            let effect_node = effect_node_for_parsed(rule, parsed, index);
            let node_id = effect_node
                .map(|effect| effect.id.as_str())
                .unwrap_or(parsed.kind.as_str());
            let Some(effect_id) = selected_node_to_effect_id.get(node_id).cloned() else {
                continue;
            };
            if existing_effect_ids
                .iter()
                .any(|existing| *existing == effect_id)
                || lowering
                    .effects
                    .iter()
                    .any(|existing| existing.effect_id == effect_id)
            {
                continue;
            }
            let input_json = parsed_effect_input_json(
                ir,
                rule,
                parsed,
                &after_context,
                &selected_binding_to_effect_id,
            );
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
                correlation_id: after_context.identity.clone(),
                source_span_json: effect_node
                    .map(|effect| source_span_json(source_path, effect.span, "effect")),
            });
        }
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

fn append_consumed_fact_ids(
    lowering: &mut OwnedLowering,
    body: &str,
    context: &RuleContext,
    facts: &[FactView],
) {
    for binding in consume_statements(body) {
        let Some((_, fact)) = context
            .bindings
            .iter()
            .find(|(candidate, _)| candidate == &binding)
        else {
            continue;
        };
        if fact.provenance_class == "effect" {
            continue;
        }
        if !facts.iter().any(|active| active.fact_id == fact.fact_id) {
            continue;
        }
        if !lowering
            .consumed_fact_ids
            .iter()
            .any(|existing| existing == &fact.fact_id)
        {
            lowering.consumed_fact_ids.push(fact.fact_id.clone());
        }
    }
}

fn consume_statements(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(';');
            let binding = line
                .strip_prefix("consume ")
                .or_else(|| line.strip_prefix("done "))?
                .split("->")
                .next()
                .unwrap_or_default()
                .trim();
            is_identifier(binding).then(|| binding.to_owned())
        })
        .collect()
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn selected_rule_body(
    body: &str,
    context: &RuleContext,
) -> (String, RuleContext, Vec<BranchReport>) {
    let lines = body.lines().collect::<Vec<_>>();
    let mut selected = Vec::new();
    let mut context = context.clone();
    let mut branch_reports = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with("after ") {
            let mut depth = brace_delta(trimmed).max(1);
            selected.push(lines[index].to_owned());
            index += 1;
            while index < lines.len() && depth > 0 {
                let line = lines[index];
                depth += brace_delta(line);
                selected.push(line.to_owned());
                index += 1;
            }
            continue;
        }
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
        let selection = select_case_branch(&case, &mut context);
        if let Some(report) = selection.report {
            branch_reports.push(report);
        }
        if let Some(branch) = selection.branch {
            let (branch_body, branch_context, nested_reports) =
                selected_rule_body(&branch.body.join("\n"), &context);
            context = branch_context;
            branch_reports.extend(nested_reports);
            selected.extend(branch_body.lines().map(str::to_owned));
        }
        index = next_index;
    }
    (selected.join("\n"), context, branch_reports)
}

fn desugar_then_chains(body: &str) -> String {
    let lines = body.lines().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut last_effect_binding: Option<String> = None;
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if let (Some(statement), Some(upstream)) = (
            trimmed.strip_prefix("then ").map(str::trim),
            last_effect_binding.as_deref(),
        ) {
            output.push(format!("after {upstream} succeeds {{"));
            output.push(format!("  {statement}"));
            let mut depth = brace_delta(statement);
            index += 1;
            while depth > 0 && index < lines.len() {
                let line = lines[index];
                depth += brace_delta(line);
                output.push(format!("  {line}"));
                index += 1;
            }
            output.push("}".to_owned());
            if let Some(binding) = effect_binding_from_statement(statement) {
                last_effect_binding = Some(binding);
            }
            continue;
        }
        output.push(lines[index].to_owned());
        if let Some(binding) = effect_binding_from_statement(trimmed) {
            last_effect_binding = Some(binding);
        }
        index += 1;
    }
    output.join("\n")
}

fn effect_binding_from_statement(statement: &str) -> Option<String> {
    if statement.starts_with("tell ")
        || statement.starts_with("coerce ")
        || statement.starts_with("claim ")
        || statement.starts_with("askHuman ")
        || statement.starts_with("call ")
        || statement.starts_with("emit ")
    {
        binding_after_as(statement)
    } else {
        None
    }
}

fn strip_after_blocks(body: &str) -> String {
    let lines = body.lines().collect::<Vec<_>>();
    let mut selected = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with("after ") {
            let mut depth = brace_delta(trimmed).max(1);
            index += 1;
            while index < lines.len() && depth > 0 {
                depth += brace_delta(lines[index]);
                index += 1;
            }
            continue;
        }
        selected.push(lines[index].to_owned());
        index += 1;
    }
    selected.join("\n")
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct CaseSelection {
    branch: Option<CaseBranch>,
    report: Option<BranchReport>,
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

fn select_case_branch(case: &CaseBlock, context: &mut RuleContext) -> CaseSelection {
    let value = parse_field_value(&case.scrutinee, context);
    let mut fallback = None;
    for branch in &case.branches {
        if matches!(branch.pattern.as_str(), "_" | "default") {
            fallback = Some(branch.clone());
            continue;
        }
        let mut candidate_context = context.clone();
        if !case_pattern_matches(&branch.pattern, &value, &mut candidate_context) {
            continue;
        }
        if let Some(guard) = branch.guard.as_deref() {
            let (status, actual, error) = eval_guard_source_result(guard, &candidate_context);
            match status {
                GuardStatus::Matched => {}
                GuardStatus::False => continue,
                GuardStatus::Error => {
                    return CaseSelection {
                        branch: None,
                        report: Some(BranchReport {
                            scrutinee: case.scrutinee.clone(),
                            status: BranchStatus::Error,
                            matched: false,
                            tag: terminal_case_tag(&value).map(str::to_owned),
                            actual,
                            error,
                        }),
                    };
                }
            }
        }
        {
            *context = candidate_context;
            return CaseSelection {
                branch: Some(branch.clone()),
                report: Some(BranchReport {
                    scrutinee: case.scrutinee.clone(),
                    status: BranchStatus::Matched,
                    matched: true,
                    tag: terminal_case_tag(&value).map(str::to_owned),
                    actual: value.clone(),
                    error: None,
                }),
            };
        }
    }
    if let Some(branch) = fallback {
        return CaseSelection {
            branch: Some(branch),
            report: Some(BranchReport {
                scrutinee: case.scrutinee.clone(),
                status: BranchStatus::Matched,
                matched: true,
                tag: terminal_case_tag(&value).map(str::to_owned),
                actual: value,
                error: None,
            }),
        };
    }
    CaseSelection {
        branch: None,
        report: terminal_case_tag(&value).map(|tag| BranchReport {
            scrutinee: case.scrutinee.clone(),
            status: BranchStatus::NoMatch,
            matched: false,
            tag: Some(tag.to_owned()),
            actual: value.clone(),
            error: Some("terminal-output case matched no branch".to_owned()),
        }),
    }
}

fn case_pattern_matches(pattern: &str, value: &Value, context: &mut RuleContext) -> bool {
    if let Some(tag) = terminal_case_tag(value) {
        let mut parts = pattern.split_whitespace();
        let Some(pattern_tag) = parts.next() else {
            return false;
        };
        if pattern_tag != tag {
            return false;
        }
        if let Some(binding) = parts.next() {
            if parts.next().is_some() {
                return false;
            }
            let payload = terminal_payload_for_tag(value, tag);
            context.bindings.push((
                binding.to_owned(),
                FactView {
                    fact_id: format!("case:{binding}"),
                    name: binding.to_owned(),
                    key: binding.to_owned(),
                    value_json: payload.to_string(),
                    provenance_class: "case".to_owned(),
                },
            ));
        }
        return true;
    }
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

fn terminal_case_tag(value: &Value) -> Option<&str> {
    value.get("tag").and_then(Value::as_str).or_else(|| {
        value
            .get("status")
            .and_then(Value::as_str)
            .and_then(terminal_tag_for_status)
    })
}

fn terminal_tag_for_status(status: &str) -> Option<&'static str> {
    match status {
        "completed" | "succeeded" => Some("Completed"),
        "failed" => Some("Failed"),
        "timed_out" | "timeout" => Some("TimedOut"),
        "cancelled" | "canceled" => Some("Cancelled"),
        _ => None,
    }
}

fn terminal_payload_for_tag(value: &Value, tag: &str) -> Value {
    match tag {
        "Completed" => value
            .get("value")
            .cloned()
            .or_else(|| value.get("output").cloned())
            .unwrap_or(Value::Null),
        "Failed" | "TimedOut" | "Cancelled" => {
            let mut payload = value
                .get("error")
                .cloned()
                .or_else(|| value.get("failure").cloned())
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            for field in ["summary", "effect_id", "run_id"] {
                if let Some(field_value) = value.get(field) {
                    payload.insert(field.to_owned(), field_value.clone());
                }
            }
            if !payload.contains_key("reason") {
                if let Some(summary) = value.get("summary") {
                    payload.insert("reason".to_owned(), summary.clone());
                }
            }
            Value::Object(payload)
        }
        _ => value.clone(),
    }
}

fn append_workflow_terminal(
    lowering: &mut OwnedLowering,
    ir: &IrProgram,
    rule: &IrRule,
    body: &str,
    context: &RuleContext,
    after: Option<(&str, &str)>,
) {
    if lowering.terminal.is_some() {
        return;
    }
    let Some(terminal) = top_level_terminal_blocks(body).into_iter().next() else {
        return;
    };
    if !workflow_contract_exists(ir, terminal.kind, &terminal.name) {
        return;
    }
    let payload = Value::Object(parse_record_fields(&terminal.body, context, None));
    let payload_json = payload.to_string();
    let mut key_parts = vec![
        rule.name.as_str(),
        terminal.kind.action(),
        terminal.name.as_str(),
        context.identity.as_deref().unwrap_or("started"),
        payload_json.as_str(),
    ];
    if let Some((binding, predicate)) = after {
        key_parts.push(binding);
        key_parts.push(predicate);
    }
    let idempotency_key = idempotency_key(&key_parts);
    lowering.terminal = Some(OwnedWorkflowTerminal {
        kind: terminal.kind,
        name: terminal.name,
        payload_json,
        idempotency_key,
    });
}

fn workflow_contract_exists(ir: &IrProgram, kind: WorkflowTerminalKind, name: &str) -> bool {
    let wanted = match kind {
        WorkflowTerminalKind::Completed => whipplescript_parser::IrWorkflowContractKind::Output,
        WorkflowTerminalKind::Failed => whipplescript_parser::IrWorkflowContractKind::Failure,
    };
    ir.workflow_contracts
        .iter()
        .any(|contract| contract.kind == wanted && contract.name == name)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordBlock {
    schema: String,
    from_binding: Option<String>,
    body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalBlock {
    kind: WorkflowTerminalKind,
    name: String,
    body: String,
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
    required_capabilities: Vec<String>,
    after: Option<AfterScope>,
}

impl ParsedEffect {
    fn required_capabilities_json(&self) -> String {
        let mut capabilities = match self.kind.as_str() {
            "baml.coerce" => vec!["baml.coerce".to_owned()],
            "loft.claim" => vec!["loft.claim".to_owned()],
            "human.ask" => vec!["human.ask".to_owned()],
            "capability.call" => vec!["capability.call".to_owned()],
            "event.emit" => vec!["event.emit".to_owned()],
            "workflow.invoke" => vec!["workflow.invoke".to_owned()],
            _ => Vec::new(),
        };
        capabilities.extend(self.required_capabilities.iter().cloned());
        capabilities.sort();
        capabilities.dedup();
        serde_json::to_string(&capabilities).unwrap_or_else(|_| "[]".to_owned())
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
        if let Some(rest) = trimmed.strip_prefix("invoke ") {
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let target = rest
                .split_whitespace()
                .next()
                .unwrap_or("workflow")
                .trim_end_matches('{')
                .to_owned();
            let body = invoke_body(&statement).unwrap_or_default();
            effects.push(ParsedEffect {
                kind: "workflow.invoke".to_owned(),
                target: Some(target),
                name: Some("invoke".to_owned()),
                binding: binding_after_as(&statement),
                args: vec![body],
                prompt: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("tell ") {
            let target_expr = rest.split_whitespace().next().unwrap_or("agent");
            let (prompt, next_index) = parse_prompt_from_lines(&lines, index, trimmed);
            effects.push(ParsedEffect {
                kind: "agent.tell".to_owned(),
                target: Some(resolve_tell_target(target_expr, context)),
                name: None,
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: Some(interpolate_prompt(&prompt, context)),
                required_capabilities: parse_required_capabilities(trimmed),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("coerce ") {
            let (statement, next_index) =
                parse_statement_until_balanced_parens(&lines, index, trimmed);
            let rest = statement.strip_prefix("coerce ").unwrap_or(rest);
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
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if trimmed.starts_with("claim ") && trimmed.contains(" with loft") {
            effects.push(ParsedEffect {
                kind: "loft.claim".to_owned(),
                target: Some("loft".to_owned()),
                name: Some("claim".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                required_capabilities: Vec::new(),
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
                required_capabilities: Vec::new(),
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
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if let Some(rest) = trimmed.strip_prefix("emit ") {
            let event_type = rest
                .split_whitespace()
                .next()
                .unwrap_or("event.emitted")
                .to_owned();
            effects.push(ParsedEffect {
                kind: "event.emit".to_owned(),
                target: Some(event_type),
                name: Some("emit".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                required_capabilities: Vec::new(),
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

fn parse_statement_until_balanced_parens(
    lines: &[&str],
    index: usize,
    trimmed: &str,
) -> (String, usize) {
    let mut statement = trimmed.to_owned();
    let mut depth = paren_delta(trimmed);
    let mut cursor = index;
    while depth > 0 && cursor + 1 < lines.len() {
        cursor += 1;
        let next = lines[cursor].trim();
        statement.push(' ');
        statement.push_str(next);
        depth += paren_delta(next);
    }
    (statement, cursor)
}

fn parse_statement_until_balanced_braces(
    lines: &[&str],
    index: usize,
    trimmed: &str,
) -> (String, usize) {
    let mut statement = trimmed.to_owned();
    let mut depth = brace_delta(trimmed);
    let mut cursor = index;
    while depth > 0 && cursor + 1 < lines.len() {
        cursor += 1;
        let next = lines[cursor].trim();
        statement.push(' ');
        statement.push_str(next);
        depth += brace_delta(next);
    }
    (statement, cursor)
}

fn invoke_body(statement: &str) -> Option<String> {
    let open = statement.find('{')?;
    let close = statement.rfind('}')?;
    (close > open).then(|| statement[open + 1..close].trim().to_owned())
}

fn paren_delta(line: &str) -> i32 {
    line.chars().fold(0, |depth, ch| match ch {
        '(' => depth + 1,
        ')' => depth - 1,
        _ => depth,
    })
}

fn resolve_tell_target(target_expr: &str, context: &RuleContext) -> String {
    parse_field_value(target_expr, context)
        .as_str()
        .unwrap_or(target_expr)
        .to_owned()
}

fn parse_required_capabilities(line: &str) -> Vec<String> {
    let Some(rest) = line.split_once(" requires ") else {
        return Vec::new();
    };
    let Some(list) = rest.1.trim_start().strip_prefix('[') else {
        return Vec::new();
    };
    let Some((items, _)) = list.split_once(']') else {
        return Vec::new();
    };
    let mut capabilities = items
        .split(',')
        .filter_map(|item| {
            let value = item.trim().trim_matches('"');
            (!value.is_empty()).then(|| value.to_owned())
        })
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    capabilities
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
        "event.emit" => json!({
            "event_type": effect.target,
            "payload": {
                "rule": rule.name,
                "bindings": context_bindings_json(context),
            },
            "bindings": context_bindings_json(context),
            "rule": rule.name,
        }),
        "workflow.invoke" => {
            let body = effect.args.first().map(String::as_str).unwrap_or_default();
            json!({
                "target_workflow": effect.target,
                "input": Value::Object(parse_record_fields(body, context, None)),
                "bindings": context_bindings_json(context),
                "rule": rule.name,
            })
        }
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
    let (binding, predicate, _) = parse_after_header(rest)?;
    Some(AfterScope { binding, predicate })
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
    let mut values = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut previous = '\0';
    for (index, ch) in args.char_indices() {
        if ch == '"' && previous != '\\' {
            in_string = !in_string;
        } else if !in_string {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    let value = args[start..index].trim();
                    if !value.is_empty() {
                        values.push(value.to_owned());
                    }
                    start = index + ch.len_utf8();
                }
                _ => {}
            }
        }
        previous = ch;
    }
    let value = args[start..].trim();
    if !value.is_empty() {
        values.push(value.to_owned());
    }
    values
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
        IrType::Object(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|field| format!("{} {}", field.name, ir_type_name(&field.ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
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
        let record_rest = trimmed.strip_prefix("record ").or_else(|| {
            trimmed
                .strip_prefix("done ")
                .and_then(|rest| rest.split_once("->"))
                .map(|(_, record)| record.trim())
                .and_then(|record| record.strip_prefix("record "))
        });
        if let Some(rest) = record_rest {
            let Some((schema, from_binding)) = parse_record_header(rest) else {
                index += 1;
                continue;
            };
            let mut record_lines = Vec::new();
            let mut depth = brace_delta(trimmed);
            index += 1;
            while index < lines.len() && depth > 0 {
                let line = lines[index];
                let before = depth;
                depth += brace_delta(line);
                if !(before == 1 && depth == 0 && line.trim() == "}") {
                    record_lines.push(line.to_owned());
                }
                index += 1;
            }
            blocks.push(RecordBlock {
                schema,
                from_binding,
                body: record_lines.join("\n"),
            });
            continue;
        }
        index += 1;
    }
    blocks
}

fn top_level_terminal_blocks(body: &str) -> Vec<TerminalBlock> {
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
        let terminal = trimmed
            .strip_prefix("complete ")
            .map(|rest| (WorkflowTerminalKind::Completed, rest))
            .or_else(|| {
                trimmed
                    .strip_prefix("fail ")
                    .map(|rest| (WorkflowTerminalKind::Failed, rest))
            });
        let Some((kind, rest)) = terminal else {
            index += 1;
            continue;
        };
        let Some(name) = rest.split('{').next().and_then(|header| {
            let mut parts = header.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some(name), None) if is_identifier(name) => Some(name.to_owned()),
                _ => None,
            }
        }) else {
            index += 1;
            continue;
        };
        let mut block_lines = Vec::new();
        let mut depth = brace_delta(trimmed);
        index += 1;
        while index < lines.len() && depth > 0 {
            let line = lines[index];
            let before = depth;
            depth += brace_delta(line);
            if !(before == 1 && depth == 0 && line.trim() == "}") {
                block_lines.push(line.to_owned());
            }
            index += 1;
        }
        blocks.push(TerminalBlock {
            kind,
            name,
            body: block_lines.join("\n"),
        });
    }
    blocks
}

fn parse_record_header(rest: &str) -> Option<(String, Option<String>)> {
    let before_brace = rest.split('{').next().unwrap_or(rest).trim();
    let mut parts = before_brace.split_whitespace();
    let schema = parts.next()?.to_owned();
    let from_binding = match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => None,
        (Some("from"), Some(binding), None) if is_identifier(binding) => Some(binding.to_owned()),
        _ => return None,
    };
    Some((schema, from_binding))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AfterBlock {
    binding: String,
    predicate: String,
    alias: Option<String>,
    body: String,
}

fn after_blocks(body: &str) -> Vec<AfterBlock> {
    let mut blocks = Vec::new();
    let lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        let Some(rest) = trimmed.strip_prefix("after ") else {
            index += 1;
            continue;
        };
        let Some((binding, predicate, alias)) = parse_after_header(rest) else {
            index += 1;
            continue;
        };
        let mut depth = brace_delta(trimmed).max(1);
        let mut inner = Vec::new();
        index += 1;
        while index < lines.len() && depth > 0 {
            let line = lines[index];
            let next_depth = depth + brace_delta(line);
            if next_depth >= 1 {
                inner.push(line.to_owned());
            }
            depth = next_depth;
            index += 1;
        }
        blocks.push(AfterBlock {
            binding,
            predicate,
            alias,
            body: inner.join("\n"),
        });
    }
    blocks
}

fn parse_after_header(rest: &str) -> Option<(String, String, Option<String>)> {
    let before_body = rest
        .split('{')
        .next()
        .unwrap_or(rest)
        .split("=>")
        .next()
        .unwrap_or(rest)
        .trim();
    let mut parts = before_body.split_whitespace();
    let binding = parts.next()?.to_owned();
    let predicate = parts.next()?.to_owned();
    let alias = match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => None,
        (Some("as"), Some(alias), None) if is_identifier(alias) => Some(alias.to_owned()),
        _ => return None,
    };
    Some((binding, predicate, alias))
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
        if predicate == "completes" {
            return Some(terminal_union_value(&payload));
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

fn terminal_union_value(payload: &Value) -> Value {
    let status = payload
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let tag = terminal_tag_for_status(status).unwrap_or("Completed");
    json!({
        "tag": tag,
        "status": status,
        "value": payload.get("value").cloned().or_else(|| payload.get("output").cloned()).unwrap_or(Value::Null),
        "error": payload.get("error").cloned().or_else(|| payload.get("failure").cloned()).unwrap_or(Value::Null),
        "summary": payload.get("summary").cloned().unwrap_or(Value::Null),
        "effect_id": payload.get("effect_id").cloned().unwrap_or(Value::Null),
        "run_id": payload.get("run_id").cloned().unwrap_or(Value::Null),
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

fn parse_record_fields(
    body: &str,
    context: &RuleContext,
    from_binding: Option<&str>,
) -> serde_json::Map<String, Value> {
    let mut object = serde_json::Map::new();
    for assignment in collect_field_assignments(body) {
        match assignment {
            FieldAssignment::Value { name, value } => {
                object.insert(
                    name.clone(),
                    parse_record_field_value(&name, &value, context, from_binding),
                );
            }
            FieldAssignment::Shorthand { name } => {
                object.insert(
                    name.clone(),
                    parse_record_shorthand_value(&name, context, from_binding),
                );
            }
        }
    }
    object
}

fn parse_record_field_value(
    _field: &str,
    value: &str,
    context: &RuleContext,
    from_binding: Option<&str>,
) -> Value {
    if let Some(binding) = from_binding {
        if is_identifier(value)
            && !context
                .bindings
                .iter()
                .any(|(candidate, _)| candidate == value)
        {
            if let Some(copied) = context_field_value(context, binding, value) {
                return copied;
            }
        }
    }
    parse_field_value(value, context)
}

fn parse_record_shorthand_value(
    field: &str,
    context: &RuleContext,
    from_binding: Option<&str>,
) -> Value {
    if let Some(binding) = from_binding {
        if let Some(value) = context_field_value(context, binding, field) {
            return value;
        }
    }
    let matches = context
        .bindings
        .iter()
        .filter_map(|(binding, _)| context_field_value(context, binding, field))
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.into_iter().next().unwrap_or(Value::Null)
    } else {
        Value::Null
    }
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
    if let Ok(expr) = whipplescript_parser::parse_expression(value) {
        if !matches!(expr, Expr::Literal(ExprLiteral::Ident(_))) {
            let empty_ir = empty_ir_program();
            return eval_expr_value(&expr, &EvalScope::rule(context, &[], &[], &empty_ir))
                .into_json();
        }
    }
    if matches!(value.as_bytes().first(), Some(b'{' | b'[')) {
        if let Some(parsed) = parse_inline_object_literal(value, context) {
            return parsed;
        }
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

fn parse_inline_object_literal(value: &str, context: &RuleContext) -> Option<Value> {
    let body = value.strip_prefix('{')?.strip_suffix('}')?.trim();
    let mut object = serde_json::Map::new();
    if body.is_empty() {
        return Some(Value::Object(object));
    }
    for field in body.split(',') {
        let field = field.trim();
        let (name, value) = field.split_once(char::is_whitespace)?;
        object.insert(name.to_owned(), parse_field_value(value.trim(), context));
    }
    Some(Value::Object(object))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FieldAssignment {
    Value { name: String, value: String },
    Shorthand { name: String },
}

fn collect_field_assignments(body: &str) -> Vec<FieldAssignment> {
    let lines = body.lines().collect::<Vec<_>>();
    let mut assignments = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim().trim_end_matches(',');
        if trimmed.is_empty() || trimmed == "}" {
            index += 1;
            continue;
        }
        let Some((name, value)) = trimmed.split_once(char::is_whitespace) else {
            if is_identifier(trimmed) {
                assignments.push(FieldAssignment::Shorthand {
                    name: trimmed.to_owned(),
                });
            }
            index += 1;
            continue;
        };
        let mut value_lines = vec![value.trim().to_owned()];
        let mut depth = brace_delta(value.trim());
        index += 1;
        while depth > 0 && index < lines.len() {
            let next = lines[index].trim().trim_end_matches(',');
            depth += brace_delta(next);
            value_lines.push(next.to_owned());
            index += 1;
        }
        assignments.push(FieldAssignment::Value {
            name: name.to_owned(),
            value: value_lines.join(" "),
        });
    }
    assignments
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
        "facts_consumed": report.facts_consumed,
        "effects_created": report.effects_created,
        "guards": report.guard_reports.iter().map(guard_report_to_json).collect::<Vec<_>>(),
        "branches": report.branch_reports.iter().map(branch_report_to_json).collect::<Vec<_>>(),
    })
}

fn branch_report_to_json(report: &BranchReport) -> Value {
    json!({
        "scrutinee": report.scrutinee,
        "status": report.status.as_str(),
        "matched": report.matched,
        "tag": report.tag,
        "actual": report.actual,
        "error": report.error,
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
    if let Some(source_span_json) = &report.source_span_json {
        if let Some(object) = value.as_object_mut() {
            object.insert("source_span".to_owned(), json_from_str(source_span_json));
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
            .consumed_fact_ids
            .iter()
            .map(|fact_id| fact_id.as_str()),
    );
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
    if let Some(terminal) = &lowering.terminal {
        ids.push(terminal.idempotency_key.as_str());
    }
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
                "{} {} workflow_version={} epoch={} updated={}",
                instance.instance_id,
                instance.status,
                instance.version_id,
                instance.revision_epoch,
                instance.updated_at
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
            "workflow_version={} epoch={}",
            status.instance.version_id, status.instance.revision_epoch
        );
        if let Some(revision) = status.revisions.last() {
            println!(
                "active_revision={} from={} to={} policy={}",
                revision.revision_id,
                revision.from_version_id,
                revision.to_version_id,
                revision.cancellation_policy
            );
        }
        if let Some(terminal) = workflow_terminal_summary(&status.recent_events) {
            println!(
                "workflow terminal: {} {}",
                terminal
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                terminal
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("<unnamed>")
            );
        }
        println!(
            "facts={} queued_effects={} blocked_effects={} active_runs={} failures={} cancel_requests={}",
            status.fact_count,
            status.queued_effect_count,
            status.blocked_effect_count,
            status.active_run_count,
            status.failure_count,
            status.cancellation_request_count
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

fn diagnostics(options: &CliOptions) -> ExitCode {
    let Some(instance_id) = single_arg(options, "usage: whip diagnostics <instance>") else {
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let diagnostics = match store.list_diagnostics(Some(instance_id)) {
        Ok(diagnostics) => diagnostics,
        Err(error) => return report_store_error("failed to list diagnostics", error),
    };

    if options.json {
        emit_json(Value::Array(
            diagnostics
                .iter()
                .map(diagnostic_to_json)
                .collect::<Vec<_>>(),
        ))
    } else {
        for diagnostic in diagnostics {
            println!(
                "{} {} code={} subject={}:{}",
                diagnostic.diagnostic_id,
                diagnostic.severity,
                diagnostic.code.as_deref().unwrap_or("-"),
                diagnostic.subject_type.as_deref().unwrap_or("-"),
                diagnostic.subject_id.as_deref().unwrap_or("-")
            );
            println!("  {}", diagnostic.message);
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
        "schema": "whipplescript.local_trace.v0",
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
        "blocked_by_dependency"
        | "blocked_by_capability"
        | "blocked_by_profile"
        | "blocked_by_capacity" => EffectStatus::Blocked,
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
    ) -> Result<whipplescript_store::StoredEvent, StoreError>,
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

fn compile_source_path_with_root(
    path: &str,
    root: Option<&str>,
) -> Result<(String, IrProgram), CompileFailure> {
    let bundle = resolve_source_bundle(Path::new(path))?;
    let compiled = whipplescript_parser::compile_program_with_root(&bundle.source, root);
    if let Some(ir) = compiled.ir {
        let mut ir = ir;
        ir.includes = bundle.includes;
        Ok((bundle.source, ir))
    } else {
        Err(CompileFailure::Diagnostics {
            source: bundle.source,
            diagnostics: compiled.diagnostics,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceBundle {
    source: String,
    includes: Vec<IrInclude>,
}

fn resolve_source_bundle(path: &Path) -> Result<SourceBundle, CompileFailure> {
    let mut resolver = SourceBundleResolver::default();
    let root = resolver.resolve_root(path)?;
    Ok(SourceBundle {
        source: root,
        includes: resolver.includes,
    })
}

#[derive(Default)]
struct SourceBundleResolver {
    visited: Vec<PathBuf>,
    active: Vec<PathBuf>,
    includes: Vec<IrInclude>,
}

impl SourceBundleResolver {
    fn resolve_root(&mut self, path: &Path) -> Result<String, CompileFailure> {
        let path = canonical_or_original(path)?;
        self.resolve_file(&path, true)
    }

    fn resolve_file(&mut self, path: &Path, is_root: bool) -> Result<String, CompileFailure> {
        if self.active.iter().any(|active| active == path) {
            return Err(CompileFailure::Diagnostics {
                source: String::new(),
                diagnostics: vec![Diagnostic {
                    span: SourceSpan { start: 0, end: 0 },
                    message: format!("include cycle through `{}`", path.display()),
                    suggestion: Some("remove the recursive include".to_owned()),
                }],
            });
        }
        if !is_root && self.visited.iter().any(|visited| visited == path) {
            return Ok(String::new());
        }

        let source = fs::read_to_string(path).map_err(CompileFailure::Io)?;
        let parsed = whipplescript_parser::parse_program(&source);
        if !parsed.diagnostics.is_empty() {
            return Err(CompileFailure::Diagnostics {
                source,
                diagnostics: parsed.diagnostics,
            });
        }

        self.active.push(path.to_path_buf());
        let mut combined = String::new();
        let mut seen_includes = BTreeSet::new();
        for item in &parsed.program.items {
            let Item::Include(include) = item else {
                continue;
            };
            if !seen_includes.insert(include.path.value.clone()) {
                self.active.pop();
                return Err(CompileFailure::Diagnostics {
                    source,
                    diagnostics: vec![Diagnostic {
                        span: include.path.span,
                        message: format!("duplicate include `{}`", include.path.value),
                        suggestion: Some("remove the duplicate include".to_owned()),
                    }],
                });
            }
            let include_path = Path::new(&include.path.value);
            if include_path.is_absolute() {
                self.active.pop();
                return Err(CompileFailure::Diagnostics {
                    source,
                    diagnostics: vec![Diagnostic {
                        span: include.path.span,
                        message: "include paths must be relative".to_owned(),
                        suggestion: Some(
                            "write an include path relative to the current file".to_owned(),
                        ),
                    }],
                });
            }
            if include_path.extension().and_then(|ext| ext.to_str()) != Some("whip") {
                self.active.pop();
                return Err(CompileFailure::Diagnostics {
                    source,
                    diagnostics: vec![Diagnostic {
                        span: include.path.span,
                        message: "only `.whip` includes are supported right now".to_owned(),
                        suggestion: Some("include a `.whip` source library file".to_owned()),
                    }],
                });
            }
            let resolved = path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(include_path);
            let resolved = canonical_or_original(&resolved)?;
            let include_source = fs::read_to_string(&resolved).map_err(CompileFailure::Io)?;
            self.includes.push(IrInclude {
                path: include.path.value.clone(),
                source_hash: Some(stable_hash_hex(&include_source)),
            });
            let included = self.resolve_file(&resolved, false)?;
            if !included.trim().is_empty() {
                combined.push_str(&included);
                if !combined.ends_with('\n') {
                    combined.push('\n');
                }
                combined.push('\n');
            }
        }
        self.active.pop();
        if !is_root {
            self.visited.push(path.to_path_buf());
        }

        combined.push_str(&source);
        Ok(combined)
    }
}

fn canonical_or_original(path: &Path) -> Result<PathBuf, CompileFailure> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(CompileFailure::Io(error))
        }
        Err(_) => Ok(path.to_path_buf()),
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
                "inspect the generated model checks or rerun with Maude available".to_owned(),
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
                    "expected {}, got {}; inspect generated check {} {} {}",
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
        "whipplescript-model-search-{}-{path_hash}.maude",
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

#[derive(Clone, Debug)]
struct MaudeBoolCases {
    true_expr: String,
    false_expr: String,
    error_expr: String,
}

#[derive(Default)]
struct MaudeExprContext {
    scalar_symbols: std::collections::BTreeMap<String, String>,
    query_symbols: std::collections::BTreeMap<String, String>,
}

fn generate_maude_model_search(
    source: &str,
    ir: &IrProgram,
    kernel_path: &Path,
) -> (String, Vec<ExpectedSearch>) {
    let mut effect_symbols = std::collections::BTreeMap::<String, String>::new();
    let mut rule_symbols = std::collections::BTreeMap::<String, String>::new();
    let mut fact_symbols = std::collections::BTreeMap::<String, String>::new();
    let mut graph_symbols = std::collections::BTreeMap::<String, String>::new();
    let mut assertion_symbols = std::collections::BTreeMap::<usize, String>::new();
    let mut expr_context = MaudeExprContext::default();
    for rule in &ir.rules {
        rule_symbols
            .entry(rule.name.clone())
            .or_insert_with(|| maude_symbol("rule", &rule.name));
        for when in &rule.whens {
            if let Some(guard) = &when.guard {
                let _ = maude_bool_cases(&guard.expr, &mut expr_context);
            }
        }
        for (when_index, when) in rule.whens.iter().enumerate() {
            let fact_key = rule_fact_key(&rule.name, when_index, &when.pattern);
            fact_symbols
                .entry(fact_key.clone())
                .or_insert_with(|| maude_symbol("fact", &fact_key));
            let graph_key = rule_graph_key(&rule.name, when_index);
            graph_symbols
                .entry(graph_key.clone())
                .or_insert_with(|| maude_symbol("graph", &graph_key));
        }
        for branch in &rule.metadata.terminal_branches {
            let graph_key = terminal_branch_graph_key(&rule.name, branch);
            graph_symbols
                .entry(graph_key.clone())
                .or_insert_with(|| maude_symbol("graph", &graph_key));
            if let Some(guard) = &branch.guard {
                let _ = maude_bool_cases(&guard.expr, &mut expr_context);
            }
        }
        for effect in &rule.metadata.effects {
            let key = effect_key(&rule.name, &effect.id);
            effect_symbols
                .entry(key.clone())
                .or_insert_with(|| maude_symbol("eff", &key));
        }
    }
    for (index, assertion) in ir.assertions.iter().enumerate() {
        assertion_symbols
            .entry(index)
            .or_insert_with(|| maude_symbol("assertion", &assertion_key(index, assertion)));
        let _ = maude_bool_cases(&assertion.expr.expr, &mut expr_context);
    }

    let mut output = String::new();
    let mut expected = Vec::new();
    output.push_str(&format!("load {}\n\n", kernel_path.display()));
    output.push_str("mod WHIPPLESCRIPT-GENERATED-CHECK is\n");
    output.push_str("  including WHIPPLESCRIPT-KERNEL .\n");
    append_maude_ops(&mut output, effect_symbols.values(), "EffectId");
    append_maude_ops(&mut output, rule_symbols.values(), "RuleId");
    append_maude_ops(&mut output, fact_symbols.values(), "FactId");
    append_maude_ops(&mut output, graph_symbols.values(), "GraphId");
    append_maude_ops(&mut output, assertion_symbols.values(), "AssertionId");
    append_maude_ops(&mut output, expr_context.scalar_symbols.values(), "Scalar");
    append_maude_ops(&mut output, expr_context.query_symbols.values(), "QueryId");
    output.push_str("endm\n\n");

    for rule in &ir.rules {
        for (when_index, when) in rule.whens.iter().enumerate() {
            let Some(guard) = &when.guard else {
                continue;
            };
            let Some(rule_symbol) = rule_symbols.get(&rule.name) else {
                continue;
            };
            let fact_key = rule_fact_key(&rule.name, when_index, &when.pattern);
            let Some(fact_symbol) = fact_symbols.get(&fact_key) else {
                continue;
            };
            let graph_key = rule_graph_key(&rule.name, when_index);
            let Some(graph_symbol) = graph_symbols.get(&graph_key) else {
                continue;
            };
            let cases = maude_bool_cases(&guard.expr, &mut expr_context);
            output.push_str(&format!(
                "--- {}: lowered true guard permits rule commit for `{}`.\n",
                rule.name, guard.source
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) guardExpr({rule_symbol}, {fact_symbol}, {})\n  =>*\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) ruleFired({rule_symbol}, {fact_symbol}, {graph_symbol}) graphReady({graph_symbol}) event(ruleCommitEvt) .\n\n",
                cases.true_expr
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::Solution,
                span: guard.span,
                description: format!("{} true guard commits rule", rule.name),
                upstream: rule.name.clone(),
                predicate: "guard-true",
                downstream: "ruleCommitEvt".to_owned(),
            });

            output.push_str(&format!(
                "--- {}: lowered false guard cannot commit rule for `{}`.\n",
                rule.name, guard.source
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) guardExpr({rule_symbol}, {fact_symbol}, {})\n  =>*\n  event(ruleCommitEvt) .\n\n",
                cases.false_expr
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::NoSolution,
                span: guard.span,
                description: format!("{} false guard cannot commit rule", rule.name),
                upstream: rule.name.clone(),
                predicate: "guard-false",
                downstream: "ruleCommitEvt".to_owned(),
            });

            output.push_str(&format!(
                "--- {}: lowered guard error emits a diagnostic and cannot commit rule for `{}`.\n",
                rule.name, guard.source
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) guardExpr({rule_symbol}, {fact_symbol}, {})\n  =>*\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) diagnostic({rule_symbol}) .\n\n",
                cases.error_expr
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::Solution,
                span: guard.span,
                description: format!("{} guard error emits diagnostic", rule.name),
                upstream: rule.name.clone(),
                predicate: "guard-error",
                downstream: "diagnostic".to_owned(),
            });

            output.push_str(&format!(
                "--- {}: guard error cannot commit rule for `{}`.\n",
                rule.name, guard.source
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) guardExpr({rule_symbol}, {fact_symbol}, {})\n  =>*\n  event(ruleCommitEvt) .\n\n",
                cases.error_expr
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::NoSolution,
                span: guard.span,
                description: format!("{} guard error cannot commit rule", rule.name),
                upstream: rule.name.clone(),
                predicate: "guard-error",
                downstream: "ruleCommitEvt".to_owned(),
            });
        }

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
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  effect({upstream}, queued) dep({upstream}, {predicate}, {downstream}) effect({downstream}, blocked)\n  =>*\n  effect({upstream}, queued) dep({upstream}, {predicate}, {downstream}) effect({downstream}, running) .\n\n"
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
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  effect({upstream}, queued) dep({upstream}, {predicate}, {downstream}) effect({downstream}, blocked)\n  =>*\n  effect({upstream}, {terminal}) dep({upstream}, {predicate}, {downstream}) effect({downstream}, running) .\n\n"
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
                    "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  effect({upstream}, queued) dep({upstream}, {predicate}, {downstream}) effect({downstream}, blocked)\n  =>*\n  effect({upstream}, {non_terminal}) dep({upstream}, {predicate}, {downstream}) effect({downstream}, running) .\n\n"
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
        for branch in &rule.metadata.terminal_branches {
            let Some(tag) = branch.tag.as_deref() else {
                continue;
            };
            let Some(rule_symbol) = rule_symbols.get(&rule.name) else {
                continue;
            };
            let Some(first_when) = rule.whens.first() else {
                continue;
            };
            let first_fact_key = rule_fact_key(&rule.name, 0, &first_when.pattern);
            let Some(fact_symbol) = fact_symbols.get(&first_fact_key) else {
                continue;
            };
            let graph_key = terminal_branch_graph_key(&rule.name, branch);
            let Some(graph_symbol) = graph_symbols.get(&graph_key) else {
                continue;
            };
            let tag_symbol = maude_terminal_tag(tag);
            let miss_tag_symbol = maude_terminal_miss_tag(tag);
            let guard_cases = branch
                .guard
                .as_ref()
                .map(|guard| maude_bool_cases(&guard.expr, &mut expr_context));
            let matching_gate = if let Some(cases) = &guard_cases {
                format!(
                    "terminalBranchGuard({rule_symbol}, {fact_symbol}, {tag_symbol}, {tag_symbol}, {}, {graph_symbol})",
                    cases.true_expr
                )
            } else {
                format!(
                    "terminalBranch({rule_symbol}, {fact_symbol}, {tag_symbol}, {tag_symbol}, {graph_symbol})"
                )
            };
            output.push_str(&format!(
                "--- {}: terminal branch `{tag}` commits only for matching terminal tag.\n",
                rule.name
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) {matching_gate} graph({graph_symbol}, tellEff)\n  =>*\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) ruleFired({rule_symbol}, {fact_symbol}, {graph_symbol}) event(ruleCommitEvt) graphCommitted({graph_symbol}) effect(tellEff, queued) .\n\n"
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::Solution,
                span: branch.pattern_span,
                description: format!(
                    "{} terminal {tag} branch commits on matching tag",
                    rule.name
                ),
                upstream: rule.name.clone(),
                predicate: "terminal-branch-match",
                downstream: "ruleCommitEvt".to_owned(),
            });

            let miss_gate = if let Some(cases) = &guard_cases {
                format!(
                    "terminalBranchGuard({rule_symbol}, {fact_symbol}, {miss_tag_symbol}, {tag_symbol}, {}, {graph_symbol})",
                    cases.true_expr
                )
            } else {
                format!(
                    "terminalBranch({rule_symbol}, {fact_symbol}, {miss_tag_symbol}, {tag_symbol}, {graph_symbol})"
                )
            };
            output.push_str(&format!(
                "--- {}: terminal branch `{tag}` cannot commit for another terminal tag.\n",
                rule.name
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) {miss_gate} graph({graph_symbol}, tellEff)\n  =>*\n  event(ruleCommitEvt) .\n\n"
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::NoSolution,
                span: branch.pattern_span,
                description: format!("{} terminal {tag} branch misses on other tag", rule.name),
                upstream: rule.name.clone(),
                predicate: "terminal-branch-miss",
                downstream: "ruleCommitEvt".to_owned(),
            });

            if let Some(cases) = &guard_cases {
                output.push_str(&format!(
                    "--- {}: terminal branch `{tag}` cannot commit when its branch guard is false.\n",
                    rule.name
                ));
                output.push_str(&format!(
                    "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  fact({fact_symbol}) rule({rule_symbol}, {fact_symbol}, {graph_symbol}) terminalBranchGuard({rule_symbol}, {fact_symbol}, {tag_symbol}, {tag_symbol}, {}, {graph_symbol}) graph({graph_symbol}, tellEff)\n  =>*\n  event(ruleCommitEvt) .\n\n",
                    cases.false_expr
                ));
                expected.push(ExpectedSearch {
                    outcome: ExpectedSearchResult::NoSolution,
                    span: branch.pattern_span,
                    description: format!("{} terminal {tag} false guard cannot commit", rule.name),
                    upstream: rule.name.clone(),
                    predicate: "terminal-branch-guard-false",
                    downstream: "ruleCommitEvt".to_owned(),
                });
            }

            output.push_str(&format!(
                "--- {}: exhaustive terminal branch miss emits a diagnostic.\n",
                rule.name
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  fact({fact_symbol}) exhaustiveTerminal({rule_symbol}, {fact_symbol}, {miss_tag_symbol}, {tag_symbol})\n  =>*\n  fact({fact_symbol}) diagnostic({rule_symbol}) .\n\n"
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::Solution,
                span: branch.pattern_span,
                description: format!("{} terminal {tag} exhaustive miss diagnoses", rule.name),
                upstream: rule.name.clone(),
                predicate: "terminal-exhaustive-miss",
                downstream: "diagnostic".to_owned(),
            });
        }
    }

    for (index, assertion) in ir.assertions.iter().enumerate() {
        let Some(assertion_symbol) = assertion_symbols.get(&index) else {
            continue;
        };
        let cases = maude_bool_cases(&assertion.expr.expr, &mut expr_context);
        for (result, expr) in [
            ("aPass", &cases.true_expr),
            ("aFail", &cases.false_expr),
            ("aError", &cases.error_expr),
        ] {
            output.push_str(&format!(
                "--- assertion {}: lowered {result} cannot mutate runtime state.\n",
                index + 1
            ));
            output.push_str(&format!(
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  assertionExpr({assertion_symbol}, {expr})\n  =>*\n  event(ruleCommitEvt) .\n\n"
            ));
            expected.push(ExpectedSearch {
                outcome: ExpectedSearchResult::NoSolution,
                span: assertion.expr.span,
                description: format!(
                    "assertion {} {result} cannot mutate runtime state",
                    index + 1
                ),
                upstream: format!("assertion{}", index + 1),
                predicate: "assertion-read-only",
                downstream: "ruleCommitEvt".to_owned(),
            });
        }
    }

    (output, expected)
}

fn maude_bool_cases(expr: &Expr, context: &mut MaudeExprContext) -> MaudeBoolCases {
    match expr {
        Expr::Literal(ExprLiteral::Bool(true)) => MaudeBoolCases {
            true_expr: "boolTrue".to_owned(),
            false_expr: "boolFalse".to_owned(),
            error_expr: "exprError".to_owned(),
        },
        Expr::Literal(ExprLiteral::Bool(false)) => MaudeBoolCases {
            true_expr: "boolTrue".to_owned(),
            false_expr: "boolFalse".to_owned(),
            error_expr: "exprError".to_owned(),
        },
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
        } => {
            let inner = maude_bool_cases(expr, context);
            MaudeBoolCases {
                true_expr: format!("notExpr({})", inner.false_expr),
                false_expr: format!("notExpr({})", inner.true_expr),
                error_expr: format!("notExpr({})", inner.error_expr),
            }
        }
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            let left = maude_bool_cases(left, context);
            let right = maude_bool_cases(right, context);
            MaudeBoolCases {
                true_expr: format!("andExpr({}, {})", left.true_expr, right.true_expr),
                false_expr: format!("andExpr({}, {})", left.false_expr, right.true_expr),
                error_expr: format!("andExpr({}, {})", left.error_expr, right.true_expr),
            }
        }
        Expr::Binary {
            op: BinaryOp::Or,
            left,
            right,
        } => {
            let left = maude_bool_cases(left, context);
            let right = maude_bool_cases(right, context);
            MaudeBoolCases {
                true_expr: format!("orExpr({}, {})", left.true_expr, right.false_expr),
                false_expr: format!("orExpr({}, {})", left.false_expr, right.false_expr),
                error_expr: format!("orExpr({}, {})", left.error_expr, right.false_expr),
            }
        }
        Expr::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } => MaudeBoolCases {
            true_expr: maude_eq_true_expr(left, right, context),
            false_expr: maude_eq_false_expr(left, right, context),
            error_expr: "exprError".to_owned(),
        },
        Expr::Binary {
            op: BinaryOp::Ne,
            left,
            right,
        } => MaudeBoolCases {
            true_expr: format!("notExpr({})", maude_eq_false_expr(left, right, context)),
            false_expr: format!(
                "neExpr({}, {})",
                {
                    let lhs = maude_scalar_expr(left, context);
                    lhs
                },
                {
                    let lhs = maude_scalar_expr(left, context);
                    lhs
                }
            ),
            error_expr: "exprError".to_owned(),
        },
        Expr::Binary {
            op: BinaryOp::Lt,
            left,
            right,
        } => maude_order_bool_cases("ltExpr", left, right, context),
        Expr::Binary {
            op: BinaryOp::Le,
            left,
            right,
        } => maude_order_bool_cases("leExpr", left, right, context),
        Expr::Binary {
            op: BinaryOp::Gt,
            left,
            right,
        } => maude_order_bool_cases("gtExpr", left, right, context),
        Expr::Binary {
            op: BinaryOp::Ge,
            left,
            right,
        } => maude_order_bool_cases("geExpr", left, right, context),
        Expr::Binary {
            op: BinaryOp::In,
            left,
            right,
        } => maude_membership_bool_cases(left, right, false, context),
        Expr::Binary {
            op: BinaryOp::NotIn,
            left,
            right,
        } => maude_membership_bool_cases(left, right, true, context),
        Expr::Call { name, args } if name == "exists" && args.len() == 1 => {
            let collection = maude_collection_expr(&args[0], "qOne", context);
            MaudeBoolCases {
                true_expr: format!("existsExpr({collection})"),
                false_expr: format!(
                    "existsExpr({})",
                    maude_collection_expr(&args[0], "qZero", context)
                ),
                error_expr: format!(
                    "existsExpr({})",
                    maude_collection_expr(&args[0], "qError", context)
                ),
            }
        }
        Expr::Call { name, args } if name == "empty" && args.len() == 1 => MaudeBoolCases {
            true_expr: format!(
                "emptyExpr({})",
                maude_collection_expr(&args[0], "qZero", context)
            ),
            false_expr: format!(
                "emptyExpr({})",
                maude_collection_expr(&args[0], "qOne", context)
            ),
            error_expr: format!(
                "emptyExpr({})",
                maude_collection_expr(&args[0], "qError", context)
            ),
        },
        _ => MaudeBoolCases {
            true_expr: "boolTrue".to_owned(),
            false_expr: "boolFalse".to_owned(),
            error_expr: "exprError".to_owned(),
        },
    }
}

fn maude_order_bool_cases(
    op: &str,
    left: &Expr,
    right: &Expr,
    context: &mut MaudeExprContext,
) -> MaudeBoolCases {
    if let Some((count_expr, literal_expr)) = maude_count_number_pair(left, right, true, context) {
        return MaudeBoolCases {
            true_expr: format!("{op}({count_expr}, {literal_expr})"),
            false_expr: maude_false_order_expr(op),
            error_expr: format!("{op}(exprError, {literal_expr})"),
        };
    }
    if let Some((literal_expr, count_expr)) = maude_count_number_pair(right, left, true, context) {
        return MaudeBoolCases {
            true_expr: format!("{op}({literal_expr}, {count_expr})"),
            false_expr: maude_false_order_expr(op),
            error_expr: format!("{op}({literal_expr}, exprError)"),
        };
    }
    let _ = maude_scalar_expr(left, context);
    let _ = maude_scalar_expr(right, context);
    MaudeBoolCases {
        true_expr: maude_true_order_expr(op),
        false_expr: maude_false_order_expr(op),
        error_expr: format!("{op}(exprError, orderHigh)"),
    }
}

fn maude_true_order_expr(op: &str) -> String {
    let (left, right) = match op {
        "ltExpr" | "leExpr" => ("orderLow", "orderHigh"),
        "gtExpr" | "geExpr" => ("orderHigh", "orderLow"),
        _ => ("orderLow", "orderHigh"),
    };
    format!("{op}({left}, {right})")
}

fn maude_false_order_expr(op: &str) -> String {
    let (left, right) = match op {
        "ltExpr" | "leExpr" => ("orderHigh", "orderLow"),
        "gtExpr" | "geExpr" => ("orderLow", "orderHigh"),
        _ => ("orderHigh", "orderLow"),
    };
    format!("{op}({left}, {right})")
}

fn maude_membership_bool_cases(
    item: &Expr,
    collection: &Expr,
    negated: bool,
    context: &mut MaudeExprContext,
) -> MaudeBoolCases {
    let item_expr = maude_scalar_expr(item, context);
    let present_collection = maude_collection_with_member(collection, &item_expr, true, context);
    let missing_collection = maude_collection_with_member(collection, &item_expr, false, context);
    let true_expr = format!("inExpr({item_expr}, {present_collection})");
    let false_expr = format!("inExpr({item_expr}, {missing_collection})");
    let error_expr = format!("inExpr(exprError, {present_collection})");
    if negated {
        MaudeBoolCases {
            true_expr: format!("notExpr({false_expr})"),
            false_expr: format!("notExpr({true_expr})"),
            error_expr,
        }
    } else {
        MaudeBoolCases {
            true_expr,
            false_expr,
            error_expr,
        }
    }
}

fn maude_eq_true_expr(left: &Expr, right: &Expr, context: &mut MaudeExprContext) -> String {
    let pair = maude_equal_pair(left, right, true, context);
    format!("eqExpr({}, {})", pair.0, pair.1)
}

fn maude_eq_false_expr(left: &Expr, right: &Expr, context: &mut MaudeExprContext) -> String {
    if let Some((query_expr, number_expr)) = maude_count_number_pair(left, right, false, context) {
        return format!("eqExpr({query_expr}, {number_expr})");
    }
    if let Some((number_expr, query_expr)) = maude_count_number_pair(right, left, false, context) {
        return format!("eqExpr({number_expr}, {query_expr})");
    }
    let lhs = maude_scalar_expr(left, context);
    format!("notExpr(eqExpr({lhs}, {lhs}))")
}

fn maude_equal_pair(
    left: &Expr,
    right: &Expr,
    equal: bool,
    context: &mut MaudeExprContext,
) -> (String, String) {
    if let Some((query_expr, number_expr)) = maude_count_number_pair(left, right, equal, context) {
        return (query_expr, number_expr);
    }
    if let Some((number_expr, query_expr)) = maude_count_number_pair(right, left, equal, context) {
        return (number_expr, query_expr);
    }

    let lhs = maude_scalar_expr(left, context);
    if equal {
        return (lhs.clone(), lhs);
    }
    (lhs.clone(), lhs)
}

fn maude_count_number_pair(
    maybe_count: &Expr,
    maybe_number: &Expr,
    equal: bool,
    context: &mut MaudeExprContext,
) -> Option<(String, String)> {
    let Expr::Call { name, args } = maybe_count else {
        return None;
    };
    if name != "count" || args.len() != 1 {
        return None;
    }
    let Expr::Literal(ExprLiteral::Number(number)) = maybe_number else {
        return None;
    };
    let expected = maude_count_literal(number)?;
    let cardinality = if equal {
        maude_query_cardinality_for_count(number)
    } else if number == "0" {
        "qOne"
    } else {
        "qZero"
    };
    let count = format!(
        "countExpr({})",
        maude_collection_expr(&args[0], cardinality, context)
    );
    Some((count, expected))
}

fn maude_count_literal(number: &str) -> Option<String> {
    match number {
        "0" => Some("countZero".to_owned()),
        "1" => Some("countOne".to_owned()),
        "2" => Some("countTwo".to_owned()),
        "3" => Some("countThree".to_owned()),
        _ => number
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 3)
            .map(|_| "countMany".to_owned()),
    }
}

fn maude_query_cardinality_for_count(number: &str) -> &'static str {
    match number {
        "0" => "qZero",
        "1" => "qOne",
        "2" => "qTwo",
        "3" => "qThree",
        _ => "qMany",
    }
}

fn maude_scalar_expr(expr: &Expr, context: &mut MaudeExprContext) -> String {
    match expr {
        Expr::Literal(ExprLiteral::Bool(true)) => "boolTrue".to_owned(),
        Expr::Literal(ExprLiteral::Bool(false)) => "boolFalse".to_owned(),
        Expr::Literal(ExprLiteral::Number(number)) => {
            maude_count_literal(number).unwrap_or_else(|| maude_scalar_symbol(expr, context))
        }
        Expr::Index { target, key } => {
            let key = maude_scalar_expr(key, context);
            let target = maude_index_target_expr(target, &key, expr, context);
            format!("indexExpr({target}, {key})")
        }
        Expr::Array(_) | Expr::Object(_) => maude_value_expr(expr, context),
        Expr::Call { name, args } if name == "count" && args.len() == 1 => {
            format!(
                "countExpr({})",
                maude_collection_expr(&args[0], "qOne", context)
            )
        }
        _ => format!("scalar({})", maude_scalar_symbol(expr, context)),
    }
}

fn maude_value_expr(expr: &Expr, context: &mut MaudeExprContext) -> String {
    match expr {
        Expr::Array(items) => maude_array_literal_expr(items, context),
        Expr::Object(fields) => maude_object_literal_expr(fields, context),
        Expr::Index { target, key } => {
            let key = maude_scalar_expr(key, context);
            let target = maude_index_target_expr(target, &key, expr, context);
            format!("indexExpr({target}, {key})")
        }
        Expr::Query { .. } => maude_collection_expr(expr, "qOne", context),
        _ => maude_scalar_expr(expr, context),
    }
}

fn maude_array_literal_expr(items: &[Expr], context: &mut MaudeExprContext) -> String {
    if items.is_empty() {
        return "arrayEmpty".to_owned();
    }
    format!("arrayOf({})", maude_expr_list(items, context))
}

fn maude_object_literal_expr(fields: &[ExprObjectField], context: &mut MaudeExprContext) -> String {
    if fields.is_empty() {
        return "objectEmpty".to_owned();
    }
    format!("objectOf({})", maude_entry_list(fields, context))
}

fn maude_expr_list(items: &[Expr], context: &mut MaudeExprContext) -> String {
    let Some((first, rest)) = items.split_first() else {
        return "exprNil".to_owned();
    };
    format!(
        "exprCons({}, {})",
        maude_scalar_expr(first, context),
        maude_expr_list(rest, context)
    )
}

fn maude_entry_list(fields: &[ExprObjectField], context: &mut MaudeExprContext) -> String {
    let Some((first, rest)) = fields.split_first() else {
        return "entryNil".to_owned();
    };
    format!(
        "entryCons(entry({}, {}), {})",
        maude_field_key_expr(&first.key, context),
        maude_scalar_expr(&first.value, context),
        maude_entry_list(rest, context)
    )
}

fn maude_collection_expr(
    expr: &Expr,
    cardinality: &'static str,
    context: &mut MaudeExprContext,
) -> String {
    match expr {
        Expr::Query { guard, .. } => {
            let query = maude_query_symbol(expr, context);
            if let Some(guard) = guard {
                let guard_cases = maude_bool_cases(guard, context);
                format!(
                    "queryFilter({query}, {}, {cardinality})",
                    guard_cases.true_expr
                )
            } else {
                format!("query({query}, {cardinality})")
            }
        }
        Expr::Array(items) => {
            let _ = cardinality;
            maude_array_literal_expr(items, context)
        }
        Expr::Object(fields) => {
            let _ = cardinality;
            maude_object_literal_expr(fields, context)
        }
        Expr::Index { .. } => maude_value_expr(expr, context),
        _ => format!(
            "query({}, {cardinality})",
            maude_query_symbol(expr, context)
        ),
    }
}

fn maude_collection_with_member(
    collection: &Expr,
    item_expr: &str,
    present: bool,
    context: &mut MaudeExprContext,
) -> String {
    match collection {
        Expr::Array(_) => {
            if present {
                format!("arrayHas({item_expr})")
            } else {
                format!("arrayMissing({item_expr})")
            }
        }
        Expr::Object(_) => {
            if present {
                format!(
                    "objectHas({item_expr}, scalar({}))",
                    maude_scalar_symbol(collection, context)
                )
            } else {
                format!("objectMissing({item_expr})")
            }
        }
        Expr::Path(_) | Expr::Index { .. } => {
            if present {
                format!(
                    "mapHas({item_expr}, scalar({}))",
                    maude_scalar_symbol(collection, context)
                )
            } else {
                format!("mapMissing({item_expr})")
            }
        }
        _ => {
            if present {
                format!("arrayHas({item_expr})")
            } else {
                format!("arrayMissing({item_expr})")
            }
        }
    }
}

fn maude_index_target_expr(
    target: &Expr,
    key_expr: &str,
    index_expr: &Expr,
    context: &mut MaudeExprContext,
) -> String {
    match target {
        Expr::Object(fields) => {
            let _ = key_expr;
            maude_object_literal_expr(fields, context)
        }
        Expr::Index { .. } => maude_value_expr(target, context),
        _ => format!(
            "mapHas({key_expr}, scalar({}))",
            maude_scalar_symbol(index_expr, context)
        ),
    }
}

fn maude_field_key_expr(key: &str, context: &mut MaudeExprContext) -> String {
    let expr = Expr::Literal(ExprLiteral::String(key.to_owned()));
    maude_scalar_expr(&expr, context)
}

fn maude_scalar_symbol(expr: &Expr, context: &mut MaudeExprContext) -> String {
    let key = expr.to_snapshot();
    context
        .scalar_symbols
        .entry(key.clone())
        .or_insert_with(|| maude_symbol("scalar", &key))
        .clone()
}

fn maude_query_symbol(expr: &Expr, context: &mut MaudeExprContext) -> String {
    let key = expr.to_snapshot();
    context
        .query_symbols
        .entry(key.clone())
        .or_insert_with(|| maude_symbol("query", &key))
        .clone()
}

fn append_maude_ops<'a>(
    output: &mut String,
    symbols: impl IntoIterator<Item = &'a String>,
    sort: &str,
) {
    let symbols = symbols.into_iter().collect::<Vec<_>>();
    if symbols.is_empty() {
        return;
    }
    output.push_str("  ops\n");
    for symbol in symbols {
        output.push_str("    ");
        output.push_str(symbol);
        output.push('\n');
    }
    output.push_str(&format!("    : -> {sort} .\n"));
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

fn rule_fact_key(rule_name: &str, when_index: usize, pattern: &str) -> String {
    format!("{rule_name}:when{when_index}:{pattern}")
}

fn rule_graph_key(rule_name: &str, when_index: usize) -> String {
    format!("{rule_name}:when{when_index}:graph")
}

fn terminal_branch_graph_key(
    rule_name: &str,
    branch: &whipplescript_parser::IrTerminalCaseBranch,
) -> String {
    let tag = branch.tag.as_deref().unwrap_or("_");
    format!("{rule_name}:terminal:{tag}:{}", branch.body_hash)
}

fn assertion_key(index: usize, assertion: &whipplescript_parser::IrAssertion) -> String {
    format!("{index}:{}", assertion.expr.source)
}

fn maude_symbol(prefix: &str, value: &str) -> String {
    format!("{prefix}{:016x}", stable_hash(value))
}

fn maude_predicate(predicate: &whipplescript_parser::DependencyPredicate) -> &'static str {
    match predicate {
        whipplescript_parser::DependencyPredicate::Succeeds => "succeeds",
        whipplescript_parser::DependencyPredicate::Fails => "fails",
        whipplescript_parser::DependencyPredicate::Completes => "completes",
    }
}

fn maude_terminal_tag(tag: &str) -> &'static str {
    match tag {
        "Completed" => "terminalCompleted",
        "Failed" => "terminalFailed",
        "TimedOut" => "terminalTimedOut",
        "Cancelled" => "terminalCancelled",
        _ => "terminalCompleted",
    }
}

fn maude_terminal_miss_tag(tag: &str) -> &'static str {
    match tag {
        "Completed" => "terminalFailed",
        "Failed" => "terminalCompleted",
        "TimedOut" => "terminalCompleted",
        "Cancelled" => "terminalCompleted",
        _ => "terminalFailed",
    }
}

fn satisfying_terminal(predicate: &whipplescript_parser::DependencyPredicate) -> &'static str {
    match predicate {
        whipplescript_parser::DependencyPredicate::Succeeds => "completed",
        whipplescript_parser::DependencyPredicate::Fails => "failed",
        whipplescript_parser::DependencyPredicate::Completes => "completed",
    }
}

fn non_satisfying_terminal(
    predicate: &whipplescript_parser::DependencyPredicate,
) -> Option<&'static str> {
    match predicate {
        whipplescript_parser::DependencyPredicate::Succeeds => Some("failed"),
        whipplescript_parser::DependencyPredicate::Fails => Some("completed"),
        whipplescript_parser::DependencyPredicate::Completes => None,
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
        StoreError::CapacityBlocked { reason, .. } => reason,
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
        "revision_epoch": instance.revision_epoch,
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
        TraceEvent::ProviderDiagnostic {
            run_id,
            effect_id,
            provider,
            status,
            summary,
            diagnostics_json,
        } => json!({
            "type": "provider_diagnostic",
            "run_id": run_id,
            "effect_id": effect_id,
            "provider": provider,
            "status": trace_status_name(status),
            "summary": summary,
            "diagnostics": json_from_str(diagnostics_json),
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
        "program_version_id": effect.program_version_id,
        "revision_epoch": effect.revision_epoch,
        "profile": effect.profile,
        "required_capabilities": json_from_str(&effect.required_capabilities_json),
        "policy_block_reason": effect.policy_block_reason,
        "cancel_requested": effect.cancel_requested,
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

fn workflow_invocation_to_json(invocation: &WorkflowInvocationView) -> Value {
    json!({
        "invocation_id": invocation.invocation_id,
        "parent_instance_id": invocation.parent_instance_id,
        "parent_effect_id": invocation.parent_effect_id,
        "parent_program_version_id": invocation.parent_program_version_id,
        "parent_revision_epoch": invocation.parent_revision_epoch,
        "child_instance_id": invocation.child_instance_id,
        "child_program_version_id": invocation.child_program_version_id,
        "child_revision_epoch": invocation.child_revision_epoch,
        "target_workflow": invocation.target_workflow,
        "input": json_from_str(&invocation.input_json),
        "status": invocation.status,
        "terminal_event_id": invocation.terminal_event_id,
        "source_span": invocation.source_span_json.as_deref().map(json_from_str),
        "created_at": invocation.created_at,
        "updated_at": invocation.updated_at,
    })
}

fn workflow_revision_to_json(revision: &WorkflowRevisionView) -> Value {
    json!({
        "revision_id": revision.revision_id,
        "instance_id": revision.instance_id,
        "epoch": revision.epoch,
        "from_version_id": revision.from_version_id,
        "to_version_id": revision.to_version_id,
        "activated_by_event_id": revision.activated_by_event_id,
        "activation_policy": json_from_str(&revision.activation_policy_json),
        "cancellation_policy": revision.cancellation_policy,
        "status": revision.status,
        "idempotency_key": revision.idempotency_key,
        "created_at": revision.created_at,
        "activated_at": revision.activated_at,
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

fn diagnostic_to_json(diagnostic: &DiagnosticView) -> Value {
    json!({
        "diagnostic_id": diagnostic.diagnostic_id,
        "instance_id": diagnostic.instance_id,
        "program_id": diagnostic.program_id,
        "program_version_id": diagnostic.program_version_id,
        "severity": diagnostic.severity,
        "code": diagnostic.code,
        "message": diagnostic.message,
        "source_span": diagnostic.source_span_json.as_deref().map(json_from_str),
        "subject_type": diagnostic.subject_type,
        "subject_id": diagnostic.subject_id,
        "event_id": diagnostic.event_id,
        "effect_id": diagnostic.effect_id,
        "run_id": diagnostic.run_id,
        "assertion_id": diagnostic.assertion_id,
        "evidence_ids": json_from_str(&diagnostic.evidence_ids_json),
        "artifact_ids": json_from_str(&diagnostic.artifact_ids_json),
        "causation_id": diagnostic.causation_id,
        "correlation_id": diagnostic.correlation_id,
        "idempotency_key": diagnostic.idempotency_key,
        "created_at": diagnostic.created_at,
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
        "workflow_terminal": workflow_terminal_summary(&status.recent_events),
        "fact_count": status.fact_count,
        "queued_effect_count": status.queued_effect_count,
        "blocked_effect_count": status.blocked_effect_count,
        "active_run_count": status.active_run_count,
        "failure_count": status.failure_count,
        "cancellation_request_count": status.cancellation_request_count,
        "revisions": status.revisions.iter().map(workflow_revision_to_json).collect::<Vec<_>>(),
        "workflow_invocations": {
            "parent": status.parent_invocation.as_ref().map(workflow_invocation_to_json),
            "children": status.child_invocations.iter().map(workflow_invocation_to_json).collect::<Vec<_>>(),
        },
        "recent_events": status.recent_events.iter().map(event_to_json).collect::<Vec<_>>(),
    })
}

fn workflow_terminal_summary(events: &[EventView]) -> Option<Value> {
    events
        .iter()
        .rev()
        .find(|event| {
            event.event_type == "workflow.completed" || event.event_type == "workflow.failed"
        })
        .map(|event| {
            let payload = json_from_str(&event.payload_json);
            json!({
                "event_id": event.event_id,
                "event_type": event.event_type,
                "status": payload
                    .get("workflow_status")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| {
                        if event.event_type == "workflow.completed" {
                            "completed"
                        } else {
                            "failed"
                        }
                    }),
                "name": payload.get("terminal_name").and_then(Value::as_str),
                "payload": payload.get("payload").cloned().unwrap_or(Value::Null),
            })
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
        assert_eq!(options.root, None);
        assert_eq!(options.paths, vec!["examples/ralph.whip"]);
    }

    #[test]
    fn parses_check_root_option() {
        let options = CheckOptions::parse(&[
            "--root".to_owned(),
            "Review".to_owned(),
            "examples/phase-review.whip".to_owned(),
        ])
        .expect("check options parse");

        assert_eq!(options.root.as_deref(), Some("Review"));
        assert_eq!(options.paths, vec!["examples/phase-review.whip"]);
    }

    #[test]
    fn parses_trace_check_option() {
        let options =
            TraceOptions::parse(&["ins_123".to_owned(), "--check".to_owned()]).expect("parse");

        assert_eq!(options.instance_id, "ins_123");
        assert!(options.check);
    }

    #[test]
    fn assertion_reports_typed_error_for_invalid_external_duration_value() {
        let source = r#"
workflow DurationAssertionExternalInvalid

class Window {
  elapsed duration
  limit duration
}

assert exists(Window where elapsed < limit)
"#;
        let ir = whipplescript_parser::compile_program(source)
            .ir
            .expect("compile");
        let facts = vec![FactView {
            fact_id: "fact-invalid-duration".to_owned(),
            name: "Window".to_owned(),
            key: "fact-invalid-duration".to_owned(),
            value_json: r#"{"elapsed":"bad-duration","limit":"PT1H"}"#.to_owned(),
            provenance_class: "external".to_owned(),
        }];

        let assertions = eval_assertions(&ir, &facts, &[], None);

        assert_eq!(assertions.len(), 1);
        assert_eq!(assertions[0].status, AssertionStatus::Error);
        assert!(assertions[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("invalid duration value `bad-duration`")));
    }

    #[test]
    fn lowering_consumes_matched_fact_binding() {
        let source = r#"
workflow ConsumeTask

class Task {
  status "queued"
}

class Done {
  status "done"
}

rule finish
  when Task as task where task.status == "queued"
=> {
  consume task
  record Done {
    status "done"
  }
}
"#;
        let ir = whipplescript_parser::compile_program(source)
            .ir
            .expect("compile");
        let fact = FactView {
            fact_id: "fact-task".to_owned(),
            name: "Task".to_owned(),
            key: "Task:queued".to_owned(),
            value_json: r#"{"status":"queued"}"#.to_owned(),
            provenance_class: "rule".to_owned(),
        };
        let facts = vec![fact];
        let effects = Vec::new();
        let ready = ready_contexts(&ir, &ir.rules[0], &facts, &effects, None);
        assert_eq!(ready.contexts.len(), 1);

        let lowering = lower_rule(
            &ir,
            &ir.rules[0],
            &ready.contexts[0],
            &facts,
            &effects,
            None,
        );

        assert_eq!(lowering.consumed_fact_ids, vec!["fact-task"]);
        assert_eq!(lowering.facts.len(), 1);
    }

    #[test]
    fn lowering_expands_done_record_from_and_then_chain() {
        let body = r#"
  tell codex as turn "write"
  then tell codex as review "review"
  then done task -> record Done from task {
    topic
    turn turn
    review review
    status "done"
  }
"#;
        let desugared = desugar_then_chains(body);
        assert!(desugared.contains("after turn succeeds"));
        assert!(desugared.contains("after review succeeds"));

        let context = RuleContext {
            bindings: vec![
                (
                    "task".to_owned(),
                    FactView {
                        fact_id: "fact-task".to_owned(),
                        name: "Task".to_owned(),
                        key: "Task:queued".to_owned(),
                        value_json: r#"{"topic":"rain","status":"queued"}"#.to_owned(),
                        provenance_class: "rule".to_owned(),
                    },
                ),
                (
                    "turn".to_owned(),
                    FactView {
                        fact_id: "turn".to_owned(),
                        name: "turn".to_owned(),
                        key: "turn".to_owned(),
                        value_json: r#"{"summary":"wrote"}"#.to_owned(),
                        provenance_class: "effect".to_owned(),
                    },
                ),
                (
                    "review".to_owned(),
                    FactView {
                        fact_id: "review".to_owned(),
                        name: "review".to_owned(),
                        key: "review".to_owned(),
                        value_json: r#"{"summary":"reviewed"}"#.to_owned(),
                        provenance_class: "effect".to_owned(),
                    },
                ),
            ],
            ..RuleContext::default()
        };
        let final_after = after_blocks(&desugared)
            .into_iter()
            .last()
            .expect("final after block");
        let record = top_level_record_blocks(&final_after.body)
            .into_iter()
            .next()
            .expect("record block");
        let value = Value::Object(parse_record_fields(
            &record.body,
            &context,
            record.from_binding.as_deref(),
        ));
        assert_eq!(value.get("topic").and_then(Value::as_str), Some("rain"));
        assert_eq!(value.get("status").and_then(Value::as_str), Some("done"));
    }

    #[test]
    fn selects_failed_terminal_case_branch_and_binds_payload() {
        let body = r#"
case classification {
  Completed result => {
    record TerminalRoute {
      branch "completed"
      detail result.summary
    }
  }
  Failed failure => {
    record TerminalRoute {
      branch "failed"
      detail failure.reason
    }
  }
}
"#;
        let mut context = RuleContext::default();
        push_effect_binding(
            &mut context,
            "classification",
            "effect",
            json!({
                "tag": "Failed",
                "status": "failed",
                "error": {
                    "reason": "fixture coerce failure"
                }
            }),
        );

        let (selected, selected_context, reports) = selected_rule_body(body, &context);

        assert!(selected.contains("branch \"failed\""));
        assert!(!selected.contains("branch \"completed\""));
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, BranchStatus::Matched);
        assert_eq!(
            parse_field_value("failure.reason", &selected_context),
            Value::String("fixture coerce failure".to_owned())
        );
    }

    #[test]
    fn terminal_case_guard_error_selects_no_sibling_branch() {
        let body = r#"
case classification {
  Completed result where result.summary => {
    askHuman "should not commit"
  }
  Failed failure => {
    askHuman "failed sibling"
  }
}
"#;
        let mut context = RuleContext::default();
        push_effect_binding(
            &mut context,
            "classification",
            "effect",
            json!({
                "tag": "Completed",
                "status": "completed",
                "value": {
                    "summary": "Fixture classification"
                }
            }),
        );

        let (selected, _selected_context, reports) = selected_rule_body(body, &context);

        assert!(!selected.contains("askHuman"));
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, BranchStatus::Error);
        assert!(reports[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("did not evaluate to bool")));
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
    fn renders_provider_diagnostic_trace_json() {
        let record = TraceRecord {
            sequence: 7,
            event: TraceEvent::ProviderDiagnostic {
                run_id: "run-1".to_owned(),
                effect_id: "effect-1".to_owned(),
                provider: "fixture".to_owned(),
                status: EffectStatus::Failed,
                summary: "provider failed".to_owned(),
                diagnostics_json: json!({"stage": "tool", "retryable": false}).to_string(),
            },
        };

        let rendered = trace_record_to_json(&record);
        let event = rendered.get("event").expect("event");
        assert_eq!(
            event.get("type").and_then(Value::as_str),
            Some("provider_diagnostic")
        );
        assert_eq!(
            event.get("provider").and_then(Value::as_str),
            Some("fixture")
        );
        assert_eq!(event.get("status").and_then(Value::as_str), Some("failed"));
        assert_eq!(
            event
                .get("diagnostics")
                .and_then(|diagnostics| diagnostics.get("stage"))
                .and_then(Value::as_str),
            Some("tool")
        );
    }

    #[test]
    fn generates_model_searches_for_effect_dependencies() {
        let source = include_str!("../../../examples/loft-worker-with-review.whip");
        let compiled = whipplescript_parser::compile_program(source);
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
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "succeeds")
                .count(),
            3
        );
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "fails")
                .count(),
            3
        );
    }

    #[test]
    fn generates_model_searches_for_guards_and_assertions() {
        let source = r#"
workflow GeneratedChecks

class Task {
  priority int
  status string
  labels string[]
  metadata map<string>
}

class Result {
  status string
  metadata map<string>
}

assert empty(Result)
assert count(Result) == 0
assert count(Result where status == "accepted") >= 0
assert empty(Result where status not in ["accepted", "queued"])
assert "urgent" in ["urgent", "later"]

rule accept
  when Task as task where task.status == "queued" && task.priority >= 1 && "urgent" in task.labels && task.metadata["phase"] == "kernel" && empty(Result where metadata["phase"] == "done")
=> {
  record Result {
    status "accepted"
    metadata { phase task.metadata["phase"] }
  }
}
"#;
        let compiled = whipplescript_parser::compile_program(source);
        let ir = compiled
            .ir
            .unwrap_or_else(|| panic!("source compiles: {:?}", compiled.diagnostics));
        let (maude, expected) =
            generate_maude_model_search(source, &ir, Path::new("/tmp/kernel.maude"));

        assert_eq!(expected.len(), 19);
        assert!(maude.contains("guardExpr("));
        assert!(maude.contains("assertionExpr("));
        assert!(maude.contains("andExpr("));
        assert!(maude.contains("eqExpr("));
        assert!(maude.contains("geExpr("));
        assert!(maude.contains("inExpr("));
        assert!(maude.contains("indexExpr("));
        assert!(maude.contains("arrayHas("));
        assert!(maude.contains("mapHas("));
        assert!(maude.contains("queryFilter("));
        assert!(maude.contains("emptyExpr(query("));
        assert!(maude.contains("countExpr(query("));
        assert!(expected.iter().any(|result| {
            result.description == "accept true guard commits rule"
                && result.outcome == ExpectedSearchResult::Solution
        }));
        assert!(expected.iter().any(|result| {
            result.description == "accept false guard cannot commit rule"
                && result.outcome == ExpectedSearchResult::NoSolution
        }));
        assert!(expected.iter().any(|result| {
            result.description == "accept guard error emits diagnostic"
                && result.outcome == ExpectedSearchResult::Solution
        }));
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "assertion-read-only"
                    && result.outcome == ExpectedSearchResult::NoSolution)
                .count(),
            15
        );
    }

    #[test]
    fn generates_model_searches_for_terminal_branches() {
        let source = include_str!("../../../examples/terminal-output-union.whip");
        let compiled = whipplescript_parser::compile_program(source);
        let ir = compiled.ir.expect("example compiles");
        let (maude, expected) =
            generate_maude_model_search(source, &ir, Path::new("/tmp/kernel.maude"));

        assert!(maude.contains("terminalBranch("));
        assert!(maude.contains("exhaustiveTerminal("));
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "terminal-branch-match")
                .count(),
            4
        );
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "terminal-branch-miss")
                .count(),
            4
        );
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "terminal-exhaustive-miss")
                .count(),
            4
        );
    }

    #[test]
    fn generates_model_searches_for_guarded_terminal_branch_misses() {
        let source = include_str!("../../../examples/terminal-output-union.whip");
        let compiled = whipplescript_parser::compile_program(source);
        let mut ir = compiled
            .ir
            .unwrap_or_else(|| panic!("source compiles: {:?}", compiled.diagnostics));
        let branch = ir.rules[0]
            .metadata
            .terminal_branches
            .first_mut()
            .expect("terminal branch");
        branch.guard = Some(whipplescript_parser::IrExpression {
            source: "true".to_owned(),
            expr: parse_expression("true").expect("guard parses"),
            span: branch.pattern_span,
        });
        let (maude, expected) =
            generate_maude_model_search(source, &ir, Path::new("/tmp/kernel.maude"));

        assert!(maude.contains("terminalBranchGuard("));
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "terminal-branch-guard-false")
                .count(),
            1
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
        let compiled = whipplescript_parser::compile_program(source);
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
    fn generated_model_search_runs_lowered_expression_fixture() {
        if find_executable_in_path(&["maude"], &path_value()).is_none() {
            return;
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let kernel_path =
            fs::canonicalize(root.join("models/maude/kernel.maude")).expect("kernel path resolves");
        let source = r#"
workflow GeneratedExpressionChecks

class Task {
  status string
}

class Result {
  status string
}

assert empty(Result)
assert count(Result) == 0

rule accept
  when Task as task where task.status == "queued" && empty(Result)
=> {
  record Result {
    status "accepted"
  }
}
"#;
        let compiled = whipplescript_parser::compile_program(source);
        let ir = compiled.ir.expect("source compiles");
        let (maude, expected) = generate_maude_model_search(source, &ir, &kernel_path);
        assert!(!expected.is_empty());

        let output = run_maude_source("generated-expression-check-fixture", &maude)
            .expect("generated expression Maude fixture runs");
        let actual = extract_maude_search_results(&output.stdout);
        assert_eq!(
            actual,
            expected
                .iter()
                .map(|expected| expected.outcome)
                .collect::<Vec<_>>(),
            "{}",
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
            std::env::temp_dir().join(format!("whipplescript-doctor-test-{}", std::process::id()));
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
