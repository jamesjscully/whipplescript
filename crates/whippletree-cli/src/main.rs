use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use serde_json::{json, Value};
use whippletree_kernel::{
    idempotency_key,
    trace::{
        check_trace, DependencyEdge, DependencyPredicate, EffectStatus, TraceEvent, TraceRecord,
    },
    ProgramVersionInput, RuntimeKernel,
};
use whippletree_parser::{Diagnostic, IrProgram, SourceSpan};
use whippletree_store::{
    EffectView, EventView, EvidenceLinkView, EvidenceView, FactView, HumanAnswer, InboxItemView,
    InstanceView, RetryEffect, RunView, SqliteStore, StatusView, StoreError,
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
    println!("commands: check, compile, run, instances, status, log, facts, effects, runs");
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
    let input_json = options.input_json.as_deref().unwrap_or("{}");
    let input_value = match serde_json::from_str::<Value>(input_json) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("invalid `--input` JSON: {error}");
            return ExitCode::from(2);
        }
    };
    let input_json = input_value.to_string();
    let (source, ir) = match compile_source_path(path) {
        Ok(compiled) => compiled,
        Err(error) => return report_compile_failure(path, error),
    };
    let snapshot = ir.to_snapshot();
    let store = match open_store(&options.store_path) {
        Ok(store) => store,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
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
            return ExitCode::FAILURE;
        }
    };
    let instance_id = match kernel.create_instance(&version, &input_json) {
        Ok(instance_id) => instance_id,
        Err(error) => {
            eprintln!("failed to create instance: {}", store_error(error));
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = kernel.ingest_external_event(
        &instance_id,
        "external.started",
        &input_json,
        Some(&idempotency_key(&[&instance_id, "external.started"])),
    ) {
        eprintln!("failed to write start event: {}", store_error(error));
        return ExitCode::FAILURE;
    }

    if options.json {
        emit_json(json!({
            "instance_id": instance_id,
            "program_id": version.program_id,
            "version_id": version.version_id,
            "workflow": ir.workflow,
            "store": options.store_path.display().to_string(),
        }))
    } else {
        println!("started {}", instance_id);
        println!("workflow {}", ir.workflow);
        println!("store {}", options.store_path.display());
        ExitCode::SUCCESS
    }
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
