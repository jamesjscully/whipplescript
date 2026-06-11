use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::Duration,
};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use whipplescript_kernel::{
    claude_agent_sdk::{ClaudeAgentSdkAdapter, ClaudeAgentSdkClient, StdioClaudeAgentSdkTransport},
    codex_app_server::{CodexAppServerAdapter, CodexAppServerClient, StdioCodexAppServerTransport},
    coerce::{BamlCoerceRequest, FakeBamlClient},
    harness::{CommandAgentHarness, CommandLaunchPlan},
    idempotency_key,
    loft::{FakeLoftClient, LoftAction, LoftEffectRequest},
    pi_rpc::{PiRpcAdapter, PiRpcClient, StdioPiRpcTransport},
    program_analysis_summary_json,
    provider::{
        builtin_provider_capabilities, validate_provider_binding, validate_provider_binding_json,
        AdapterSurface, CancellationDepth, NativeProviderAdapter, NativeProviderArtifactRef,
        NativeProviderBoundaryError, NativeProviderCancellation, NativeProviderEvent,
        NativeProviderEventKind, NativeProviderTurnRequest, ProviderBindingConfig,
        ProviderCapability, ProviderKind, ProviderValidationResult, ProviderValidationStatus,
    },
    trace::{
        check_trace, DependencyEdge, DependencyPredicate, EffectStatus, TraceEvent, TraceRecord,
    },
    AgentTurnExecution, BamlCoerceExecution, HumanAskExecution, LoftEffectExecution,
    ProgramVersionInput, RuntimeKernel,
};
use whipplescript_parser::{
    parse_duration_seconds, parse_expression, parse_time_epoch_seconds, BinaryOp,
    DependencyPredicate as IrDependencyPredicate, Diagnostic, Expr, ExprLiteral, ExprObjectField,
    IrEffectKind, IrEffectNode, IrInclude, IrPrimitiveType, IrProgram, IrProjectionRead, IrRule,
    IrSchema, IrType, IrWorkflowContract, IrWorkflowContractKind, Item, QueryKind, SourceSpan,
    UnaryOp,
};
use whipplescript_store::{
    ArtifactView, CapabilityBinding, CapabilitySchemaRegistration, ClaimableEffect, DerivedFact,
    DiagnosticRecord, DiagnosticView, EffectCancellation, EffectCancellationRequest,
    EffectCompletion, EffectView, EventView, EvidenceLink, EvidenceLinkView, EvidenceRecord,
    EvidenceView, FactView, HumanAnswer, InboxItemView, InstanceView, NewEffect,
    NewEffectDependency, NewEvent, NewFact, NewInboxItem, NewWorkflowInvocation,
    ProviderValidationEvidence, RetryEffect, RevisionActivation, RevisionCancellationImpact,
    RevisionCandidate, RevisionCompatibilityDiagnostic, RevisionCompatibilityReport, RuleCommit,
    RuleCommitRevisionGuard, RunStart, RunView, SqliteStore, StatusView, StoreError,
    WorkflowInvocationView, WorkflowRevisionView, WorkflowTerminal, WorkflowTerminalKind,
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

    if options
        .args
        .iter()
        .any(|arg| arg == "--help" || arg == "-h")
    {
        match options.command.as_deref().and_then(command_usage) {
            Some(usage) => println!("{usage}"),
            None => print_usage(),
        }
        return ExitCode::SUCCESS;
    }
    if options.command.as_deref() == Some("help") {
        match options
            .args
            .first()
            .map(String::as_str)
            .and_then(command_usage)
        {
            Some(usage) => println!("{usage}"),
            None => print_usage(),
        }
        return ExitCode::SUCCESS;
    }

    match options.command.as_deref() {
        Some("doctor") => doctor(&options),
        Some("check") => check(&options),
        Some("compile") => compile(&options),
        Some("run") => run(&options),
        Some("revise") => revise(&options),
        Some("step") => step(&options),
        Some("worker") => worker(&options),
        Some("dev") => dev(&options),
        Some("accept") => accept(&options),
        Some("instances") => instances(&options),
        Some("status") => status(&options),
        Some("log") => log(&options),
        Some("facts") => facts(&options),
        Some("effects") => effects(&options),
        Some("runs") => runs(&options),
        Some("artifacts") => artifacts(&options),
        Some("inbox") => inbox(&options),
        Some("notify") => notify(&options),
        Some("otel-export") => otel_export(&options),
        Some("leases") => coordination_list(&options, "leases"),
        Some("ledger") => coordination_list(&options, "ledger"),
        Some("counters") => coordination_list(&options, "counters"),
        Some("items") => items(&options),
        Some("evidence") => evidence(&options),
        Some("diagnostics") => diagnostics(&options),
        Some("trace") => trace(&options),
        Some("pause") => pause(&options),
        Some("resume") => resume(&options),
        Some("cancel") => cancel(&options),
        Some("retry") => retry(&options),
        Some("recover") => recover(&options),
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
    println!("commands: check, compile, run, revise, step, worker, dev, accept, instances, status, log, facts, effects, runs");
    println!(
        "          artifacts, inbox, items, evidence, diagnostics, trace, pause, resume, cancel, retry, recover, doctor"
    );
    println!("run `whip <command> --help` or `whip help <command>` for command usage");
}

fn command_usage(command: &str) -> Option<&'static str> {
    Some(match command {
        "check" => "usage: whip check [--model-search] [--root <workflow>] [--exec-profile dev|hosted] [--script-manifest <path>] <workflow.whip>...",
        "compile" => "usage: whip compile [--root <workflow>] <workflow.whip>...",
        "run" => "usage: whip [--store path] [--input <json>] run <workflow.whip> [--root <workflow>]",
        "revise" => "usage: whip revise <instance> <workflow.whip> [--root <workflow>] [--dry-run] [--cancel keep|queued|running]",
        "step" => "usage: whip step <instance> --program <workflow.whip> [--root <workflow>]",
        "worker" => "usage: whip worker <instance> [--provider <name>] [--provider-config <path>] [--program <path>] [--root <workflow>] [--exec-profile dev|hosted] [--script-manifest <path>] [--once] [--fail|--timeout|--cancel] [--max-child-iterations <n>]",
        "dev" => "usage: whip dev <workflow.whip> [--provider <name>] [--provider-config <path>] [--root <workflow>] [--exec-profile dev|hosted] [--script-manifest <path>] [--include-tag <tag>] [--exclude-tag <tag>] [--stream ndjson] [--until idle] [--max-iterations <n>] [--fail|--timeout|--cancel]",
        "accept" => "usage: whip accept <fixture.json>",
        "instances" => "usage: whip [--store path] [--json] instances",
        "status" => "usage: whip status <instance>",
        "log" => "usage: whip log <instance>",
        "facts" => "usage: whip facts <instance>",
        "effects" => "usage: whip effects <instance>",
        "runs" => "usage: whip runs <instance>",
        "artifacts" => "usage: whip artifacts <run-id>",
        "inbox" => "usage: whip inbox [<instance>|show <item>|answer <item> (--choice X|--text X) [--by NAME]]",
        "notify" => "usage: whip notify <instance> --event <name> --data <json> --program <workflow.whip> [--root <workflow>]",
        "otel-export" => "usage: whip otel-export <instance> [--dry-run] (reads OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_SERVICE_NAME)",
        "leases" => "usage: whip leases [<resource>]",
        "ledger" => "usage: whip ledger [<ledger>] [--partition <value>]",
        "counters" => "usage: whip counters [<counter>]",
        "items" => "usage: whip items [list [--queue Q] [--status S]|add --queue Q --title T [--body B] [--label L]...|show <id>]",
        "evidence" => "usage: whip evidence <instance>",
        "diagnostics" => "usage: whip diagnostics <instance>",
        "trace" => "usage: whip trace <instance> [--check]",
        "pause" => "usage: whip pause <instance>",
        "resume" => "usage: whip resume <instance>",
        "cancel" => "usage: whip cancel <instance>",
        "retry" => "usage: whip retry <instance> <effect>",
        "recover" => "usage: whip recover <instance>",
        "doctor" => "usage: whip doctor [--providers] [--provider-config <path>] [--record-provider-evidence <instance>]",
        _ => return None,
    })
}

fn doctor(options: &CliOptions) -> ExitCode {
    let doctor_options = match DoctorOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
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
    let provider_capabilities = builtin_provider_capabilities();
    let provider_configs = doctor_provider_config_checks(
        &doctor_options.provider_config_paths,
        &provider_capabilities,
    );
    let provider_health_checks = if doctor_options.providers {
        doctor_provider_health_checks(&provider_capabilities, &tools)
    } else {
        Vec::new()
    };
    let provider_validation_evidence_recorded = if let Some(instance_id) = doctor_options
        .record_provider_evidence_instance_id
        .as_deref()
    {
        let store = match open_store_or_exit(options) {
            Ok(store) => store,
            Err(code) => return code,
        };
        match record_doctor_provider_validation_evidence(
            &store,
            instance_id,
            &provider_configs,
            &provider_capabilities,
        ) {
            Ok(count) => count,
            Err(error) => {
                return report_store_error("failed to record provider validation evidence", error)
            }
        }
    } else {
        0
    };
    if options.json {
        emit_json(json!({
            "stage": whipplescript_kernel::kernel_stage(),
            "store": store_status,
            "tools": tools.iter().map(tool_check_to_json).collect::<Vec<_>>(),
            "provider_capabilities": provider_capabilities
                .iter()
                .map(|capability| capability.to_json())
                .collect::<Vec<_>>(),
            "provider_config_checks": provider_configs
                .iter()
                .map(provider_config_check_to_json)
                .collect::<Vec<_>>(),
            "provider_health_checks": provider_health_checks
                .iter()
                .map(provider_health_check_to_json)
                .collect::<Vec<_>>(),
            "provider_validation_evidence_recorded": provider_validation_evidence_recorded,
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
        println!("provider capabilities:");
        for capability in provider_capabilities {
            println!(
                "  {:<8} {:<20} cancellation={}",
                capability.provider_kind.as_str(),
                capability.surface.as_str(),
                capability
                    .cancellation_depths
                    .iter()
                    .map(|depth| depth.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        if !provider_configs.is_empty() {
            println!("provider config checks:");
            for config in provider_configs {
                println!("  {}", config.path.display());
                for result in config.results {
                    println!(
                        "    {:<5} {:<32} {}",
                        result.status.as_str(),
                        result.code,
                        result.message
                    );
                }
            }
        }
        if !provider_health_checks.is_empty() {
            println!("provider health checks:");
            for check in provider_health_checks {
                println!(
                    "  {:<8} {:<20} {:<8} {}",
                    check.provider,
                    check.check,
                    check.status.as_str(),
                    check.message
                );
            }
        }
        if provider_validation_evidence_recorded > 0 {
            println!(
                "provider validation evidence recorded: {provider_validation_evidence_recorded}"
            );
        }
        ExitCode::SUCCESS
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DoctorOptions {
    provider_config_paths: Vec<PathBuf>,
    record_provider_evidence_instance_id: Option<String>,
    providers: bool,
}

impl DoctorOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut provider_config_paths = Vec::new();
        let mut record_provider_evidence_instance_id = None;
        let mut providers = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--providers" => providers = true,
                "--provider-config" => {
                    let Some(path) = iter.next() else {
                        return Err("expected path after `--provider-config`".to_owned());
                    };
                    provider_config_paths.push(PathBuf::from(path));
                }
                "--record-provider-evidence" => {
                    let Some(instance_id) = iter.next() else {
                        return Err(
                            "expected instance id after `--record-provider-evidence`".to_owned()
                        );
                    };
                    record_provider_evidence_instance_id = Some(instance_id.clone());
                }
                other => {
                    return Err(format!(
                        "unknown doctor option `{other}`; expected --providers, --provider-config, or --record-provider-evidence"
                    ));
                }
            }
        }
        Ok(Self {
            provider_config_paths,
            record_provider_evidence_instance_id,
            providers,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DoctorProviderConfigCheck {
    path: PathBuf,
    results: Vec<ProviderValidationResult>,
    bindings: Vec<DoctorProviderBindingCheck>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DoctorProviderBindingCheck {
    config: ProviderBindingConfig,
    results: Vec<ProviderValidationResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DoctorProviderHealthCheck {
    provider: String,
    surface: String,
    check: String,
    status: ProviderValidationStatus,
    message: String,
}

fn doctor_provider_health_checks(
    capabilities: &[ProviderCapability],
    tools: &[ToolCheck],
) -> Vec<DoctorProviderHealthCheck> {
    capabilities
        .iter()
        .filter(|capability| matches!(capability.provider_kind.as_str(), "codex" | "claude" | "pi"))
        .flat_map(|capability| {
            let mut checks = Vec::new();
            let provider = capability.provider_kind.as_str().to_owned();
            let surface = capability.surface.as_str().to_owned();
            checks.push(command_health_check(capability, tools));
            for requirement in &capability.auth_requirements {
                checks.push(auth_health_check(&provider, &surface, requirement));
            }
            for health_check in &capability.health_checks {
                if !matches!(
                    health_check.as_str(),
                    "codex_cli" | "claude_sdk" | "pi_cli" | "api_key"
                ) {
                    checks.push(DoctorProviderHealthCheck {
                        provider: provider.clone(),
                        surface: surface.clone(),
                        check: health_check.clone(),
                        status: ProviderValidationStatus::Skip,
                        message: "live provider check requires explicit real-provider validation"
                            .to_owned(),
                    });
                }
            }
            checks
        })
        .collect()
}

fn command_health_check(
    capability: &ProviderCapability,
    tools: &[ToolCheck],
) -> DoctorProviderHealthCheck {
    let provider = capability.provider_kind.as_str();
    let tool = tools.iter().find(|tool| tool.id == provider);
    let available = tool.is_some_and(|tool| tool.available);
    DoctorProviderHealthCheck {
        provider: provider.to_owned(),
        surface: capability.surface.as_str().to_owned(),
        check: format!("{provider}_cli"),
        status: if available {
            ProviderValidationStatus::Pass
        } else {
            ProviderValidationStatus::Fail
        },
        message: if available {
            format!("{provider} CLI is available")
        } else {
            format!("{provider} CLI is not on PATH")
        },
    }
}

fn auth_health_check(
    provider: &str,
    surface: &str,
    requirement: &str,
) -> DoctorProviderHealthCheck {
    let env_name = match provider {
        "codex" => Some("OPENAI_API_KEY"),
        "claude" => Some("ANTHROPIC_API_KEY"),
        "pi" => Some("PI_API_KEY"),
        _ => None,
    };
    let env_set = env_name.is_some_and(|name| env::var_os(name).is_some());
    DoctorProviderHealthCheck {
        provider: provider.to_owned(),
        surface: surface.to_owned(),
        check: requirement.to_owned(),
        status: if env_set {
            ProviderValidationStatus::Pass
        } else {
            ProviderValidationStatus::Skip
        },
        message: if let Some(env_name) = env_name {
            if env_set {
                format!("{env_name} is set; value redacted")
            } else {
                format!("{env_name} is unset; credential reference may come from provider config")
            }
        } else {
            "credential reference posture unavailable".to_owned()
        },
    }
}

fn doctor_provider_config_checks(
    paths: &[PathBuf],
    capabilities: &[ProviderCapability],
) -> Vec<DoctorProviderConfigCheck> {
    paths
        .iter()
        .map(|path| {
            let (results, bindings) = match fs::read_to_string(path) {
                Ok(config_json) => {
                    validate_doctor_provider_config_json_with_bindings(&config_json, capabilities)
                }
                Err(error) => (
                    vec![ProviderValidationResult::fail(
                        "",
                        "",
                        "provider.config.unreadable",
                        "read_error",
                        format!("could not read provider config: {error}"),
                    )],
                    Vec::new(),
                ),
            };
            DoctorProviderConfigCheck {
                path: path.clone(),
                results,
                bindings,
            }
        })
        .collect()
}

#[cfg(test)]
fn validate_doctor_provider_config_json(config_json: &str) -> Vec<ProviderValidationResult> {
    validate_doctor_provider_config_json_with_bindings(
        config_json,
        &builtin_provider_capabilities(),
    )
    .0
}

fn validate_doctor_provider_config_json_with_bindings(
    config_json: &str,
    capabilities: &[ProviderCapability],
) -> (
    Vec<ProviderValidationResult>,
    Vec<DoctorProviderBindingCheck>,
) {
    let value = match serde_json::from_str::<Value>(config_json) {
        Ok(value) => value,
        Err(_) => return (validate_provider_binding_json(config_json), Vec::new()),
    };
    let provider_values = if let Some(providers) = value.get("providers").and_then(Value::as_array)
    {
        providers.iter().collect::<Vec<_>>()
    } else if let Some(providers) = value.as_array() {
        providers.iter().collect::<Vec<_>>()
    } else {
        vec![&value]
    };

    let mut all_results = Vec::new();
    let mut bindings = Vec::new();
    for provider in provider_values {
        match ProviderBindingConfig::from_value(provider) {
            Ok(config) => {
                let mut results = validate_provider_binding(&config, capabilities);
                results.extend(validate_provider_runtime_config(&config));
                all_results.extend(results.iter().cloned());
                bindings.push(DoctorProviderBindingCheck { config, results });
            }
            Err(results) => all_results.extend(results),
        }
    }
    (all_results, bindings)
}

fn validate_provider_runtime_config(
    config: &ProviderBindingConfig,
) -> Vec<ProviderValidationResult> {
    if config.provider_kind != ProviderKind::Command || config.surface != AdapterSurface::Command {
        return Vec::new();
    }
    match command_launch_plan_from_config(config) {
        Ok(_) => vec![ProviderValidationResult::pass(
            config.provider_id.clone(),
            config.surface.as_str(),
            "provider.command.valid",
            "command_config_valid",
            "command provider config is launchable",
        )],
        Err(StoreError::Conflict(message)) => vec![ProviderValidationResult::fail(
            config.provider_id.clone(),
            config.surface.as_str(),
            "provider.command.invalid",
            "invalid_command_config",
            message,
        )],
        Err(error) => vec![ProviderValidationResult::fail(
            config.provider_id.clone(),
            config.surface.as_str(),
            "provider.command.invalid",
            "invalid_command_config",
            format!("{error:?}"),
        )],
    }
}

fn record_doctor_provider_validation_evidence(
    store: &SqliteStore,
    instance_id: &str,
    provider_configs: &[DoctorProviderConfigCheck],
    capabilities: &[ProviderCapability],
) -> Result<usize, StoreError> {
    let mut count = 0usize;
    for config_check in provider_configs {
        for binding in &config_check.bindings {
            let status = if binding
                .results
                .iter()
                .any(|result| result.status == ProviderValidationStatus::Fail)
            {
                "fail"
            } else {
                "pass"
            };
            let capability = capabilities
                .iter()
                .find(|capability| {
                    capability.provider_kind == binding.config.provider_kind
                        && capability.surface == binding.config.surface
                })
                .map(ProviderCapability::to_json)
                .unwrap_or_else(|| {
                    json!({
                        "provider_kind": binding.config.provider_kind.as_str(),
                        "surface": binding.config.surface.as_str(),
                        "registered": false,
                    })
                });
            let config_json = binding.config.to_json_redacted().to_string();
            let capability_json = capability.to_string();
            let validation_results_json = Value::Array(
                binding
                    .results
                    .iter()
                    .map(ProviderValidationResult::to_json)
                    .collect::<Vec<_>>(),
            )
            .to_string();
            let correlation_id = format!(
                "provider-validation:{}:{}",
                binding.config.provider_id,
                binding.config.surface.as_str()
            );
            let source_path = config_check.path.display().to_string();
            store.record_provider_validation_evidence(ProviderValidationEvidence {
                instance_id,
                provider_id: &binding.config.provider_id,
                provider_kind: binding.config.provider_kind.as_str(),
                surface: binding.config.surface.as_str(),
                status,
                config_json: &config_json,
                capability_json: &capability_json,
                validation_results_json: &validation_results_json,
                source_path: Some(&source_path),
                correlation_id: Some(&correlation_id),
            })?;
            count += 1;
        }
    }
    Ok(count)
}

fn provider_config_check_to_json(check: &DoctorProviderConfigCheck) -> Value {
    json!({
        "path": check.path.display().to_string(),
        "results": check
            .results
            .iter()
            .map(ProviderValidationResult::to_json)
            .collect::<Vec<_>>(),
    })
}

fn provider_health_check_to_json(check: &DoctorProviderHealthCheck) -> Value {
    json!({
        "provider": check.provider,
        "surface": check.surface,
        "check": check.check,
        "status": check.status.as_str(),
        "message": check.message,
    })
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
            "pi",
            "provider",
            &["pi"][..],
            false,
            "needed for Pi RPC provider runs",
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
    let mut reports = Vec::new();
    let script_manifest = match load_script_manifest(check_options.script_manifest_path.as_deref())
    {
        Ok(manifest) => manifest,
        Err(message) => {
            if options.json {
                let _ = emit_json(json!([{
                    "schema": "whipplescript.check_report.v0",
                    "status": "error",
                    "error": {"kind": "script_manifest", "message": message},
                }]));
            } else {
                eprintln!("{message}");
            }
            return ExitCode::from(2);
        }
    };
    for path in check_options.paths {
        match compile_source_path_for_validation(&path, check_options.root.as_deref()) {
            Ok((source, ir)) => {
                let hosted_exec_diagnostics = lint_hosted_exec(
                    &source,
                    &ir,
                    check_options.exec_profile,
                    script_manifest.as_ref(),
                );
                if !hosted_exec_diagnostics.is_empty() {
                    failed = true;
                    if options.json {
                        reports.push(json!({
                            "schema": "whipplescript.check_report.v0",
                            "path": display_path(&path),
                            "status": "error",
                            "error": {
                                "kind": "diagnostics",
                                "diagnostics": hosted_exec_diagnostics
                                    .iter()
                                    .map(parser_diagnostic_to_json)
                                    .collect::<Vec<_>>(),
                            },
                        }));
                    } else {
                        for diagnostic in hosted_exec_diagnostics {
                            eprint!("{}", render_diagnostic(&path, &source, &diagnostic));
                        }
                    }
                    continue;
                }
                let snapshot = ir.to_snapshot();
                let mut report = json!({
                    "schema": "whipplescript.check_report.v0",
                    "path": display_path(&path),
                    "status": "ok",
                    "workflow": ir.workflow.as_str(),
                    "source_hash": stable_hash_hex(&source),
                    "ir_hash": stable_hash_hex(&snapshot),
                    "snapshot": snapshot,
                    "source_metadata": source_metadata_json(&ir),
                });
                if !options.json {
                    println!("== {}", display_path(&path));
                    print!("{}", ir.to_snapshot());
                }
                if check_options.model_search {
                    match run_model_search(&path, &source, &ir) {
                        Ok(model_report) if model_report.searches == 0 => {
                            if options.json {
                                insert_json_field(
                                    &mut report,
                                    "model_search",
                                    json!({
                                        "status": "ok",
                                        "searches": 0,
                                        "solutions": 0,
                                        "no_solutions": 0,
                                    }),
                                );
                            } else {
                                println!("model search: no generated checks");
                            }
                        }
                        Ok(model_report) => {
                            if options.json {
                                insert_json_field(
                                    &mut report,
                                    "model_search",
                                    json!({
                                        "status": "ok",
                                        "searches": model_report.searches,
                                        "solutions": model_report.solutions,
                                        "no_solutions": model_report.no_solutions,
                                    }),
                                );
                            } else {
                                println!(
                                    "model search: {} checks passed (solutions={}, no_solutions={})",
                                    model_report.searches,
                                    model_report.solutions,
                                    model_report.no_solutions
                                );
                            }
                        }
                        Err(message) => {
                            if options.json {
                                insert_json_field(
                                    &mut report,
                                    "model_search",
                                    json!({
                                        "status": "error",
                                        "message": message,
                                    }),
                                );
                            } else {
                                eprintln!("{path}: model search failed: {message}");
                            }
                            failed = true;
                        }
                    }
                }
                reports.push(report);
            }
            Err(CompileFailure::Io(error)) => {
                if options.json {
                    reports.push(json!({
                        "schema": "whipplescript.check_report.v0",
                        "path": display_path(&path),
                        "status": "error",
                        "error": {
                            "kind": "io",
                            "message": error.to_string(),
                        },
                    }));
                } else {
                    eprintln!("{path}: failed to read: {error}");
                }
                failed = true;
            }
            Err(CompileFailure::Diagnostics {
                source,
                diagnostics,
            }) => {
                failed = true;
                if options.json {
                    reports.push(json!({
                        "schema": "whipplescript.check_report.v0",
                        "path": display_path(&path),
                        "status": "error",
                        "error": {
                            "kind": "diagnostics",
                            "diagnostics": diagnostics
                                .iter()
                                .map(parser_diagnostic_to_json)
                                .collect::<Vec<_>>(),
                        },
                    }));
                } else {
                    for diagnostic in diagnostics {
                        eprint!("{}", render_diagnostic(&path, &source, &diagnostic));
                    }
                }
            }
        }
    }

    if options.json {
        let code = if failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
        let _ = emit_json(Value::Array(reports));
        return code;
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
    exec_profile: ExecProfile,
    script_manifest_path: Option<PathBuf>,
    paths: Vec<String>,
}

impl CheckOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut model_search = false;
        let mut root = None;
        let mut exec_profile = ExecProfile::from_env();
        let mut script_manifest_path = script_manifest_path_from_env();
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
                "--exec-profile" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected profile after `--exec-profile`".to_owned());
                    };
                    exec_profile = ExecProfile::parse(value)?;
                }
                "--script-manifest" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected path after `--script-manifest`".to_owned());
                    };
                    script_manifest_path = Some(PathBuf::from(value));
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
            exec_profile,
            script_manifest_path,
            paths,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecProfile {
    Dev,
    Hosted,
}

impl ExecProfile {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "dev" => Ok(Self::Dev),
            "hosted" => Ok(Self::Hosted),
            other => Err(format!(
                "unknown exec profile `{other}`; expected `dev` or `hosted`"
            )),
        }
    }

    fn from_env() -> Self {
        env::var("WHIPPLESCRIPT_EXEC_PROFILE")
            .ok()
            .and_then(|value| Self::parse(&value).ok())
            .unwrap_or(Self::Dev)
    }

    fn is_hosted(self) -> bool {
        self == Self::Hosted
    }
}

fn script_manifest_path_from_env() -> Option<PathBuf> {
    env::var_os("WHIPPLESCRIPT_SCRIPT_MANIFEST").map(PathBuf::from)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScriptCapability {
    name: String,
    argv: Vec<String>,
    sha256: String,
    env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ScriptManifest {
    capabilities: BTreeMap<String, ScriptCapability>,
}

impl ScriptManifest {
    fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|error| {
            format!(
                "failed to read script manifest `{}`: {error}",
                path.display()
            )
        })?;
        let value = serde_json::from_str::<Value>(&text).map_err(|error| {
            format!(
                "failed to parse script manifest `{}`: {error}",
                path.display()
            )
        })?;
        let object = value.as_object().ok_or_else(|| {
            format!(
                "script manifest `{}` must be a JSON object keyed by capability name",
                path.display()
            )
        })?;
        let mut capabilities = BTreeMap::new();
        for (name, entry) in object {
            let entry_object = entry
                .as_object()
                .ok_or_else(|| format!("script manifest entry `{name}` must be a JSON object"))?;
            let argv = entry_object
                .get("argv")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("script manifest entry `{name}` must have argv array"))?
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        format!("script manifest entry `{name}` argv values must be strings")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if argv.is_empty() {
                return Err(format!(
                    "script manifest entry `{name}` argv must not be empty"
                ));
            }
            let sha256 = entry_object
                .get("sha256")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("script manifest entry `{name}` must have sha256 string"))?
                .trim_start_matches("sha256:")
                .to_ascii_lowercase();
            if sha256.len() != 64 || !sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(format!(
                    "script manifest entry `{name}` sha256 must be a 64-character hex digest"
                ));
            }
            let env = entry_object
                .get("env")
                .and_then(Value::as_object)
                .map(|env_object| {
                    env_object
                        .iter()
                        .map(|(key, value)| {
                            value
                                .as_str()
                                .map(|value| (key.clone(), value.to_owned()))
                                .ok_or_else(|| {
                                    format!(
                                        "script manifest entry `{name}` env value `{key}` must be a string reference"
                                    )
                                })
                        })
                        .collect::<Result<BTreeMap<_, _>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            capabilities.insert(
                name.clone(),
                ScriptCapability {
                    name: name.clone(),
                    argv,
                    sha256,
                    env,
                },
            );
        }
        Ok(Self { capabilities })
    }

    fn names(&self) -> Vec<String> {
        self.capabilities.keys().cloned().collect()
    }

    fn get(&self, name: &str) -> Option<&ScriptCapability> {
        self.capabilities.get(name)
    }
}

fn load_script_manifest(path: Option<&Path>) -> Result<Option<ScriptManifest>, String> {
    path.map(ScriptManifest::load).transpose()
}

fn register_script_manifest_capabilities(
    store: &SqliteStore,
    manifest: &ScriptManifest,
    program_id: &str,
) -> Result<(), StoreError> {
    for name in manifest.capabilities.keys() {
        let capability = format!("script.{name}");
        store.register_capability_schema(CapabilitySchemaRegistration {
            capability: &capability,
            description: "Run an operator-pinned script capability.",
            schema_json: "{}",
            registered_by_plugin_id: None,
        })?;
        let binding_id = format!("binding_script_{}_{}", name, stable_hash_hex(program_id));
        store.bind_capability(CapabilityBinding {
            binding_id: &binding_id,
            program_id: Some(program_id),
            capability: &capability,
            provider: "builtin-script",
            config_json: "{}",
        })?;
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExecSurface {
    Raw,
    Capability { name: String },
}

fn exec_surface_from_statement(statement: &str) -> Option<ExecSurface> {
    let rest = statement.trim().strip_prefix("exec ")?.trim_start();
    if rest.starts_with('"') {
        return Some(ExecSurface::Raw);
    }
    let name = rest.split_whitespace().next()?.to_owned();
    Some(ExecSurface::Capability { name })
}

fn lint_hosted_exec(
    source: &str,
    ir: &IrProgram,
    exec_profile: ExecProfile,
    script_manifest: Option<&ScriptManifest>,
) -> Vec<Diagnostic> {
    if !exec_profile.is_hosted() {
        return Vec::new();
    }
    let available = script_manifest
        .map(ScriptManifest::names)
        .unwrap_or_default();
    let mut diagnostics = Vec::new();
    for rule in &ir.rules {
        for line in rule.body.lines() {
            let statement = line.trim();
            if !statement.starts_with("exec ") {
                continue;
            }
            let span = source
                .find(statement)
                .map(|start| SourceSpan {
                    start,
                    end: start + statement.len(),
                })
                .unwrap_or(SourceSpan { start: 0, end: 0 });
            match exec_surface_from_statement(statement) {
                Some(ExecSurface::Raw) => diagnostics.push(Diagnostic {
                    span,
                    message: "raw `exec \"...\"` is not allowed in hosted exec profile".to_owned(),
                    suggestion: Some(
                        "use `exec <capability> with <record> -> <Type> as <binding>` from the script manifest"
                            .to_owned(),
                    ),
                }),
                Some(ExecSurface::Capability { name }) => {
                    if script_manifest
                        .and_then(|manifest| manifest.get(&name))
                        .is_none()
                    {
                        let declared = if available.is_empty() {
                            "no script capabilities declared".to_owned()
                        } else {
                            format!("declared script capabilities: {}", available.join(", "))
                        };
                        diagnostics.push(Diagnostic {
                            span,
                            message: format!(
                                "exec capability `{name}` is not declared in the script manifest"
                            ),
                            suggestion: Some(declared),
                        });
                    }
                }
                None => {}
            }
        }
    }
    diagnostics
}

fn insert_json_field(value: &mut Value, key: &str, field: Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.to_owned(), field);
    }
}

fn source_metadata_json(ir: &IrProgram) -> Value {
    let tags = ir
        .source_tags
        .iter()
        .map(|tag| {
            json!({
                "name": tag.name,
                "target_kind": tag.target_kind,
                "target": tag.target,
                "source_span": source_span_to_json(tag.span),
            })
        })
        .collect::<Vec<_>>();
    let descriptions = ir
        .source_descriptions
        .iter()
        .map(|description| {
            json!({
                "value": description.value,
                "target_kind": description.target_kind,
                "target": description.target,
                "source_span": source_span_to_json(description.span),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "tags": tags,
        "descriptions": descriptions,
        "targets": source_metadata_targets_json(ir),
    })
}

fn source_metadata_targets_json(ir: &IrProgram) -> Value {
    let mut targets = serde_json::Map::new();
    for tag in &ir.source_tags {
        let key = source_metadata_target_key(&tag.target_kind, &tag.target);
        let entry = source_metadata_target_entry(&mut targets, &key, &tag.target_kind, &tag.target);
        if let Some(tags) = entry.get_mut("tags").and_then(Value::as_array_mut) {
            tags.push(Value::String(tag.name.clone()));
        }
    }
    for description in &ir.source_descriptions {
        let key = source_metadata_target_key(&description.target_kind, &description.target);
        let entry = source_metadata_target_entry(
            &mut targets,
            &key,
            &description.target_kind,
            &description.target,
        );
        entry.insert(
            "description".to_owned(),
            Value::String(description.value.clone()),
        );
    }
    Value::Object(targets)
}

fn source_metadata_target_entry<'a>(
    targets: &'a mut serde_json::Map<String, Value>,
    key: &str,
    target_kind: &str,
    target: &str,
) -> &'a mut serde_json::Map<String, Value> {
    let value = targets.entry(key.to_owned()).or_insert_with(|| {
        json!({
            "target_kind": target_kind,
            "target": target,
            "tags": [],
        })
    });
    value.as_object_mut().expect("metadata target is object")
}

fn source_metadata_target_key(target_kind: &str, target: &str) -> String {
    format!("{target_kind}:{target}")
}

fn parser_diagnostic_to_json(diagnostic: &Diagnostic) -> Value {
    json!({
        "message": diagnostic.message,
        "suggestion": diagnostic.suggestion,
        "source_span": source_span_to_json(diagnostic.span),
    })
}

fn source_span_to_json(span: SourceSpan) -> Value {
    json!({
        "start": span.start,
        "end": span.end,
    })
}

fn compile(options: &CliOptions) -> ExitCode {
    let compile_options = match CompileOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let (source, ir) = match compile_source_path_for_validation(
        &compile_options.program_path,
        compile_options.root.as_deref(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => return report_compile_failure(&compile_options.program_path, error),
    };
    let snapshot = ir.to_snapshot();

    if options.json {
        emit_json(json!({
            "schema": "whipplescript.compile_report.v0",
            "path": display_path(&compile_options.program_path),
            "workflow": ir.workflow.as_str(),
            "source_hash": stable_hash_hex(&source),
            "ir_hash": stable_hash_hex(&snapshot),
            "snapshot": snapshot,
            "source_metadata": source_metadata_json(&ir),
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReviseOptions {
    instance_id: String,
    program_path: String,
    root: Option<String>,
    dry_run: bool,
    cancellation_policy: String,
}

impl ReviseOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut instance_id = None;
        let mut program_path = None;
        let mut root = None;
        let mut dry_run = false;
        let mut cancellation_policy = "keep".to_owned();
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
                "--dry-run" => dry_run = true,
                "--cancel" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected policy after `--cancel`".to_owned());
                    };
                    cancellation_policy = match value.as_str() {
                        "keep" => "keep".to_owned(),
                        "queued" | "cancel_queued" | "cancel-queued" => "queued".to_owned(),
                        "running" | "request_running" | "request-running" => "running".to_owned(),
                        other => {
                            return Err(format!(
                                "unknown revision cancellation policy `{other}`; expected keep, queued, or running"
                            ));
                        }
                    };
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown revise option `{other}`"));
                }
                value if instance_id.is_none() => instance_id = Some(value.to_owned()),
                value if program_path.is_none() => program_path = Some(value.to_owned()),
                _ => {
                    return Err(
                        "usage: whip revise <instance> <workflow.whip> [--root Workflow] [--dry-run] [--cancel keep|queued|running]"
                            .to_owned(),
                    );
                }
            }
            index += 1;
        }
        let Some(instance_id) = instance_id else {
            return Err(
                "usage: whip revise <instance> <workflow.whip> [--root Workflow] [--dry-run] [--cancel keep|queued|running]"
                    .to_owned(),
            );
        };
        let Some(program_path) = program_path else {
            return Err(
                "usage: whip revise <instance> <workflow.whip> [--root Workflow] [--dry-run] [--cancel keep|queued|running]"
                    .to_owned(),
            );
        };
        Ok(Self {
            instance_id,
            program_path,
            root,
            dry_run,
            cancellation_policy,
        })
    }
}

fn revise(options: &CliOptions) -> ExitCode {
    let revise_options = match ReviseOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let store = match open_store(&options.store_path) {
        Ok(store) => store,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let (source, ir) = match compile_source_path_with_root(
        &revise_options.program_path,
        revise_options.root.as_deref(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => {
            if let Err(store_error) =
                persist_revision_source_bundle_diagnostic(&store, &revise_options, &error)
            {
                return report_store_error(
                    "failed to record revision source diagnostic",
                    store_error,
                );
            }
            return report_compile_failure(&revise_options.program_path, error);
        }
    };
    let snapshot = ir.to_snapshot();
    let source_hash = stable_hash_hex(&source);
    let ir_hash = stable_hash_hex(&snapshot);
    let analysis_summary_json = program_analysis_summary_json(&ir);
    let candidate_label = format!("candidate:{ir_hash}");

    let compatibility = match store.analyze_revision_candidate(
        &revise_options.instance_id,
        RevisionCandidate {
            candidate_version_id: &candidate_label,
            program_name: &ir.workflow,
            analysis_summary_json: &analysis_summary_json,
        },
    ) {
        Ok(report) => report,
        Err(error) => return report_store_error("failed to analyze revision", error),
    };
    let impact = match store.revision_cancellation_impact(
        &revise_options.instance_id,
        &revise_options.cancellation_policy,
    ) {
        Ok(impact) => impact,
        Err(error) => return report_store_error("failed to analyze cancellation impact", error),
    };
    let agent_impact = match revision_agent_impact(&store, &revise_options.instance_id, &ir) {
        Ok(impact) => impact,
        Err(error) => return report_store_error("failed to analyze agent impact", error),
    };

    if revise_options.dry_run {
        return emit_revision_dry_run(
            options,
            &revise_options,
            &ir,
            &source_hash,
            &ir_hash,
            &compatibility,
            &impact,
            &agent_impact,
        );
    }

    if !compatibility.compatible {
        if let Err(error) = persist_revision_compatibility_diagnostics(
            &store,
            &revise_options.instance_id,
            &compatibility,
        ) {
            return report_store_error("failed to record revision diagnostics", error);
        }
        emit_revision_report(
            options,
            &revise_options,
            &ir,
            &source_hash,
            &ir_hash,
            &compatibility,
            &impact,
            &agent_impact,
            None,
        );
        return ExitCode::FAILURE;
    }

    let mut kernel = RuntimeKernel::new(store);
    let version = match kernel.create_program_version_for_program(
        ProgramVersionInput {
            program_name: &ir.workflow,
            source_hash: &source_hash,
            ir_hash: &ir_hash,
            compiler_version: whipplescript_core::version(),
        },
        &ir,
    ) {
        Ok(version) => version,
        Err(error) => {
            eprintln!(
                "failed to create candidate program version: {}",
                store_error(error)
            );
            return ExitCode::FAILURE;
        }
    };
    let mut store = kernel.into_store();
    let activation_policy = json!({
        "source_path": display_path(&revise_options.program_path),
        "root_workflow": &ir.workflow,
        "source_hash": &source_hash,
        "ir_hash": &ir_hash,
        "compatibility": revision_compatibility_to_json(&compatibility),
        "cancellation": revision_cancellation_impact_to_json(&impact),
        "agent_impact": revision_agent_impact_to_json(&agent_impact),
    })
    .to_string();
    let activation = match store.activate_revision(RevisionActivation {
        instance_id: &revise_options.instance_id,
        from_version_id: &compatibility.active_version_id,
        to_version_id: &version.version_id,
        activation_policy_json: &activation_policy,
        cancellation_policy: &revise_options.cancellation_policy,
        idempotency_key: Some(&idempotency_key(&[
            &revise_options.instance_id,
            "revision",
            &compatibility.active_version_id,
            &version.version_id,
        ])),
    }) {
        Ok(revision) => revision,
        Err(error) => return report_store_error("failed to activate revision", error),
    };

    emit_revision_report(
        options,
        &revise_options,
        &ir,
        &source_hash,
        &ir_hash,
        &compatibility,
        &impact,
        &agent_impact,
        Some(&activation),
    );
    ExitCode::SUCCESS
}

fn persist_revision_compatibility_diagnostics(
    store: &SqliteStore,
    instance_id: &str,
    compatibility: &RevisionCompatibilityReport,
) -> Result<(), StoreError> {
    let diagnostic_json = compatibility
        .diagnostics
        .iter()
        .map(revision_compatibility_diagnostic_to_json)
        .collect::<Vec<_>>();
    let payload = json!({
        "instance_id": instance_id,
        "active_version_id": compatibility.active_version_id,
        "candidate_version_id": compatibility.candidate_version_id,
        "compatible": false,
        "diagnostics": diagnostic_json,
    })
    .to_string();
    let event = store.append_event(NewEvent {
        instance_id,
        event_type: "workflow.revision_rejected",
        payload_json: &payload,
        source: "cli",
        causation_id: None,
        correlation_id: Some(&compatibility.candidate_version_id),
        idempotency_key: None,
    })?;

    let mut diagnostic_ids = Vec::new();
    for diagnostic in &compatibility.diagnostics {
        let subject_id = diagnostic
            .subject
            .as_deref()
            .unwrap_or(&compatibility.candidate_version_id)
            .to_owned();
        let idempotency_key = idempotency_key(&[
            instance_id,
            "revision-compatibility",
            &compatibility.candidate_version_id,
            &diagnostic.code,
            &subject_id,
        ]);
        let diagnostic_id = store.record_diagnostic(DiagnosticRecord {
            instance_id: Some(instance_id),
            program_id: None,
            program_version_id: None,
            severity: "error",
            code: Some(&diagnostic.code),
            message: &diagnostic.message,
            source_span_json: diagnostic.source_span_json.as_deref(),
            subject_type: Some("revision_compatibility"),
            subject_id: Some(&subject_id),
            event_id: Some(&event.event_id),
            effect_id: None,
            run_id: None,
            assertion_id: None,
            evidence_ids_json: "[]",
            artifact_ids_json: "[]",
            causation_id: Some(&event.event_id),
            correlation_id: Some(&compatibility.candidate_version_id),
            idempotency_key: Some(&idempotency_key),
        })?;
        diagnostic_ids.push(diagnostic_id);
    }

    let evidence_metadata = json!({
        "event_id": event.event_id,
        "active_version_id": compatibility.active_version_id,
        "candidate_version_id": compatibility.candidate_version_id,
        "diagnostic_ids": diagnostic_ids,
        "diagnostics": diagnostic_json,
    })
    .to_string();
    let evidence_id = store.record_evidence(EvidenceRecord {
        instance_id,
        kind: "workflow.revision.compatibility_rejected",
        subject_type: "revision_candidate",
        subject_id: &compatibility.candidate_version_id,
        causation_id: Some(&event.event_id),
        correlation_id: Some(&compatibility.candidate_version_id),
        summary: Some("workflow revision rejected by compatibility diagnostics"),
        metadata_json: &evidence_metadata,
    })?;
    store.link_evidence(EvidenceLink {
        evidence_id: &evidence_id,
        instance_id,
        target_type: "event",
        target_id: &event.event_id,
        relation: "rejected",
    })?;
    for diagnostic_id in &diagnostic_ids {
        store.link_evidence(EvidenceLink {
            evidence_id: &evidence_id,
            instance_id,
            target_type: "diagnostic",
            target_id: diagnostic_id,
            relation: "compatibility_diagnostic",
        })?;
    }
    Ok(())
}

fn persist_revision_source_bundle_diagnostic(
    store: &SqliteStore,
    revise_options: &ReviseOptions,
    error: &CompileFailure,
) -> Result<(), StoreError> {
    let CompileFailure::Io(error) = error else {
        return Ok(());
    };
    let subject_id = display_path(&revise_options.program_path);
    let idempotency_key = idempotency_key(&[
        &revise_options.instance_id,
        "revision-source-bundle",
        &subject_id,
    ]);
    let message = format!("failed to read revision source bundle `{subject_id}`: {error}");
    store.record_diagnostic(DiagnosticRecord {
        instance_id: Some(&revise_options.instance_id),
        program_id: None,
        program_version_id: None,
        severity: "error",
        code: Some("revision.source_bundle_unavailable"),
        message: &message,
        source_span_json: None,
        subject_type: Some("revision_source_bundle"),
        subject_id: Some(&subject_id),
        event_id: None,
        effect_id: None,
        run_id: None,
        assertion_id: None,
        evidence_ids_json: "[]",
        artifact_ids_json: "[]",
        causation_id: None,
        correlation_id: Some(&revise_options.instance_id),
        idempotency_key: Some(&idempotency_key),
    })?;
    Ok(())
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

/// Self-contained structural schema embedded in a parsing effect's input so
/// the worker can validate ingested bytes without the program IR
/// (spec/json-ingestion.md). The effect carries its own contract, which also
/// keeps replay independent of later source edits.
fn ingest_shape_json(ir: &IrProgram, ty: &IrType, depth: usize) -> Value {
    if depth > 8 {
        return json!("json");
    }
    let object_fields = |fields: &[whipplescript_parser::IrClassField]| -> Value {
        Value::Object(
            fields
                .iter()
                .map(|field| {
                    (
                        field.name.clone(),
                        ingest_shape_json(ir, &field.ty, depth + 1),
                    )
                })
                .collect(),
        )
    };
    match ty {
        IrType::Primitive(primitive) => {
            json!(ir_type_name(&IrType::Primitive(primitive.clone())))
        }
        IrType::LiteralString(value) => json!({ "literal": value }),
        IrType::AgentRef(_) => json!("string"),
        IrType::Ref(name) => {
            for schema in &ir.schemas {
                match schema {
                    IrSchema::Class(class) if class.name == *name => {
                        return json!({ "class": name, "fields": object_fields(&class.fields) });
                    }
                    IrSchema::Enum(decl) if decl.name == *name => {
                        return json!({ "enum": decl.variants });
                    }
                    _ => {}
                }
            }
            json!("json")
        }
        IrType::Object(fields) => json!({ "fields": object_fields(fields) }),
        IrType::Optional(inner) => json!({ "optional": ingest_shape_json(ir, inner, depth + 1) }),
        IrType::Array(inner) => json!({ "array": ingest_shape_json(ir, inner, depth + 1) }),
        IrType::Map(inner) => json!({ "map": ingest_shape_json(ir, inner, depth + 1) }),
        IrType::Union(types) => json!({
            "union": types
                .iter()
                .map(|candidate| ingest_shape_json(ir, candidate, depth + 1))
                .collect::<Vec<_>>()
        }),
    }
}

/// Deterministic fixture value for an embedded structural shape: literal
/// fields yield their literal (which is how a variant fixture carries its
/// `variant` tag), enums pick the first variant, scalars get stable
/// placeholders (spec/sum-types.md fixture).
fn fixture_value_for_shape(shape: &Value) -> Value {
    match shape {
        Value::String(primitive) => match primitive.as_str() {
            "int" => json!(1),
            "float" => json!(0.5),
            "bool" => json!(true),
            "null" => Value::Null,
            "time" => json!("2026-01-01T00:00:00Z"),
            "json" => json!({}),
            _ => json!("fixture"),
        },
        Value::Object(map) => {
            if let Some(literal) = map.get("literal") {
                return literal.clone();
            }
            if let Some(variants) = map.get("enum").and_then(Value::as_array) {
                return variants
                    .first()
                    .cloned()
                    .unwrap_or_else(|| json!("fixture"));
            }
            if let Some(inner) = map.get("optional") {
                return fixture_value_for_shape(inner);
            }
            if let Some(inner) = map.get("array") {
                return json!([fixture_value_for_shape(inner)]);
            }
            if let Some(inner) = map.get("map") {
                return json!({ "fixture": fixture_value_for_shape(inner) });
            }
            if let Some(options) = map.get("union").and_then(Value::as_array) {
                return options
                    .first()
                    .map(fixture_value_for_shape)
                    .unwrap_or_else(|| json!({}));
            }
            if let Some(fields) = map.get("fields").and_then(Value::as_object) {
                return Value::Object(
                    fields
                        .iter()
                        .map(|(name, field_shape)| {
                            (name.clone(), fixture_value_for_shape(field_shape))
                        })
                        .collect(),
                );
            }
            json!({})
        }
        _ => json!("fixture"),
    }
}

/// Validates ingested JSON against the embedded structural shape — the
/// worker-side mirror of `validate_json_for_ir_type`, reading the contract
/// the effect carries instead of the program IR.
fn validate_ingest_value(value: &Value, shape: &Value, path: &str, errors: &mut Vec<String>) {
    match shape {
        Value::String(primitive) => {
            let valid = match primitive.as_str() {
                "int" => value.as_i64().is_some(),
                "float" => value.as_f64().is_some(),
                "bool" => value.is_boolean(),
                "null" => value.is_null(),
                "time" => value
                    .as_str()
                    .is_some_and(whipplescript_parser::body::is_iso8601_instant),
                "json" => true,
                // string plus media/duration primitives serialize as strings
                _ => value.is_string(),
            };
            if !valid {
                errors.push(format!("{path} must be {primitive}"));
            }
        }
        Value::Object(map) => {
            if let Some(literal) = map.get("literal") {
                if value != literal {
                    errors.push(format!("{path} must be literal {literal}"));
                }
            } else if let Some(variants) = map.get("enum").and_then(Value::as_array) {
                if !variants.iter().any(|candidate| candidate == value) {
                    errors.push(format!(
                        "{path} must be one of: {}",
                        variants
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            } else if let Some(inner) = map.get("optional") {
                if !value.is_null() {
                    validate_ingest_value(value, inner, path, errors);
                }
            } else if let Some(inner) = map.get("array") {
                match value.as_array() {
                    Some(items) => {
                        for (index, item) in items.iter().enumerate() {
                            validate_ingest_value(item, inner, &format!("{path}[{index}]"), errors);
                        }
                    }
                    None => errors.push(format!("{path} must be an array")),
                }
            } else if let Some(inner) = map.get("map") {
                match value.as_object() {
                    Some(entries) => {
                        for (key, item) in entries {
                            validate_ingest_value(item, inner, &format!("{path}.{key}"), errors);
                        }
                    }
                    None => errors.push(format!("{path} must be an object map")),
                }
            } else if let Some(options) = map.get("union").and_then(Value::as_array) {
                let matches_any = options.iter().any(|option| {
                    let mut probe = Vec::new();
                    validate_ingest_value(value, option, path, &mut probe);
                    probe.is_empty()
                });
                if !matches_any {
                    errors.push(format!("{path} matches no arm of the declared union"));
                }
            } else if let Some(fields) = map.get("fields").and_then(Value::as_object) {
                let label = map
                    .get("class")
                    .and_then(Value::as_str)
                    .map(|class| format!(" ({class})"))
                    .unwrap_or_default();
                let Some(object) = value.as_object() else {
                    errors.push(format!("{path} must be an object{label}"));
                    return;
                };
                for key in object.keys() {
                    if !fields.contains_key(key) {
                        errors.push(format!("{path}.{key} is not declared{label}"));
                    }
                }
                for (name, field_shape) in fields {
                    let field_path = format!("{path}.{name}");
                    match object.get(name) {
                        Some(field_value) => {
                            validate_ingest_value(field_value, field_shape, &field_path, errors)
                        }
                        None if field_shape.get("optional").is_some() => {}
                        None => errors.push(format!("{field_path} is required")),
                    }
                }
            }
        }
        _ => {}
    }
}

fn step(options: &CliOptions) -> ExitCode {
    let step_options = match StepOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let (source, ir) = match compile_source_path_with_root(
        &step_options.program_path,
        step_options.root.as_deref(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => return report_compile_failure(&step_options.program_path, error),
    };
    let active_version_id = match validate_step_program_version(
        &options.store_path,
        &step_options.instance_id,
        &step_options.program_path,
        &source,
        &ir,
    ) {
        Ok(active_version_id) => active_version_id,
        Err(error) => return report_store_error("failed to validate step program", error),
    };
    match step_instance(
        &options.store_path,
        &step_options.instance_id,
        &ir,
        Some(Path::new(&step_options.program_path)),
        Some(&active_version_id),
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

fn validate_step_program_version(
    store_path: &Path,
    instance_id: &str,
    program_path: &str,
    source: &str,
    ir: &IrProgram,
) -> Result<String, StoreError> {
    let store = SqliteStore::open(store_path)?;
    let instance = store
        .get_instance(instance_id)?
        .ok_or_else(|| StoreError::Conflict("instance does not exist".to_owned()))?;
    let active_version = store
        .get_program_version(&instance.version_id)?
        .ok_or_else(|| StoreError::Conflict("active program version does not exist".to_owned()))?;
    let snapshot = ir.to_snapshot();
    let source_hash = stable_hash_hex(source);
    let ir_hash = stable_hash_hex(&snapshot);
    if source_hash != active_version.source_hash || ir_hash != active_version.ir_hash {
        let message = format!(
            "step program `{program_path}` does not match active version {} at epoch {} (expected source_hash={} ir_hash={}, got source_hash={} ir_hash={}); activate the candidate with `whip revise` before stepping it",
            active_version.version_id,
            instance.revision_epoch,
            active_version.source_hash,
            active_version.ir_hash,
            source_hash,
            ir_hash
        );
        persist_stale_step_program_diagnostic(
            &store,
            instance_id,
            program_path,
            &active_version.version_id,
            instance.revision_epoch,
            &message,
        )?;
        return Err(StoreError::Conflict(message));
    }
    Ok(active_version.version_id)
}

fn persist_stale_step_program_diagnostic(
    store: &SqliteStore,
    instance_id: &str,
    program_path: &str,
    active_version_id: &str,
    revision_epoch: i64,
    message: &str,
) -> Result<(), StoreError> {
    let subject_id = display_path(program_path);
    let revision_epoch = revision_epoch.to_string();
    let idempotency_key = idempotency_key(&[
        instance_id,
        "stale-step-program",
        active_version_id,
        &subject_id,
        &revision_epoch,
    ]);
    store.record_diagnostic(DiagnosticRecord {
        instance_id: Some(instance_id),
        program_id: None,
        program_version_id: Some(active_version_id),
        severity: "error",
        code: Some("revision.stale_program_path"),
        message,
        source_span_json: None,
        subject_type: Some("program_path"),
        subject_id: Some(&subject_id),
        event_id: None,
        effect_id: None,
        run_id: None,
        assertion_id: None,
        evidence_ids_json: "[]",
        artifact_ids_json: "[]",
        causation_id: None,
        correlation_id: Some(active_version_id),
        idempotency_key: Some(&idempotency_key),
    })?;
    Ok(())
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
    target_id: String,
    event_id: Option<String>,
    diagnostic_ids: Vec<String>,
    expr: String,
    reads: Vec<AssertionReadReport>,
    tags: Vec<String>,
    description: Option<String>,
    source_span_json: Option<String>,
    status: AssertionStatus,
    passed: bool,
    actual: Value,
    actual_values: Value,
    expected: Value,
    failure_reason: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AssertionReadReport {
    kind: String,
    head: String,
    guard: Option<String>,
    source: String,
    match_count: usize,
    matches: Vec<AssertionReadMatch>,
    error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AssertionReadMatch {
    id: String,
    name: String,
    key: Option<String>,
    status: Option<String>,
    prompt_content_type: Option<String>,
    provenance_class: Option<String>,
    source_span_json: Option<String>,
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
    active_version_guard: Option<&str>,
) -> Result<StepReport, StoreError> {
    let mut report = StepReport {
        instance_id: instance_id.to_owned(),
        ..StepReport::default()
    };
    let mut made_progress = true;
    while made_progress {
        made_progress = false;
        let store = SqliteStore::open(store_path)?;
        let status = store
            .status(instance_id)?
            .ok_or_else(|| StoreError::Conflict("instance does not exist".to_owned()))?;
        if status.instance.status != "running" {
            break;
        }
        if let Some(active_version_guard) = active_version_guard {
            if status.instance.version_id != active_version_guard {
                return Err(StoreError::Conflict(format!(
                    "active version changed during step from {active_version_guard} to {}; rerun `whip step` with the active program",
                    status.instance.version_id
                )));
            }
        }
        let active_version_id = status.instance.version_id;
        let active_revision_epoch = status.instance.revision_epoch;
        let active_revision_epoch_key = active_revision_epoch.to_string();
        drop(store);
        project_queue_items(store_path, instance_id, ir)?;
        let store = SqliteStore::open(store_path)?;
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
                let lowering = lower_rule(
                    instance_id,
                    ir,
                    rule,
                    &context,
                    &all_facts,
                    &effects,
                    source_path,
                );
                report
                    .branch_reports
                    .extend(lowering.branch_reports.iter().cloned());
                if !lowering.errors.is_empty() {
                    let message = format!(
                        "rule `{}` lowering failed: {}",
                        rule.name,
                        lowering.errors.join("; ")
                    );
                    store.record_diagnostic(DiagnosticRecord {
                        instance_id: Some(instance_id),
                        program_id: None,
                        program_version_id: Some(&active_version_id),
                        severity: "error",
                        code: Some("rule.lowering.unresolved"),
                        message: &message,
                        source_span_json: None,
                        subject_type: Some("rule"),
                        subject_id: Some(&rule.name),
                        event_id: None,
                        effect_id: None,
                        run_id: None,
                        assertion_id: None,
                        evidence_ids_json: "[]",
                        artifact_ids_json: "[]",
                        causation_id: context.trigger_event_id.as_deref(),
                        correlation_id: context.identity.as_deref(),
                        idempotency_key: Some(&idempotency_key(&[
                            instance_id,
                            &active_version_id,
                            &active_revision_epoch_key,
                            &rule.name,
                            "lowering-error",
                            &lowering.errors.join("|"),
                        ])),
                    })?;
                    return Err(StoreError::Conflict(message));
                }
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
                    &active_version_id,
                    &active_revision_epoch_key,
                    &rule.name,
                    context.identity.as_deref().unwrap_or("started"),
                    &lowering_key,
                ]);
                let event = kernel.commit_rule_with_revision_guard(
                    RuleCommit {
                        instance_id,
                        rule: &rule.name,
                        trigger_event_id: context.trigger_event_id.as_deref(),
                        facts: &new_facts,
                        consumed_fact_ids: &consumed_fact_ids,
                        effects: &new_effects,
                        dependencies: &new_dependencies,
                        terminal,
                        idempotency_key: Some(&commit_key),
                    },
                    RuleCommitRevisionGuard {
                        program_version_id: &active_version_id,
                        revision_epoch: active_revision_epoch,
                    },
                );
                store = kernel.into_store();
                drop(store);
                match event {
                    Ok(committed) => {
                        report.committed_rules += 1;
                        report.facts_created += new_facts.len();
                        report.facts_consumed += consumed_fact_ids.len();
                        report.effects_created += new_effects.len();
                        // Holder-lifetime bound (spec/coordination.md): an
                        // instance reaching a workflow terminal auto-releases
                        // every lease it held.
                        if lowering.terminal.is_some() {
                            let mut coordination =
                                whipplescript_store::coordination::CoordinationStore::open(
                                    coordination_store_path(),
                                )?;
                            let _ = coordination.release_all_for_holder(instance_id)?;
                        }
                        apply_rule_cancels(
                            store_path,
                            instance_id,
                            &rule.name,
                            &lowering.cancels,
                            &committed.event_id,
                        )?;
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
                "worker {} ran={} provider={} cancellation_acknowledgements={} cancellation_diagnostics={}",
                report.instance_id,
                report.ran_effects,
                report.provider,
                report.cancellation_acknowledgements,
                report.cancellation_diagnostics
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
    exec_profile: ExecProfile,
    script_manifest_path: Option<PathBuf>,
    outcome: FixtureOutcome,
    /// Fixture knob: which sum-type variant a fixture coerce returns
    /// (spec/sum-types.md); default is the first declared variant.
    variant: Option<String>,
    program_path: Option<PathBuf>,
    root: Option<String>,
    provider_config_paths: Vec<PathBuf>,
    max_child_iterations: usize,
}

impl WorkerOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut instance_id = None;
        let mut provider = "fixture".to_owned();
        let mut exec_profile = ExecProfile::from_env();
        let mut script_manifest_path = script_manifest_path_from_env();
        let mut outcome = FixtureOutcome::Completed;
        let mut variant = None;
        let mut program_path = None;
        let mut root = None;
        let mut provider_config_paths = Vec::new();
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
                "--provider-config" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected path after `--provider-config`".to_owned());
                    };
                    provider_config_paths.push(PathBuf::from(value));
                }
                "--exec-profile" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected profile after `--exec-profile`".to_owned());
                    };
                    exec_profile = ExecProfile::parse(value)?;
                }
                "--script-manifest" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected path after `--script-manifest`".to_owned());
                    };
                    script_manifest_path = Some(PathBuf::from(value));
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
                "--variant" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected variant name after `--variant`".to_owned());
                    };
                    variant = Some(value.clone());
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
                        "usage: whip worker <instance> [--provider fixture] [--provider-config path] [--once] [--fail|--timeout|--cancel]".to_owned(),
                    )
                }
            }
            index += 1;
        }
        let Some(instance_id) = instance_id else {
            return Err(
                "usage: whip worker <instance> [--provider fixture] [--provider-config path] [--once] [--fail|--timeout|--cancel]"
                    .to_owned(),
            );
        };
        Ok(Self {
            instance_id,
            provider,
            exec_profile,
            script_manifest_path,
            outcome,
            variant,
            program_path,
            root,
            provider_config_paths,
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
    timers_fired: usize,
    deadlines_expired: usize,
    cancellation_acknowledgements: usize,
    cancellation_diagnostics: usize,
    terminal_events: Vec<String>,
}

fn run_worker_once(store_path: &Path, options: &WorkerOptions) -> Result<WorkerReport, StoreError> {
    let mut report = WorkerReport {
        instance_id: options.instance_id.clone(),
        provider: options.provider.clone(),
        ..WorkerReport::default()
    };
    let cancellation_report =
        process_running_cancellations(store_path, &options.instance_id, &options.provider)?;
    report.cancellation_acknowledgements = cancellation_report.acknowledgements;
    report.cancellation_diagnostics = cancellation_report.diagnostics;
    let time_report = resolve_due_time_effects(store_path, &options.instance_id)?;
    report.timers_fired = time_report.timers_fired;
    report.deadlines_expired = time_report.deadlines_expired;
    report.terminal_events.extend(time_report.terminal_events);
    let script_manifest = load_script_manifest(options.script_manifest_path.as_deref())
        .map_err(StoreError::Conflict)?;
    if let Some(manifest) = script_manifest.as_ref() {
        let store = SqliteStore::open(store_path)?;
        let instance = store
            .get_instance(&options.instance_id)?
            .ok_or_else(|| StoreError::Conflict("instance not found".to_owned()))?;
        register_script_manifest_capabilities(&store, manifest, &instance.program_id)?;
    }
    let store = SqliteStore::open(store_path)?;
    let mut claimable = store.claimable_effects(&options.instance_id)?;
    let mut seen_claimable = claimable
        .iter()
        .map(|effect| effect.effect_id.clone())
        .collect::<BTreeSet<_>>();
    for effect in store.list_effects(&options.instance_id)? {
        if effect.kind == "exec.command"
            && effect.status == "queued"
            && effect.policy_block_reason.is_none()
            && !effect.cancel_requested
            && seen_claimable.insert(effect.effect_id.clone())
        {
            claimable.push(ClaimableEffect {
                effect_id: effect.effect_id,
                kind: effect.kind,
                target: effect.target,
                profile: effect.profile,
                input_json: effect.input_json,
                required_capabilities_json: effect.required_capabilities_json,
                declared_profiles_json: effect.declared_profiles_json,
            });
        }
    }
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
            "queue.file" | "queue.claim" | "queue.release" | "queue.finish" => {
                run_queue_effect(store_path, &options.instance_id, &effect, options)?
            }
            "exec.command" => run_exec_effect(
                store_path,
                &options.instance_id,
                &effect,
                options.exec_profile,
                script_manifest.as_ref(),
            )?,
            "lease.acquire" | "lease.release" | "ledger.append" | "counter.consume" => {
                run_coordination_effect(store_path, &options.instance_id, &effect)?
            }
            "event.notify" => run_notify_effect(store_path, &options.instance_id, &effect)?,
            _ => continue,
        };
        report.ran_effects += 1;
        report.terminal_events.push(terminal.event_id);
    }
    Ok(report)
}

/// Workspace-scoped coordination state (spec/coordination.md): shared
/// lease/ledger/counter durable state outlives disposable run stores.
fn coordination_store_path() -> PathBuf {
    env::var("WHIPPLESCRIPT_COORDINATION_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".whipplescript/coordination.sqlite"))
}

/// Workspace-scoped builtin tracker path: the backlog outlives disposable
/// run stores.
fn items_store_path() -> PathBuf {
    env::var("WHIPPLESCRIPT_ITEMS_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".whipplescript/items.sqlite"))
}

/// Projects ready work items from declared builtin queues into
/// instance-local `queue.item.ready` facts, and retires projections whose
/// items are no longer ready. The tracker is the source of truth; the run
/// store holds a cache keyed (queue, id).
fn project_queue_items(
    store_path: &Path,
    instance_id: &str,
    ir: &IrProgram,
) -> Result<(), StoreError> {
    if ir.queues.is_empty() {
        return Ok(());
    }
    for queue in &ir.queues {
        if queue.tracker != "builtin" {
            continue;
        }
        let items = whipplescript_store::items::WorkItemStore::open(items_store_path())?;
        // Keep a projection alive while this instance holds the claim: the
        // dispatching rule's multi-stage chain needs its trigger fact until
        // the item is finished or released. Re-fires are idempotent (effect
        // ids are identity-derived), matching the engine's existing idiom.
        let ready = items
            .list_items(Some(&queue.name), None)?
            .into_iter()
            .filter(|item| {
                (item.status == "open" && item.claimed_by.is_none())
                    || (item.status == "in_progress"
                        && item.claimed_by.as_deref() == Some(instance_id))
            })
            .collect::<Vec<_>>();
        drop(items);
        let store = SqliteStore::open(store_path)?;
        let existing = store
            .list_facts(instance_id)?
            .into_iter()
            .filter(|fact| fact.name == "queue.item.ready")
            .filter(|fact| {
                json_from_str(&fact.value_json)
                    .get("queue")
                    .and_then(Value::as_str)
                    == Some(queue.name.as_str())
            })
            .collect::<Vec<_>>();
        drop(store);
        let ready_prefixes = ready
            .iter()
            .map(|item| format!("{}:{}:", queue.name, item.id))
            .collect::<Vec<_>>();
        for item in &ready {
            let prefix = format!("{}:{}:", queue.name, item.id);
            if existing.iter().any(|fact| fact.key.starts_with(&prefix)) {
                continue;
            }
            // Salt the key with the item's update generation: a released
            // item re-projects as a fresh fact instead of colliding with
            // its retired predecessor.
            let key = format!("{prefix}{}", stable_hash_hex(&item.updated_at));
            let value_json = json!({
                "queue": queue.name,
                "id": item.id,
                "title": item.title,
                "body": item.body,
                "status": item.status,
                "labels": item.labels,
                "metadata": item.metadata,
            })
            .to_string();
            let store = SqliteStore::open(store_path)?;
            let mut kernel = RuntimeKernel::new(store);
            // Salt with updated_at: a released item re-projects as a fresh
            // fact generation instead of colliding with its retired one.
            kernel.derive_fact(
                instance_id,
                "queue.item.ready",
                &key,
                &value_json,
                None,
                Some(&idempotency_key(&[
                    instance_id,
                    "queue.item.ready",
                    &key,
                    &item.updated_at,
                ])),
            )?;
        }
        for fact in existing {
            if !ready_prefixes
                .iter()
                .any(|prefix| fact.key.starts_with(prefix))
            {
                let mut store = SqliteStore::open(store_path)?;
                store.retire_fact(instance_id, &fact.fact_id)?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TimePassReport {
    timers_fired: usize,
    deadlines_expired: usize,
    terminal_events: Vec<String>,
}

/// Resolves creation-anchored time: fires due `timer.wait` effects and
/// expires effects whose `timeout` deadline passed. Wall-clock reads live
/// here, on worker passes — never in rule evaluation.
fn resolve_due_time_effects(
    store_path: &Path,
    instance_id: &str,
) -> Result<TimePassReport, StoreError> {
    let mut report = TimePassReport::default();
    let store = SqliteStore::open(store_path)?;
    let due = store.due_time_effects(instance_id)?;
    drop(store);
    for effect in due {
        if effect.kind == "timer.wait" {
            let run_id = idempotency_key(&[instance_id, &effect.effect_id, "timer-run"]);
            let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "timer-lease"]);
            let store = SqliteStore::open(store_path)?;
            let mut kernel = RuntimeKernel::new(store);
            kernel.start_run(RunStart {
                instance_id,
                effect_id: &effect.effect_id,
                run_id: &run_id,
                provider: "timer",
                worker_id: "whip-timer",
                lease_id: &lease_id,
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: &json!({
                    "duration_seconds": effect.timeout_seconds,
                })
                .to_string(),
            })?;
            let terminal = kernel.complete_run(EffectCompletion {
                instance_id,
                effect_id: &effect.effect_id,
                run_id: &run_id,
                provider: "timer",
                worker_id: "whip-timer",
                status: "completed",
                exit_code: Some(0),
                summary: Some("timer fired"),
                metadata_json: &json!({
                    "duration_seconds": effect.timeout_seconds,
                })
                .to_string(),
                idempotency_key: Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "timer-terminal",
                ])),
            })?;
            let value_json = json!({
                "effect_id": effect.effect_id,
                "run_id": run_id,
                "status": "completed",
                "fired": true,
                "duration_seconds": effect.timeout_seconds,
            })
            .to_string();
            kernel.derive_fact(
                instance_id,
                "timer.fired",
                &effect.effect_id,
                &value_json,
                Some(&terminal.event_id),
                Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "timer.fired",
                ])),
            )?;
            report.timers_fired += 1;
            report.terminal_events.push(terminal.event_id);
            continue;
        }
        // Deadline expiry: running effects time out at the run level and get
        // a cancellation request; never-run effects expire directly.
        let store = SqliteStore::open(store_path)?;
        let running_run = store
            .list_runs(instance_id)?
            .into_iter()
            .find(|run| run.effect_id == effect.effect_id && run.status == "running");
        drop(store);
        let terminal_event_id = match running_run {
            Some(run) => {
                let store = SqliteStore::open(store_path)?;
                let mut kernel = RuntimeKernel::new(store);
                let terminal = kernel.timeout_run(EffectCompletion {
                    instance_id,
                    effect_id: &effect.effect_id,
                    run_id: &run.run_id,
                    provider: &run.provider,
                    worker_id: &run.worker_id,
                    status: "timed_out",
                    exit_code: None,
                    summary: Some("deadline exceeded"),
                    metadata_json: &json!({
                        "timeout_seconds": effect.timeout_seconds,
                        "reason": "deadline exceeded",
                    })
                    .to_string(),
                    idempotency_key: Some(&idempotency_key(&[
                        instance_id,
                        &effect.effect_id,
                        "deadline-terminal",
                    ])),
                })?;
                let store = kernel.into_store();
                let mut store = store;
                let _ = store.request_effect_cancellation(EffectCancellationRequest {
                    instance_id,
                    effect_id: &effect.effect_id,
                    revision_id: None,
                    reason: Some("deadline exceeded"),
                    requested_by: "deadline",
                    causation_event_id: Some(&terminal.event_id),
                    idempotency_key: Some(&idempotency_key(&[
                        instance_id,
                        &effect.effect_id,
                        "deadline-cancel-request",
                    ])),
                });
                terminal.event_id
            }
            None => {
                let mut store = SqliteStore::open(store_path)?;
                let terminal = store.expire_effect(
                    instance_id,
                    &effect.effect_id,
                    Some(&idempotency_key(&[
                        instance_id,
                        &effect.effect_id,
                        "deadline-terminal",
                    ])),
                )?;
                terminal.event_id
            }
        };
        let store = SqliteStore::open(store_path)?;
        let mut kernel = RuntimeKernel::new(store);
        let value_json = json!({
            "effect_id": effect.effect_id,
            "status": "timed_out",
            "reason": "deadline exceeded",
            "timeout_seconds": effect.timeout_seconds,
        })
        .to_string();
        kernel.derive_fact(
            instance_id,
            "effect.timed_out",
            &effect.effect_id,
            &value_json,
            Some(&terminal_event_id),
            Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "effect.timed_out",
            ])),
        )?;
        report.deadlines_expired += 1;
        report.terminal_events.push(terminal_event_id);
    }
    Ok(report)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RunningCancellationReport {
    acknowledgements: usize,
    diagnostics: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderCancellationPolicy {
    Unsupported,
    NativeStop {
        acknowledgement_order: CancellationAcknowledgementOrder,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CancellationAcknowledgementOrder {
    BeforeTerminal,
    AfterTerminalAllowed,
}

impl CancellationAcknowledgementOrder {
    fn as_str(self) -> &'static str {
        match self {
            Self::BeforeTerminal => "before_terminal",
            Self::AfterTerminalAllowed => "after_terminal_allowed",
        }
    }
}

fn process_running_cancellations(
    store_path: &Path,
    instance_id: &str,
    provider: &str,
) -> Result<RunningCancellationReport, StoreError> {
    let store = SqliteStore::open(store_path)?;
    let requests = store.list_effect_cancellation_requests(instance_id)?;
    let open_requests = requests
        .iter()
        .filter(|request| request.status == "requested")
        .map(|request| (request.effect_id.as_str(), request))
        .collect::<BTreeMap<_, _>>();
    if open_requests.is_empty() {
        return Ok(RunningCancellationReport::default());
    }

    let mut report = RunningCancellationReport::default();
    for run in store
        .list_runs(instance_id)?
        .into_iter()
        .filter(|run| run.status == "running" && run.cancel_requested && run.provider == provider)
    {
        let Some(request) = open_requests.get(run.effect_id.as_str()) else {
            continue;
        };
        if let ProviderCancellationPolicy::NativeStop {
            acknowledgement_order,
        } = provider_cancellation_policy(provider)
        {
            let metadata_json = json!({
                "cancellation": "provider_acknowledged",
                "cancellation_depth": "native_stop",
                "acknowledgement_order": acknowledgement_order.as_str(),
                "request_id": request.request_id,
                "revision_id": request.revision_id,
                "reason": request.reason,
                "provider": provider,
                "run_id": run.run_id,
            })
            .to_string();
            let store = SqliteStore::open(store_path)?;
            let mut kernel = RuntimeKernel::new(store);
            kernel.cancel_run(EffectCompletion {
                instance_id,
                effect_id: &run.effect_id,
                run_id: &run.run_id,
                provider: &run.provider,
                worker_id: &run.worker_id,
                status: "cancelled",
                exit_code: Some(0),
                summary: Some("provider acknowledged cancellation request"),
                metadata_json: &metadata_json,
                idempotency_key: Some(&idempotency_key(&[
                    instance_id,
                    &run.run_id,
                    "provider-cancellation-acknowledged",
                ])),
            })?;
            report.acknowledgements += 1;
            continue;
        }

        let message = format!(
            "provider `{provider}` does not support out-of-band cancellation; effect `{}` remains running until the provider exits, times out, or lease recovery resolves it",
            run.effect_id
        );
        let idempotency_key = format!(
            "provider-cancellation-unsupported:{}:{}:{}",
            request.request_id, run.run_id, provider
        );
        store.record_diagnostic(DiagnosticRecord {
            instance_id: Some(instance_id),
            program_id: None,
            program_version_id: None,
            severity: "warning",
            code: Some("provider.cancellation.unsupported"),
            message: &message,
            source_span_json: None,
            subject_type: Some("effect"),
            subject_id: Some(&run.effect_id),
            event_id: request.causation_event_id.as_deref(),
            effect_id: Some(&run.effect_id),
            run_id: Some(&run.run_id),
            assertion_id: None,
            evidence_ids_json: "[]",
            artifact_ids_json: "[]",
            causation_id: Some(&request.request_id),
            correlation_id: request.revision_id.as_deref(),
            idempotency_key: Some(&idempotency_key),
        })?;
        report.diagnostics += 1;
    }
    Ok(report)
}

fn provider_cancellation_policy(provider: &str) -> ProviderCancellationPolicy {
    let normalized = provider.to_ascii_lowercase();
    if normalized == "fixture-cancellable"
        || normalized == "codex"
        || normalized.starts_with("codex-")
    {
        return ProviderCancellationPolicy::NativeStop {
            acknowledgement_order: CancellationAcknowledgementOrder::BeforeTerminal,
        };
    }
    if normalized == "pi" || normalized.starts_with("pi-") {
        return ProviderCancellationPolicy::NativeStop {
            acknowledgement_order: CancellationAcknowledgementOrder::AfterTerminalAllowed,
        };
    }
    ProviderCancellationPolicy::Unsupported
}

fn run_agent_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    let input_json = resolve_effect_input_after_bindings(store_path, instance_id, effect)?;
    let config_paths = provider_config_paths_with_env(&options.provider_config_paths);
    let provider_selection =
        agent_provider_selection_with_config_paths(effect, &options.provider, &config_paths)?;
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "lease"]);
    let execution = AgentTurnExecution {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: &provider_selection.provider_id,
        worker_id: "whip-worker",
        lease_id: &lease_id,
        lease_expires_at: "2030-01-01T00:00:00Z",
        agent: effect.target.as_deref().unwrap_or("agent"),
        profile: effect.profile.as_deref(),
        input_json: &input_json,
        skill_names: &[],
    };
    if provider_selection.kind == "native-fixture" {
        let mut adapter = NativeFixtureAdapter::new(
            &provider_selection.provider_id,
            &run_id,
            options.outcome.is_failed(),
        );
        let metadata_json = agent_provider_selection_metadata_json(&provider_selection);
        return kernel.run_native_agent_turn_with_metadata(
            execution,
            native_fixture_turn_request(execution, &input_json),
            &mut adapter,
            8,
            &metadata_json,
        );
    }
    if provider_selection.kind == "codex" {
        let request = codex_native_turn_request(
            execution,
            effect,
            &input_json,
            provider_selection.provider_config.as_ref(),
        )?;
        let mut unavailable_adapter;
        let mut adapter;
        let native_adapter: &mut dyn NativeProviderAdapter =
            match codex_app_server_adapter(&provider_selection.provider_id) {
                Ok(healthy_adapter) => {
                    adapter = healthy_adapter;
                    &mut adapter
                }
                Err(error) => {
                    unavailable_adapter = unavailable_native_provider_adapter(
                        &provider_selection.provider_id,
                        ProviderKind::Codex,
                        AdapterSurface::CodexAppServer,
                        error,
                    )?;
                    &mut unavailable_adapter
                }
            };
        let metadata_json = agent_provider_selection_metadata_json(&provider_selection);
        return kernel.run_native_agent_turn_with_metadata(
            execution,
            request,
            native_adapter,
            native_provider_max_events(),
            &metadata_json,
        );
    }
    if provider_selection.kind == "claude" {
        let request = claude_native_turn_request(
            execution,
            effect,
            &input_json,
            provider_selection.provider_config.as_ref(),
        )?;
        let mut unavailable_adapter;
        let mut adapter;
        let native_adapter: &mut dyn NativeProviderAdapter =
            match claude_agent_sdk_adapter(&provider_selection.provider_id) {
                Ok(healthy_adapter) => {
                    adapter = healthy_adapter;
                    &mut adapter
                }
                Err(error) => {
                    unavailable_adapter = unavailable_native_provider_adapter(
                        &provider_selection.provider_id,
                        ProviderKind::Claude,
                        AdapterSurface::ClaudeAgentSdk,
                        error,
                    )?;
                    &mut unavailable_adapter
                }
            };
        let metadata_json = agent_provider_selection_metadata_json(&provider_selection);
        return kernel.run_native_agent_turn_with_metadata(
            execution,
            request,
            native_adapter,
            native_provider_max_events(),
            &metadata_json,
        );
    }
    if provider_selection.kind == "pi" {
        let request = pi_native_turn_request(
            execution,
            effect,
            &input_json,
            provider_selection.provider_config.as_ref(),
        )?;
        let mut unavailable_adapter;
        let mut adapter;
        let native_adapter: &mut dyn NativeProviderAdapter =
            match pi_rpc_adapter(&provider_selection.provider_id) {
                Ok(healthy_adapter) => {
                    adapter = healthy_adapter;
                    &mut adapter
                }
                Err(error) => {
                    unavailable_adapter = unavailable_native_provider_adapter(
                        &provider_selection.provider_id,
                        ProviderKind::Pi,
                        AdapterSurface::PiRpc,
                        error,
                    )?;
                    &mut unavailable_adapter
                }
            };
        let metadata_json = agent_provider_selection_metadata_json(&provider_selection);
        return kernel.run_native_agent_turn_with_metadata(
            execution,
            request,
            native_adapter,
            native_provider_max_events(),
            &metadata_json,
        );
    }
    if provider_selection.kind == "command" {
        let Some(plan) = provider_selection.command_plan.clone() else {
            return Err(StoreError::Conflict(format!(
                "agent `{}` is bound to command harness `{}`, but no command provider config was found",
                effect.target.as_deref().unwrap_or("<unknown>"),
                provider_selection.provider_id
            )));
        };
        let harness = CommandAgentHarness::new(plan);
        let metadata_json = agent_provider_selection_metadata_json(&provider_selection);
        return kernel.run_agent_turn_with_metadata(execution, &harness, &metadata_json);
    }

    let harness = fixture_harness(&provider_selection.provider_id, options.outcome.is_failed());
    let metadata_json = agent_provider_selection_metadata_json(&provider_selection);
    kernel.run_agent_turn_with_metadata(execution, &harness, &metadata_json)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentProviderSelection {
    provider_id: String,
    kind: String,
    source_harness_id: Option<String>,
    surface: Option<String>,
    provider_config: Option<ProviderBindingConfig>,
    command_plan: Option<CommandLaunchPlan>,
}

fn agent_provider_selection_with_config_paths(
    effect: &ClaimableEffect,
    fallback_provider: &str,
    config_paths: &[PathBuf],
) -> Result<AgentProviderSelection, StoreError> {
    let Some(agent) = effect.target.as_deref() else {
        return Ok(fallback_provider_selection(fallback_provider));
    };
    let declared = serde_json::from_str::<Value>(&effect.declared_profiles_json)?;
    if let Some(harness) = declared_agent_harness_in_value(&declared, agent) {
        let Some(kind) = declared_harness_kind_in_value(&declared, &harness) else {
            return Err(StoreError::Conflict(format!(
                "agent `{agent}` is bound to harness `{harness}`, but that harness is not declared"
            )));
        };
        if let Some(config) = provider_binding_for_harness(&harness, config_paths)? {
            let config_kind = config.provider_kind.as_str();
            if config_kind != kind {
                return Err(StoreError::Conflict(format!(
                    "provider config for harness `{harness}` has kind `{config_kind}`, but source declares `{kind}`"
                )));
            }
            let command_plan = if kind == "command" {
                Some(command_launch_plan_from_config(&config)?)
            } else {
                None
            };
            return Ok(AgentProviderSelection {
                provider_id: config.provider_id.clone(),
                kind,
                source_harness_id: Some(harness),
                surface: Some(config.surface.as_str().to_owned()),
                provider_config: Some(config),
                command_plan,
            });
        }
        let execution_kind = if fallback_provider == "fixture" {
            "fixture".to_owned()
        } else {
            kind
        };
        return Ok(AgentProviderSelection {
            provider_id: harness.clone(),
            kind: execution_kind,
            source_harness_id: Some(harness),
            surface: None,
            provider_config: None,
            command_plan: None,
        });
    }

    if let Some(provider) = declared_agent_provider_in_value(&declared, agent) {
        let kind = provider.clone();
        if let Some(config) = provider_binding_for_harness(&provider, config_paths)? {
            let config_kind = config.provider_kind.as_str();
            if config_kind != kind {
                return Err(StoreError::Conflict(format!(
                    "provider config for direct provider `{provider}` has kind `{config_kind}`, but source declares `{kind}`"
                )));
            }
            let command_plan = if kind == "command" {
                Some(command_launch_plan_from_config(&config)?)
            } else {
                None
            };
            return Ok(AgentProviderSelection {
                provider_id: config.provider_id.clone(),
                kind,
                source_harness_id: None,
                surface: Some(config.surface.as_str().to_owned()),
                provider_config: Some(config),
                command_plan,
            });
        }
        let execution_kind = if fallback_provider == "fixture" {
            "fixture".to_owned()
        } else {
            kind
        };
        return Ok(AgentProviderSelection {
            provider_id: provider,
            kind: execution_kind,
            source_harness_id: None,
            surface: None,
            provider_config: None,
            command_plan: None,
        });
    }

    Ok(fallback_provider_selection(fallback_provider))
}

fn fallback_provider_selection(provider: &str) -> AgentProviderSelection {
    AgentProviderSelection {
        provider_id: provider.to_owned(),
        kind: fallback_provider_kind(provider).to_owned(),
        source_harness_id: None,
        surface: None,
        provider_config: None,
        command_plan: None,
    }
}

fn agent_provider_selection_metadata_json(selection: &AgentProviderSelection) -> String {
    json!({
        "provider_selection": {
            "provider_id": selection.provider_id,
            "provider_kind": selection.kind,
            "source_harness_id": selection.source_harness_id,
            "surface": selection.surface,
        }
    })
    .to_string()
}

fn provider_config_paths_with_env(explicit_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = explicit_paths.to_vec();
    paths.extend(provider_config_paths_from_env());
    paths
}

fn provider_config_paths_from_env() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for env_key in [
        "WHIPPLESCRIPT_PROVIDER_CONFIGS",
        "WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS",
    ] {
        if let Some(value) = env::var_os(env_key) {
            paths.extend(env::split_paths(&value).filter(|path| !path.as_os_str().is_empty()));
        }
    }
    paths
}

fn provider_binding_for_harness(
    harness: &str,
    config_paths: &[PathBuf],
) -> Result<Option<ProviderBindingConfig>, StoreError> {
    for path in config_paths {
        let config_json = fs::read_to_string(path)?;
        let (_results, bindings) = validate_doctor_provider_config_json_with_bindings(
            &config_json,
            &builtin_provider_capabilities(),
        );
        for binding in bindings {
            if binding.config.provider_id != harness {
                continue;
            }
            if let Some(failure) = binding
                .results
                .iter()
                .find(|result| result.status == ProviderValidationStatus::Fail)
            {
                return Err(StoreError::Conflict(format!(
                    "provider config for harness `{harness}` failed validation: {}",
                    failure.message
                )));
            }
            return Ok(Some(binding.config));
        }
    }
    Ok(None)
}

fn command_launch_plan_from_config(
    config: &ProviderBindingConfig,
) -> Result<CommandLaunchPlan, StoreError> {
    if config.provider_kind != ProviderKind::Command || config.surface != AdapterSurface::Command {
        return Err(StoreError::Conflict(format!(
            "provider config `{}` must use provider_kind `command` and surface `command` for a command harness",
            config.provider_id
        )));
    }
    let executable = required_extra_string(config, "executable")?;
    let mut plan = CommandLaunchPlan::new(&config.provider_id, executable);

    for arg in optional_extra_string_array(config, "args")? {
        plan = plan.arg(arg);
    }
    if let Some(cwd) = optional_extra_string(config, "cwd")? {
        plan = plan.cwd(cwd);
    }
    for (key, value) in optional_extra_string_map(config, "env")? {
        plan = plan.env(key, value);
    }
    for key in optional_extra_string_array(config, "required_env")? {
        plan = plan.require_env(key);
    }
    for command in optional_extra_string_array(config, "required_commands")? {
        plan = plan.require_command(command);
    }
    if let Some(timeout_ms) = config.timeout_ms {
        plan = plan.timeout(Duration::from_millis(timeout_ms));
    }
    if optional_extra_bool(config, "require_stdout_json")?.unwrap_or(false) {
        plan = plan.require_stdout_json();
    }

    Ok(plan)
}

fn required_extra_string(config: &ProviderBindingConfig, key: &str) -> Result<String, StoreError> {
    optional_extra_string(config, key)?.ok_or_else(|| {
        StoreError::Conflict(format!(
            "provider config `{}` is missing required command field `{key}`",
            config.provider_id
        ))
    })
}

fn optional_extra_string(
    config: &ProviderBindingConfig,
    key: &str,
) -> Result<Option<String>, StoreError> {
    let Some(value) = config.extra.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(StoreError::Conflict(format!(
            "provider config `{}` field `{key}` must be a string",
            config.provider_id
        )));
    };
    Ok(Some(value.to_owned()))
}

fn optional_extra_bool(
    config: &ProviderBindingConfig,
    key: &str,
) -> Result<Option<bool>, StoreError> {
    let Some(value) = config.extra.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_bool() else {
        return Err(StoreError::Conflict(format!(
            "provider config `{}` field `{key}` must be a boolean",
            config.provider_id
        )));
    };
    Ok(Some(value))
}

fn optional_extra_string_array(
    config: &ProviderBindingConfig,
    key: &str,
) -> Result<Vec<String>, StoreError> {
    let Some(value) = config.extra.get(key) else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(StoreError::Conflict(format!(
            "provider config `{}` field `{key}` must be an array of strings",
            config.provider_id
        )));
    };
    let mut values = Vec::new();
    for item in items {
        let Some(item) = item.as_str() else {
            return Err(StoreError::Conflict(format!(
                "provider config `{}` field `{key}` must be an array of strings",
                config.provider_id
            )));
        };
        values.push(item.to_owned());
    }
    Ok(values)
}

fn optional_extra_string_map(
    config: &ProviderBindingConfig,
    key: &str,
) -> Result<BTreeMap<String, String>, StoreError> {
    let Some(value) = config.extra.get(key) else {
        return Ok(BTreeMap::new());
    };
    let Some(object) = value.as_object() else {
        return Err(StoreError::Conflict(format!(
            "provider config `{}` field `{key}` must be an object of string values",
            config.provider_id
        )));
    };
    let mut values = BTreeMap::new();
    for (entry_key, entry_value) in object {
        let Some(entry_value) = entry_value.as_str() else {
            return Err(StoreError::Conflict(format!(
                "provider config `{}` field `{key}` must be an object of string values",
                config.provider_id
            )));
        };
        values.insert(entry_key.clone(), entry_value.to_owned());
    }
    Ok(values)
}

fn fallback_provider_kind(provider: &str) -> &'static str {
    if provider == "native-fixture" {
        "native-fixture"
    } else if is_codex_native_provider(provider) {
        "codex"
    } else if is_claude_native_provider(provider) {
        "claude"
    } else if is_pi_native_provider(provider) {
        "pi"
    } else {
        "fixture"
    }
}

fn declared_agent_harness_in_value(value: &Value, agent: &str) -> Option<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| declared_agent_harness_in_value(item, agent)),
        Value::Object(object) => {
            if let Some(entry) = object.get(agent) {
                return entry
                    .get("harness")
                    .or_else(|| entry.get("harness_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            let object_agent = object
                .get("name")
                .or_else(|| object.get("agent_name"))
                .and_then(Value::as_str);
            if object_agent == Some(agent) {
                return object
                    .get("harness")
                    .or_else(|| object.get("harness_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            object
                .get("agents")
                .and_then(|agents| declared_agent_harness_in_value(agents, agent))
        }
        _ => None,
    }
}

fn declared_agent_provider_in_value(value: &Value, agent: &str) -> Option<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| declared_agent_provider_in_value(item, agent)),
        Value::Object(object) => {
            if let Some(entry) = object.get(agent) {
                return entry
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            let object_agent = object
                .get("name")
                .or_else(|| object.get("agent_name"))
                .and_then(Value::as_str);
            if object_agent == Some(agent) {
                return object
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            object
                .get("agents")
                .and_then(|agents| declared_agent_provider_in_value(agents, agent))
        }
        _ => None,
    }
}

fn declared_harness_kind_in_value(value: &Value, harness: &str) -> Option<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| declared_harness_kind_in_value(item, harness)),
        Value::Object(object) => {
            if let Some(entry) = object.get(harness) {
                if let Some(kind) = entry
                    .get("kind")
                    .or_else(|| entry.get("provider_kind"))
                    .and_then(Value::as_str)
                {
                    return Some(kind.to_owned());
                }
            }
            let object_harness = object
                .get("name")
                .or_else(|| object.get("harness_id"))
                .and_then(Value::as_str);
            if object_harness == Some(harness) {
                return object
                    .get("kind")
                    .or_else(|| object.get("provider_kind"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            object
                .get("harnesses")
                .and_then(|harnesses| declared_harness_kind_in_value(harnesses, harness))
        }
        _ => None,
    }
}

fn is_codex_native_provider(provider: &str) -> bool {
    let normalized = provider.to_ascii_lowercase();
    normalized == "codex" || normalized.starts_with("codex-")
}

fn is_claude_native_provider(provider: &str) -> bool {
    let normalized = provider.to_ascii_lowercase();
    normalized == "claude" || normalized.starts_with("claude-")
}

fn is_pi_native_provider(provider: &str) -> bool {
    let normalized = provider.to_ascii_lowercase();
    normalized == "pi" || normalized.starts_with("pi-")
}

fn codex_app_server_adapter(
    provider: &str,
) -> Result<CodexAppServerAdapter<StdioCodexAppServerTransport>, StoreError> {
    let model = env::var("WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL")
        .unwrap_or_else(|_| "gpt-5.4-mini".to_owned());
    let model_config = format!("model={model:?}");
    let args = [
        "app-server",
        "--listen",
        "stdio://",
        "-c",
        model_config.as_str(),
        "-c",
        "sandbox_mode=\"read-only\"",
        "-c",
        "approval_policy=\"never\"",
    ];
    let command =
        env::var("WHIPPLESCRIPT_CODEX_APP_SERVER_COMMAND").unwrap_or_else(|_| "codex".to_owned());
    let transport = StdioCodexAppServerTransport::spawn(&command, &args).map_err(|error| {
        StoreError::Conflict(format!("failed to launch Codex app-server: {error:?}"))
    })?;
    let capability = builtin_provider_capabilities()
        .into_iter()
        .find(|capability| {
            capability.provider_kind == ProviderKind::Codex
                && capability.surface == AdapterSurface::CodexAppServer
        })
        .ok_or_else(|| StoreError::Conflict("missing built-in Codex capability".to_owned()))?;
    Ok(CodexAppServerAdapter::new(
        provider,
        capability,
        CodexAppServerClient::new(transport),
    ))
}

fn codex_native_turn_request(
    execution: AgentTurnExecution<'_>,
    effect: &ClaimableEffect,
    input_json: &str,
    config: Option<&ProviderBindingConfig>,
) -> Result<NativeProviderTurnRequest, StoreError> {
    let input = serde_json::from_str::<Value>(input_json)?;
    let prompt_json = input
        .get("prompt")
        .and_then(Value::as_str)
        .map(|prompt| Value::String(prompt.to_owned()))
        .unwrap_or(input);
    let mut provider_options = BTreeMap::new();
    provider_options.insert("cwd".to_owned(), Value::String(provider_cwd(config)?));
    provider_options.insert(
        "model".to_owned(),
        Value::String(provider_model(
            config,
            "WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL",
            "gpt-5.4-mini",
        )),
    );
    apply_provider_config_options(&mut provider_options, config);
    Ok(NativeProviderTurnRequest {
        provider_id: execution.provider.to_owned(),
        provider_kind: ProviderKind::Codex,
        surface: AdapterSurface::CodexAppServer,
        run_id: execution.run_id.to_owned(),
        effect_id: execution.effect_id.to_owned(),
        agent: execution.agent.to_owned(),
        profile: execution.profile.map(str::to_owned),
        prompt_json,
        workspace_policy: provider_workspace_policy(config, "read_only"),
        required_capabilities: native_required_capabilities(effect)?,
        cancellation_depth: provider_cancellation_depth(config, CancellationDepth::NativeStop),
        artifact_policy: provider_artifact_policy(config, "metadata"),
        credential_ref: provider_credential_ref(config),
        provider_options,
    })
}

fn claude_agent_sdk_adapter(
    provider: &str,
) -> Result<ClaudeAgentSdkAdapter<StdioClaudeAgentSdkTransport>, StoreError> {
    let sidecar = env::var("WHIPPLESCRIPT_CLAUDE_AGENT_SDK_SIDECAR")
        .unwrap_or_else(|_| "scripts/claude-agent-sdk-sidecar.mjs".to_owned());
    let command =
        env::var("WHIPPLESCRIPT_CLAUDE_AGENT_SDK_COMMAND").unwrap_or_else(|_| "node".to_owned());
    let transport =
        StdioClaudeAgentSdkTransport::spawn(&command, &[sidecar.as_str()]).map_err(|error| {
            StoreError::Conflict(format!(
                "failed to launch Claude Agent SDK sidecar: {error:?}"
            ))
        })?;
    let capability = builtin_provider_capabilities()
        .into_iter()
        .find(|capability| {
            capability.provider_kind == ProviderKind::Claude
                && capability.surface == AdapterSurface::ClaudeAgentSdk
        })
        .ok_or_else(|| StoreError::Conflict("missing built-in Claude capability".to_owned()))?;
    Ok(ClaudeAgentSdkAdapter::new(
        provider,
        capability,
        ClaudeAgentSdkClient::new(transport),
    ))
}

fn claude_native_turn_request(
    execution: AgentTurnExecution<'_>,
    effect: &ClaimableEffect,
    input_json: &str,
    config: Option<&ProviderBindingConfig>,
) -> Result<NativeProviderTurnRequest, StoreError> {
    let input = serde_json::from_str::<Value>(input_json)?;
    let prompt_json = input
        .get("prompt")
        .and_then(Value::as_str)
        .map(|prompt| Value::String(prompt.to_owned()))
        .unwrap_or(input);
    let mut provider_options = BTreeMap::new();
    provider_options.insert("cwd".to_owned(), Value::String(provider_cwd(config)?));
    provider_options.insert(
        "model".to_owned(),
        Value::String(provider_model(
            config,
            "WHIPPLESCRIPT_CLAUDE_AGENT_SDK_MODEL",
            "sonnet",
        )),
    );
    apply_provider_config_options(&mut provider_options, config);
    Ok(NativeProviderTurnRequest {
        provider_id: execution.provider.to_owned(),
        provider_kind: ProviderKind::Claude,
        surface: AdapterSurface::ClaudeAgentSdk,
        run_id: execution.run_id.to_owned(),
        effect_id: execution.effect_id.to_owned(),
        agent: execution.agent.to_owned(),
        profile: execution
            .profile
            .map(str::to_owned)
            .or_else(|| Some("repo-reader".to_owned())),
        prompt_json,
        workspace_policy: provider_workspace_policy(config, "read_only"),
        required_capabilities: native_required_capabilities(effect)?,
        cancellation_depth: provider_cancellation_depth(
            config,
            CancellationDepth::CooperativeRequest,
        ),
        artifact_policy: provider_artifact_policy(config, "metadata"),
        credential_ref: provider_credential_ref(config),
        provider_options,
    })
}

fn pi_rpc_adapter(provider: &str) -> Result<PiRpcAdapter<StdioPiRpcTransport>, StoreError> {
    let args = ["--mode", "rpc", "--no-session"];
    let command = env::var("WHIPPLESCRIPT_PI_RPC_COMMAND").unwrap_or_else(|_| "pi".to_owned());
    let transport = StdioPiRpcTransport::spawn(&command, &args)
        .map_err(|error| StoreError::Conflict(format!("failed to launch Pi RPC: {error:?}")))?;
    let capability = builtin_provider_capabilities()
        .into_iter()
        .find(|capability| {
            capability.provider_kind == ProviderKind::Pi
                && capability.surface == AdapterSurface::PiRpc
        })
        .ok_or_else(|| StoreError::Conflict("missing built-in Pi capability".to_owned()))?;
    Ok(PiRpcAdapter::new(
        provider,
        capability,
        PiRpcClient::new(transport),
    ))
}

fn pi_native_turn_request(
    execution: AgentTurnExecution<'_>,
    effect: &ClaimableEffect,
    input_json: &str,
    config: Option<&ProviderBindingConfig>,
) -> Result<NativeProviderTurnRequest, StoreError> {
    let input = serde_json::from_str::<Value>(input_json)?;
    let prompt_json = input
        .get("prompt")
        .and_then(Value::as_str)
        .map(|prompt| Value::String(prompt.to_owned()))
        .unwrap_or(input);
    let mut provider_options = BTreeMap::new();
    if let Ok(provider) = env::var("WHIPPLESCRIPT_PI_RPC_PROVIDER") {
        provider_options.insert("provider".to_owned(), Value::String(provider));
    }
    if let Ok(model) = env::var("WHIPPLESCRIPT_PI_RPC_MODEL") {
        provider_options.insert("model".to_owned(), Value::String(model));
    }
    if let Some(model) = config.and_then(|config| config.default_model.clone()) {
        provider_options.insert("model".to_owned(), Value::String(model));
    }
    if let Some(config) = config {
        if let Some(provider) = optional_extra_string(config, "model_provider")? {
            provider_options.insert("provider".to_owned(), Value::String(provider));
        }
    }
    apply_provider_config_options(&mut provider_options, config);
    Ok(NativeProviderTurnRequest {
        provider_id: execution.provider.to_owned(),
        provider_kind: ProviderKind::Pi,
        surface: AdapterSurface::PiRpc,
        run_id: execution.run_id.to_owned(),
        effect_id: execution.effect_id.to_owned(),
        agent: execution.agent.to_owned(),
        profile: execution
            .profile
            .map(str::to_owned)
            .or_else(|| Some("repo-reader".to_owned())),
        prompt_json,
        workspace_policy: provider_workspace_policy(config, "read_only"),
        required_capabilities: native_required_capabilities(effect)?,
        cancellation_depth: provider_cancellation_depth(config, CancellationDepth::NativeStop),
        artifact_policy: provider_artifact_policy(config, "metadata"),
        credential_ref: provider_credential_ref(config),
        provider_options,
    })
}

fn provider_workspace_policy(config: Option<&ProviderBindingConfig>, default: &str) -> String {
    config
        .map(|config| config.workspace_policy.clone())
        .unwrap_or_else(|| default.to_owned())
}

fn provider_artifact_policy(config: Option<&ProviderBindingConfig>, default: &str) -> String {
    config
        .map(|config| config.artifact_policy.clone())
        .unwrap_or_else(|| default.to_owned())
}

fn provider_cancellation_depth(
    config: Option<&ProviderBindingConfig>,
    default: CancellationDepth,
) -> CancellationDepth {
    config
        .map(|config| config.cancellation_depth)
        .unwrap_or(default)
}

fn provider_credential_ref(config: Option<&ProviderBindingConfig>) -> Option<String> {
    config.and_then(|config| config.credentials_ref.clone())
}

fn provider_model(config: Option<&ProviderBindingConfig>, env_key: &str, default: &str) -> String {
    config
        .and_then(|config| config.default_model.clone())
        .or_else(|| env::var(env_key).ok())
        .unwrap_or_else(|| default.to_owned())
}

fn provider_cwd(config: Option<&ProviderBindingConfig>) -> Result<String, StoreError> {
    if let Some(config) = config {
        if let Some(cwd) = optional_extra_string(config, "cwd")? {
            return Ok(cwd);
        }
    }
    Ok(env::current_dir()
        .map_err(StoreError::Io)?
        .to_string_lossy()
        .into_owned())
}

fn apply_provider_config_options(
    provider_options: &mut BTreeMap<String, Value>,
    config: Option<&ProviderBindingConfig>,
) {
    let Some(config) = config else {
        return;
    };
    if let Some(timeout_ms) = config.timeout_ms {
        provider_options.insert("timeout_ms".to_owned(), json!(timeout_ms));
    }
    if !config.profile_ids.is_empty() {
        provider_options.insert("profile_ids".to_owned(), json!(config.profile_ids));
    }
    if !config.health_checks.is_empty() {
        provider_options.insert("health_checks".to_owned(), json!(config.health_checks));
    }
    if !config.extra.is_empty() {
        provider_options.insert(
            "provider_config_extra_keys".to_owned(),
            json!(config.extra.keys().cloned().collect::<Vec<_>>()),
        );
    }
}

fn native_required_capabilities(effect: &ClaimableEffect) -> Result<Vec<String>, StoreError> {
    let value = serde_json::from_str::<Value>(&effect.required_capabilities_json)?;
    let mut capabilities = value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            "repo.read" | "repo.write" | "command.run"
        )
    }) {
        capabilities.push("repo.read".to_owned());
    }
    capabilities.sort();
    capabilities.dedup();
    Ok(capabilities)
}

fn native_provider_max_events() -> usize {
    env::var("WHIPPLESCRIPT_NATIVE_PROVIDER_MAX_EVENTS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256)
}

struct UnavailableNativeProviderAdapter {
    provider_id: String,
    capability: ProviderCapability,
    message: String,
}

fn unavailable_native_provider_adapter(
    provider_id: &str,
    provider_kind: ProviderKind,
    surface: AdapterSurface,
    error: StoreError,
) -> Result<UnavailableNativeProviderAdapter, StoreError> {
    let capability = builtin_provider_capabilities()
        .into_iter()
        .find(|capability| {
            capability.provider_kind == provider_kind && capability.surface == surface
        })
        .ok_or_else(|| {
            StoreError::Conflict(format!(
                "missing built-in {} {} capability",
                provider_kind.as_str(),
                surface.as_str()
            ))
        })?;
    Ok(UnavailableNativeProviderAdapter {
        provider_id: provider_id.to_owned(),
        capability,
        message: format!("{error:?}"),
    })
}

impl NativeProviderAdapter for UnavailableNativeProviderAdapter {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn capability(&self) -> &ProviderCapability {
        &self.capability
    }

    fn start_turn(
        &mut self,
        request: NativeProviderTurnRequest,
    ) -> Result<NativeProviderEvent, NativeProviderBoundaryError> {
        Err(NativeProviderBoundaryError {
            provider_id: self.provider_id.clone(),
            surface: request.surface,
            code: "provider_health_unavailable".to_owned(),
            message: self.message.clone(),
            recoverable: true,
            evidence: json!({
                "provider_kind": request.provider_kind.as_str(),
                "surface": request.surface.as_str(),
                "request": request.to_json_redacted(),
            }),
        })
    }

    fn next_event(
        &mut self,
        _run_id: &str,
    ) -> Result<Option<NativeProviderEvent>, NativeProviderBoundaryError> {
        Ok(None)
    }

    fn cancel_turn(
        &mut self,
        cancellation: NativeProviderCancellation,
    ) -> Result<NativeProviderEvent, NativeProviderBoundaryError> {
        Err(NativeProviderBoundaryError {
            provider_id: self.provider_id.clone(),
            surface: self.capability.surface,
            code: "provider_health_unavailable".to_owned(),
            message: self.message.clone(),
            recoverable: true,
            evidence: json!({
                "run_id": cancellation.run_id,
                "requested_depth": cancellation.requested_depth.as_str(),
            }),
        })
    }
}

struct NativeFixtureAdapter {
    provider_id: String,
    run_id: String,
    failed: bool,
    next_index: usize,
    capability: ProviderCapability,
}

impl NativeFixtureAdapter {
    fn new(provider_id: &str, run_id: &str, failed: bool) -> Self {
        Self {
            provider_id: provider_id.to_owned(),
            run_id: run_id.to_owned(),
            failed,
            next_index: 0,
            capability: ProviderCapability {
                provider_kind: ProviderKind::Fixture,
                surface: AdapterSurface::Fixture,
                protocol_version: Some("native-fixture.v1".to_owned()),
                session_identity_fields: vec!["session_id".to_owned(), "turn_id".to_owned()],
                stream_event_kinds: vec![
                    "fixture.turn.started".to_owned(),
                    "fixture.artifact.captured".to_owned(),
                    "fixture.turn.completed".to_owned(),
                    "fixture.turn.failed".to_owned(),
                ],
                tool_policy: "fixture".to_owned(),
                cancellation_depths: vec![CancellationDepth::CooperativeRequest],
                artifact_manifest: true,
                health_checks: Vec::new(),
                auth_requirements: Vec::new(),
            },
        }
    }

    fn session_id(&self) -> String {
        format!("{}-session-{}", self.provider_id, self.run_id)
    }

    fn turn_id(&self) -> String {
        format!("{}-turn-{}", self.provider_id, self.run_id)
    }
}

impl NativeProviderAdapter for NativeFixtureAdapter {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn capability(&self) -> &ProviderCapability {
        &self.capability
    }

    fn start_turn(
        &mut self,
        request: NativeProviderTurnRequest,
    ) -> Result<NativeProviderEvent, NativeProviderBoundaryError> {
        Ok(NativeProviderEvent {
            provider_id: self.provider_id.clone(),
            run_id: request.run_id.clone(),
            event_kind: NativeProviderEventKind::Started,
            provider_event_type: "fixture.turn.started".to_owned(),
            provider_session_id: Some(self.session_id()),
            provider_turn_id: Some(self.turn_id()),
            sequence: Some(1),
            evidence: json!({
                "prompt_shape": request.to_json_redacted().get("prompt_shape").cloned(),
                "workspace_policy": request.workspace_policy,
            }),
            artifacts: Vec::new(),
        })
    }

    fn next_event(
        &mut self,
        run_id: &str,
    ) -> Result<Option<NativeProviderEvent>, NativeProviderBoundaryError> {
        self.next_index += 1;
        match self.next_index {
            1 => Ok(Some(NativeProviderEvent {
                provider_id: self.provider_id.clone(),
                run_id: run_id.to_owned(),
                event_kind: NativeProviderEventKind::ArtifactCaptured,
                provider_event_type: "fixture.artifact.captured".to_owned(),
                provider_session_id: Some(self.session_id()),
                provider_turn_id: Some(self.turn_id()),
                sequence: Some(2),
                evidence: json!({"artifact": "metadata-only"}),
                artifacts: vec![NativeProviderArtifactRef {
                    artifact_id: Some("native-fixture-artifact-1".to_owned()),
                    kind: "transcript".to_owned(),
                    uri: format!("provider://{}/runs/{run_id}/transcript", self.provider_id),
                    content_hash: Some("sha256:native-fixture-transcript".to_owned()),
                    mime_type: Some("text/plain".to_owned()),
                    required: true,
                }],
            })),
            2 => {
                let event_kind = if self.failed {
                    NativeProviderEventKind::Failed
                } else {
                    NativeProviderEventKind::Completed
                };
                Ok(Some(NativeProviderEvent {
                    provider_id: self.provider_id.clone(),
                    run_id: run_id.to_owned(),
                    event_kind,
                    provider_event_type: if self.failed {
                        "fixture.turn.failed"
                    } else {
                        "fixture.turn.completed"
                    }
                    .to_owned(),
                    provider_session_id: Some(self.session_id()),
                    provider_turn_id: Some(self.turn_id()),
                    sequence: Some(3),
                    evidence: json!({"terminal": event_kind.as_str()}),
                    artifacts: Vec::new(),
                }))
            }
            _ => Ok(None),
        }
    }

    fn cancel_turn(
        &mut self,
        cancellation: NativeProviderCancellation,
    ) -> Result<NativeProviderEvent, NativeProviderBoundaryError> {
        Ok(NativeProviderEvent {
            provider_id: self.provider_id.clone(),
            run_id: cancellation.run_id,
            event_kind: NativeProviderEventKind::Cancelled,
            provider_event_type: "fixture.turn.cancelled".to_owned(),
            provider_session_id: cancellation
                .provider_session_id
                .or_else(|| Some(self.session_id())),
            provider_turn_id: cancellation
                .provider_turn_id
                .or_else(|| Some(self.turn_id())),
            sequence: Some(99),
            evidence: json!({"reason": cancellation.reason}),
            artifacts: Vec::new(),
        })
    }
}

fn native_fixture_turn_request(
    execution: AgentTurnExecution<'_>,
    input_json: &str,
) -> NativeProviderTurnRequest {
    NativeProviderTurnRequest {
        provider_id: execution.provider.to_owned(),
        provider_kind: ProviderKind::Fixture,
        surface: AdapterSurface::Fixture,
        run_id: execution.run_id.to_owned(),
        effect_id: execution.effect_id.to_owned(),
        agent: execution.agent.to_owned(),
        profile: execution.profile.map(str::to_owned),
        prompt_json: serde_json::from_str(input_json).unwrap_or_else(|_| json!({"raw": "invalid"})),
        workspace_policy: "isolated".to_owned(),
        required_capabilities: Vec::new(),
        cancellation_depth: CancellationDepth::CooperativeRequest,
        artifact_policy: "metadata".to_owned(),
        credential_ref: None,
        provider_options: BTreeMap::new(),
    }
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
    // Sum-type outputs carry embedded per-variant fixtures: return the
    // selected (or first declared) tagged variant (spec/sum-types.md).
    let value = input
        .get("fixture_variants")
        .and_then(Value::as_object)
        .and_then(|variants| {
            let selected = options
                .variant
                .as_deref()
                .or_else(|| input.get("fixture_default").and_then(Value::as_str))?;
            variants.get(selected).map(Value::to_string)
        })
        .unwrap_or_else(|| fixture_baml_value(&output_type));
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
    let terminal = kernel.run_human_ask(HumanAskExecution {
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
    })?;
    // The ask is issued: a completed-status fact lets `after ask succeeds`
    // branches fire (e.g. flow await-state records carrying the ask's
    // effect id for answer correlation).
    let issued_json = json!({
        "effect_id": effect.effect_id,
        "run_id": run_id,
        "inbox_item_id": inbox_item_id,
        "status": "completed",
    })
    .to_string();
    kernel.derive_fact(
        instance_id,
        "human.ask.issued",
        &effect.effect_id,
        &issued_json,
        Some(&terminal.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "human.ask.issued",
        ])),
    )?;
    Ok(terminal)
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
            program_version_id: None,
            revision_epoch: 0,
            name: binding.to_owned(),
            key: upstream_effect_id.to_owned(),
            value_json: binding_value.to_string(),
            provenance_class: "effect".to_owned(),
            source_span_json: None,
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
                program_version_id: None,
                revision_epoch: 0,
                name: binding.clone(),
                key: binding.clone(),
                value_json: value.to_string(),
                provenance_class: "input".to_owned(),
                source_span_json: None,
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

/// Executes a granted `exec` effect. Grants are operator-config-only: the
/// `WHIPPLESCRIPT_EXEC_ALLOW` allowlist (colon-separated glob prefixes).
/// Source declares; config grants; there is no self-granting.
fn exec_command_granted(command: &str) -> bool {
    let Ok(allow) = env::var("WHIPPLESCRIPT_EXEC_ALLOW") else {
        return false;
    };
    allow
        .split(':')
        .filter(|pattern| !pattern.is_empty())
        .any(|pattern| {
            if let Some(prefix) = pattern.strip_suffix('*') {
                command.starts_with(prefix)
            } else {
                command == pattern
            }
        })
}

/// In-workflow injection (spec/event-ingress.md): one instance lands a
/// typed, schema-validated, durable event in another known instance — still
/// "inject a durable event", not "open a channel". A payload that fails
/// validation fails the effect; no ill-typed fact lands in the peer.
fn run_notify_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    let input = json_from_str(&effect.input_json);
    let target = input
        .get("target_instance")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let event_name = input
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let payload = input.get("payload").cloned().unwrap_or(Value::Null);
    let shape = input.get("shape").cloned().unwrap_or(Value::Null);

    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "notify-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "notify-lease"]);
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    kernel.start_run(RunStart {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: "notify",
        worker_id: "whip-notify",
        lease_id: &lease_id,
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: &json!({"target": target, "event": event_name}).to_string(),
    })?;

    let mut errors = Vec::new();
    validate_ingest_value(&payload, &shape, "$", &mut errors);
    let target_exists = SqliteStore::open(store_path)?
        .get_instance(&target)?
        .is_some();
    if !target_exists {
        errors.push(format!("target instance `{target}` not found"));
    }
    if !errors.is_empty() {
        let reason = format!("notify of `{event_name}` rejected: {}", errors.join("; "));
        let terminal = kernel.fail_run(EffectCompletion {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: "notify",
            worker_id: "whip-notify",
            status: "failed",
            exit_code: None,
            summary: Some(&reason),
            metadata_json: &json!({"failure": {"message": reason}}).to_string(),
            idempotency_key: Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "terminal",
            ])),
        })?;
        return Ok(terminal);
    }

    let payload_json = payload.to_string();
    let received = kernel.ingest_external_event(
        &target,
        &event_name,
        &payload_json,
        Some(&idempotency_key(&[&target, "notify", &effect.effect_id])),
    )?;
    kernel.derive_fact(
        &target,
        &event_name,
        &received.event_id,
        &payload_json,
        Some(&received.event_id),
        Some(&idempotency_key(&[
            &target,
            "notify-fact",
            &effect.effect_id,
        ])),
    )?;
    let terminal = kernel.complete_run(EffectCompletion {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: "notify",
        worker_id: "whip-notify",
        status: "completed",
        exit_code: Some(0),
        summary: Some(&format!("notified {target} with `{event_name}`")),
        metadata_json: &json!({"target": target, "event": event_name}).to_string(),
        idempotency_key: Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "terminal",
        ])),
    })?;
    kernel.derive_fact(
        instance_id,
        "event.notify.completed",
        &effect.effect_id,
        &json!({
            "effect_id": effect.effect_id,
            "run_id": run_id,
            "status": "completed",
            "value": {"target": target, "event": event_name},
        })
        .to_string(),
        Some(&terminal.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "notify-self-fact",
        ])),
    )?;
    Ok(terminal)
}

/// Executes one coordination verb (spec/coordination.md): one atomic store
/// transaction, completed with a sum-typed outcome value the after-block
/// predicates (`held`/`contended`/`ok`/`over`) dispatch on. Contention and
/// over-budget are completed outcomes, never failures.
fn run_coordination_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    use whipplescript_store::coordination::{AcquireOutcome, ConsumeOutcome, CoordinationStore};

    let input = json_from_str(&effect.input_json);
    let mut coordination = CoordinationStore::open(coordination_store_path())?;
    let field = |name: &str| {
        input
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let value = match effect.kind.as_str() {
        "lease.acquire" => {
            let resource = field("resource");
            let key = field("key");
            let outcome = coordination.try_acquire(
                &resource,
                &key,
                input.get("slots").and_then(Value::as_i64).unwrap_or(1),
                input
                    .get("ttl_seconds")
                    .and_then(Value::as_i64)
                    .unwrap_or(600),
                instance_id,
            )?;
            match outcome {
                AcquireOutcome::Held => json!({
                    "variant": "Held",
                    "resource": resource,
                    "key": key,
                }),
                AcquireOutcome::Contended { holders } => json!({
                    "variant": "Contended",
                    "resource": resource,
                    "key": key,
                    "holders": holders,
                }),
            }
        }
        "lease.release" => {
            // The release names its acquire; resource and key come from the
            // recorded acquire input, so they cannot drift.
            let acquire_effect_id = field("acquire_effect_id");
            let store = SqliteStore::open(store_path)?;
            let acquire_input = store
                .list_effects(instance_id)?
                .into_iter()
                .find(|candidate| candidate.effect_id == acquire_effect_id)
                .map(|candidate| json_from_str(&candidate.input_json))
                .unwrap_or(Value::Null);
            drop(store);
            let resource = acquire_input
                .get("resource")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let key = acquire_input
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let released = coordination.release(&resource, &key, instance_id)?;
            json!({
                "variant": "Released",
                "resource": resource,
                "key": key,
                "released": released,
            })
        }
        "ledger.append" => {
            let ledger = field("ledger");
            let partition = field("partition");
            let entry = input.get("entry").cloned().unwrap_or(Value::Null);
            let seq = coordination.append(
                &ledger,
                &partition,
                &entry.to_string(),
                instance_id,
                input
                    .get("retain_seconds")
                    .and_then(Value::as_i64)
                    .unwrap_or(86400),
            )?;
            json!({
                "variant": "Appended",
                "ledger": ledger,
                "partition": partition,
                "seq": seq,
            })
        }
        "counter.consume" => {
            let counter = field("counter");
            let key = field("key");
            let period = coordination.current_period(&field("reset"))?;
            let outcome = coordination.consume(
                &counter,
                &key,
                input.get("amount").and_then(Value::as_i64).unwrap_or(0),
                input.get("cap").and_then(Value::as_i64).unwrap_or(0),
                &period,
            )?;
            match outcome {
                ConsumeOutcome::Ok { remaining } => json!({
                    "variant": "Ok",
                    "counter": counter,
                    "key": key,
                    "remaining": remaining,
                }),
                ConsumeOutcome::Over { remaining } => json!({
                    "variant": "Over",
                    "counter": counter,
                    "key": key,
                    "remaining": remaining,
                }),
            }
        }
        other => {
            return Err(StoreError::Conflict(format!(
                "unknown coordination effect kind `{other}`"
            )))
        }
    };

    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "coord-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "coord-lease"]);
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    kernel.start_run(RunStart {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: "coordination",
        worker_id: "whip-coordination",
        lease_id: &lease_id,
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: &json!({"kind": effect.kind}).to_string(),
    })?;
    let terminal = kernel.complete_run(EffectCompletion {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: "coordination",
        worker_id: "whip-coordination",
        status: "completed",
        exit_code: Some(0),
        summary: Some(&format!(
            "{} -> {}",
            effect.kind,
            value.get("variant").and_then(Value::as_str).unwrap_or("?")
        )),
        metadata_json: &value.to_string(),
        idempotency_key: Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "terminal",
        ])),
    })?;
    let fact = json!({
        "effect_id": effect.effect_id,
        "run_id": run_id,
        "status": "completed",
        "value": value,
    });
    kernel.derive_fact(
        instance_id,
        &format!("{}.completed", effect.kind),
        &effect.effect_id,
        &fact.to_string(),
        Some(&terminal.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "coord-fact",
        ])),
    )?;
    Ok(terminal)
}

/// The validated result of `-> Schema` / `-> each Schema` stdout ingestion.
enum ExecIngest {
    Single(Value),
    Stream(Vec<Value>),
}

type ExecOutcome =
    Result<(i32, String, String, Option<ExecIngest>), (Option<(i32, String, String)>, String)>;

/// Parses and validates exec stdout against the effect's embedded contract
/// (spec/json-ingestion.md). Streams are all-or-nothing: any malformed line
/// fails the whole effect so a partial stream never half-commits.
fn ingest_exec_stdout(contract: &Value, stdout: &str) -> Result<ExecIngest, String> {
    let schema = contract
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or("json");
    let shape = contract.get("shape").cloned().unwrap_or(Value::Null);
    let each = contract
        .get("each")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let text = stdout.trim();
    if !each {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("stdout is not valid JSON for `{schema}`: {error}"))?;
        if !value.is_object() {
            return Err(format!(
                "stdout must be a single JSON object conforming to `{schema}`"
            ));
        }
        let mut errors = Vec::new();
        validate_ingest_value(&value, &shape, "$", &mut errors);
        if !errors.is_empty() {
            return Err(format!(
                "stdout does not conform to `{schema}`: {}",
                errors.join("; ")
            ));
        }
        return Ok(ExecIngest::Single(value));
    }
    let elements: Vec<Value> = if text.starts_with('[') {
        serde_json::from_str::<Value>(text)
            .map_err(|error| format!("stdout is not a valid JSON array of `{schema}`: {error}"))?
            .as_array()
            .cloned()
            .ok_or_else(|| format!("stdout must be a JSON array or JSONL stream of `{schema}`"))?
    } else {
        let mut items = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value = serde_json::from_str(line).map_err(|error| {
                format!(
                    "line {} is not valid JSON for `{schema}`: {error}",
                    index + 1
                )
            })?;
            items.push(value);
        }
        items
    };
    let mut errors = Vec::new();
    for (index, element) in elements.iter().enumerate() {
        validate_ingest_value(element, &shape, &format!("[{index}]"), &mut errors);
    }
    if !errors.is_empty() {
        return Err(format!(
            "stream does not conform to `{schema}`: {}",
            errors.join("; ")
        ));
    }
    Ok(ExecIngest::Stream(elements))
}

fn truncate_exec_bytes(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.chars().take(8192).collect::<String>()
}

fn exec_output_to_outcome(
    output: std::process::Output,
    parse_contract: &Option<Value>,
) -> ExecOutcome {
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout_full = String::from_utf8_lossy(&output.stdout).to_string();
    let stdout = truncate_exec_bytes(&output.stdout);
    let stderr = truncate_exec_bytes(&output.stderr);
    if output.status.success() {
        match parse_contract {
            Some(contract) => match ingest_exec_stdout(contract, &stdout_full) {
                Ok(ingested) => Ok((exit_code, stdout, stderr, Some(ingested))),
                Err(reason) => Err((Some((exit_code, stdout, stderr)), reason)),
            },
            None => Ok((exit_code, stdout, stderr, None)),
        }
    } else {
        Err((
            Some((exit_code, stdout, stderr)),
            format!("exec command exited with status {exit_code}"),
        ))
    }
}

fn run_script_capability_exec(
    script_manifest: Option<&ScriptManifest>,
    capability: &str,
    input: &Value,
    parse_contract: &Option<Value>,
) -> ExecOutcome {
    let Some(manifest) = script_manifest else {
        return Err((
            None,
            "script manifest is required for hosted exec capabilities".to_owned(),
        ));
    };
    let Some(script) = manifest.get(capability) else {
        return Err((
            None,
            format!("script capability `{capability}` is not declared in the manifest"),
        ));
    };
    if script
        .argv
        .iter()
        .any(|arg| executable_basename(arg) == "whip")
    {
        return Err((
            None,
            "script capabilities may not execute the `whip` control-plane binary".to_owned(),
        ));
    }
    let Some(script_index) = script.argv.iter().position(|arg| Path::new(arg).is_file()) else {
        return Err((
            None,
            format!("script capability `{capability}` argv does not name a readable script file"),
        ));
    };
    let script_path = Path::new(&script.argv[script_index]);
    let bytes = match fs::read(script_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err((
                None,
                format!(
                    "script capability `{capability}` failed to read `{}`: {error}",
                    script_path.display()
                ),
            ))
        }
    };
    let actual_sha256 = sha256_hex(&bytes);
    if actual_sha256 != script.sha256 {
        return Err((
            None,
            format!(
                "script capability `{capability}` hash mismatch: expected {}, got {}",
                script.sha256, actual_sha256
            ),
        ));
    }
    let verified_path =
        match write_verified_script_copy(capability, &actual_sha256, &bytes, script_path) {
            Ok(path) => path,
            Err(error) => {
                return Err((
                    None,
                    format!(
                        "script capability `{capability}` failed to stage verified copy: {error}"
                    ),
                ))
            }
        };
    let mut argv = script.argv.clone();
    argv[script_index] = verified_path.display().to_string();
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    for (name, reference) in &script.env {
        let Some(env_name) = reference.strip_prefix("env:") else {
            return Err((
                None,
                format!("script capability `{capability}` env `{name}` must use an env: reference"),
            ));
        };
        match env::var(env_name) {
            Ok(value) => {
                command.env(name, value);
            }
            Err(_) => {
                return Err((
                    None,
                    format!(
                        "script capability `{capability}` requires missing environment variable `{env_name}`"
                    ),
                ))
            }
        }
    }
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let stdin_json = input
        .get("stdin")
        .cloned()
        .unwrap_or(Value::Null)
        .to_string();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = fs::remove_file(&verified_path);
            return Err((None, format!("exec command failed to start: {error}")));
        }
    };
    if let Some(stdin) = child.stdin.as_mut() {
        if let Err(error) = stdin.write_all(stdin_json.as_bytes()) {
            let _ = child.kill();
            let _ = fs::remove_file(&verified_path);
            return Err((None, format!("exec command failed to write stdin: {error}")));
        }
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => {
            let _ = fs::remove_file(&verified_path);
            return Err((None, format!("exec command failed: {error}")));
        }
    };
    let _ = fs::remove_file(&verified_path);
    exec_output_to_outcome(output, parse_contract)
}

fn executable_basename(arg: &str) -> &str {
    Path::new(arg)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(arg)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_verified_script_copy(
    capability: &str,
    sha256: &str,
    bytes: &[u8],
    original_path: &Path,
) -> io::Result<PathBuf> {
    let sanitized = capability
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    let extension = original_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let path = env::temp_dir().join(format!(
        "whipplescript-script-{sanitized}-{sha256}{extension}"
    ));
    fs::write(&path, bytes)?;
    if let Ok(metadata) = fs::metadata(original_path) {
        let _ = fs::set_permissions(&path, metadata.permissions());
    }
    Ok(path)
}

fn run_exec_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    exec_profile: ExecProfile,
    script_manifest: Option<&ScriptManifest>,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    let input = json_from_str(&effect.input_json);
    let mode = input.get("mode").and_then(Value::as_str).unwrap_or("raw");
    let command = input
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let capability = input
        .get("capability")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let parse_contract = input.get("parse").cloned();
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "exec-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "exec-lease"]);
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    kernel.start_run(RunStart {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: "exec",
        worker_id: "whip-exec",
        lease_id: &lease_id,
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: &json!({"mode": mode, "command": command, "capability": capability})
            .to_string(),
    })?;

    let outcome = if mode == "capability" {
        run_script_capability_exec(script_manifest, &capability, &input, &parse_contract)
    } else if exec_profile.is_hosted() {
        Err((
            None,
            "raw exec is not allowed in hosted exec profile".to_owned(),
        ))
    } else if !exec_command_granted(&command) {
        Err((
            None,
            format!("exec command `{command}` is not granted; add it to WHIPPLESCRIPT_EXEC_ALLOW"),
        ))
    } else {
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
        {
            Ok(output) => exec_output_to_outcome(output, &parse_contract),
            Err(error) => Err((None, format!("exec command failed to start: {error}"))),
        }
    };

    match outcome {
        Ok((exit_code, stdout, stderr, ingested)) => {
            let mut value = json!({
                "mode": mode,
                "command": command,
                "capability": capability,
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
            });
            if mode == "capability" {
                if let Some(sha256) = script_manifest
                    .and_then(|manifest| manifest.get(&capability))
                    .map(|script| script.sha256.clone())
                {
                    insert_json_field(&mut value, "sha256", Value::String(sha256));
                }
            }
            let terminal = kernel.complete_run(EffectCompletion {
                instance_id,
                effect_id: &effect.effect_id,
                run_id: &run_id,
                provider: "exec",
                worker_id: "whip-exec",
                status: "completed",
                exit_code: Some(i64::from(exit_code)),
                summary: Some("exec completed"),
                metadata_json: &value.to_string(),
                idempotency_key: Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "terminal",
                ])),
            })?;
            let mut fact = json!({
                "effect_id": effect.effect_id,
                "run_id": run_id,
                "status": "completed",
                "mode": mode,
                "capability": capability,
                "exit_code": exit_code,
                "stdout": value.get("stdout").cloned().unwrap_or(Value::Null),
            });
            match ingested {
                // `-> Schema`: the typed value is the success payload, bound
                // by `after x succeeds as r`.
                Some(ExecIngest::Single(parsed)) => {
                    insert_json_field(&mut fact, "value", parsed);
                }
                // `-> each Schema`: one typed fact per element, reacted to by
                // ordinary per-fact rule fan-out.
                Some(ExecIngest::Stream(elements)) => {
                    let schema = parse_contract
                        .as_ref()
                        .and_then(|contract| contract.get("schema"))
                        .and_then(Value::as_str)
                        .unwrap_or("json")
                        .to_owned();
                    for (index, element) in elements.iter().enumerate() {
                        kernel.ingest_fact(
                            instance_id,
                            &schema,
                            &format!("{}:{index}", effect.effect_id),
                            &element.to_string(),
                            Some(&terminal.event_id),
                            Some(&idempotency_key(&[
                                instance_id,
                                &effect.effect_id,
                                "ingest",
                                &index.to_string(),
                            ])),
                        )?;
                    }
                    insert_json_field(&mut fact, "ingested_count", json!(elements.len()));
                }
                None => {}
            }
            kernel.derive_fact(
                instance_id,
                "exec.command.completed",
                &effect.effect_id,
                &fact.to_string(),
                Some(&terminal.event_id),
                Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "exec-fact",
                ])),
            )?;
            Ok(terminal)
        }
        Err((detail, reason)) => {
            let metadata = match &detail {
                Some((exit_code, stdout, stderr)) => json!({
                    "failure": {"message": reason},
                    "exit_code": exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                }),
                None => json!({"failure": {"message": reason}}),
            };
            let terminal = kernel.fail_run(EffectCompletion {
                instance_id,
                effect_id: &effect.effect_id,
                run_id: &run_id,
                provider: "exec",
                worker_id: "whip-exec",
                status: "failed",
                exit_code: detail.as_ref().map(|(code, _, _)| i64::from(*code)),
                summary: Some(&reason),
                metadata_json: &metadata.to_string(),
                idempotency_key: Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "terminal",
                ])),
            })?;
            let fact = json!({
                "effect_id": effect.effect_id,
                "run_id": run_id,
                "status": "failed",
                "mode": mode,
                "capability": capability,
                "error": {"message": reason},
            })
            .to_string();
            kernel.derive_fact(
                instance_id,
                "exec.command.failed",
                &effect.effect_id,
                &fact,
                Some(&terminal.event_id),
                Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "exec-fact",
                ])),
            )?;
            Ok(terminal)
        }
    }
}

fn run_queue_effect(
    store_path: &Path,
    instance_id: &str,
    effect: &ClaimableEffect,
    options: &WorkerOptions,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    use whipplescript_store::items::{ClaimOutcome, WorkItemStore};
    let input = json_from_str(&effect.input_json);
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "queue-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "queue-lease"]);
    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    kernel.start_run(RunStart {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: "queue",
        worker_id: "whip-queue",
        lease_id: &lease_id,
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: &effect.input_json,
    })?;
    drop(kernel);

    let mut items = WorkItemStore::open(items_store_path())?;
    let outcome: Result<Value, String> = match effect.kind.as_str() {
        "queue.file" => {
            let queue = effect.target.clone().unwrap_or_default();
            let item = input.get("item").cloned().unwrap_or_else(|| json!({}));
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let body = item.get("body").and_then(Value::as_str).unwrap_or_default();
            let labels = item
                .get("labels")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let metadata = item.get("metadata").cloned().unwrap_or_else(|| json!({}));
            let filed_by = format!("workflow:{instance_id}");
            items
                .file_item(&queue, title, body, &labels, &metadata, Some(&filed_by))
                .map(|filed| {
                    json!({
                        "queue": filed.queue,
                        "id": filed.id,
                        "title": filed.title,
                    })
                })
                .map_err(|error| format!("file failed: {error:?}"))
        }
        "queue.claim" => {
            let id = input.get("id").and_then(Value::as_str).unwrap_or_default();
            match items.claim_item(id, instance_id) {
                Ok(ClaimOutcome::Claimed) => Ok(json!({"id": id, "claimed_by": instance_id})),
                Ok(ClaimOutcome::AlreadyClaimed { holder }) => {
                    Err(format!("already claimed by `{holder}`"))
                }
                Ok(ClaimOutcome::NotFound) => Err(format!("item `{id}` not found")),
                Err(error) => Err(format!("claim failed: {error:?}")),
            }
        }
        "queue.release" => {
            let id = input.get("id").and_then(Value::as_str).unwrap_or_default();
            match items.release_item(id) {
                Ok(true) => Ok(json!({"id": id, "status": "open"})),
                Ok(false) => Err(format!("item `{id}` was not in progress")),
                Err(error) => Err(format!("release failed: {error:?}")),
            }
        }
        "queue.finish" => {
            let id = input.get("id").and_then(Value::as_str).unwrap_or_default();
            let summary = input
                .pointer("/payload/summary")
                .and_then(Value::as_str)
                .map(str::to_owned);
            match items.finish_item(id, summary.as_deref()) {
                Ok(true) => Ok(json!({"id": id, "status": "done", "summary": summary})),
                Ok(false) => Err(format!("item `{id}` cannot finish from its current status")),
                Err(error) => Err(format!("finish failed: {error:?}")),
            }
        }
        other => Err(format!("unknown queue effect kind `{other}`")),
    };
    drop(items);

    let store = SqliteStore::open(store_path)?;
    let mut kernel = RuntimeKernel::new(store);
    let _ = options;
    match outcome {
        Ok(value) => {
            let terminal = kernel.complete_run(EffectCompletion {
                instance_id,
                effect_id: &effect.effect_id,
                run_id: &run_id,
                provider: "queue",
                worker_id: "whip-queue",
                status: "completed",
                exit_code: Some(0),
                summary: Some("queue operation completed"),
                metadata_json: &value.to_string(),
                idempotency_key: Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "terminal",
                ])),
            })?;
            let fact_value = json!({
                "effect_id": effect.effect_id,
                "run_id": run_id,
                "status": "completed",
                "value": value,
            })
            .to_string();
            kernel.derive_fact(
                instance_id,
                &format!("{}.completed", effect.kind),
                &effect.effect_id,
                &fact_value,
                Some(&terminal.event_id),
                Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "queue-fact",
                ])),
            )?;
            Ok(terminal)
        }
        Err(reason) => {
            let terminal = kernel.fail_run(EffectCompletion {
                instance_id,
                effect_id: &effect.effect_id,
                run_id: &run_id,
                provider: "queue",
                worker_id: "whip-queue",
                status: "failed",
                exit_code: Some(1),
                summary: Some(&reason),
                metadata_json: &json!({"failure": {"message": reason}}).to_string(),
                idempotency_key: Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "terminal",
                ])),
            })?;
            let fact_value = json!({
                "effect_id": effect.effect_id,
                "run_id": run_id,
                "status": "failed",
                "error": {"message": reason},
            })
            .to_string();
            kernel.derive_fact(
                instance_id,
                &format!("{}.failed", effect.kind),
                &effect.effect_id,
                &fact_value,
                Some(&terminal.event_id),
                Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "queue-fact",
                ])),
            )?;
            Ok(terminal)
        }
    }
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
    let mut emitted_value = payload.as_object().cloned().unwrap_or_default();
    emitted_value.insert(
        "event_id".to_owned(),
        Value::String(emitted.event_id.clone()),
    );
    emitted_value.insert(
        "event_type".to_owned(),
        Value::String(event_type.to_owned()),
    );
    emitted_value.insert("payload".to_owned(), payload.clone());
    let value_json = json!({
        "effect_id": effect.effect_id,
        "run_id": run_id,
        "event_id": emitted.event_id,
        "event_type": event_type,
        "status": "completed",
        "value": Value::Object(emitted_value),
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
            let source_span_json =
                invocation_store.effect_source_span_json(instance_id, &effect.effect_id)?;
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
                    "target_workflow": target_workflow,
                })
                .to_string(),
            })?;
            let (child_started, child_ir) = match start_child_workflow_instance(
                store_path,
                program_path,
                target_workflow,
                &child_input.to_string(),
            ) {
                Ok(started) => started,
                Err(error) => {
                    let metadata_json = json!({
                        "input": input,
                        "target_workflow": target_workflow,
                        "error": format!("{error:?}"),
                    })
                    .to_string();
                    return kernel.fail_run(EffectCompletion {
                        instance_id,
                        effect_id: &effect.effect_id,
                        run_id: &run_id,
                        provider: &options.provider,
                        worker_id: "whip-worker",
                        status: "failed",
                        exit_code: Some(2),
                        summary: Some("child workflow failed to start"),
                        metadata_json: &metadata_json,
                        idempotency_key: Some(&idempotency_key(&[
                            instance_id,
                            &effect.effect_id,
                            "terminal",
                        ])),
                    });
                }
            };
            let child_instance_id = child_started.instance_id.clone();
            let invocation_id = idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "invokes",
                &child_instance_id,
            ]);
            let invocation_key =
                idempotency_key(&[instance_id, &effect.effect_id, target_workflow]);
            if let Err(error) = invocation_store.record_workflow_invocation(NewWorkflowInvocation {
                invocation_id: &invocation_id,
                parent_instance_id: instance_id,
                parent_effect_id: &effect.effect_id,
                child_instance_id: &child_instance_id,
                target_workflow,
                input_json: &child_input.to_string(),
                source_span_json: source_span_json.as_deref(),
                idempotency_key: &invocation_key,
            }) {
                let metadata_json = json!({
                    "input": input,
                    "target_workflow": target_workflow,
                    "child_instance_id": child_instance_id,
                    "error": format!("{error:?}"),
                })
                .to_string();
                return kernel.fail_run(EffectCompletion {
                    instance_id,
                    effect_id: &effect.effect_id,
                    run_id: &run_id,
                    provider: &options.provider,
                    worker_id: "whip-worker",
                    status: "failed",
                    exit_code: Some(2),
                    summary: Some("child workflow invocation link failed"),
                    metadata_json: &metadata_json,
                    idempotency_key: Some(&idempotency_key(&[
                        instance_id,
                        &effect.effect_id,
                        "terminal",
                    ])),
                });
            }
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
            None,
        )?;
        let child_worker = WorkerOptions {
            instance_id: child_instance_id.clone(),
            provider: options.provider.clone(),
            exec_profile: options.exec_profile,
            script_manifest_path: options.script_manifest_path.clone(),
            outcome: options.outcome,
            variant: options.variant.clone(),
            program_path: Some(program_path.to_path_buf()),
            root: Some(target_workflow.to_owned()),
            provider_config_paths: options.provider_config_paths.clone(),
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
            "workflow.invoke.timed_out",
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

fn fixture_harness(provider: &str, fail: bool) -> CommandAgentHarness {
    let script = if fail {
        "cat >/dev/null; echo fixture failure >&2; exit 42"
    } else {
        "cat >/dev/null; echo fixture completed"
    };
    CommandAgentHarness::new(CommandLaunchPlan::new(provider, "sh").arg("-c").arg(script))
}

fn dev(options: &CliOptions) -> ExitCode {
    let dev_options = match DevOptions::parse(&options.args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    if let Err(code) = validate_hosted_exec_for_path(
        &dev_options.program_path,
        dev_options.root.as_deref(),
        dev_options.exec_profile,
        dev_options.script_manifest_path.as_deref(),
        options.json,
    ) {
        return code;
    }
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
    let mut stream = DevStream::new(dev_options.stream);
    if stream.enabled()
        && stream
            .emit(
                "dev.started",
                json!({
                    "instance_id": started.instance_id,
                    "workflow": started.workflow,
                    "program_id": started.program_id,
                    "version_id": started.version_id,
                    "provider": dev_options.provider,
                }),
            )
            .is_err()
    {
        return ExitCode::FAILURE;
    }
    let mut last_streamed_event_sequence = 0_i64;
    if let Err(code) = stream_dev_event_deltas(
        &mut stream,
        &options.store_path,
        &started.instance_id,
        last_streamed_event_sequence,
    )
    .map(|sequence| last_streamed_event_sequence = sequence)
    {
        return code;
    }
    let mut steps = Vec::new();
    let mut workers = Vec::new();
    for _ in 0..dev_options.max_iterations {
        let step_report = match step_instance(
            &options.store_path,
            &started.instance_id,
            &ir,
            Some(Path::new(&dev_options.program_path)),
            None,
        ) {
            Ok(report) => report,
            Err(error) => return report_store_error("failed to step instance", error),
        };
        if stream
            .emit("dev.step", step_report_to_json(&step_report))
            .is_err()
        {
            return ExitCode::FAILURE;
        }
        if let Err(code) = stream_dev_event_deltas(
            &mut stream,
            &options.store_path,
            &started.instance_id,
            last_streamed_event_sequence,
        )
        .map(|sequence| last_streamed_event_sequence = sequence)
        {
            return code;
        }
        let worker_report = match run_worker_once(
            &options.store_path,
            &WorkerOptions {
                instance_id: started.instance_id.clone(),
                provider: dev_options.provider.clone(),
                exec_profile: dev_options.exec_profile,
                script_manifest_path: dev_options.script_manifest_path.clone(),
                outcome: dev_options.outcome,
                variant: dev_options.variant.clone(),
                program_path: Some(PathBuf::from(&dev_options.program_path)),
                root: dev_options.root.clone(),
                provider_config_paths: dev_options.provider_config_paths.clone(),
                max_child_iterations: 8,
            },
        ) {
            Ok(report) => report,
            Err(error) => return report_store_error("failed to run worker", error),
        };
        if stream
            .emit("dev.worker", worker_report_to_json(&worker_report))
            .is_err()
        {
            return ExitCode::FAILURE;
        }
        if let Err(code) = stream_dev_event_deltas(
            &mut stream,
            &options.store_path,
            &started.instance_id,
            last_streamed_event_sequence,
        )
        .map(|sequence| last_streamed_event_sequence = sequence)
        {
            return code;
        }
        let idle = step_report.committed_rules == 0 && worker_report.ran_effects == 0;
        steps.push(step_report);
        workers.push(worker_report);
        if idle {
            if stream
                .emit(
                    "dev.idle",
                    json!({
                        "instance_id": started.instance_id,
                        "iterations": steps.len(),
                    }),
                )
                .is_err()
            {
                return ExitCode::FAILURE;
            }
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
    let runs = match store.list_runs(&started.instance_id) {
        Ok(runs) => runs,
        Err(error) => return report_store_error("failed to list runs", error),
    };
    let artifact_counts = match artifact_counts_for_runs(&store, &runs) {
        Ok(counts) => counts,
        Err(error) => return report_store_error("failed to list artifacts", error),
    };
    let artifacts = match artifacts_for_runs(&store, &runs) {
        Ok(artifacts) => artifacts,
        Err(error) => return report_store_error("failed to list artifacts", error),
    };
    let evidence = match store.list_evidence(&started.instance_id) {
        Ok(evidence) => evidence,
        Err(error) => return report_store_error("failed to list evidence", error),
    };
    let mut assertions = eval_assertions(
        &ir,
        &facts,
        &effects,
        Some(Path::new(&dev_options.program_path)),
        &dev_options.assertion_filter,
    );
    let assertion_events = match persist_assertion_events(
        &store,
        &started.instance_id,
        &started.version_id,
        &facts,
        &effects,
        &assertions,
    ) {
        Ok(events) => events,
        Err(error) => return report_store_error("failed to record assertion events", error),
    };
    for assertion in &mut assertions {
        assertion.event_id = assertion_events.get(&assertion.target_id).cloned();
    }
    if let Err(error) = persist_assertion_diagnostics(
        &store,
        &started.instance_id,
        &started.program_id,
        &started.version_id,
        &mut assertions,
        &assertion_events,
    ) {
        return report_store_error("failed to record assertion diagnostics", error);
    }
    if let Err(code) = stream_dev_event_deltas(
        &mut stream,
        &options.store_path,
        &started.instance_id,
        last_streamed_event_sequence,
    )
    .map(|sequence| last_streamed_event_sequence = sequence)
    {
        return code;
    }
    let diagnostics = match store.list_diagnostics(Some(&started.instance_id)) {
        Ok(diagnostics) => diagnostics,
        Err(error) => return report_store_error("failed to list diagnostics", error),
    };
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
    let dev_report = dev_report_to_json(DevReportJsonInput {
        started: &started,
        ir: &ir,
        steps: &steps,
        workers: &workers,
        diagnostics: &diagnostics,
        assertion_filter: &dev_options.assertion_filter,
        assertions: &assertions,
        runs: &runs,
        artifact_counts: &artifact_counts,
        artifacts: &artifacts,
        evidence: &evidence,
    });
    if stream.enabled()
        && stream
            .emit(
                "dev.assertions",
                executable_spec_report_to_json(&assertions),
            )
            .is_err()
    {
        return ExitCode::FAILURE;
    }
    if stream.enabled() && stream.emit("dev.report", dev_report.clone()).is_err() {
        return ExitCode::FAILURE;
    }
    if stream.enabled() {
        if failed_assertions.is_empty() && guard_errors.is_empty() && branch_errors.is_empty() {
            return ExitCode::SUCCESS;
        }
        return ExitCode::from(1);
    }
    if options.json {
        let _ = emit_json(dev_report);
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

fn validate_hosted_exec_for_path(
    path: &str,
    root: Option<&str>,
    exec_profile: ExecProfile,
    script_manifest_path: Option<&Path>,
    json_output: bool,
) -> Result<(), ExitCode> {
    if !exec_profile.is_hosted() {
        return Ok(());
    }
    let script_manifest = match load_script_manifest(script_manifest_path) {
        Ok(manifest) => manifest,
        Err(message) => {
            if json_output {
                let _ = emit_json(json!({"status": "error", "error": message}));
            } else {
                eprintln!("{message}");
            }
            return Err(ExitCode::from(2));
        }
    };
    let (source, ir) = match compile_source_path_with_root(path, root) {
        Ok(compiled) => compiled,
        Err(error) => return Err(report_compile_failure(path, error)),
    };
    let diagnostics = lint_hosted_exec(&source, &ir, exec_profile, script_manifest.as_ref());
    if diagnostics.is_empty() {
        return Ok(());
    }
    if json_output {
        let _ = emit_json(json!({
            "status": "error",
            "error": {
                "kind": "diagnostics",
                "diagnostics": diagnostics.iter().map(parser_diagnostic_to_json).collect::<Vec<_>>(),
            },
        }));
    } else {
        for diagnostic in diagnostics {
            eprint!("{}", render_diagnostic(path, &source, &diagnostic));
        }
    }
    Err(ExitCode::FAILURE)
}

fn stream_dev_event_deltas(
    stream: &mut DevStream,
    store_path: &Path,
    instance_id: &str,
    after_sequence: i64,
) -> Result<i64, ExitCode> {
    if !stream.enabled() {
        return Ok(after_sequence);
    }
    let store = match SqliteStore::open(store_path) {
        Ok(store) => store,
        Err(error) => return Err(report_store_error("failed to open store", error)),
    };
    let events = match store.list_events(instance_id) {
        Ok(events) => events,
        Err(error) => return Err(report_store_error("failed to stream dev events", error)),
    };
    let new_events = events
        .iter()
        .filter(|event| event.sequence > after_sequence)
        .collect::<Vec<_>>();
    if new_events.is_empty() {
        return Ok(after_sequence);
    }
    let latest_sequence = new_events
        .iter()
        .map(|event| event.sequence)
        .max()
        .unwrap_or(after_sequence);
    if stream
        .emit(
            "dev.events",
            json!({
                "instance_id": instance_id,
                "after_sequence": after_sequence,
                "count": new_events.len(),
                "events": new_events
                    .iter()
                    .map(|event| event_to_json(event))
                    .collect::<Vec<_>>(),
            }),
        )
        .is_err()
    {
        return Err(ExitCode::FAILURE);
    }
    Ok(latest_sequence)
}

struct DevReportJsonInput<'a> {
    started: &'a StartedWorkflow,
    ir: &'a IrProgram,
    steps: &'a [StepReport],
    workers: &'a [WorkerReport],
    diagnostics: &'a [DiagnosticView],
    assertion_filter: &'a AssertionTagFilter,
    assertions: &'a [AssertionReport],
    runs: &'a [RunView],
    artifact_counts: &'a BTreeMap<String, usize>,
    artifacts: &'a [ArtifactView],
    evidence: &'a [EvidenceView],
}

fn dev_report_to_json(input: DevReportJsonInput<'_>) -> Value {
    json!({
        "schema": "whipplescript.dev_report.v0",
        "instance_id": input.started.instance_id,
        "workflow": input.started.workflow,
        "source_metadata": source_metadata_json(input.ir),
        "steps": input.steps.iter().map(step_report_to_json).collect::<Vec<_>>(),
        "workers": input.workers.iter().map(worker_report_to_json).collect::<Vec<_>>(),
        "diagnostics": input.diagnostics.iter().map(diagnostic_to_json).collect::<Vec<_>>(),
        "assertion_filter": assertion_filter_to_json(input.assertion_filter, input.ir.assertions.len(), input.assertions.len()),
        "executable_spec": executable_spec_report_to_json(input.assertions),
        "assertions": input.assertions.iter().map(assertion_report_to_json).collect::<Vec<_>>(),
        "provider_runs": provider_runs_summary_json(input.runs, input.artifact_counts),
        "provider_artifacts": provider_artifacts_summary_json(input.artifacts),
        "provider_evidence": provider_evidence_summary_json(input.evidence),
    })
}

fn accept(options: &CliOptions) -> ExitCode {
    let Some(path) = single_arg(options, "usage: whip accept <fixture.json>") else {
        return ExitCode::from(2);
    };
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{path}: failed to read acceptance fixture: {error}");
            return ExitCode::FAILURE;
        }
    };
    let fixture = match serde_json::from_str::<Value>(&source) {
        Ok(fixture) => fixture,
        Err(error) => {
            eprintln!("{path}: invalid acceptance fixture JSON: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(message) = acceptance_validate_fixture_shape(&fixture) {
        eprintln!("{path}: {message}");
        return ExitCode::from(2);
    }
    let fixture_dir = Path::new(&path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let dev_options = match acceptance_dev_options(&fixture, fixture_dir) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{path}: {message}");
            return ExitCode::from(2);
        }
    };
    let input_json = fixture.get("input").map(Value::to_string);
    let acceptance_run =
        match acceptance_dev_report(options, &dev_options, &fixture, input_json.as_deref()) {
            Ok(report) => report,
            Err(code) => return code,
        };
    let failures = acceptance_failures(&fixture, &acceptance_run);
    let passed = failures.is_empty();
    let observed = acceptance_observed_json(&acceptance_run);
    let report = json!({
        "schema": "whipplescript.acceptance_report.v0",
        "fixture": path,
        "workflow": dev_options.program_path,
        "passed": passed,
        "failures": failures,
        "observed": observed,
        "dev_report": acceptance_run.dev_report,
    });
    if options.json {
        let _ = emit_json(report);
    } else if passed {
        println!("accept {path}: passed");
    } else {
        eprintln!("accept {path}: failed");
        for failure in report
            .get("failures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            eprintln!("- {failure}");
        }
    }
    if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn acceptance_validate_fixture_shape(fixture: &Value) -> Result<(), String> {
    match fixture.get("expect") {
        Some(expect) if expect.is_object() => acceptance_validate_expect_shape(expect)?,
        Some(_) => return Err("acceptance fixture expect must be an object".to_owned()),
        None => return Err("acceptance fixture requires object field `expect`".to_owned()),
    }
    if let Some(actions) = fixture.get("actions") {
        if !actions.is_array() {
            return Err("acceptance fixture actions must be an array".to_owned());
        }
    }
    if let Some(setup) = fixture.get("setup") {
        if !setup.is_object() {
            return Err("acceptance fixture setup must be an object".to_owned());
        }
        for key in ["effects", "artifacts"] {
            if setup.get(key).is_some() {
                return Err(format!(
                    "acceptance fixture setup.{key} is not supported in acceptance_fixture.v0"
                ));
            }
        }
        for key in ["facts", "inbox"] {
            if setup.get(key).is_some_and(|value| !value.is_array()) {
                return Err(format!("acceptance fixture setup.{key} must be an array"));
            }
        }
    }
    Ok(())
}

fn acceptance_validate_expect_shape(expect: &Value) -> Result<(), String> {
    acceptance_validate_optional_enum(expect, "dev_status", &["success", "failure"])?;
    acceptance_validate_optional_enum(expect, "status", &["passed", "failed", "error"])?;
    acceptance_validate_optional_string_field(expect, "workflow")?;
    acceptance_validate_optional_u64_field(expect, "diagnostics")?;
    for key in [
        "source_metadata",
        "trace",
        "assertions",
        "assertion_untagged",
        "summary",
    ] {
        acceptance_validate_optional_object_field(expect, key)?;
    }
    for key in [
        "diagnostics_by_code",
        "actions",
        "assertion_reads",
        "inbox",
        "assertion_tags",
        "facts",
        "effects",
        "runs",
        "artifacts",
        "evidence",
    ] {
        acceptance_validate_optional_array_field(expect, key)?;
    }
    if let Some(assertions) = expect.get("assertions") {
        acceptance_validate_optional_u64_fields(
            assertions,
            "expect.assertions",
            &["total", "passed", "failed", "error"],
        )?;
    }
    if let Some(assertion_untagged) = expect.get("assertion_untagged") {
        acceptance_validate_optional_u64_fields(
            assertion_untagged,
            "expect.assertion_untagged",
            &["total", "passed", "failed", "error"],
        )?;
    }
    if let Some(summary) = expect.get("summary") {
        acceptance_validate_optional_u64_fields(summary, "expect.summary", &["facts", "effects"])?;
    }
    if let Some(source_metadata) = expect.get("source_metadata") {
        acceptance_validate_optional_array_field_with_path(
            source_metadata,
            "expect.source_metadata",
            "targets",
        )?;
    }
    if let Some(trace) = expect.get("trace") {
        acceptance_validate_optional_object_field_with_path(trace, "expect.trace", "summary")?;
        acceptance_validate_optional_object_field_with_path(trace, "expect.trace", "conformance")?;
        acceptance_validate_optional_array_field_with_path(trace, "expect.trace", "groups")?;
        acceptance_validate_optional_array_field_with_path(trace, "expect.trace", "items")?;
        if let Some(summary) = trace.get("summary") {
            acceptance_validate_optional_u64_fields(
                summary,
                "expect.trace.summary",
                &["events", "abstract_events"],
            )?;
        }
        if let Some(conformance) = trace.get("conformance") {
            acceptance_validate_optional_bool_field_with_path(
                conformance,
                "expect.trace.conformance",
                "ok",
            )?;
        }
    }
    Ok(())
}

fn acceptance_validate_optional_enum(
    object: &Value,
    key: &str,
    allowed: &[&str],
) -> Result<(), String> {
    let Some(value) = object.get(key) else {
        return Ok(());
    };
    let Some(value) = value.as_str() else {
        return Err(format!("`expect.{key}` must be a string"));
    };
    if !allowed.contains(&value) {
        return Err(format!("unknown expect.{key} `{value}`"));
    }
    Ok(())
}

fn acceptance_validate_optional_string_field(object: &Value, key: &str) -> Result<(), String> {
    if object.get(key).is_some_and(|value| !value.is_string()) {
        return Err(format!("`expect.{key}` must be a string"));
    }
    Ok(())
}

fn acceptance_validate_optional_u64_field(object: &Value, key: &str) -> Result<(), String> {
    if object
        .get(key)
        .is_some_and(|value| value.as_u64().is_none())
    {
        return Err(format!("`expect.{key}` must be a non-negative integer"));
    }
    Ok(())
}

fn acceptance_validate_optional_u64_fields(
    object: &Value,
    path: &str,
    keys: &[&str],
) -> Result<(), String> {
    for key in keys {
        if object
            .get(*key)
            .is_some_and(|value| value.as_u64().is_none())
        {
            return Err(format!("`{path}.{key}` must be a non-negative integer"));
        }
    }
    Ok(())
}

fn acceptance_validate_optional_object_field(object: &Value, key: &str) -> Result<(), String> {
    if object.get(key).is_some_and(|value| !value.is_object()) {
        return Err(format!("`expect.{key}` must be an object"));
    }
    Ok(())
}

fn acceptance_validate_optional_object_field_with_path(
    object: &Value,
    path: &str,
    key: &str,
) -> Result<(), String> {
    if object.get(key).is_some_and(|value| !value.is_object()) {
        return Err(format!("`{path}.{key}` must be an object"));
    }
    Ok(())
}

fn acceptance_validate_optional_array_field(object: &Value, key: &str) -> Result<(), String> {
    if object.get(key).is_some_and(|value| !value.is_array()) {
        return Err(format!("`expect.{key}` must be an array"));
    }
    Ok(())
}

fn acceptance_validate_optional_array_field_with_path(
    object: &Value,
    path: &str,
    key: &str,
) -> Result<(), String> {
    if object.get(key).is_some_and(|value| !value.is_array()) {
        return Err(format!("`{path}.{key}` must be an array"));
    }
    Ok(())
}

fn acceptance_validate_optional_bool_field_with_path(
    object: &Value,
    path: &str,
    key: &str,
) -> Result<(), String> {
    if object.get(key).is_some_and(|value| !value.is_boolean()) {
        return Err(format!("`{path}.{key}` must be a boolean"));
    }
    Ok(())
}

fn acceptance_dev_options(fixture: &Value, fixture_dir: &Path) -> Result<DevOptions, String> {
    if fixture.get("schema").and_then(Value::as_str) != Some("whipplescript.acceptance_fixture.v0")
    {
        return Err(
            "expected schema `whipplescript.acceptance_fixture.v0` in acceptance fixture"
                .to_owned(),
        );
    }
    let workflow = fixture
        .get("workflow")
        .and_then(Value::as_str)
        .ok_or_else(|| "acceptance fixture requires string field `workflow`".to_owned())?;
    let workflow = acceptance_fixture_path(fixture_dir, workflow);
    let provider_config_paths = fixture
        .get("provider_config_paths")
        .map(|value| {
            let paths = value
                .as_array()
                .ok_or_else(|| "`provider_config_paths` must be an array of strings".to_owned())?;
            paths
                .iter()
                .map(|path| {
                    path.as_str()
                        .map(|path| acceptance_fixture_path(fixture_dir, path))
                        .ok_or_else(|| "`provider_config_paths` entries must be strings".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let assertion_filter = AssertionTagFilter {
        include_tags: acceptance_string_array(fixture, "include_tags")?,
        exclude_tags: acceptance_string_array(fixture, "exclude_tags")?,
    };
    let root = acceptance_optional_string(fixture, "root")?;
    let provider =
        acceptance_optional_string(fixture, "provider")?.unwrap_or_else(|| "fixture".to_owned());
    let outcome_value =
        acceptance_optional_string(fixture, "outcome")?.unwrap_or_else(|| "completed".to_owned());
    let outcome = match outcome_value.as_str() {
        "completed" => FixtureOutcome::Completed,
        "failed" => FixtureOutcome::Failed,
        "timed_out" | "timeout" => FixtureOutcome::TimedOut,
        "cancelled" | "cancel" => FixtureOutcome::Cancelled,
        other => return Err(format!("unknown acceptance fixture outcome `{other}`")),
    };
    let max_iterations = match fixture.get("max_iterations") {
        Some(value) => {
            let value = value
                .as_u64()
                .ok_or_else(|| "`max_iterations` must be a positive integer".to_owned())?;
            if value == 0 {
                return Err("`max_iterations` must be at least 1".to_owned());
            }
            usize::try_from(value)
                .map_err(|_| "`max_iterations` is too large for this platform".to_owned())?
        }
        None => 8,
    };
    Ok(DevOptions {
        program_path: workflow.to_string_lossy().into_owned(),
        root,
        provider,
        exec_profile: ExecProfile::Dev,
        script_manifest_path: None,
        provider_config_paths,
        outcome,
        variant: acceptance_optional_string(fixture, "variant")?,
        max_iterations,
        assertion_filter,
        stream: None,
    })
}

fn acceptance_optional_string(fixture: &Value, key: &str) -> Result<Option<String>, String> {
    fixture
        .get(key)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`{key}` must be a string"))
        })
        .transpose()
}

fn acceptance_fixture_path(fixture_dir: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        fixture_dir.join(path)
    }
}

fn acceptance_string_array(fixture: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(values) = fixture.get(key) else {
        return Ok(Vec::new());
    };
    let values = values
        .as_array()
        .ok_or_else(|| format!("`{key}` must be an array of strings"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`{key}` entries must be strings"))
        })
        .collect()
}

#[derive(Clone, Debug)]
struct AcceptanceDevRun {
    dev_report: Value,
    dev_success: bool,
    actions: Vec<AcceptanceActionReport>,
    facts: Vec<FactView>,
    effects: Vec<EffectView>,
    runs: Vec<RunView>,
    artifact_counts: BTreeMap<String, usize>,
    artifacts: Vec<ArtifactView>,
    evidence: Vec<EvidenceView>,
    inbox_items: Vec<InboxItemView>,
    events: Vec<EventView>,
}

#[derive(Clone, Debug)]
struct AcceptanceActionReport {
    action_type: String,
    event_id: String,
    sequence: i64,
}

fn acceptance_dev_report(
    options: &CliOptions,
    dev_options: &DevOptions,
    fixture: &Value,
    input_json: Option<&str>,
) -> Result<AcceptanceDevRun, ExitCode> {
    let started = start_workflow_instance(
        &dev_options.program_path,
        dev_options.root.as_deref(),
        input_json,
        options,
    )?;
    let (_source, ir) =
        match compile_source_path_with_root(&dev_options.program_path, dev_options.root.as_deref())
        {
            Ok(compiled) => compiled,
            Err(error) => return Err(report_compile_failure(&dev_options.program_path, error)),
        };
    acceptance_seed_setup_facts(fixture, options, &started.instance_id, &ir)?;
    acceptance_seed_setup_inbox(fixture, options, &started.instance_id)?;
    let actions = acceptance_run_actions(fixture, options, &started.instance_id)?;
    let mut steps = Vec::new();
    let mut workers = Vec::new();
    for _ in 0..dev_options.max_iterations {
        let step_report = match step_instance(
            &options.store_path,
            &started.instance_id,
            &ir,
            Some(Path::new(&dev_options.program_path)),
            None,
        ) {
            Ok(report) => report,
            Err(error) => return Err(report_store_error("failed to step instance", error)),
        };
        let worker_report = match run_worker_once(
            &options.store_path,
            &WorkerOptions {
                instance_id: started.instance_id.clone(),
                provider: dev_options.provider.clone(),
                exec_profile: dev_options.exec_profile,
                script_manifest_path: dev_options.script_manifest_path.clone(),
                outcome: dev_options.outcome,
                variant: dev_options.variant.clone(),
                program_path: Some(PathBuf::from(&dev_options.program_path)),
                root: dev_options.root.clone(),
                provider_config_paths: dev_options.provider_config_paths.clone(),
                max_child_iterations: 8,
            },
        ) {
            Ok(report) => report,
            Err(error) => return Err(report_store_error("failed to run worker", error)),
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
        Err(error) => return Err(report_store_error("failed to open store", error)),
    };
    let facts = match store.list_facts(&started.instance_id) {
        Ok(facts) => facts,
        Err(error) => return Err(report_store_error("failed to list facts", error)),
    };
    let effects = match store.list_effects(&started.instance_id) {
        Ok(effects) => effects,
        Err(error) => return Err(report_store_error("failed to list effects", error)),
    };
    let runs = match store.list_runs(&started.instance_id) {
        Ok(runs) => runs,
        Err(error) => return Err(report_store_error("failed to list runs", error)),
    };
    let artifact_counts = match artifact_counts_for_runs(&store, &runs) {
        Ok(counts) => counts,
        Err(error) => return Err(report_store_error("failed to list artifacts", error)),
    };
    let artifacts = match artifacts_for_runs(&store, &runs) {
        Ok(artifacts) => artifacts,
        Err(error) => return Err(report_store_error("failed to list artifacts", error)),
    };
    let evidence = match store.list_evidence(&started.instance_id) {
        Ok(evidence) => evidence,
        Err(error) => return Err(report_store_error("failed to list evidence", error)),
    };
    let inbox_items = match store.list_inbox_items(None) {
        Ok(items) => items
            .into_iter()
            .filter(|item| item.instance_id == started.instance_id)
            .collect::<Vec<_>>(),
        Err(error) => return Err(report_store_error("failed to list inbox items", error)),
    };
    let events = match store.list_events(&started.instance_id) {
        Ok(events) => events,
        Err(error) => return Err(report_store_error("failed to list events", error)),
    };
    let mut assertions = eval_assertions(
        &ir,
        &facts,
        &effects,
        Some(Path::new(&dev_options.program_path)),
        &dev_options.assertion_filter,
    );
    let assertion_events = match persist_assertion_events(
        &store,
        &started.instance_id,
        &started.version_id,
        &facts,
        &effects,
        &assertions,
    ) {
        Ok(events) => events,
        Err(error) => {
            return Err(report_store_error(
                "failed to record assertion events",
                error,
            ))
        }
    };
    for assertion in &mut assertions {
        assertion.event_id = assertion_events.get(&assertion.target_id).cloned();
    }
    if let Err(error) = persist_assertion_diagnostics(
        &store,
        &started.instance_id,
        &started.program_id,
        &started.version_id,
        &mut assertions,
        &assertion_events,
    ) {
        return Err(report_store_error(
            "failed to record assertion diagnostics",
            error,
        ));
    }
    let diagnostics = match store.list_diagnostics(Some(&started.instance_id)) {
        Ok(diagnostics) => diagnostics,
        Err(error) => return Err(report_store_error("failed to list diagnostics", error)),
    };
    let failed_assertions = assertions.iter().any(|assertion| !assertion.passed);
    let guard_errors = steps
        .iter()
        .flat_map(|step| &step.guard_reports)
        .any(|guard| guard.status == GuardStatus::Error);
    let branch_errors = steps
        .iter()
        .flat_map(|step| &step.branch_reports)
        .any(|branch| branch.status != BranchStatus::Matched);
    let report = dev_report_to_json(DevReportJsonInput {
        started: &started,
        ir: &ir,
        steps: &steps,
        workers: &workers,
        diagnostics: &diagnostics,
        assertion_filter: &dev_options.assertion_filter,
        assertions: &assertions,
        runs: &runs,
        artifact_counts: &artifact_counts,
        artifacts: &artifacts,
        evidence: &evidence,
    });
    Ok(AcceptanceDevRun {
        dev_report: report,
        dev_success: !failed_assertions && !guard_errors && !branch_errors,
        actions,
        facts,
        effects,
        runs,
        artifact_counts,
        artifacts,
        evidence,
        inbox_items,
        events,
    })
}

fn acceptance_seed_setup_facts(
    fixture: &Value,
    options: &CliOptions,
    instance_id: &str,
    ir: &IrProgram,
) -> Result<(), ExitCode> {
    let Some(setup) = fixture.get("setup") else {
        return Ok(());
    };
    if !setup.is_object() {
        eprintln!("acceptance fixture setup must be an object");
        return Err(ExitCode::from(2));
    }
    let Some(facts) = setup.get("facts").and_then(Value::as_array) else {
        if setup.get("facts").is_some() {
            eprintln!("acceptance fixture setup.facts must be an array");
            return Err(ExitCode::from(2));
        }
        return Ok(());
    };
    let mut store = open_store_or_exit(options)?;
    for (index, fact) in facts.iter().enumerate() {
        let Some(name) = fact.get("name").and_then(Value::as_str) else {
            eprintln!("acceptance fixture setup.facts[{index}].name must be a string");
            return Err(ExitCode::from(2));
        };
        let Some(value) = fact.get("value") else {
            eprintln!("acceptance fixture setup.facts[{index}].value is required");
            return Err(ExitCode::from(2));
        };
        let mut errors = Vec::new();
        validate_json_for_ir_type(
            ir,
            value,
            &IrType::Ref(name.to_owned()),
            &format!("setup.facts[{index}].value"),
            &mut errors,
        );
        if !errors.is_empty() {
            eprintln!(
                "invalid acceptance fixture setup.facts[{index}] for `{name}`: {}",
                errors.join("; ")
            );
            return Err(ExitCode::from(2));
        }
        let value_json = value.to_string();
        let key = fact
            .get("key")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| record_fact_key(name, &value_json));
        let fact_id = idempotency_key(&[
            instance_id,
            "acceptance.setup.fact",
            &index.to_string(),
            name,
            &key,
            &value_json,
        ]);
        let event_idempotency = idempotency_key(&[
            instance_id,
            "acceptance.setup.fact.event",
            &index.to_string(),
            name,
            &key,
        ]);
        if let Err(error) = store.derive_fact(DerivedFact {
            instance_id,
            fact: NewFact {
                fact_id: &fact_id,
                name,
                key: &key,
                value_json: &value_json,
                schema_id: Some(name),
                provenance_class: "fixture",
                correlation_id: Some("acceptance.fixture"),
                source_span_json: None,
            },
            source: "acceptance.fixture",
            causation_id: None,
            idempotency_key: Some(&event_idempotency),
        }) {
            eprintln!(
                "failed to seed acceptance setup fact: {}",
                store_error(error)
            );
            return Err(ExitCode::FAILURE);
        }
    }
    Ok(())
}

fn acceptance_seed_setup_inbox(
    fixture: &Value,
    options: &CliOptions,
    instance_id: &str,
) -> Result<(), ExitCode> {
    let Some(setup) = fixture.get("setup") else {
        return Ok(());
    };
    let Some(inbox_items) = setup.get("inbox").and_then(Value::as_array) else {
        if setup.get("inbox").is_some() {
            eprintln!("acceptance fixture setup.inbox must be an array");
            return Err(ExitCode::from(2));
        }
        return Ok(());
    };
    let store = open_store_or_exit(options)?;
    for (index, item) in inbox_items.iter().enumerate() {
        let Some(prompt) = item.get("prompt").and_then(Value::as_str) else {
            eprintln!("acceptance fixture setup.inbox[{index}].prompt must be a string");
            return Err(ExitCode::from(2));
        };
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        let severity = item
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or("normal");
        let choices_json = match acceptance_optional_array_json(item, "choices") {
            Ok(value) => value,
            Err(message) => {
                eprintln!("acceptance fixture setup.inbox[{index}].{message}");
                return Err(ExitCode::from(2));
            }
        };
        let related_effects_json = match acceptance_optional_array_json(item, "related_effects") {
            Ok(value) => value,
            Err(message) => {
                eprintln!("acceptance fixture setup.inbox[{index}].{message}");
                return Err(ExitCode::from(2));
            }
        };
        let related_artifacts_json = match acceptance_optional_array_json(item, "related_artifacts")
        {
            Ok(value) => value,
            Err(message) => {
                eprintln!("acceptance fixture setup.inbox[{index}].{message}");
                return Err(ExitCode::from(2));
            }
        };
        let freeform_allowed = item
            .get("freeform_allowed")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let inbox_item_id = item
            .get("inbox_item_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                idempotency_key(&[
                    instance_id,
                    "acceptance.setup.inbox",
                    &index.to_string(),
                    prompt,
                    status,
                    severity,
                ])
            });
        if let Err(error) = store.create_inbox_item(NewInboxItem {
            inbox_item_id: &inbox_item_id,
            instance_id,
            effect_id: item.get("effect_id").and_then(Value::as_str),
            status,
            prompt,
            choices_json: &choices_json,
            freeform_allowed,
            severity,
            related_effects_json: &related_effects_json,
            related_artifacts_json: &related_artifacts_json,
        }) {
            eprintln!(
                "failed to seed acceptance setup inbox item: {}",
                store_error(error)
            );
            return Err(ExitCode::FAILURE);
        }
    }
    Ok(())
}

fn acceptance_optional_array_json(item: &Value, key: &str) -> Result<String, String> {
    match item.get(key) {
        Some(value) if value.is_array() => Ok(value.to_string()),
        Some(_) => Err(format!("{key} must be an array")),
        None => Ok("[]".to_owned()),
    }
}

fn acceptance_run_actions(
    fixture: &Value,
    options: &CliOptions,
    instance_id: &str,
) -> Result<Vec<AcceptanceActionReport>, ExitCode> {
    let Some(actions) = fixture.get("actions").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let store = open_store_or_exit(options)?;
    let mut kernel = RuntimeKernel::new(store);
    let mut reports = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        let Some(action_type) = action.get("type").and_then(Value::as_str) else {
            eprintln!("acceptance fixture actions[{index}].type must be a string");
            return Err(ExitCode::from(2));
        };
        let idempotency = idempotency_key(&[
            instance_id,
            "acceptance-action",
            &index.to_string(),
            action_type,
        ]);
        let event = match action_type {
            "pause" => {
                let reason = action
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("acceptance fixture pause");
                kernel.pause_instance(instance_id, Some(reason), Some(&idempotency))
            }
            "resume" => kernel.resume_instance(instance_id, Some(&idempotency)),
            "cancel" => {
                let reason = action
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("acceptance fixture cancel");
                kernel.cancel_instance(instance_id, Some(reason), Some(&idempotency))
            }
            other => {
                eprintln!(
                    "unknown acceptance fixture action `{other}`; expected pause, resume, or cancel"
                );
                return Err(ExitCode::from(2));
            }
        };
        let event = match event {
            Ok(event) => event,
            Err(error) => return Err(report_store_error("failed to run fixture action", error)),
        };
        reports.push(AcceptanceActionReport {
            action_type: action_type.to_owned(),
            event_id: event.event_id,
            sequence: event.sequence,
        });
    }
    Ok(reports)
}

fn acceptance_failures(fixture: &Value, run: &AcceptanceDevRun) -> Vec<String> {
    let mut failures = Vec::new();
    let expect = fixture.get("expect").unwrap_or(&Value::Null);
    let expected_dev_status = expect
        .get("dev_status")
        .and_then(Value::as_str)
        .unwrap_or("success");
    match expected_dev_status {
        "success" if !run.dev_success => failures.push("expected dev run to succeed".to_owned()),
        "failure" if run.dev_success => failures.push("expected dev run to fail".to_owned()),
        "success" | "failure" => {}
        other => failures.push(format!("unknown expected dev_status `{other}`")),
    }
    if let Some(expected) = expect.get("workflow").and_then(Value::as_str) {
        acceptance_expect_str(&run.dev_report, "/workflow", expected, &mut failures);
    }
    if let Some(expected) = expect.get("status").and_then(Value::as_str) {
        acceptance_expect_str(
            &run.dev_report,
            "/executable_spec/status",
            expected,
            &mut failures,
        );
    }
    acceptance_expect_source_metadata(expect, &run.dev_report, &mut failures);
    if let Some(expected) = expect.get("diagnostics").and_then(Value::as_u64) {
        let actual = run
            .dev_report
            .get("diagnostics")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0) as u64;
        if actual != expected {
            failures.push(format!("expected diagnostics={expected}, got {actual}"));
        }
    }
    acceptance_expect_diagnostics_by_code(expect, &run.dev_report, &mut failures);
    acceptance_expect_actions(expect, &run.actions, &mut failures);
    acceptance_expect_assertion_reads(
        expect,
        &run.dev_report,
        &run.events,
        &run.evidence,
        &mut failures,
    );
    if let Some(assertions) = expect.get("assertions").and_then(Value::as_object) {
        for key in ["total", "passed", "failed", "error"] {
            if let Some(expected) = assertions.get(key).and_then(Value::as_u64) {
                let pointer = format!("/executable_spec/summary/{key}");
                let actual = run.dev_report.pointer(&pointer).and_then(Value::as_u64);
                if actual != Some(expected) {
                    failures.push(format!(
                        "expected assertions.{key}={expected}, got {}",
                        actual
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "missing".to_owned())
                    ));
                }
            }
        }
    }
    acceptance_expect_assertion_tags(expect, &run.dev_report, &mut failures);
    acceptance_expect_assertion_untagged(expect, &run.dev_report, &mut failures);
    acceptance_expect_summary(expect, &run.facts, &run.effects, &mut failures);
    acceptance_expect_facts(expect, &run.facts, &mut failures);
    acceptance_expect_effects(expect, &run.effects, &mut failures);
    acceptance_expect_runs(expect, &run.runs, &run.artifact_counts, &mut failures);
    acceptance_expect_artifacts(expect, &run.artifacts, &mut failures);
    acceptance_expect_evidence(expect, &run.evidence, &mut failures);
    acceptance_expect_inbox(expect, &run.inbox_items, &mut failures);
    acceptance_expect_trace(expect, &run.events, &mut failures);
    failures
}

fn acceptance_expect_actions(
    expect: &Value,
    action_reports: &[AcceptanceActionReport],
    failures: &mut Vec<String>,
) {
    let Some(expected_actions) = expect.get("actions").and_then(Value::as_array) else {
        return;
    };
    for (index, expected_action) in expected_actions.iter().enumerate() {
        let Some(action_type) = expected_action.get("type").and_then(Value::as_str) else {
            failures.push(format!("expect.actions[{index}].type must be a string"));
            continue;
        };
        let Some(expected_count) = expected_action.get("count").and_then(Value::as_u64) else {
            failures.push(format!("expect.actions[{index}].count must be an integer"));
            continue;
        };
        let actual_count = action_reports
            .iter()
            .filter(|report| report.action_type == action_type)
            .count() as u64;
        if actual_count != expected_count {
            failures.push(format!(
                "expected actions[{index}] type={action_type:?} count={expected_count}, got {actual_count}"
            ));
        }
    }
}

fn acceptance_expect_trace(expect: &Value, events: &[EventView], failures: &mut Vec<String>) {
    let Some(expected_trace) = expect.get("trace") else {
        return;
    };
    let actual_trace = acceptance_observed_trace_json(events);
    if let Some(expected_ok) = expected_trace
        .pointer("/conformance/ok")
        .and_then(Value::as_bool)
    {
        let actual_ok = actual_trace
            .pointer("/conformance/ok")
            .and_then(Value::as_bool);
        if actual_ok != Some(expected_ok) {
            failures.push(format!(
                "expected trace.conformance.ok={expected_ok}, got {}",
                actual_ok
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "missing".to_owned())
            ));
        }
    }
    if let Some(expected) = expected_trace
        .pointer("/summary/events")
        .and_then(Value::as_u64)
    {
        let actual = actual_trace
            .pointer("/summary/events")
            .and_then(Value::as_u64);
        if actual != Some(expected) {
            failures.push(format!(
                "expected trace.summary.events={expected}, got {}",
                actual
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "missing".to_owned())
            ));
        }
    }
    if let Some(expected) = expected_trace
        .pointer("/summary/abstract_events")
        .and_then(Value::as_u64)
    {
        let actual = actual_trace
            .pointer("/summary/abstract_events")
            .and_then(Value::as_u64);
        if actual != Some(expected) {
            failures.push(format!(
                "expected trace.summary.abstract_events={expected}, got {}",
                actual
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "missing".to_owned())
            ));
        }
    }
    if let Some(expected_groups) = expected_trace.get("groups").and_then(Value::as_array) {
        let actual_groups = actual_trace
            .get("groups")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for (index, expected_group) in expected_groups.iter().enumerate() {
            let Some(event_type) = expected_group.get("type").and_then(Value::as_str) else {
                failures.push(format!(
                    "expect.trace.groups[{index}].type must be a string"
                ));
                continue;
            };
            let Some(expected_count) = expected_group.get("count").and_then(Value::as_u64) else {
                failures.push(format!(
                    "expect.trace.groups[{index}].count must be an integer"
                ));
                continue;
            };
            let actual_count = actual_groups
                .iter()
                .find(|group| group.get("type").and_then(Value::as_str) == Some(event_type))
                .and_then(|group| group.get("count"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if actual_count != expected_count {
                failures.push(format!(
                    "expected trace.groups[{index}] type={event_type:?} count={expected_count}, got {actual_count}"
                ));
            }
        }
    }
    if let Some(expected_items) = expected_trace.get("items").and_then(Value::as_array) {
        let actual_items = actual_trace
            .get("items")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for (index, expected_item) in expected_items.iter().enumerate() {
            if !acceptance_trace_item_has_selector(expected_item) {
                failures.push(format!(
                    "expect.trace.items[{index}] must include at least one selector"
                ));
                continue;
            }
            if !actual_items
                .iter()
                .any(|actual_item| acceptance_trace_item_matches(expected_item, actual_item))
            {
                failures.push(format!(
                    "expected trace.items[{index}] {}, got no matching trace item",
                    acceptance_trace_item_selector(expected_item)
                ));
            }
        }
    }
}

fn acceptance_trace_item_has_selector(expected_item: &Value) -> bool {
    expected_item
        .get("sequence")
        .and_then(Value::as_u64)
        .is_some()
        || [
            "type",
            "effect_id",
            "run_id",
            "status",
            "predicate",
            "reason",
            "provider",
        ]
        .iter()
        .any(|key| expected_item.get(key).and_then(Value::as_str).is_some())
}

fn acceptance_trace_item_matches(expected_item: &Value, actual_item: &Value) -> bool {
    if let Some(expected_sequence) = expected_item.get("sequence").and_then(Value::as_u64) {
        if actual_item.get("sequence").and_then(Value::as_u64) != Some(expected_sequence) {
            return false;
        }
    }
    if let Some(expected_type) = expected_item.get("type").and_then(Value::as_str) {
        if actual_item.pointer("/event/type").and_then(Value::as_str) != Some(expected_type) {
            return false;
        }
    }
    for key in [
        "effect_id",
        "run_id",
        "status",
        "predicate",
        "reason",
        "provider",
    ] {
        if let Some(expected) = expected_item.get(key).and_then(Value::as_str) {
            let pointer = format!("/event/{key}");
            if actual_item.pointer(&pointer).and_then(Value::as_str) != Some(expected) {
                return false;
            }
        }
    }
    true
}

fn acceptance_trace_item_selector(expected_item: &Value) -> String {
    let mut selectors = Vec::new();
    if let Some(sequence) = expected_item.get("sequence").and_then(Value::as_u64) {
        selectors.push(format!("sequence={sequence}"));
    }
    for key in [
        "type",
        "effect_id",
        "run_id",
        "status",
        "predicate",
        "reason",
        "provider",
    ] {
        if let Some(value) = expected_item.get(key).and_then(Value::as_str) {
            selectors.push(format!("{key}={value:?}"));
        }
    }
    selectors.join(" ")
}

fn acceptance_expect_inbox(
    expect: &Value,
    inbox_items: &[InboxItemView],
    failures: &mut Vec<String>,
) {
    let Some(expected_items) = expect.get("inbox").and_then(Value::as_array) else {
        return;
    };
    for (index, expected_item) in expected_items.iter().enumerate() {
        let Some(status) = expected_item.get("status").and_then(Value::as_str) else {
            failures.push(format!("expect.inbox[{index}].status must be a string"));
            continue;
        };
        let Some(expected_count) = expected_item.get("count").and_then(Value::as_u64) else {
            failures.push(format!("expect.inbox[{index}].count must be an integer"));
            continue;
        };
        let severity = expected_item.get("severity").and_then(Value::as_str);
        let actual_count = inbox_items
            .iter()
            .filter(|item| {
                item.status == status
                    && severity
                        .map(|expected_severity| item.severity == expected_severity)
                        .unwrap_or(true)
            })
            .count() as u64;
        if actual_count != expected_count {
            let severity_clause = severity
                .map(|severity| format!(" severity={severity:?}"))
                .unwrap_or_default();
            failures.push(format!(
                "expected inbox[{index}] status={status:?}{severity_clause} count={expected_count}, got {actual_count}"
            ));
        }
    }
}

fn acceptance_expect_assertion_reads(
    expect: &Value,
    dev_report: &Value,
    events: &[EventView],
    evidence: &[EvidenceView],
    failures: &mut Vec<String>,
) {
    let Some(expected_reads) = expect.get("assertion_reads").and_then(Value::as_array) else {
        return;
    };
    let observed_reads = acceptance_observed_assertion_reads(dev_report, events, evidence);
    let actual_reads = observed_reads
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .collect::<Vec<_>>();
    for (index, expected_read) in expected_reads.iter().enumerate() {
        if !acceptance_assertion_read_has_selector(expected_read) {
            failures.push(format!(
                "expect.assertion_reads[{index}] must include at least one selector"
            ));
            continue;
        }
        let Some(actual_read) = acceptance_find_assertion_read(expected_read, &actual_reads) else {
            failures.push(format!(
                "expected assertion_reads[{index}] {}, got no matching assertion read",
                acceptance_assertion_read_selector(expected_read)
            ));
            continue;
        };
        if let Some(expected) = expected_read.get("match_count").and_then(Value::as_u64) {
            let actual = actual_read.get("match_count").and_then(Value::as_u64);
            if actual != Some(expected) {
                failures.push(format!(
                    "expected assertion_reads[{index}] match_count={expected}, got {}",
                    actual
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "missing".to_owned())
                ));
            }
        }
        let Some(expected_matches) = expected_read.get("matches").and_then(Value::as_array) else {
            continue;
        };
        let actual_matches = actual_read
            .get("matches")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for (match_index, expected_match) in expected_matches.iter().enumerate() {
            let Some(expected_count) = expected_match.get("count").and_then(Value::as_u64) else {
                failures.push(format!(
                    "expect.assertion_reads[{index}].matches[{match_index}].count must be an integer"
                ));
                continue;
            };
            let actual_count = actual_matches
                .iter()
                .filter(|actual_match| {
                    acceptance_assertion_read_match_matches(expected_match, actual_match)
                })
                .filter_map(|actual_match| actual_match.get("count").and_then(Value::as_u64))
                .sum::<u64>();
            if actual_count != expected_count {
                failures.push(format!(
                    "expected assertion_reads[{index}].matches[{match_index}] {} count={expected_count}, got {actual_count}",
                    acceptance_assertion_read_match_selector(expected_match)
                ));
            }
        }
    }
}

fn acceptance_assertion_read_has_selector(expected_read: &Value) -> bool {
    ["source", "kind", "head", "guard"]
        .iter()
        .any(|key| expected_read.get(key).and_then(Value::as_str).is_some())
}

fn acceptance_find_assertion_read<'a>(
    expected_read: &Value,
    actual_reads: &'a [&'a Value],
) -> Option<&'a Value> {
    actual_reads.iter().copied().find(|actual_read| {
        for key in ["source", "kind", "head", "guard"] {
            if let Some(expected) = expected_read.get(key).and_then(Value::as_str) {
                if actual_read.get(key).and_then(Value::as_str) != Some(expected) {
                    return false;
                }
            }
        }
        true
    })
}

fn acceptance_assertion_read_selector(expected_read: &Value) -> String {
    ["source", "kind", "head", "guard"]
        .iter()
        .filter_map(|key| {
            expected_read
                .get(key)
                .and_then(Value::as_str)
                .map(|value| format!("{key}={value:?}"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn acceptance_assertion_read_match_matches(expected_match: &Value, actual_match: &Value) -> bool {
    for key in ["name", "status", "prompt_content_type", "provenance_class"] {
        if let Some(expected) = expected_match.get(key).and_then(Value::as_str) {
            if actual_match.get(key).and_then(Value::as_str) != Some(expected) {
                return false;
            }
        }
    }
    for key in ["trace_items", "evidence_items"] {
        if let Some(expected) = expected_match.get(key).and_then(Value::as_u64) {
            if actual_match.get(key).and_then(Value::as_u64) != Some(expected) {
                return false;
            }
        }
    }
    if let Some(expected) = expected_match
        .get("trace_sequences")
        .and_then(Value::as_array)
    {
        let actual = actual_match
            .get("trace_sequences")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if actual != expected.as_slice() {
            return false;
        }
    }
    if let Some(expected) = expected_match.get("evidence_ids").and_then(Value::as_array) {
        let actual = actual_match
            .get("evidence_ids")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if actual != expected.as_slice() {
            return false;
        }
    }
    true
}

fn acceptance_assertion_read_match_selector(expected_match: &Value) -> String {
    let mut selectors = ["name", "status", "prompt_content_type", "provenance_class"]
        .iter()
        .filter_map(|key| {
            expected_match
                .get(key)
                .and_then(Value::as_str)
                .map(|value| format!("{key}={value:?}"))
        })
        .collect::<Vec<_>>();
    selectors.extend(["trace_items", "evidence_items"].iter().filter_map(|key| {
        expected_match
            .get(key)
            .and_then(Value::as_u64)
            .map(|value| format!("{key}={value}"))
    }));
    selectors.extend(
        ["trace_sequences", "evidence_ids"]
            .iter()
            .filter_map(|key| {
                expected_match
                    .get(key)
                    .and_then(Value::as_array)
                    .map(|value| format!("{key}={}", Value::Array(value.clone())))
            }),
    );
    selectors.join(" ")
}

fn acceptance_expect_diagnostics_by_code(
    expect: &Value,
    dev_report: &Value,
    failures: &mut Vec<String>,
) {
    let Some(expected_codes) = expect.get("diagnostics_by_code").and_then(Value::as_array) else {
        return;
    };
    let diagnostics = dev_report
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for (index, expected_code) in expected_codes.iter().enumerate() {
        let Some(code) = expected_code.get("code").and_then(Value::as_str) else {
            failures.push(format!(
                "expect.diagnostics_by_code[{index}].code must be a string"
            ));
            continue;
        };
        let Some(expected_count) = expected_code.get("count").and_then(Value::as_u64) else {
            failures.push(format!(
                "expect.diagnostics_by_code[{index}].count must be an integer"
            ));
            continue;
        };
        let actual_count = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.get("code").and_then(Value::as_str) == Some(code))
            .count() as u64;
        if actual_count != expected_count {
            failures.push(format!(
                "expected diagnostics_by_code[{index}] code={code:?} count={expected_count}, got {actual_count}"
            ));
        }
    }
}

fn acceptance_expect_assertion_tags(
    expect: &Value,
    dev_report: &Value,
    failures: &mut Vec<String>,
) {
    let Some(expected_tags) = expect.get("assertion_tags").and_then(Value::as_array) else {
        return;
    };
    let actual_tags = dev_report
        .pointer("/executable_spec/tags")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for (index, expected_tag) in expected_tags.iter().enumerate() {
        let Some(tag) = expected_tag.get("tag").and_then(Value::as_str) else {
            failures.push(format!(
                "expect.assertion_tags[{index}].tag must be a string"
            ));
            continue;
        };
        let Some(actual_tag) = actual_tags
            .iter()
            .find(|actual| actual.get("tag").and_then(Value::as_str) == Some(tag))
        else {
            failures.push(format!(
                "expected assertion_tags[{index}] tag={tag:?}, got no matching executable-spec tag group"
            ));
            continue;
        };
        for key in ["total", "passed", "failed", "error"] {
            if let Some(expected) = expected_tag.get(key).and_then(Value::as_u64) {
                let pointer = format!("/summary/{key}");
                let actual = actual_tag.pointer(&pointer).and_then(Value::as_u64);
                if actual != Some(expected) {
                    failures.push(format!(
                        "expected assertion_tags[{index}] tag={tag:?} {key}={expected}, got {}",
                        actual
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "missing".to_owned())
                    ));
                }
            }
        }
    }
}

fn acceptance_expect_assertion_untagged(
    expect: &Value,
    dev_report: &Value,
    failures: &mut Vec<String>,
) {
    let Some(expected_untagged) = expect.get("assertion_untagged").and_then(Value::as_object)
    else {
        return;
    };
    let actual_untagged = dev_report.pointer("/executable_spec/untagged");
    for key in ["total", "passed", "failed", "error"] {
        if let Some(expected) = expected_untagged.get(key).and_then(Value::as_u64) {
            let pointer = format!("/summary/{key}");
            let actual = actual_untagged
                .and_then(|untagged| untagged.pointer(&pointer))
                .and_then(Value::as_u64);
            if actual != Some(expected) {
                failures.push(format!(
                    "expected assertion_untagged.{key}={expected}, got {}",
                    actual
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "missing".to_owned())
                ));
            }
        }
    }
}

fn acceptance_expect_source_metadata(
    expect: &Value,
    dev_report: &Value,
    failures: &mut Vec<String>,
) {
    let Some(expected_targets) = expect
        .pointer("/source_metadata/targets")
        .and_then(Value::as_array)
    else {
        return;
    };
    let actual_targets = dev_report
        .pointer("/source_metadata/targets")
        .and_then(Value::as_object);
    for (index, expected_target) in expected_targets.iter().enumerate() {
        let Some(target_kind) = expected_target.get("target_kind").and_then(Value::as_str) else {
            failures.push(format!(
                "expect.source_metadata.targets[{index}].target_kind must be a string"
            ));
            continue;
        };
        let Some(target) = expected_target.get("target").and_then(Value::as_str) else {
            failures.push(format!(
                "expect.source_metadata.targets[{index}].target must be a string"
            ));
            continue;
        };
        let target_key = source_metadata_target_key(target_kind, target);
        let Some(actual_target) = actual_targets.and_then(|targets| targets.get(&target_key))
        else {
            failures.push(format!(
                "expected source_metadata.targets[{index}] {target_key:?}, got no matching target"
            ));
            continue;
        };
        if let Some(expected_description) =
            expected_target.get("description").and_then(Value::as_str)
        {
            let actual_description = actual_target.get("description").and_then(Value::as_str);
            if actual_description != Some(expected_description) {
                failures.push(format!(
                    "expected source_metadata.targets[{index}] {target_key:?} description={expected_description:?}, got {}",
                    actual_description
                        .map(|value| format!("{value:?}"))
                        .unwrap_or_else(|| "missing".to_owned())
                ));
            }
        }
        if let Some(expected_tags) = expected_target.get("tags").and_then(Value::as_array) {
            let actual_tags = actual_target
                .get("tags")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for expected_tag in expected_tags {
                let Some(expected_tag) = expected_tag.as_str() else {
                    failures.push(format!(
                        "expect.source_metadata.targets[{index}].tags entries must be strings"
                    ));
                    continue;
                };
                if !actual_tags
                    .iter()
                    .any(|actual_tag| actual_tag.as_str() == Some(expected_tag))
                {
                    failures.push(format!(
                        "expected source_metadata.targets[{index}] {target_key:?} tag={expected_tag:?}, got no matching tag"
                    ));
                }
            }
        }
    }
}

fn acceptance_expect_summary(
    expect: &Value,
    facts: &[FactView],
    effects: &[EffectView],
    failures: &mut Vec<String>,
) {
    let Some(summary) = expect.get("summary").and_then(Value::as_object) else {
        return;
    };
    if let Some(expected) = summary.get("facts").and_then(Value::as_u64) {
        let actual = facts.len() as u64;
        if actual != expected {
            failures.push(format!("expected summary.facts={expected}, got {actual}"));
        }
    }
    if let Some(expected) = summary.get("effects").and_then(Value::as_u64) {
        let actual = effects.len() as u64;
        if actual != expected {
            failures.push(format!("expected summary.effects={expected}, got {actual}"));
        }
    }
}

fn acceptance_observed_json(run: &AcceptanceDevRun) -> Value {
    let fact_total = run.facts.len();
    let effect_total = run.effects.len();
    let mut fact_counts = BTreeMap::new();
    for fact in &run.facts {
        *fact_counts.entry(fact.name.clone()).or_insert(0_u64) += 1;
    }
    let facts = fact_counts
        .into_iter()
        .map(|(name, count)| json!({"name": name, "count": count}))
        .collect::<Vec<_>>();

    let mut effect_counts = BTreeMap::new();
    for effect in &run.effects {
        *effect_counts
            .entry((effect.kind.clone(), effect.status.clone()))
            .or_insert(0_u64) += 1;
    }
    let effects = effect_counts
        .into_iter()
        .map(|((kind, status), count)| json!({"kind": kind, "status": status, "count": count}))
        .collect::<Vec<_>>();

    json!({
        "summary": {
            "facts": fact_total,
            "effects": effect_total,
        },
        "facts": facts,
        "effects": effects,
        "actions": acceptance_actions_json(&run.actions),
        "source_metadata": acceptance_observed_source_metadata(&run.dev_report),
        "runs": provider_runs_summary_json(&run.runs, &run.artifact_counts),
        "artifacts": provider_artifacts_summary_json(&run.artifacts),
        "evidence": provider_evidence_summary_json(&run.evidence),
        "inbox": acceptance_inbox_summary_json(&run.inbox_items),
        "trace": acceptance_observed_trace_json(&run.events),
        "diagnostics_by_code": acceptance_observed_diagnostics_by_code(&run.dev_report),
        "executable_spec": acceptance_observed_executable_spec(&run.dev_report),
        "assertion_reads": acceptance_observed_assertion_reads(&run.dev_report, &run.events, &run.evidence),
    })
}

fn acceptance_observed_trace_json(events: &[EventView]) -> Value {
    let abstract_records = reconstruct_trace_records(events);
    let conformance = check_trace(&abstract_records);
    let mut groups = BTreeMap::<String, u64>::new();
    for record in &abstract_records {
        let event = trace_event_to_json(&record.event);
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        *groups.entry(event_type).or_insert(0) += 1;
    }
    let groups = groups
        .into_iter()
        .map(|(event_type, count)| json!({"type": event_type, "count": count}))
        .collect::<Vec<_>>();
    let items = abstract_records
        .iter()
        .map(trace_record_to_json)
        .collect::<Vec<_>>();
    let conformance = match conformance {
        Ok(()) => json!({"ok": true}),
        Err(violation) => json!({
            "ok": false,
            "sequence": violation.sequence,
            "message": violation.message,
        }),
    };
    json!({
        "summary": {
            "events": events.len(),
            "abstract_events": abstract_records.len(),
        },
        "groups": groups,
        "items": items,
        "conformance": conformance,
    })
}

fn acceptance_inbox_summary_json(inbox_items: &[InboxItemView]) -> Value {
    let mut groups = BTreeMap::<(String, String), u64>::new();
    for item in inbox_items {
        *groups
            .entry((item.status.clone(), item.severity.clone()))
            .or_insert(0) += 1;
    }
    let groups = groups
        .into_iter()
        .map(|((status, severity), count)| {
            json!({
                "status": status,
                "severity": severity,
                "count": count,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "summary": {
            "total": inbox_items.len(),
        },
        "groups": groups,
    })
}

fn acceptance_actions_json(action_reports: &[AcceptanceActionReport]) -> Value {
    Value::Array(
        action_reports
            .iter()
            .map(|report| {
                json!({
                    "type": report.action_type,
                    "event_id": report.event_id,
                    "sequence": report.sequence,
                })
            })
            .collect(),
    )
}

fn acceptance_observed_source_metadata(dev_report: &Value) -> Value {
    let targets = dev_report
        .pointer("/source_metadata/targets")
        .and_then(Value::as_object)
        .map(|targets| {
            targets
                .iter()
                .map(|(key, target)| {
                    json!({
                        "key": key,
                        "target_kind": target.get("target_kind").cloned().unwrap_or(Value::Null),
                        "target": target.get("target").cloned().unwrap_or(Value::Null),
                        "tags": target.get("tags").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
                        "description": target.get("description").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "summary": {
            "targets": targets.len(),
        },
        "targets": targets,
    })
}

fn acceptance_observed_diagnostics_by_code(dev_report: &Value) -> Value {
    let mut counts = BTreeMap::new();
    for diagnostic in dev_report
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(code) = diagnostic.get("code").and_then(Value::as_str) {
            *counts.entry(code.to_owned()).or_insert(0_u64) += 1;
        }
    }
    Value::Array(
        counts
            .into_iter()
            .map(|(code, count)| json!({"code": code, "count": count}))
            .collect(),
    )
}

fn acceptance_observed_assertion_reads(
    dev_report: &Value,
    events: &[EventView],
    evidence: &[EvidenceView],
) -> Value {
    let trace_records = reconstruct_trace_records(events);
    let reads = dev_report
        .get("assertions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|assertion| {
            assertion
                .get("reads")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        })
        .map(|read| acceptance_observed_assertion_read(read, &trace_records, evidence))
        .collect::<Vec<_>>();
    Value::Array(reads)
}

fn acceptance_observed_assertion_read(
    read: &Value,
    trace_records: &[TraceRecord],
    evidence: &[EvidenceView],
) -> Value {
    let mut groups = BTreeMap::<
        (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
        (u64, BTreeSet<u64>, BTreeSet<String>),
    >::new();
    for match_report in read
        .get("matches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = (
            match_report
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            match_report
                .get("status")
                .and_then(Value::as_str)
                .map(str::to_owned),
            match_report
                .get("prompt_content_type")
                .and_then(Value::as_str)
                .map(str::to_owned),
            match_report
                .get("provenance_class")
                .and_then(Value::as_str)
                .map(str::to_owned),
        );
        let entry = groups
            .entry(key)
            .or_insert_with(|| (0, BTreeSet::new(), BTreeSet::new()));
        entry.0 += 1;
        if read.get("kind").and_then(Value::as_str) == Some("effect") {
            if let Some(effect_id) = match_report.get("id").and_then(Value::as_str) {
                entry
                    .1
                    .extend(trace_sequences_for_effect(trace_records, effect_id));
                entry.2.extend(evidence_ids_for_effect(evidence, effect_id));
            }
        }
    }
    let match_groups = groups
        .into_iter()
        .map(
            |(
                (name, status, prompt_content_type, provenance_class),
                (count, trace_sequences, evidence_ids),
            )| {
                let trace_sequences = trace_sequences.into_iter().collect::<Vec<_>>();
                let evidence_ids = evidence_ids.into_iter().collect::<Vec<_>>();
                let mut group = json!({
                    "name": name,
                    "status": status,
                    "prompt_content_type": prompt_content_type,
                    "provenance_class": provenance_class,
                    "count": count,
                });
                if !trace_sequences.is_empty() || !evidence_ids.is_empty() {
                    insert_json_field(&mut group, "trace_items", json!(trace_sequences.len()));
                    insert_json_field(&mut group, "evidence_items", json!(evidence_ids.len()));
                    insert_json_field(&mut group, "trace_sequences", json!(trace_sequences));
                    insert_json_field(&mut group, "evidence_ids", json!(evidence_ids));
                }
                group
            },
        )
        .collect::<Vec<_>>();
    let mut observed = json!({
        "kind": read.get("kind").cloned().unwrap_or(Value::Null),
        "head": read.get("head").cloned().unwrap_or(Value::Null),
        "source": read.get("source").cloned().unwrap_or(Value::Null),
        "match_count": read.get("match_count").cloned().unwrap_or(Value::Null),
        "matches": match_groups,
    });
    if let Some(guard) = read.get("guard").and_then(Value::as_str) {
        insert_json_field(&mut observed, "guard", Value::String(guard.to_owned()));
    }
    observed
}

fn trace_sequences_for_effect(trace_records: &[TraceRecord], effect_id: &str) -> Vec<u64> {
    trace_records
        .iter()
        .filter(|record| trace_event_mentions_effect(&record.event, effect_id))
        .map(|record| record.sequence)
        .collect()
}

fn trace_event_mentions_effect(event: &TraceEvent, effect_id: &str) -> bool {
    match event {
        TraceEvent::EffectCreated {
            effect_id: candidate,
            ..
        }
        | TraceEvent::EffectClaimed {
            effect_id: candidate,
        }
        | TraceEvent::RunStarted {
            effect_id: candidate,
            ..
        }
        | TraceEvent::LeaseExpired {
            effect_id: candidate,
            ..
        }
        | TraceEvent::EffectTerminal {
            effect_id: candidate,
            ..
        }
        | TraceEvent::ProviderDiagnostic {
            effect_id: candidate,
            ..
        }
        | TraceEvent::EffectBlocked {
            effect_id: candidate,
            ..
        }
        | TraceEvent::EffectCancelled {
            effect_id: candidate,
        }
        | TraceEvent::EffectCancellationRequested {
            effect_id: candidate,
            ..
        } => candidate == effect_id,
        TraceEvent::DependencyCreated(edge) => {
            edge.upstream_effect_id == effect_id || edge.downstream_effect_id == effect_id
        }
        TraceEvent::RevisionActivated {
            terminal_cancel_effects,
            request_cancel_effects,
            ..
        } => {
            terminal_cancel_effects
                .iter()
                .any(|candidate| candidate == effect_id)
                || request_cancel_effects
                    .iter()
                    .any(|candidate| candidate == effect_id)
        }
        TraceEvent::InstancePaused
        | TraceEvent::InstanceResumed
        | TraceEvent::InstanceCancelled => false,
    }
}

fn evidence_ids_for_effect(evidence: &[EvidenceView], effect_id: &str) -> Vec<String> {
    evidence
        .iter()
        .filter(|item| {
            item.subject_id == effect_id
                || item.causation_id.as_deref() == Some(effect_id)
                || item.correlation_id.as_deref() == Some(effect_id)
        })
        .map(|item| item.evidence_id.clone())
        .collect()
}

fn provider_runs_summary_json(
    runs: &[RunView],
    artifact_counts: &BTreeMap<String, usize>,
) -> Value {
    let total_artifact_count = runs
        .iter()
        .map(|run| artifact_counts.get(&run.run_id).copied().unwrap_or(0) as u64)
        .sum::<u64>();
    let mut groups = BTreeMap::<(String, String), (u64, u64)>::new();
    for run in runs {
        let artifact_count = artifact_counts.get(&run.run_id).copied().unwrap_or(0) as u64;
        let entry = groups
            .entry((run.provider.clone(), run.status.clone()))
            .or_insert((0, 0));
        entry.0 += 1;
        entry.1 += artifact_count;
    }
    let groups = groups
        .into_iter()
        .map(|((provider, status), (count, artifact_count))| {
            json!({
                "provider": provider,
                "status": status,
                "count": count,
                "artifact_count": artifact_count,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "summary": {
            "total": runs.len(),
            "artifact_count": total_artifact_count,
        },
        "groups": groups,
    })
}

fn provider_artifacts_summary_json(artifacts: &[ArtifactView]) -> Value {
    let mut groups = BTreeMap::<(String, Option<String>), u64>::new();
    for artifact in artifacts {
        *groups
            .entry((artifact.kind.clone(), artifact.mime_type.clone()))
            .or_insert(0) += 1;
    }
    let groups = groups
        .into_iter()
        .map(|((kind, mime_type), count)| {
            json!({
                "kind": kind,
                "mime_type": mime_type,
                "count": count,
            })
        })
        .collect::<Vec<_>>();
    let items = artifacts
        .iter()
        .map(|artifact| {
            json!({
                "artifact_id": artifact.artifact_id,
                "run_id": artifact.run_id,
                "kind": artifact.kind,
                "mime_type": artifact.mime_type,
                "content_hash": artifact.content_hash,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "summary": {
            "total": artifacts.len(),
        },
        "groups": groups,
        "items": items,
    })
}

fn provider_evidence_summary_json(evidence: &[EvidenceView]) -> Value {
    let mut groups = BTreeMap::<(String, String), u64>::new();
    for item in evidence {
        *groups
            .entry((item.kind.clone(), item.subject_type.clone()))
            .or_insert(0) += 1;
    }
    let groups = groups
        .into_iter()
        .map(|((kind, subject_type), count)| {
            json!({
                "kind": kind,
                "subject_type": subject_type,
                "count": count,
            })
        })
        .collect::<Vec<_>>();
    let items = evidence
        .iter()
        .map(|item| {
            json!({
                "evidence_id": item.evidence_id,
                "kind": item.kind,
                "subject_type": item.subject_type,
                "subject_id": item.subject_id,
                "causation_id": item.causation_id,
                "correlation_id": item.correlation_id,
                "summary": item.summary,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "summary": {
            "total": evidence.len(),
        },
        "groups": groups,
        "items": items,
    })
}

fn acceptance_observed_executable_spec(dev_report: &Value) -> Value {
    let executable_spec = dev_report.get("executable_spec").and_then(Value::as_object);
    let tags = executable_spec
        .and_then(|spec| spec.get("tags"))
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .map(|tag| {
                    json!({
                        "tag": tag.get("tag").cloned().unwrap_or(Value::Null),
                        "status": tag.get("status").cloned().unwrap_or(Value::Null),
                        "summary": tag.get("summary").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut observed = json!({
        "status": executable_spec
            .and_then(|spec| spec.get("status"))
            .cloned()
            .unwrap_or(Value::Null),
        "summary": executable_spec
            .and_then(|spec| spec.get("summary"))
            .cloned()
            .unwrap_or(Value::Null),
        "tags": tags,
    });
    if let Some(untagged) = executable_spec
        .and_then(|spec| spec.get("untagged"))
        .and_then(Value::as_object)
    {
        insert_json_field(
            &mut observed,
            "untagged",
            json!({
                "status": untagged.get("status").cloned().unwrap_or(Value::Null),
                "summary": untagged.get("summary").cloned().unwrap_or(Value::Null),
            }),
        );
    }
    observed
}

fn acceptance_expect_facts(expect: &Value, facts: &[FactView], failures: &mut Vec<String>) {
    let Some(expected_facts) = expect.get("facts").and_then(Value::as_array) else {
        return;
    };
    for (index, expected_fact) in expected_facts.iter().enumerate() {
        let Some(name) = expected_fact.get("name").and_then(Value::as_str) else {
            failures.push(format!("expect.facts[{index}].name must be a string"));
            continue;
        };
        let Some(expected_count) = expected_fact.get("count").and_then(Value::as_u64) else {
            failures.push(format!("expect.facts[{index}].count must be an integer"));
            continue;
        };
        let actual_count = facts.iter().filter(|fact| fact.name == name).count() as u64;
        if actual_count != expected_count {
            failures.push(format!(
                "expected facts[{index}] name={name:?} count={expected_count}, got {actual_count}"
            ));
        }
    }
}

fn acceptance_expect_effects(expect: &Value, effects: &[EffectView], failures: &mut Vec<String>) {
    let Some(expected_effects) = expect.get("effects").and_then(Value::as_array) else {
        return;
    };
    for (index, expected_effect) in expected_effects.iter().enumerate() {
        let Some(kind) = expected_effect.get("kind").and_then(Value::as_str) else {
            failures.push(format!("expect.effects[{index}].kind must be a string"));
            continue;
        };
        let status = expected_effect.get("status").and_then(Value::as_str);
        let Some(expected_count) = expected_effect.get("count").and_then(Value::as_u64) else {
            failures.push(format!("expect.effects[{index}].count must be an integer"));
            continue;
        };
        let actual_count = effects
            .iter()
            .filter(|effect| {
                effect.kind == kind
                    && status
                        .map(|expected_status| effect.status == expected_status)
                        .unwrap_or(true)
            })
            .count() as u64;
        if actual_count != expected_count {
            let status_clause = status
                .map(|status| format!(" status={status:?}"))
                .unwrap_or_default();
            failures.push(format!(
                "expected effects[{index}] kind={kind:?}{status_clause} count={expected_count}, got {actual_count}"
            ));
        }
    }
}

fn acceptance_expect_runs(
    expect: &Value,
    runs: &[RunView],
    artifact_counts: &BTreeMap<String, usize>,
    failures: &mut Vec<String>,
) {
    let Some(expected_runs) = expect.get("runs").and_then(Value::as_array) else {
        return;
    };
    for (index, expected_run) in expected_runs.iter().enumerate() {
        let Some(provider) = expected_run.get("provider").and_then(Value::as_str) else {
            failures.push(format!("expect.runs[{index}].provider must be a string"));
            continue;
        };
        let status = expected_run.get("status").and_then(Value::as_str);
        let Some(expected_count) = expected_run.get("count").and_then(Value::as_u64) else {
            failures.push(format!("expect.runs[{index}].count must be an integer"));
            continue;
        };
        let matching_runs = runs
            .iter()
            .filter(|run| {
                run.provider == provider
                    && status
                        .map(|expected_status| run.status == expected_status)
                        .unwrap_or(true)
            })
            .collect::<Vec<_>>();
        let actual_count = matching_runs.len() as u64;
        if actual_count != expected_count {
            let status_clause = status
                .map(|status| format!(" status={status:?}"))
                .unwrap_or_default();
            failures.push(format!(
                "expected runs[{index}] provider={provider:?}{status_clause} count={expected_count}, got {actual_count}"
            ));
        }
        if let Some(expected_artifact_count) =
            expected_run.get("artifact_count").and_then(Value::as_u64)
        {
            let actual_artifact_count = matching_runs
                .iter()
                .map(|run| artifact_counts.get(&run.run_id).copied().unwrap_or(0) as u64)
                .sum::<u64>();
            if actual_artifact_count != expected_artifact_count {
                let status_clause = status
                    .map(|status| format!(" status={status:?}"))
                    .unwrap_or_default();
                failures.push(format!(
                    "expected runs[{index}] provider={provider:?}{status_clause} artifact_count={expected_artifact_count}, got {actual_artifact_count}"
                ));
            }
        }
    }
}

fn acceptance_expect_artifacts(
    expect: &Value,
    artifacts: &[ArtifactView],
    failures: &mut Vec<String>,
) {
    let Some(expected_artifacts) = expect.get("artifacts").and_then(Value::as_array) else {
        return;
    };
    for (index, expected_artifact) in expected_artifacts.iter().enumerate() {
        let Some(kind) = expected_artifact.get("kind").and_then(Value::as_str) else {
            failures.push(format!("expect.artifacts[{index}].kind must be a string"));
            continue;
        };
        let mime_type = expected_artifact.get("mime_type").and_then(Value::as_str);
        let Some(expected_count) = expected_artifact.get("count").and_then(Value::as_u64) else {
            failures.push(format!(
                "expect.artifacts[{index}].count must be an integer"
            ));
            continue;
        };
        let actual_count = artifacts
            .iter()
            .filter(|artifact| {
                artifact.kind == kind
                    && mime_type
                        .map(|expected_mime| artifact.mime_type.as_deref() == Some(expected_mime))
                        .unwrap_or(true)
            })
            .count() as u64;
        if actual_count != expected_count {
            let mime_clause = mime_type
                .map(|mime_type| format!(" mime_type={mime_type:?}"))
                .unwrap_or_default();
            failures.push(format!(
                "expected artifacts[{index}] kind={kind:?}{mime_clause} count={expected_count}, got {actual_count}"
            ));
        }
    }
}

fn acceptance_expect_evidence(
    expect: &Value,
    evidence: &[EvidenceView],
    failures: &mut Vec<String>,
) {
    let Some(expected_evidence) = expect.get("evidence").and_then(Value::as_array) else {
        return;
    };
    for (index, expected_item) in expected_evidence.iter().enumerate() {
        let Some(kind) = expected_item.get("kind").and_then(Value::as_str) else {
            failures.push(format!("expect.evidence[{index}].kind must be a string"));
            continue;
        };
        let subject_type = expected_item.get("subject_type").and_then(Value::as_str);
        let Some(expected_count) = expected_item.get("count").and_then(Value::as_u64) else {
            failures.push(format!("expect.evidence[{index}].count must be an integer"));
            continue;
        };
        let actual_count = evidence
            .iter()
            .filter(|item| {
                item.kind == kind
                    && subject_type
                        .map(|expected_subject_type| item.subject_type == expected_subject_type)
                        .unwrap_or(true)
            })
            .count() as u64;
        if actual_count != expected_count {
            let subject_clause = subject_type
                .map(|subject_type| format!(" subject_type={subject_type:?}"))
                .unwrap_or_default();
            failures.push(format!(
                "expected evidence[{index}] kind={kind:?}{subject_clause} count={expected_count}, got {actual_count}"
            ));
        }
    }
}

fn acceptance_expect_str(
    report: &Value,
    pointer: &str,
    expected: &str,
    failures: &mut Vec<String>,
) {
    let actual = report.pointer(pointer).and_then(Value::as_str);
    if actual != Some(expected) {
        failures.push(format!(
            "expected {pointer}={expected:?}, got {}",
            actual
                .map(|value| format!("{value:?}"))
                .unwrap_or_else(|| "missing".to_owned())
        ));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DevStreamFormat {
    Ndjson,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DevStream {
    format: Option<DevStreamFormat>,
    sequence: usize,
}

impl DevStream {
    fn new(format: Option<DevStreamFormat>) -> Self {
        Self {
            format,
            sequence: 0,
        }
    }

    fn enabled(&self) -> bool {
        self.format.is_some()
    }

    fn emit(&mut self, event: &str, data: Value) -> io::Result<()> {
        let Some(DevStreamFormat::Ndjson) = self.format else {
            return Ok(());
        };
        let envelope = json!({
            "schema": "whipplescript.dev_stream.v0",
            "sequence": self.sequence,
            "event": event,
            "data": data,
        });
        self.sequence += 1;
        let rendered = serde_json::to_string(&envelope)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{rendered}")?;
        stdout.flush()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DevOptions {
    program_path: String,
    root: Option<String>,
    provider: String,
    exec_profile: ExecProfile,
    script_manifest_path: Option<PathBuf>,
    provider_config_paths: Vec<PathBuf>,
    outcome: FixtureOutcome,
    variant: Option<String>,
    max_iterations: usize,
    assertion_filter: AssertionTagFilter,
    stream: Option<DevStreamFormat>,
}

impl DevOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut program_path = None;
        let mut root = None;
        let mut provider = "fixture".to_owned();
        let mut exec_profile = ExecProfile::from_env();
        let mut script_manifest_path = script_manifest_path_from_env();
        let mut provider_config_paths = Vec::new();
        let mut outcome = FixtureOutcome::Completed;
        let mut variant = None;
        let mut max_iterations = 8usize;
        let mut assertion_filter = AssertionTagFilter::default();
        let mut stream = None;
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
                "--provider-config" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected path after `--provider-config`".to_owned());
                    };
                    provider_config_paths.push(PathBuf::from(value));
                }
                "--exec-profile" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected profile after `--exec-profile`".to_owned());
                    };
                    exec_profile = ExecProfile::parse(value)?;
                }
                "--script-manifest" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected path after `--script-manifest`".to_owned());
                    };
                    script_manifest_path = Some(PathBuf::from(value));
                }
                "--include-tag" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected tag after `--include-tag`".to_owned());
                    };
                    assertion_filter.include_tags.push(value.clone());
                }
                "--exclude-tag" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected tag after `--exclude-tag`".to_owned());
                    };
                    assertion_filter.exclude_tags.push(value.clone());
                }
                "--stream" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected value after `--stream`".to_owned());
                    };
                    stream = Some(match value.as_str() {
                        "ndjson" => DevStreamFormat::Ndjson,
                        _ => return Err("only `--stream ndjson` is supported".to_owned()),
                    });
                }
                "--variant" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err("expected variant name after `--variant`".to_owned());
                    };
                    variant = Some(value.clone());
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
                        "usage: whip dev <workflow.whip> [--provider fixture] [--provider-config path] [--until idle] [--include-tag tag] [--exclude-tag tag] [--stream ndjson] [--fail|--timeout|--cancel]"
                            .to_owned(),
                    )
                }
            }
            index += 1;
        }
        let Some(program_path) = program_path else {
            return Err(
                "usage: whip dev <workflow.whip> [--provider fixture] [--provider-config path] [--until idle] [--include-tag tag] [--exclude-tag tag] [--stream ndjson] [--fail|--timeout|--cancel]"
                    .to_owned(),
            );
        };
        Ok(Self {
            program_path,
            root,
            provider,
            exec_profile,
            script_manifest_path,
            provider_config_paths,
            outcome,
            variant,
            max_iterations,
            assertion_filter,
            stream,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct AssertionTagFilter {
    include_tags: Vec<String>,
    exclude_tags: Vec<String>,
}

impl AssertionTagFilter {
    fn matches(&self, tags: &[String]) -> bool {
        if !self.exclude_tags.is_empty()
            && tags
                .iter()
                .any(|tag| self.exclude_tags.iter().any(|excluded| excluded == tag))
        {
            return false;
        }
        self.include_tags.is_empty()
            || tags
                .iter()
                .any(|tag| self.include_tags.iter().any(|included| included == tag))
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
    errors: Vec<String>,
    /// Effect ids targeted by `cancel <binding>` operations in live scopes.
    cancels: Vec<String>,
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
    source_span_json: Option<String>,
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
            source_span_json: self.source_span_json.as_deref(),
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
    timeout_seconds: Option<i64>,
}

impl OwnedEffect {
    fn as_new_effect(&self) -> NewEffect<'_> {
        NewEffect {
            timeout_seconds: self.timeout_seconds,
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
                .filter(|fact| pattern_agent_matches(ir, schema, fact))
                .filter(|fact| pattern_queue_matches(schema, fact))
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
        // Special readiness patterns without an `as` binding (for example
        // `ralph completed turn`) require a matching fact but bind nothing.
        let normalized = normalize_pattern_name(pattern);
        if normalized != pattern {
            let satisfied = facts.iter().any(|fact| {
                fact.name == normalized
                    && pattern_agent_matches(ir, pattern, fact)
                    && pattern_queue_matches(pattern, fact)
            });
            if !satisfied {
                return ReadyContexts::empty(guard_reports);
            }
            continue;
        }
        return ReadyContexts::empty(guard_reports);
    }
    ReadyContexts {
        contexts,
        guard_reports,
    }
}

/// For `<queue> has ready item` patterns, only the named queue's projected
/// items match.
fn pattern_queue_matches(pattern: &str, fact: &FactView) -> bool {
    let mut words = pattern.split_whitespace();
    let Some(queue) = words.next() else {
        return true;
    };
    if !(words.next() == Some("has")
        && words.next() == Some("ready")
        && words.next() == Some("item"))
    {
        return true;
    }
    json_from_str(&fact.value_json)
        .get("queue")
        .and_then(Value::as_str)
        .is_none_or(|fact_queue| fact_queue == queue)
}

/// For `<agent> completed turn` patterns where the leading word names a
/// declared agent, only that agent's turns match. The generic `worker` form
/// (or any word that is not a declared agent) matches turns from any agent.
fn pattern_agent_matches(ir: &IrProgram, pattern: &str, fact: &FactView) -> bool {
    let Some(agent) = completed_turn_agent(pattern) else {
        return true;
    };
    if !ir.agents.iter().any(|declared| declared.name == agent) {
        return true;
    }
    serde_json::from_str::<Value>(&fact.value_json)
        .ok()
        .and_then(|value| {
            value
                .get("agent")
                .and_then(|v| v.as_str().map(str::to_owned))
        })
        .is_none_or(|fact_agent| fact_agent == agent)
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

#[allow(clippy::too_many_arguments)]
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
    filter: &AssertionTagFilter,
) -> Vec<AssertionReport> {
    ir.assertions
        .iter()
        .filter_map(|assertion| {
            let target_id = stable_hash_hex(assertion.expr.source.as_str());
            let (tags, description) = source_metadata_for_target(ir, "assertion", &target_id);
            if !filter.matches(&tags) {
                return None;
            }
            let reads = assertion
                .projection_reads
                .iter()
                .map(|read| assertion_read_report(read, facts, effects, ir))
                .collect::<Vec<_>>();
            Some(eval_assertion(AssertionEvalInput {
                target_id: &target_id,
                source: assertion.expr.source.as_str(),
                expr: &assertion.expr.expr,
                reads,
                tags,
                description,
                ir,
                span: source_path.map(|_| assertion.expr.span),
                source_path,
                facts,
                effects,
            }))
        })
        .collect()
}

fn assertion_read_report(
    read: &IrProjectionRead,
    facts: &[FactView],
    effects: &[EffectView],
    ir: &IrProgram,
) -> AssertionReadReport {
    let kind = match read.kind {
        QueryKind::Fact => "fact",
        QueryKind::Effect => "effect",
    }
    .to_owned();
    let source = match &read.guard {
        Some(guard) => format!("{kind}:{} where {guard}", read.head),
        None => format!("{kind}:{}", read.head),
    };
    let (matches, error) = assertion_read_matches(read, facts, effects, ir);
    AssertionReadReport {
        kind,
        head: read.head.clone(),
        guard: read.guard.clone(),
        source,
        match_count: matches.len(),
        matches,
        error,
    }
}

fn assertion_read_matches(
    read: &IrProjectionRead,
    facts: &[FactView],
    effects: &[EffectView],
    ir: &IrProgram,
) -> (Vec<AssertionReadMatch>, Option<String>) {
    let guard = match &read.guard {
        Some(guard) => match parse_expression(guard) {
            Ok(expr) => Some(expr),
            Err(error) => return (Vec::new(), Some(error)),
        },
        None => None,
    };
    let scope = EvalScope::assertions(facts, effects, ir);
    match read.kind {
        QueryKind::Fact => {
            let mut matches = Vec::new();
            for fact in facts.iter().filter(|fact| fact.name == read.head.trim()) {
                let value = json_from_str(&fact.value_json);
                if let Some(guard) = &guard {
                    match guard_filter_matches(eval_expr_value(
                        guard,
                        &scope.projection(&value, Some(read.head.trim())),
                    )) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(error) => return (matches, Some(error.into_json().to_string())),
                    }
                }
                matches.push(AssertionReadMatch {
                    id: fact.fact_id.clone(),
                    name: fact.name.clone(),
                    key: Some(fact.key.clone()),
                    status: None,
                    prompt_content_type: None,
                    provenance_class: Some(fact.provenance_class.clone()),
                    source_span_json: fact.source_span_json.clone(),
                });
            }
            (matches, None)
        }
        QueryKind::Effect => {
            let kind = read
                .head
                .trim()
                .strip_prefix("kind ")
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let mut matches = Vec::new();
            for effect in effects
                .iter()
                .filter(|effect| kind.is_none_or(|kind| effect.kind == kind))
            {
                let value = json!({
                    "kind": effect.kind,
                    "target": effect.target,
                    "status": effect.status,
                    "profile": effect.profile,
                });
                if let Some(guard) = &guard {
                    match guard_filter_matches(eval_expr_value(
                        guard,
                        &scope.projection(&value, None),
                    )) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(error) => return (matches, Some(error.into_json().to_string())),
                    }
                }
                matches.push(AssertionReadMatch {
                    id: effect.effect_id.clone(),
                    name: effect.kind.clone(),
                    key: None,
                    status: Some(effect.status.clone()),
                    prompt_content_type: effect_prompt_content_type(effect),
                    provenance_class: None,
                    source_span_json: None,
                });
            }
            (matches, None)
        }
    }
}

fn effect_prompt_content_type(effect: &EffectView) -> Option<String> {
    json_from_str(&effect.input_json)
        .get("prompt_content_type")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn assertion_filter_to_json(
    filter: &AssertionTagFilter,
    total_assertions: usize,
    selected_assertions: usize,
) -> Value {
    json!({
        "include_tags": filter.include_tags,
        "exclude_tags": filter.exclude_tags,
        "total": total_assertions,
        "selected": selected_assertions,
    })
}

fn source_metadata_for_target(
    ir: &IrProgram,
    target_kind: &str,
    target: &str,
) -> (Vec<String>, Option<String>) {
    let tags = ir
        .source_tags
        .iter()
        .filter(|tag| tag.target_kind == target_kind && tag.target == target)
        .map(|tag| tag.name.clone())
        .collect::<Vec<_>>();
    let description = ir
        .source_descriptions
        .iter()
        .find(|description| description.target_kind == target_kind && description.target == target)
        .map(|description| description.value.clone());
    (tags, description)
}

fn persist_assertion_events(
    store: &SqliteStore,
    instance_id: &str,
    version_id: &str,
    facts: &[FactView],
    effects: &[EffectView],
    assertions: &[AssertionReport],
) -> Result<BTreeMap<String, String>, StoreError> {
    let mut events = BTreeMap::new();
    let read_set = json!({
        "facts": facts
            .iter()
            .map(|fact| json!({
                "fact_id": fact.fact_id,
                "name": fact.name,
                "key": fact.key,
                "program_version_id": fact.program_version_id,
                "revision_epoch": fact.revision_epoch,
            }))
            .collect::<Vec<_>>(),
        "effects": effects
            .iter()
            .map(|effect| json!({
                "effect_id": effect.effect_id,
                "kind": effect.kind,
                "status": effect.status,
                "program_version_id": effect.program_version_id,
                "revision_epoch": effect.revision_epoch,
            }))
            .collect::<Vec<_>>(),
    });
    let read_set_json = read_set.to_string();
    for assertion in assertions {
        let assertion_id = idempotency_key(&[instance_id, "assertion", &assertion.expr]);
        let result = match assertion.status {
            AssertionStatus::Passed => "pass",
            AssertionStatus::Failed => "fail",
            AssertionStatus::Error => "error",
        };
        let event_type = match assertion.status {
            AssertionStatus::Passed => "assertion.passed",
            AssertionStatus::Failed => "assertion.failed",
            AssertionStatus::Error => "assertion.errored",
        };
        let idempotency = idempotency_key(&[
            instance_id,
            version_id,
            "assertion-event",
            &assertion.expr,
            result,
            &assertion.actual.to_string(),
            &read_set_json,
        ]);
        let payload = json!({
            "assertion_id": assertion_id,
            "assertion_text": assertion.expr,
            "result": result,
            "program_version_id": version_id,
            "rule_name": null,
            "source_span": assertion.source_span_json.as_deref().map(json_from_str),
            "read_set": read_set.clone(),
            "actual_json": assertion.actual,
            "expected_json": assertion.expected,
            "error_code": assertion.error.as_ref().map(|_| "assertion.eval_error"),
            "message": assertion.failure_reason,
            "diagnostic_ids": [],
            "evidence_ids": [],
            "correlation_id": assertion_id,
            "idempotency_key": idempotency,
        })
        .to_string();
        let event = store.append_event(NewEvent {
            instance_id,
            event_type,
            payload_json: &payload,
            source: "assertion",
            causation_id: None,
            correlation_id: Some(&assertion_id),
            idempotency_key: Some(&idempotency),
        })?;
        events.insert(assertion.target_id.clone(), event.event_id);
    }
    Ok(events)
}

fn persist_assertion_diagnostics(
    store: &SqliteStore,
    instance_id: &str,
    program_id: &str,
    version_id: &str,
    assertions: &mut [AssertionReport],
    assertion_events: &BTreeMap<String, String>,
) -> Result<(), StoreError> {
    for assertion in assertions.iter_mut().filter(|assertion| !assertion.passed) {
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
        let diagnostic_id = store.record_diagnostic(DiagnosticRecord {
            instance_id: Some(instance_id),
            program_id: Some(program_id),
            program_version_id: Some(version_id),
            severity: "error",
            code: Some(match assertion.status {
                AssertionStatus::Failed => "assertion.failed",
                AssertionStatus::Error => "assertion.errored",
                AssertionStatus::Passed => unreachable!("passed assertions filtered out"),
            }),
            message: &message,
            source_span_json: assertion.source_span_json.as_deref(),
            subject_type: Some("assertion"),
            subject_id: Some(&assertion.expr),
            event_id: assertion_events
                .get(&assertion.target_id)
                .map(String::as_str),
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
        assertion.diagnostic_ids.push(diagnostic_id);
    }
    Ok(())
}

struct AssertionEvalInput<'a> {
    target_id: &'a str,
    source: &'a str,
    expr: &'a Expr,
    reads: Vec<AssertionReadReport>,
    tags: Vec<String>,
    description: Option<String>,
    ir: &'a IrProgram,
    span: Option<SourceSpan>,
    source_path: Option<&'a Path>,
    facts: &'a [FactView],
    effects: &'a [EffectView],
}

fn eval_assertion(input: AssertionEvalInput<'_>) -> AssertionReport {
    let source = input.source.trim();
    let scope = EvalScope::assertions(input.facts, input.effects, input.ir);
    let (status, actual, error) = assertion_result(eval_expr_value(input.expr, &scope));
    let (expected, actual_values) = assertion_details(input.expr, &scope, &actual);
    let failure_reason = assertion_failure_reason(input.expr, &status, error.as_deref());
    let passed = status == AssertionStatus::Passed;
    AssertionReport {
        target_id: input.target_id.to_owned(),
        event_id: None,
        diagnostic_ids: Vec::new(),
        expr: source.to_owned(),
        reads: input.reads,
        tags: input.tags,
        description: input.description,
        source_span_json: input
            .span
            .map(|span| source_span_json(input.source_path, span, "assertion")),
        status,
        passed,
        actual,
        actual_values,
        expected,
        failure_reason,
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

fn assertion_details(expr: &Expr, scope: &EvalScope<'_>, result: &Value) -> (Value, Value) {
    if let Expr::Binary { op, left, right } = expr {
        if assertion_binary_op_is_predicate(*op) {
            return (
                json!({
                    "predicate": binary_op_label(*op),
                    "left": left.to_snapshot(),
                    "right": right.to_snapshot(),
                }),
                json!({
                    "left": eval_expr_value(left, scope).into_json(),
                    "right": eval_expr_value(right, scope).into_json(),
                    "result": result,
                }),
            );
        }
    }

    (
        json!({
            "predicate": "truthy",
            "expr": expr.to_snapshot(),
        }),
        json!({
            "value": result,
            "result": result,
        }),
    )
}

fn assertion_binary_op_is_predicate(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::In
            | BinaryOp::NotIn
    )
}

fn assertion_failure_reason(
    expr: &Expr,
    status: &AssertionStatus,
    error: Option<&str>,
) -> Option<String> {
    match status {
        AssertionStatus::Passed => None,
        AssertionStatus::Error => error.map(str::to_owned),
        AssertionStatus::Failed => {
            if let Expr::Binary { op, .. } = expr {
                if assertion_binary_op_is_predicate(*op) {
                    return Some(format!(
                        "predicate `{}` evaluated to false",
                        binary_op_label(*op)
                    ));
                }
            }
            Some("assertion expression evaluated to false".to_owned())
        }
    }
}

fn binary_op_label(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Or => "||",
        BinaryOp::And => "&&",
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::In => "in",
        BinaryOp::NotIn => "not in",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
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
        source_tags: Vec::new(),
        source_descriptions: Vec::new(),
        includes: Vec::new(),
        pattern_applications: Vec::new(),
        workflow_contracts: Vec::new(),
        uses: Vec::new(),
        harnesses: Vec::new(),
        queues: Vec::new(),
        events: Vec::new(),
        leases: Vec::new(),
        ledgers: Vec::new(),
        counters: Vec::new(),
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
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
            let left = eval_expr_value(left, scope);
            let right = eval_expr_value(right, scope);
            let (Some(lhs), Some(rhs)) = (number_value(&left), number_value(&right)) else {
                return EvalValue::error("arithmetic requires numeric operands");
            };
            let result = match op {
                BinaryOp::Add => lhs + rhs,
                BinaryOp::Sub => lhs - rhs,
                BinaryOp::Mul => lhs * rhs,
                _ => {
                    if rhs == 0.0 {
                        return EvalValue::error("division by zero");
                    }
                    lhs / rhs
                }
            };
            let integer_operands = matches!(&left, EvalValue::Json(Value::Number(n)) if n.is_i64())
                && matches!(&right, EvalValue::Json(Value::Number(n)) if n.is_i64());
            if integer_operands && result.fract() == 0.0 && op != BinaryOp::Div {
                EvalValue::Json(Value::Number((result as i64).into()))
            } else if integer_operands && op == BinaryOp::Div && result.fract() == 0.0 {
                EvalValue::Json(Value::Number((result as i64).into()))
            } else {
                serde_json::Number::from_f64(result)
                    .map(Value::Number)
                    .map(EvalValue::Json)
                    .unwrap_or_else(|| EvalValue::error("arithmetic produced a non-finite value"))
            }
        }
    }
}

fn eval_call(name: &str, args: &[Expr], scope: &EvalScope<'_>) -> EvalValue {
    match (name, args) {
        ("count", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Number(count.into())))
            .unwrap_or_else(|value| value),
        ("count", [expr]) => match eval_expr_value(expr, scope) {
            EvalValue::Json(Value::Array(items)) => {
                EvalValue::Json(Value::Number((items.len() as i64).into()))
            }
            EvalValue::Json(Value::Object(items)) => {
                EvalValue::Json(Value::Number((items.len() as i64).into()))
            }
            EvalValue::Json(Value::String(value)) => {
                EvalValue::Json(Value::Number((value.chars().count() as i64).into()))
            }
            EvalValue::Missing => EvalValue::error("missing value for count"),
            EvalValue::Error(message) => EvalValue::Error(message),
            _ => EvalValue::error("unsupported value for count"),
        },
        ("exists", [query @ Expr::Query { .. }]) => eval_query_count(query, scope)
            .map(|count| EvalValue::Json(Value::Bool(count > 0)))
            .unwrap_or_else(|value| value),
        ("exists", [expr]) => EvalValue::Json(Value::Bool(
            !eval_expr_value(expr, scope).is_missing_or_null(),
        )),
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
    // The projection (the fact currently examined by a query's inner
    // `where`) is the innermost scope: its fields shadow outer rule
    // bindings. A path whose root is not a projection field falls through
    // to the rule context, so `count(Item where owner == task.owner)`
    // resolves `owner` against each Item and `task` against the binding.
    if let Some(projection) = scope.projection {
        if path
            .first()
            .is_some_and(|first| projection.get(first).is_some())
        {
            let mut current = projection;
            for field in path {
                let Some(next) = current.get(field) else {
                    return EvalValue::Missing;
                };
                current = next;
            }
            return EvalValue::Json(current.clone());
        }
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
        return EvalValue::Missing;
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

fn lower_rule(
    instance_id: &str,
    ir: &IrProgram,
    rule: &IrRule,
    context: &RuleContext,
    facts: &[FactView],
    effects: &[EffectView],
    source_path: Option<&Path>,
) -> OwnedLowering {
    let (body, context, branch_reports) = selected_rule_body(&rule.body, context);
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

    for (record_index, block) in top_level_record_blocks(&pre_terminal_body)
        .into_iter()
        .enumerate()
    {
        let value = parse_record_fields(
            &block.body,
            &context,
            block.from_binding.as_deref(),
            &mut lowering.errors,
        );
        let value_json = Value::Object(value).to_string();
        let fact_key = record_fact_key(&block.schema, &value_json);
        let fact_id = idempotency_key(&[
            instance_id,
            &rule.name,
            &block.schema,
            &fact_key,
            &value_json,
        ]);
        if existing_fact_ids
            .iter()
            .any(|existing| *existing == fact_id)
        {
            continue;
        }
        let record_source = rule
            .metadata
            .record_sources
            .get(record_index)
            .filter(|source| source.schema == block.schema);
        lowering.facts.push(OwnedFact {
            fact_id,
            name: block.schema.clone(),
            key: fact_key,
            value_json,
            schema_id: Some(block.schema),
            provenance_class: record_source
                .map(|source| {
                    if source.construct == "table_row" {
                        "table"
                    } else {
                        "rule"
                    }
                })
                .unwrap_or("rule")
                .to_owned(),
            correlation_id: context.identity.clone(),
            source_span_json: record_source
                .map(|source| source_span_json(source_path, source.span, &source.construct)),
        });
    }

    let mut parsed_effects = parse_effect_statements(&pre_terminal_body, &context);
    rewrite_lease_releases(&mut parsed_effects, &rule.body);
    let parsed_effects = parsed_effects;
    let mut node_to_effect_id = std::collections::BTreeMap::new();
    let mut binding_to_effect_id = std::collections::BTreeMap::new();
    for (index, parsed) in parsed_effects.iter().enumerate() {
        let effect_node = effect_node_for_parsed(rule, parsed, index);
        let node_id = effect_node
            .map(|effect| effect.id.as_str())
            .unwrap_or(parsed.kind.as_str());
        let effect_id = idempotency_key(&[
            instance_id,
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
        if parsed
            .prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("{{"))
        {
            lowering.errors.push(format!(
                "unresolved interpolation in `{}` effect `{node_id}`",
                parsed.kind
            ));
        }
        let input_json = parsed_effect_input_json(
            ir,
            rule,
            parsed,
            &context,
            &binding_to_effect_id,
            &mut lowering.errors,
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
            correlation_id: context.identity.clone(),
            source_span_json: effect_node
                .map(|effect| source_span_json(source_path, effect.span, "effect")),
            timeout_seconds: parsed.timeout_seconds,
        });
    }

    for binding in cancel_statements(&pre_terminal_body) {
        if let Some(effect_id) = binding_to_effect_id.get(&binding) {
            lowering.cancels.push(effect_id.clone());
        } else {
            lowering
                .errors
                .push(format!("cancel of unknown effect binding `{binding}`"));
        }
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
            let value = parse_record_fields(
                &record.body,
                &after_context,
                record.from_binding.as_deref(),
                &mut lowering.errors,
            );
            let value_json = Value::Object(value).to_string();
            let fact_key = record_fact_key(&record.schema, &value_json);
            let fact_id = idempotency_key(&[
                instance_id,
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
                source_span_json: None,
            });
        }
        let mut selected_effects = parse_effect_statements(&selected_after_body, &after_context);
        rewrite_lease_releases(&mut selected_effects, &rule.body);
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
                instance_id,
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
        for binding in cancel_statements(&selected_after_body) {
            if let Some(effect_id) = selected_binding_to_effect_id.get(&binding) {
                lowering.cancels.push(effect_id.clone());
            } else {
                lowering
                    .errors
                    .push(format!("cancel of unknown effect binding `{binding}`"));
            }
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
            if parsed
                .prompt
                .as_deref()
                .is_some_and(|prompt| prompt.contains("{{"))
            {
                lowering.errors.push(format!(
                    "unresolved interpolation in `{}` effect `{node_id}`",
                    parsed.kind
                ));
            }
            let input_json = parsed_effect_input_json(
                ir,
                rule,
                parsed,
                &after_context,
                &selected_binding_to_effect_id,
                &mut lowering.errors,
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
                timeout_seconds: parsed.timeout_seconds,
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
            program_version_id: None,
            revision_epoch: 0,
            name: binding.to_owned(),
            key: effect_id.to_owned(),
            value_json: value.to_string(),
            provenance_class: "effect".to_owned(),
            source_span_json: None,
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

fn cancel_statements(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let binding = line
                .trim()
                .trim_end_matches(';')
                .strip_prefix("cancel ")?
                .trim();
            is_identifier(binding).then(|| binding.to_owned())
        })
        .collect()
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
                    program_version_id: None,
                    revision_epoch: 0,
                    name: binding.to_owned(),
                    key: binding.to_owned(),
                    value_json: payload.to_string(),
                    provenance_class: "case".to_owned(),
                    source_span_json: None,
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
                program_version_id: None,
                revision_epoch: 0,
                name: binding.to_owned(),
                key: binding.to_owned(),
                value_json: value.to_string(),
                provenance_class: "case".to_owned(),
                source_span_json: None,
            },
        ));
        return true;
    }
    // Sum-type value (spec/sum-types.md): an internally-tagged record
    // dispatching on the synthesized `variant` discriminant, compared
    // exactly — BAML already normalized the tag. `Variant as b` binds the
    // matched variant record.
    if let Some(variant) = value.get("variant").and_then(Value::as_str) {
        let mut parts = pattern.split_whitespace();
        if parts.next() != Some(variant) {
            return false;
        }
        let binding = match (parts.next(), parts.next(), parts.next()) {
            (None, _, _) => None,
            (Some("as"), Some(binding), None) => Some(binding),
            _ => return false,
        };
        if let Some(binding) = binding {
            context.bindings.push((
                binding.to_owned(),
                FactView {
                    fact_id: format!("case:{binding}"),
                    program_version_id: None,
                    revision_epoch: 0,
                    name: binding.to_owned(),
                    key: binding.to_owned(),
                    value_json: value.to_string(),
                    provenance_class: "case".to_owned(),
                    source_span_json: None,
                },
            ));
        }
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
                .or_else(|| value.pointer("/metadata/error").cloned())
                .or_else(|| value.pointer("/metadata/failure").cloned())
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
    let payload = Value::Object(parse_record_fields(
        &terminal.body,
        context,
        None,
        &mut lowering.errors,
    ));
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
    prompt_content_type: Option<String>,
    required_capabilities: Vec<String>,
    after: Option<AfterScope>,
    timeout_seconds: Option<i64>,
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
            "exec.command" if self.name.as_deref() == Some("capability") => self
                .target
                .as_ref()
                .map(|target| vec![format!("script.{target}")])
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        capabilities.extend(self.required_capabilities.iter().cloned());
        capabilities.sort();
        capabilities.dedup();
        serde_json::to_string(&capabilities).unwrap_or_else(|_| "[]".to_owned())
    }
}

/// Extracts a `timeout <duration>` clause from an effect statement line.
fn parse_timeout_clause_seconds(line: &str) -> Option<i64> {
    let mut words = line.split_whitespace().peekable();
    while let Some(word) = words.next() {
        if word == "timeout" {
            let value = words.peek()?;
            return whipplescript_parser::body::parse_short_duration_seconds(value)
                .map(|seconds| seconds as i64);
        }
    }
    None
}

/// Extracts a leading double-quoted string, honoring `\"` and `\\` escapes,
/// returning the unescaped content and the text after the closing quote.
fn extract_quoted_string(text: &str) -> Option<(String, &str)> {
    let rest = text.strip_prefix('"')?;
    let mut content = String::new();
    let mut chars = rest.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some((_, escaped @ ('"' | '\\'))) => content.push(escaped),
                Some((_, other)) => {
                    content.push('\\');
                    content.push(other);
                }
                None => return None,
            },
            '"' => return Some((content, &rest[index + 1..])),
            _ => content.push(ch),
        }
    }
    None
}

/// Lease/counter keys are entity identities serialized to a stable string.
fn coordination_key_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

/// `release <x>` is queue-or-lease by referent: a binding acquired in this
/// rule body is a lease release (spec/coordination.md); anything else stays
/// the queue verb.
fn rewrite_lease_releases(effects: &mut [ParsedEffect], rule_body: &str) {
    let acquires = rule_body
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .starts_with("acquire ")
                .then(|| binding_after_as(trimmed))
                .flatten()
        })
        .collect::<std::collections::BTreeSet<_>>();
    for effect in effects {
        if effect.kind == "queue.release"
            && effect
                .args
                .first()
                .is_some_and(|binding| acquires.contains(binding))
        {
            effect.kind = "lease.release".to_owned();
        }
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
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "workflow.invoke".to_owned(),
                target: Some(target),
                name: Some("invoke".to_owned()),
                binding: binding_after_as(&statement),
                args: vec![body],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("timer until ") {
            // Absolute deadline (spec/scheduled-time.md): the operand is a
            // time literal or a time-typed path resolved from context.
            let operand = rest
                .split(" as ")
                .next()
                .unwrap_or_default()
                .trim()
                .trim_matches('"')
                .to_owned();
            let deadline = parse_field_value(&operand, context)
                .as_str()
                .map(str::to_owned)
                .unwrap_or(operand);
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "timer.wait".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![deadline],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index += 1;
        } else if let Some(rest) = trimmed.strip_prefix("timer ") {
            let duration = rest.split_whitespace().next().unwrap_or_default();
            let duration_seconds =
                whipplescript_parser::body::parse_short_duration_seconds(duration)
                    .map(|seconds| seconds as i64);
            effects.push(ParsedEffect {
                timeout_seconds: duration_seconds,
                kind: "timer.wait".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![duration.to_owned()],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index += 1;
        } else if let Some(rest) = trimmed.strip_prefix("file ") {
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let queue = rest
                .strip_prefix("item into ")
                .and_then(|tail| tail.split_whitespace().next())
                .unwrap_or_default()
                .trim_end_matches('{')
                .to_owned();
            let body = invoke_body(&statement).unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "queue.file".to_owned(),
                target: Some(queue),
                name: None,
                binding: binding_after_as(&statement),
                args: vec![body],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if trimmed.starts_with("claim ") && !trimmed.contains(" with ") {
            let item = trimmed
                .strip_prefix("claim ")
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "queue.claim".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![item],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index += 1;
        } else if let Some(rest) = trimmed.strip_prefix("release ") {
            let item = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "queue.release".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![item],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index += 1;
        } else if let Some(rest) = trimmed.strip_prefix("finish ") {
            let (statement, next_index) = if trimmed.contains('{') {
                parse_statement_until_balanced_braces(&lines, index, trimmed)
            } else {
                (trimmed.to_owned(), index + 1)
            };
            let item = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let body = invoke_body(&statement).unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "queue.finish".to_owned(),
                target: None,
                name: None,
                binding: binding_after_as(&statement),
                args: vec![item, body],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("decide ") {
            // Inline anonymous coercion: decide "<prompt>" -> { fields } as x.
            let prompt = rest
                .strip_prefix('"')
                .and_then(|tail| tail.split_once('"'))
                .map(|(prompt, _)| prompt.to_owned())
                .unwrap_or_default();
            let shape = trimmed
                .split_once("->")
                .and_then(|(_, tail)| tail.split_once('{'))
                .and_then(|(_, tail)| tail.split_once('}'))
                .map(|(shape, _)| shape.trim().to_owned())
                .unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "baml.coerce".to_owned(),
                target: None,
                name: Some("decide".to_owned()),
                binding: binding_after_as(trimmed),
                args: vec![shape],
                prompt: Some(interpolate_prompt(&prompt, context)),
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index += 1;
        } else if let Some(rest) = trimmed.strip_prefix("exec ") {
            let mut target = None;
            let mut name = None;
            let mut args = Vec::new();
            let parse_spec = if rest.trim_start().starts_with('"') {
                // Escape-aware: a JSON-emitting command (`echo '{\"k\": 1}'`)
                // contains escaped quotes that a naive split would cut at.
                let (command, after_command) = extract_quoted_string(rest).unwrap_or_default();
                args.push(command);
                after_command
                    .trim_start()
                    .strip_prefix("->")
                    .map(str::trim)
                    .map(str::to_owned)
            } else {
                let capability = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let stdin_binding = rest
                    .split_once(" with ")
                    .map(|(_, tail)| {
                        tail.split("->")
                            .next()
                            .unwrap_or_default()
                            .split(" as ")
                            .next()
                            .unwrap_or_default()
                            .split(" timeout ")
                            .next()
                            .unwrap_or_default()
                            .trim()
                            .to_owned()
                    })
                    .unwrap_or_default();
                target = Some(capability);
                name = Some("capability".to_owned());
                args.push(stdin_binding);
                rest.split_once("->")
                    .map(|(_, tail)| tail.trim().to_owned())
            };
            // `-> [each] Schema`: typed stdout ingestion contract
            // (spec/json-ingestion.md), resolved into the effect input.
            if let Some(spec) = parse_spec.as_deref().filter(|spec| !spec.is_empty()) {
                let mut words = spec.split_whitespace();
                match words.next() {
                    Some("each") => {
                        let schema = words.next().unwrap_or_default();
                        args.push(format!("each {schema}"));
                    }
                    Some(schema) => args.push(schema.to_owned()),
                    None => {}
                }
            }
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "exec.command".to_owned(),
                target,
                name,
                binding: binding_after_as(trimmed),
                args,
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index += 1;
        } else if let Some(rest) = trimmed.strip_prefix("notify ") {
            // notify <instance-expr> event <name> { payload }
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let target_expr = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let event = statement
                .split_once(" event ")
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches('{')
                .to_owned();
            let fields = statement
                .split_once('{')
                .and_then(|(_, tail)| tail.rsplit_once('}'))
                .map(|(fields, _)| fields.to_owned())
                .unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "event.notify".to_owned(),
                target: None,
                name: Some(event.clone()),
                binding: binding_after_as(&statement),
                args: vec![target_expr, event, fields],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if let Some(rest) = trimmed.strip_prefix("acquire ") {
            // acquire <lease> for <key-expr> [until ttl] as <slot>
            let resource = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let key_expr = rest
                .split_once(" for ")
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .split(" as ")
                .next()
                .unwrap_or_default()
                .replace(" until ttl", "")
                .trim()
                .to_owned();
            let until_ttl = rest.contains(" until ttl");
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "lease.acquire".to_owned(),
                target: Some(resource),
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![key_expr, until_ttl.to_string()],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index += 1;
        } else if let Some(rest) = trimmed.strip_prefix("append ") {
            // append <Schema> { fields } to <ledger> [as x]
            let (statement, next_index) =
                parse_statement_until_balanced_braces(&lines, index, trimmed);
            let schema = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let fields = statement
                .split_once('{')
                .and_then(|(_, tail)| tail.rsplit_once('}'))
                .map(|(fields, _)| fields.to_owned())
                .unwrap_or_default();
            let ledger = statement
                .rsplit_once(" to ")
                .map(|(_, tail)| {
                    tail.split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_owned()
                })
                .unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "ledger.append".to_owned(),
                target: Some(ledger),
                name: None,
                binding: binding_after_as(&statement),
                args: vec![schema, fields],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if trimmed.starts_with("consume ")
            && trimmed.contains(" for ")
            && trimmed.contains(" amount ")
        {
            // consume <counter> for <key-expr> amount <expr> as <binding>
            let rest = trimmed.strip_prefix("consume ").unwrap_or_default();
            let counter = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let key_expr = rest
                .split_once(" for ")
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .split(" amount ")
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            let amount_expr = rest
                .split_once(" amount ")
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .split(" as ")
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            effects.push(ParsedEffect {
                timeout_seconds: None,
                kind: "counter.consume".to_owned(),
                target: Some(counter),
                name: None,
                binding: binding_after_as(trimmed),
                args: vec![key_expr, amount_expr],
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index += 1;
        } else if let Some(rest) = trimmed.strip_prefix("tell ") {
            let target_expr = rest.split_whitespace().next().unwrap_or("agent");
            let (prompt, next_index) = parse_prompt_from_lines(&lines, index, trimmed);
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "agent.tell".to_owned(),
                target: Some(resolve_tell_target(target_expr, context)),
                name: None,
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: Some(interpolate_prompt(&prompt.text, context)),
                prompt_content_type: prompt.content_type,
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
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "baml.coerce".to_owned(),
                target: Some(name.to_owned()),
                name: Some(name.to_owned()),
                binding: binding_after_as(trimmed),
                args,
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
            index = next_index;
        } else if trimmed.starts_with("claim ") && trimmed.contains(" with loft") {
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "loft.claim".to_owned(),
                target: Some("loft".to_owned()),
                name: Some("claim".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                prompt_content_type: None,
                required_capabilities: Vec::new(),
                after: current_after,
            });
        } else if trimmed.starts_with("askHuman ") {
            let (prompt, next_index) = parse_prompt_from_lines(&lines, index, trimmed);
            // Typed choices declared in source drive the inbox options.
            let choices = trimmed
                .split_once("choices ")
                .and_then(|(_, tail)| tail.split_once('['))
                .and_then(|(_, tail)| tail.split_once(']'))
                .map(|(inner, _)| {
                    inner
                        .split(',')
                        .filter_map(|value| {
                            value
                                .trim()
                                .strip_prefix('"')
                                .and_then(|v| v.strip_suffix('"'))
                        })
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            effects.push(ParsedEffect {
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "human.ask".to_owned(),
                target: Some("human".to_owned()),
                name: Some("askHuman".to_owned()),
                binding: binding_after_as(trimmed),
                args: choices,
                prompt: Some(interpolate_prompt(&prompt.text, context)),
                prompt_content_type: prompt.content_type,
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
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "capability.call".to_owned(),
                target: Some(target),
                name: Some("call".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                prompt_content_type: None,
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
                timeout_seconds: parse_timeout_clause_seconds(trimmed),
                kind: "event.emit".to_owned(),
                target: Some(event_type),
                name: Some("emit".to_owned()),
                binding: binding_after_as(trimmed),
                args: Vec::new(),
                prompt: None,
                prompt_content_type: None,
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
    errors: &mut Vec<String>,
) -> String {
    let mut input = match effect.kind.as_str() {
        "agent.tell" => json!({
            "prompt": effect.prompt.as_deref().unwrap_or_default(),
            "rule": rule.name,
            "bindings": context_bindings_json(context),
        }),
        "baml.coerce" => {
            let function_name = effect.name.as_deref().unwrap_or("coerce");
            let coerce_prompt = coerce_prompt_from_ir(ir, function_name);
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
            let mut input = json!({
                "function_name": function_name,
                "arguments": Value::Object(arguments),
                "argument_exprs": effect.args,
                "output_type": output_type,
                "bindings": context_bindings_json(context),
                "rule": rule.name,
            });
            // Sum-type output (spec/sum-types.md): embed deterministic
            // per-variant fixture values so a fixture run returns a tagged
            // variant (first declared by default; `--variant` selects an
            // arm) without the worker needing the IR.
            if let Some(decl) = ir.schemas.iter().find_map(|schema| match schema {
                IrSchema::Enum(decl) if decl.name == output_type => Some(decl),
                _ => None,
            }) {
                let has_payloads = ir.schemas.iter().any(|schema| {
                    matches!(schema, IrSchema::Class(class)
                        if class.name.starts_with(&format!("{}.", decl.name)))
                });
                if has_payloads {
                    let mut fixtures = serde_json::Map::new();
                    for variant in &decl.variants {
                        let generated = format!("{}.{variant}", decl.name);
                        let value = if ir.schemas.iter().any(|schema| {
                            matches!(schema, IrSchema::Class(class) if class.name == generated)
                        }) {
                            fixture_value_for_shape(&ingest_shape_json(
                                ir,
                                &IrType::Ref(generated),
                                0,
                            ))
                        } else {
                            Value::String(variant.clone())
                        };
                        fixtures.insert(variant.clone(), value);
                    }
                    if let Some(object) = input.as_object_mut() {
                        object.insert("fixture_variants".to_owned(), Value::Object(fixtures));
                        if let Some(first) = decl.variants.first() {
                            object
                                .insert("fixture_default".to_owned(), Value::String(first.clone()));
                        }
                    }
                }
            }
            if let Some(prompt) = coerce_prompt {
                if let Some(object) = input.as_object_mut() {
                    object.insert("prompt_template".to_owned(), Value::String(prompt.text));
                    if let Some(content_type) = prompt.content_type {
                        object.insert(
                            "prompt_content_type".to_owned(),
                            Value::String(content_type),
                        );
                    }
                }
            }
            input
        }
        "loft.claim" => json!({
            "action": "claim",
            "issue": context_bindings_json(context),
            "rule": rule.name,
        }),
        "human.ask" => {
            let choices = if effect.args.is_empty() {
                json!(["accept", "revise", "block"])
            } else {
                json!(effect.args)
            };
            json!({
                "prompt": effect.prompt.as_deref().unwrap_or_default(),
                "choices": choices,
                "severity": "normal",
                "rule": rule.name,
            })
        }
        "queue.file" => {
            let fields = parse_record_fields(
                effect.args.first().map(String::as_str).unwrap_or_default(),
                context,
                None,
                errors,
            );
            json!({
                "queue": effect.target,
                "item": Value::Object(fields),
                "rule": rule.name,
            })
        }
        "queue.claim" | "queue.release" | "queue.finish" => {
            let binding = effect.args.first().map(String::as_str).unwrap_or_default();
            let item = parse_field_value(binding, context);
            let mut input = json!({
                "queue": item.get("queue").cloned().unwrap_or(Value::Null),
                "id": item.get("id").cloned().unwrap_or(Value::Null),
                "rule": rule.name,
            });
            if effect.kind == "queue.finish" {
                let fields = parse_record_fields(
                    effect.args.get(1).map(String::as_str).unwrap_or_default(),
                    context,
                    None,
                    errors,
                );
                insert_json_field(&mut input, "payload", Value::Object(fields));
            }
            input
        }
        "event.notify" => {
            let event_name = effect.args.get(1).cloned().unwrap_or_default();
            let event = ir.events.iter().find(|event| event.name == event_name);
            if event.is_none() {
                errors.push(format!("notify of undeclared event `{event_name}`"));
            }
            let target = parse_field_value(
                effect.args.first().map(String::as_str).unwrap_or_default(),
                context,
            );
            let payload = Value::Object(parse_record_fields(
                effect.args.get(2).map(String::as_str).unwrap_or_default(),
                context,
                None,
                errors,
            ));
            let shape = event
                .map(|event| ingest_shape_json(ir, &IrType::Object(event.fields.clone()), 0))
                .unwrap_or(Value::Null);
            json!({
                "target_instance": coordination_key_string(&target),
                "event": event_name,
                "payload": payload,
                "shape": shape,
                "rule": rule.name,
            })
        }
        "lease.acquire" => {
            let lease = ir
                .leases
                .iter()
                .find(|lease| Some(&lease.name) == effect.target.as_ref());
            if lease.is_none() {
                errors.push(format!(
                    "acquire of undeclared lease `{}`",
                    effect.target.as_deref().unwrap_or_default()
                ));
            }
            let key = parse_field_value(
                effect.args.first().map(String::as_str).unwrap_or_default(),
                context,
            );
            json!({
                "resource": effect.target,
                "key": coordination_key_string(&key),
                "slots": lease.map(|lease| lease.slots).unwrap_or(1),
                "ttl_seconds": lease.map(|lease| lease.ttl_seconds).unwrap_or(600),
                "until_ttl": effect.args.get(1).map(String::as_str) == Some("true"),
                "rule": rule.name,
            })
        }
        "lease.release" => json!({
            "acquire_effect_id": effect
                .args
                .first()
                .and_then(|binding| effect_bindings.get(binding)),
            "rule": rule.name,
        }),
        "ledger.append" => {
            let ledger = ir
                .ledgers
                .iter()
                .find(|ledger| Some(&ledger.name) == effect.target.as_ref());
            if ledger.is_none() {
                errors.push(format!(
                    "append to undeclared ledger `{}`",
                    effect.target.as_deref().unwrap_or_default()
                ));
            }
            let entry = Value::Object(parse_record_fields(
                effect.args.get(1).map(String::as_str).unwrap_or_default(),
                context,
                None,
                errors,
            ));
            let partition = ledger
                .and_then(|ledger| entry.get(&ledger.partition_field))
                .map(coordination_key_string)
                .unwrap_or_default();
            json!({
                "ledger": effect.target,
                "schema": effect.args.first().cloned().unwrap_or_default(),
                "entry": entry,
                "partition": partition,
                "retain_seconds": ledger.map(|ledger| ledger.retain_seconds).unwrap_or(86400),
                "rule": rule.name,
            })
        }
        "counter.consume" => {
            let counter = ir
                .counters
                .iter()
                .find(|counter| Some(&counter.name) == effect.target.as_ref());
            if counter.is_none() {
                errors.push(format!(
                    "consume of undeclared counter `{}`",
                    effect.target.as_deref().unwrap_or_default()
                ));
            }
            let key = parse_field_value(
                effect.args.first().map(String::as_str).unwrap_or_default(),
                context,
            );
            let amount = parse_field_value(
                effect.args.get(1).map(String::as_str).unwrap_or_default(),
                context,
            );
            json!({
                "counter": effect.target,
                "key": coordination_key_string(&key),
                "amount": amount.as_i64().unwrap_or(0),
                "cap": counter.map(|counter| counter.cap).unwrap_or(0),
                "reset": counter.map(|counter| counter.reset.clone()).unwrap_or_else(|| "daily".to_owned()),
                "rule": rule.name,
            })
        }
        "exec.command" => {
            let mut input = if effect.name.as_deref() == Some("capability") {
                let stdin_expr = effect.args.first().map(String::as_str).unwrap_or_default();
                json!({
                    "mode": "capability",
                    "capability": effect.target,
                    "stdin": parse_field_value(stdin_expr, context),
                    "stdin_binding": stdin_expr,
                    "rule": rule.name,
                })
            } else {
                json!({
                    "mode": "raw",
                    "command": effect.args.first().cloned().unwrap_or_default(),
                    "rule": rule.name,
                })
            };
            let parse_spec_index = if effect.name.as_deref() == Some("capability") {
                1
            } else {
                1
            };
            if let Some(spec) = effect.args.get(parse_spec_index) {
                let (each, schema) = match spec.strip_prefix("each ") {
                    Some(schema) => (true, schema),
                    None => (false, spec.as_str()),
                };
                insert_json_field(
                    &mut input,
                    "parse",
                    json!({
                        "schema": schema,
                        "each": each,
                        "shape": ingest_shape_json(ir, &IrType::Ref(schema.to_owned()), 0),
                    }),
                );
            }
            input
        }
        "timer.wait" if effect.timeout_seconds.is_none() => json!({
            "deadline_at": effect.args.first().cloned().unwrap_or_default(),
            "rule": rule.name,
        }),
        "timer.wait" => json!({
            "duration": effect.args.first().cloned().unwrap_or_default(),
            "duration_seconds": effect.timeout_seconds,
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
                "input": Value::Object(parse_record_fields(body, context, None, errors)),
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
    if matches!(effect.kind.as_str(), "agent.tell" | "human.ask") {
        if let Some(content_type) = &effect.prompt_content_type {
            if let Some(object) = input.as_object_mut() {
                object.insert(
                    "prompt_content_type".to_owned(),
                    Value::String(content_type.clone()),
                );
            }
        }
    }
    input.to_string()
}

fn coerce_prompt_from_ir(ir: &IrProgram, function_name: &str) -> Option<ParsedPrompt> {
    let coerce = ir
        .coerces
        .iter()
        .find(|coerce| coerce.name == function_name)?;
    let lines = coerce.body.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("prompt ") || trimmed == "prompt" {
            return Some(parse_prompt_from_lines(&lines, index, trimmed).0);
        }
    }
    None
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedPrompt {
    text: String,
    content_type: Option<String>,
}

fn parse_prompt_from_lines(lines: &[&str], index: usize, trimmed: &str) -> (ParsedPrompt, usize) {
    if trimmed.contains("\"\"\"") {
        let mut prompt_lines = Vec::new();
        let after_open = trimmed
            .split_once("\"\"\"")
            .map(|(_, tail)| tail)
            .unwrap_or("");
        let content_type = prompt_content_type_from_opening_tail(after_open);
        if !after_open.is_empty() && content_type.is_none() {
            prompt_lines.push(after_open.to_owned());
        }
        let mut cursor = index + 1;
        while cursor < lines.len() {
            let line = lines[cursor];
            if let Some((head, _tail)) = line.split_once("\"\"\"") {
                prompt_lines.push(head.to_owned());
                return (
                    ParsedPrompt {
                        text: prompt_lines.join("\n").trim().to_owned(),
                        content_type,
                    },
                    cursor,
                );
            }
            prompt_lines.push(line.to_owned());
            cursor += 1;
        }
        return (
            ParsedPrompt {
                text: prompt_lines.join("\n").trim().to_owned(),
                content_type,
            },
            cursor,
        );
    }
    let prompt = trimmed
        .split_once('"')
        .and_then(|(_, tail)| tail.rsplit_once('"').map(|(prompt, _)| prompt))
        .unwrap_or("")
        .to_owned();
    (
        ParsedPrompt {
            text: prompt,
            content_type: None,
        },
        index,
    )
}

fn prompt_content_type_from_opening_tail(after_open: &str) -> Option<String> {
    let candidate = after_open.trim();
    if candidate.is_empty() || candidate.contains("\"\"\"") {
        return None;
    }
    is_supported_prompt_content_type(candidate).then(|| candidate.to_ascii_lowercase())
}

fn is_supported_prompt_content_type(candidate: &str) -> bool {
    if !is_prompt_content_type_token(candidate) {
        return false;
    }
    let normalized = candidate.to_ascii_lowercase();
    normalized.contains('/')
        || matches!(
            normalized.as_str(),
            "markdown" | "json" | "text" | "plain" | "html" | "xml" | "yaml" | "yml"
        )
}

fn is_prompt_content_type_token(candidate: &str) -> bool {
    let mut chars = candidate.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '+' | '-' | '_'))
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
    whipplescript_parser::runtime_fact_name_for_pattern(pattern)
        .unwrap_or_else(|| pattern.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Matches `<agent-or-worker> completed turn ...` readiness patterns and
/// returns the leading word. `worker` is the generic form (any agent).
fn completed_turn_agent(pattern: &str) -> Option<&str> {
    let mut words = pattern.split_whitespace();
    let first = words.next()?;
    if words.next() == Some("completed") && words.next() == Some("turn") {
        Some(first)
    } else {
        None
    }
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
            // Inline form: the block opens and closes on the statement line.
            if let Some(body) = inline_block_body(trimmed) {
                blocks.push(RecordBlock {
                    schema,
                    from_binding,
                    body,
                });
                index += 1;
                continue;
            }
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

/// Extracts the inner text of a `{ ... }` block that opens and closes on the
/// same statement line, e.g. `complete result { total 2 }`.
fn inline_block_body(line: &str) -> Option<String> {
    let open = line.find('{')?;
    let close = line.rfind('}')?;
    if close <= open || brace_delta(line) != 0 {
        return None;
    }
    Some(line[open + 1..close].trim().to_owned())
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
        if let Some(body) = inline_block_body(trimmed) {
            blocks.push(TerminalBlock { kind, name, body });
            index += 1;
            continue;
        }
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
        // Coordination outcomes (spec/coordination.md): the op completed and
        // its sum-typed value carries the matching variant.
        "held" | "contended" | "ok" | "over" => {
            status == Some("completed")
                && payload
                    .pointer("/value/variant")
                    .and_then(Value::as_str)
                    .is_some_and(|variant| variant.eq_ignore_ascii_case(predicate))
        }
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
    errors: &mut Vec<String>,
) -> serde_json::Map<String, Value> {
    let mut object = serde_json::Map::new();
    for assignment in collect_field_assignments(body) {
        match assignment {
            FieldAssignment::Value { name, value } => {
                object.insert(
                    name.clone(),
                    parse_record_field_value(&name, &value, context, from_binding, errors),
                );
            }
            FieldAssignment::Shorthand { name } => {
                object.insert(
                    name.clone(),
                    parse_record_shorthand_value(&name, context, from_binding, errors),
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
    errors: &mut Vec<String>,
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
            errors.push(format!("could not resolve `{binding}.{value}`"));
            return Value::Null;
        }
    }
    let is_plain_path = value.contains('.')
        && value
            .split('.')
            .all(|segment| !segment.is_empty() && is_identifier(segment));
    if is_plain_path {
        if let Some((binding, field)) = value.split_once('.') {
            if context
                .bindings
                .iter()
                .any(|(candidate, _)| candidate == binding)
                && context_field_value(context, binding, field).is_none()
            {
                errors.push(format!("could not resolve `{value}`"));
                return Value::Null;
            }
        }
    }
    parse_field_value(value, context)
}

fn parse_record_shorthand_value(
    field: &str,
    context: &RuleContext,
    from_binding: Option<&str>,
    errors: &mut Vec<String>,
) -> Value {
    if let Some(binding) = from_binding {
        if let Some(value) = context_field_value(context, binding, field) {
            return value;
        }
        errors.push(format!("could not resolve `{binding}.{field}`"));
        return Value::Null;
    }
    let matches = context
        .bindings
        .iter()
        .filter_map(|(binding, _)| context_field_value(context, binding, field))
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.into_iter().next().unwrap_or(Value::Null)
    } else {
        if matches.is_empty() {
            errors.push(format!("could not resolve shorthand field `{field}`"));
        } else {
            errors.push(format!("shorthand field `{field}` is ambiguous"));
        }
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
    // Variant construction `Approved { score 0.9 }` builds the
    // internally-tagged record (spec/sum-types.md); the author never writes
    // the discriminant.
    if let Some((head, rest)) = value.split_once('{') {
        let head = head.trim();
        let rest = rest.trim_end();
        if is_identifier(head)
            && head
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
            && rest.ends_with('}')
        {
            let inner = &rest[..rest.len() - 1];
            let mut object = serde_json::Map::new();
            object.insert("variant".to_owned(), Value::String(head.to_owned()));
            let mut nested_errors = Vec::new();
            for (name, field_value) in parse_record_fields(inner, context, None, &mut nested_errors)
            {
                object.insert(name, field_value);
            }
            return Value::Object(object);
        }
    }
    if let Ok(number) = value.parse::<i64>() {
        return Value::Number(number.into());
    }
    if let Some((binding, field)) = value.split_once('.') {
        if let Some(value) = context_field_value(context, binding, field) {
            return value;
        }
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
    // Token-level splitting: structure comes from tokens, never line breaks,
    // so single-line blocks (`{ id "a" status "done" }`) and multi-line
    // blocks behave identically.
    whipplescript_parser::body::split_field_assignments(body)
        .into_iter()
        .map(|assignment| match assignment.value {
            Some(value) => FieldAssignment::Value {
                name: assignment.name,
                value,
            },
            None => FieldAssignment::Shorthand {
                name: assignment.name,
            },
        })
        .collect()
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
        "target_id": report.target_id,
        "expr": report.expr,
        "reads": assertion_reads_to_json(&report.reads),
        "tags": report.tags,
        "status": report.status.as_str(),
        "passed": report.passed,
        "actual": report.actual,
        "actual_values": report.actual_values,
        "expected": report.expected,
    });
    if let Some(event_id) = &report.event_id {
        if let Some(object) = value.as_object_mut() {
            object.insert("event_id".to_owned(), Value::String(event_id.clone()));
        }
    }
    if !report.diagnostic_ids.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "diagnostic_ids".to_owned(),
                Value::Array(
                    report
                        .diagnostic_ids
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
    }
    if let Some(failure_reason) = &report.failure_reason {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "failure_reason".to_owned(),
                Value::String(failure_reason.clone()),
            );
        }
    }
    if let Some(description) = &report.description {
        if let Some(object) = value.as_object_mut() {
            object.insert("description".to_owned(), Value::String(description.clone()));
        }
    }
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

fn assertion_reads_to_json(reads: &[AssertionReadReport]) -> Value {
    Value::Array(reads.iter().map(assertion_read_to_json).collect::<Vec<_>>())
}

fn assertion_read_to_json(read: &AssertionReadReport) -> Value {
    let mut value = json!({
        "kind": read.kind,
        "head": read.head,
        "source": read.source,
        "match_count": read.match_count,
        "matches": read.matches.iter().map(assertion_read_match_to_json).collect::<Vec<_>>(),
    });
    if let Some(guard) = &read.guard {
        if let Some(object) = value.as_object_mut() {
            object.insert("guard".to_owned(), Value::String(guard.clone()));
        }
    }
    if let Some(error) = &read.error {
        if let Some(object) = value.as_object_mut() {
            object.insert("error".to_owned(), Value::String(error.clone()));
        }
    }
    value
}

fn assertion_read_match_to_json(match_report: &AssertionReadMatch) -> Value {
    let mut value = json!({
        "id": match_report.id,
        "name": match_report.name,
    });
    if let Some(key) = &match_report.key {
        if let Some(object) = value.as_object_mut() {
            object.insert("key".to_owned(), Value::String(key.clone()));
        }
    }
    if let Some(status) = &match_report.status {
        if let Some(object) = value.as_object_mut() {
            object.insert("status".to_owned(), Value::String(status.clone()));
        }
    }
    if let Some(prompt_content_type) = &match_report.prompt_content_type {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "prompt_content_type".to_owned(),
                Value::String(prompt_content_type.clone()),
            );
        }
    }
    if let Some(provenance_class) = &match_report.provenance_class {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "provenance_class".to_owned(),
                Value::String(provenance_class.clone()),
            );
        }
    }
    if let Some(source_span_json) = &match_report.source_span_json {
        if let Some(object) = value.as_object_mut() {
            object.insert("source_span".to_owned(), json_from_str(source_span_json));
        }
    }
    value
}

fn executable_spec_report_to_json(assertions: &[AssertionReport]) -> Value {
    let mut tag_groups = BTreeMap::<String, Vec<&AssertionReport>>::new();
    let mut untagged = Vec::new();
    for assertion in assertions {
        if assertion.tags.is_empty() {
            untagged.push(assertion);
        } else {
            for tag in &assertion.tags {
                tag_groups.entry(tag.clone()).or_default().push(assertion);
            }
        }
    }

    let mut value = json!({
        "status": assertion_group_status(assertions),
        "summary": assertion_group_summary(assertions),
        "tags": tag_groups
            .iter()
            .map(|(tag, assertions)| {
                json!({
                    "tag": tag,
                    "status": assertion_group_status(assertions.iter().copied()),
                    "summary": assertion_group_summary(assertions.iter().copied()),
                    "assertions": assertions
                        .iter()
                        .map(|assertion| executable_spec_assertion_json(assertion))
                        .collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    });

    if !untagged.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "untagged".to_owned(),
                json!({
                    "status": assertion_group_status(untagged.iter().copied()),
                    "summary": assertion_group_summary(untagged.iter().copied()),
                    "assertions": untagged
                        .iter()
                        .map(|assertion| executable_spec_assertion_json(assertion))
                        .collect::<Vec<_>>(),
                }),
            );
        }
    }

    value
}

fn assertion_group_status<'a, I>(assertions: I) -> &'static str
where
    I: IntoIterator<Item = &'a AssertionReport>,
{
    let mut saw_failed = false;
    for assertion in assertions {
        match assertion.status {
            AssertionStatus::Error => return "error",
            AssertionStatus::Failed => saw_failed = true,
            AssertionStatus::Passed => {}
        }
    }
    if saw_failed {
        "failed"
    } else {
        "passed"
    }
}

fn assertion_group_summary<'a, I>(assertions: I) -> Value
where
    I: IntoIterator<Item = &'a AssertionReport>,
{
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut error = 0usize;
    for assertion in assertions {
        total += 1;
        match assertion.status {
            AssertionStatus::Passed => passed += 1,
            AssertionStatus::Failed => failed += 1,
            AssertionStatus::Error => error += 1,
        }
    }
    json!({
        "total": total,
        "passed": passed,
        "failed": failed,
        "error": error,
    })
}

fn executable_spec_assertion_json(report: &AssertionReport) -> Value {
    let mut value = json!({
        "target_id": report.target_id,
        "expr": report.expr,
        "reads": assertion_reads_to_json(&report.reads),
        "status": report.status.as_str(),
        "passed": report.passed,
    });
    if let Some(event_id) = &report.event_id {
        if let Some(object) = value.as_object_mut() {
            object.insert("event_id".to_owned(), Value::String(event_id.clone()));
        }
    }
    if !report.diagnostic_ids.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "diagnostic_ids".to_owned(),
                Value::Array(
                    report
                        .diagnostic_ids
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
    }
    if let Some(description) = &report.description {
        if let Some(object) = value.as_object_mut() {
            object.insert("description".to_owned(), Value::String(description.clone()));
        }
    }
    value
}

fn worker_report_to_json(report: &WorkerReport) -> Value {
    json!({
        "instance_id": report.instance_id,
        "provider": report.provider,
        "ran_effects": report.ran_effects,
        "cancellation_acknowledgements": report.cancellation_acknowledgements,
        "cancellation_diagnostics": report.cancellation_diagnostics,
        "terminal_events": report.terminal_events,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecoverReport {
    instance_id: String,
    recovered_events: Vec<RecoveredEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecoveredEvent {
    event_id: String,
    event_type: String,
}

fn recover(options: &CliOptions) -> ExitCode {
    let Some(instance_id) = options.args.first() else {
        eprintln!("usage: whip recover <instance>");
        return ExitCode::from(2);
    };
    if options.args.len() != 1 {
        eprintln!("usage: whip recover <instance>");
        return ExitCode::from(2);
    }
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let mut kernel = RuntimeKernel::new(store);
    let recovered = match kernel.recover_running_provider_runs(instance_id) {
        Ok(events) => events,
        Err(error) => return report_store_error("failed to recover provider runs", error),
    };
    let report = RecoverReport {
        instance_id: instance_id.clone(),
        recovered_events: recovered
            .into_iter()
            .map(|event| RecoveredEvent {
                event_id: event.event_id,
                event_type: "effect.terminal".to_owned(),
            })
            .collect(),
    };
    if options.json {
        emit_json(recover_report_to_json(&report))
    } else {
        println!(
            "recover {} recovered={}",
            report.instance_id,
            report.recovered_events.len()
        );
        ExitCode::SUCCESS
    }
}

fn recover_report_to_json(report: &RecoverReport) -> Value {
    json!({
        "instance_id": report.instance_id,
        "recovered_count": report.recovered_events.len(),
        "recovered_events": report
            .recovered_events
            .iter()
            .map(|event| json!({
                "event_id": event.event_id,
                "event_type": event.event_type,
            }))
            .collect::<Vec<_>>(),
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
        let effects = match store.list_effects(instance_id) {
            Ok(effects) => effects,
            Err(error) => return report_store_error("failed to load effects", error),
        };
        let runs = match store.list_runs(instance_id) {
            Ok(runs) => runs,
            Err(error) => return report_store_error("failed to load runs", error),
        };
        emit_json(status_to_json_with_effects_and_runs(
            &status, &effects, &runs,
        ))
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
        if let Ok(pending) = store.pending_time_effects(instance_id) {
            if !pending.is_empty() {
                println!("pending time effects:");
                for effect in pending {
                    println!(
                        "  {} {} due_in<= {}s (status={})",
                        effect.effect_id, effect.kind, effect.timeout_seconds, effect.status
                    );
                }
            }
        }
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
            let details = log_event_details(&event)
                .map(|details| format!(" {details}"))
                .unwrap_or_default();
            println!(
                "#{:04} {} {} source={}{}",
                event.sequence, event.occurred_at, event.event_type, event.source, details
            );
        }
        ExitCode::SUCCESS
    }
}

fn log_event_details(event: &EventView) -> Option<String> {
    let payload = json_from_str(&event.payload_json);
    match event.event_type.as_str() {
        "workflow.revision_activated" => Some(format!(
            "revision={} epoch={}->{} from={} to={} cancel={} terminal_cancel={} request_cancel={}",
            payload_str(&payload, "revision_id"),
            payload_i64(&payload, "from_epoch"),
            payload_i64(&payload, "to_epoch"),
            payload_str(&payload, "from_version_id"),
            payload_str(&payload, "to_version_id"),
            payload_str(&payload, "cancellation_policy"),
            payload_array_len(&payload, "terminal_cancel_effects"),
            payload_array_len(&payload, "request_cancel_effects"),
        )),
        "workflow.revision_rejected" => Some(format!(
            "candidate={} active={} diagnostics={}",
            payload_str(&payload, "candidate_version_id"),
            payload_str(&payload, "active_version_id"),
            payload_array_len(&payload, "diagnostics"),
        )),
        "effect.cancellation_requested" => Some(format!(
            "effect={} revision={} by={} reason={}",
            payload_str(&payload, "effect_id"),
            payload_str(&payload, "revision_id"),
            payload_str(&payload, "requested_by"),
            payload_str(&payload, "reason"),
        )),
        _ => None,
    }
}

fn payload_str<'a>(payload: &'a Value, field: &str) -> &'a str {
    payload.get(field).and_then(Value::as_str).unwrap_or("-")
}

fn payload_i64(payload: &Value, field: &str) -> String {
    payload
        .get(field)
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn payload_array_len(payload: &Value, field: &str) -> usize {
    payload
        .get(field)
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
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
                "{} {} status={} target={} profile={} version={} epoch={} cancel_requested={} reason={}",
                effect.effect_id,
                effect.kind,
                effect.status,
                effect.target.as_deref().unwrap_or("-"),
                effect.profile.as_deref().unwrap_or("-"),
                effect.program_version_id.as_deref().unwrap_or("-"),
                effect.revision_epoch,
                effect.cancel_requested,
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
    let events = match store.list_events(instance_id) {
        Ok(events) => events,
        Err(error) => return report_store_error("failed to load events", error),
    };
    let artifact_counts = match artifact_counts_for_runs(&store, &runs) {
        Ok(counts) => counts,
        Err(error) => return report_store_error("failed to load artifacts", error),
    };

    if options.json {
        emit_json(Value::Array(
            runs.iter()
                .map(|run| run_to_json_with_lifecycle_and_artifacts(run, &events, &artifact_counts))
                .collect::<Vec<_>>(),
        ))
    } else {
        for run in runs {
            let lifecycle = latest_native_lifecycle_for_run(&events, &run.run_id);
            println!(
                "{} effect={} status={} worker={} started={} cancel_requested={} artifacts={} native_status={}",
                run.run_id,
                run.effect_id,
                run.status,
                run.worker_id,
                run.started_at,
                run.cancel_requested,
                artifact_counts.get(&run.run_id).copied().unwrap_or(0),
                lifecycle
                    .as_ref()
                    .and_then(|value| value.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            );
        }
        ExitCode::SUCCESS
    }
}

fn artifacts(options: &CliOptions) -> ExitCode {
    let Some(run_id) = single_arg(options, "usage: whip artifacts <run-id>") else {
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let artifacts = match store.list_artifacts_for_run(run_id) {
        Ok(artifacts) => artifacts,
        Err(error) => return report_store_error("failed to list artifacts", error),
    };

    if options.json {
        emit_json(json!({
            "run_id": run_id,
            "artifacts": artifacts.iter().map(artifact_to_json).collect::<Vec<_>>(),
        }))
    } else {
        for artifact in artifacts {
            let path = redact_cli_metadata(&artifact.path);
            let content_hash = artifact
                .content_hash
                .as_deref()
                .map(redact_cli_metadata)
                .unwrap_or_else(|| "-".to_owned());
            println!(
                "{} {} path={} hash={} mime={} created={}",
                artifact.artifact_id,
                artifact.kind,
                path,
                content_hash,
                artifact.mime_type.as_deref().unwrap_or("-"),
                artifact.created_at
            );
        }
        ExitCode::SUCCESS
    }
}

fn items(options: &CliOptions) -> ExitCode {
    use whipplescript_store::items::WorkItemStore;
    let usage = "usage: whip items [list [--queue Q] [--status S]|add --queue Q --title T [--body B] [--label L]...|show <id>]";
    let args = &options.args;
    let command = args.first().map(String::as_str).unwrap_or("list");
    let store = match WorkItemStore::open(items_store_path()) {
        Ok(store) => store,
        Err(error) => return report_store_error("failed to open items store", error),
    };
    match command {
        "list" => {
            let mut queue = None;
            let mut status = None;
            let mut iter = args.iter().skip(1);
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--queue" => queue = iter.next().cloned(),
                    "--status" => status = iter.next().cloned(),
                    _ => {
                        eprintln!("{usage}");
                        return ExitCode::from(2);
                    }
                }
            }
            let listed = match store.list_items(queue.as_deref(), status.as_deref()) {
                Ok(items) => items,
                Err(error) => return report_store_error("failed to list items", error),
            };
            if options.json {
                return emit_json(Value::Array(
                    listed.iter().map(work_item_to_json).collect::<Vec<_>>(),
                ));
            }
            if listed.is_empty() {
                println!("no items");
            }
            for item in listed {
                println!(
                    "{} [{}] queue={} {}{}",
                    item.id,
                    item.status,
                    item.queue,
                    item.title,
                    item.claimed_by
                        .as_deref()
                        .map(|claimed| format!(" (claimed by {claimed})"))
                        .unwrap_or_default()
                );
            }
            ExitCode::SUCCESS
        }
        "add" => {
            let mut queue = None;
            let mut title = None;
            let mut item_body = String::new();
            let mut labels = Vec::new();
            let mut iter = args.iter().skip(1);
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--queue" => queue = iter.next().cloned(),
                    "--title" => title = iter.next().cloned(),
                    "--body" => item_body = iter.next().cloned().unwrap_or_default(),
                    "--label" => {
                        if let Some(label) = iter.next() {
                            labels.push(label.clone());
                        }
                    }
                    _ => {
                        eprintln!("{usage}");
                        return ExitCode::from(2);
                    }
                }
            }
            let (Some(queue), Some(title)) = (queue, title) else {
                eprintln!("{usage}");
                return ExitCode::from(2);
            };
            // Run-identity provenance: anything filed from inside a turn is
            // attributed to the exact turn that produced it.
            let filed_by = ["WHIPPLESCRIPT_RUN_ID", "WHIPPLESCRIPT_INSTANCE_ID"]
                .iter()
                .filter_map(|name| env::var(name).ok().map(|value| format!("{name}={value}")))
                .collect::<Vec<_>>()
                .join(",");
            let filed_by = (!filed_by.is_empty()).then_some(filed_by);
            let mut store = store;
            match store.file_item(
                &queue,
                &title,
                &item_body,
                &labels,
                &json!({}),
                filed_by.as_deref(),
            ) {
                Ok(item) => {
                    if options.json {
                        emit_json(work_item_to_json(&item))
                    } else {
                        println!("{} filed into {}", item.id, item.queue);
                        ExitCode::SUCCESS
                    }
                }
                Err(error) => report_store_error("failed to file item", error),
            }
        }
        "show" => {
            let Some(id) = args.get(1) else {
                eprintln!("{usage}");
                return ExitCode::from(2);
            };
            match store.get_item(id) {
                Ok(Some(item)) => {
                    if options.json {
                        emit_json(work_item_to_json(&item))
                    } else {
                        println!("{} [{}] queue={}", item.id, item.status, item.queue);
                        println!("title: {}", item.title);
                        if !item.body.is_empty() {
                            println!("body: {}", item.body);
                        }
                        if !item.labels.is_empty() {
                            println!("labels: {}", item.labels.join(", "));
                        }
                        if let Some(claimed) = &item.claimed_by {
                            println!("claimed by: {claimed}");
                        }
                        if let Some(filed) = &item.filed_by {
                            println!("filed by: {filed}");
                        }
                        ExitCode::SUCCESS
                    }
                }
                Ok(None) => {
                    eprintln!("item `{id}` was not found");
                    ExitCode::FAILURE
                }
                Err(error) => report_store_error("failed to load item", error),
            }
        }
        _ => {
            eprintln!("{usage}");
            ExitCode::from(2)
        }
    }
}

fn work_item_to_json(item: &whipplescript_store::items::WorkItem) -> Value {
    json!({
        "id": item.id,
        "queue": item.queue,
        "title": item.title,
        "body": item.body,
        "status": item.status,
        "labels": item.labels,
        "metadata": item.metadata,
        "claimed_by": item.claimed_by,
        "filed_by": item.filed_by,
        "created_at": item.created_at,
        "updated_at": item.updated_at,
    })
}

fn inbox(options: &CliOptions) -> ExitCode {
    match options.args.first().map(String::as_str) {
        None => inbox_list(options, None),
        Some("show") => inbox_show(options),
        Some("answer") => inbox_answer(options),
        Some(instance_id) if options.args.len() == 1 && instance_id.starts_with("ins_") => {
            inbox_list(options, Some(instance_id))
        }
        _ => {
            eprintln!(
                "usage: whip inbox [<instance>|show <item>|answer <item> (--choice X|--text X)]"
            );
            ExitCode::from(2)
        }
    }
}

fn inbox_list(options: &CliOptions, instance_id: Option<&str>) -> ExitCode {
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let mut items = match store.list_inbox_items(Some("pending")) {
        Ok(items) => items,
        Err(error) => return report_store_error("failed to list inbox items", error),
    };
    if let Some(instance_id) = instance_id {
        items.retain(|item| item.instance_id == instance_id);
    }

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

/// `whip notify <instance> --event <name> --data <json> --program <path>`:
/// lands a typed external event as a durable fact (spec/event-ingress.md).
/// The payload is validated against the declared `event` schema at this
/// boundary, so a malformed delivery cannot land an ill-typed fact. The
/// recorded fact is the source of truth; replay re-reads it, never the
/// delivery.
fn notify(options: &CliOptions) -> ExitCode {
    let usage = "usage: whip notify <instance> --event <name> --data <json> --program <workflow.whip> [--root <workflow>]";
    let mut instance_id = None;
    let mut event_name = None;
    let mut data = None;
    let mut program_path = None;
    let mut root = None;
    let mut index = 0;
    while index < options.args.len() {
        match options.args[index].as_str() {
            "--event" => {
                index += 1;
                event_name = options.args.get(index).cloned();
            }
            "--data" => {
                index += 1;
                data = options.args.get(index).cloned();
            }
            "--program" => {
                index += 1;
                program_path = options.args.get(index).cloned();
            }
            "--root" => {
                index += 1;
                root = options.args.get(index).cloned();
            }
            other if other.starts_with('-') => {
                eprintln!("unknown notify option `{other}`");
                eprintln!("{usage}");
                return ExitCode::from(2);
            }
            value if instance_id.is_none() => instance_id = Some(value.to_owned()),
            _ => {
                eprintln!("{usage}");
                return ExitCode::from(2);
            }
        }
        index += 1;
    }
    let (Some(instance_id), Some(event_name), Some(data), Some(program_path)) =
        (instance_id, event_name, data, program_path)
    else {
        eprintln!("{usage}");
        return ExitCode::from(2);
    };
    let (_source, ir) = match compile_source_path_with_root(&program_path, root.as_deref()) {
        Ok(compiled) => compiled,
        Err(error) => return report_compile_failure(&program_path, error),
    };
    let Some(event) = ir.events.iter().find(|event| event.name == event_name) else {
        let declared = ir
            .events
            .iter()
            .map(|event| event.name.as_str())
            .collect::<Vec<_>>();
        eprintln!(
            "event `{event_name}` is not declared in {program_path}{}",
            if declared.is_empty() {
                "; the program declares no events".to_owned()
            } else {
                format!("; declared events: {}", declared.join(", "))
            }
        );
        return ExitCode::from(2);
    };
    let payload: Value = match serde_json::from_str(&data) {
        Ok(payload) => payload,
        Err(error) => {
            eprintln!("invalid `--data` JSON: {error}");
            return ExitCode::from(2);
        }
    };
    let mut errors = Vec::new();
    validate_json_for_object(&ir, &payload, &event.fields, "$", &mut errors);
    if !errors.is_empty() {
        eprintln!(
            "payload does not conform to event `{event_name}`: {}",
            errors.join("; ")
        );
        return ExitCode::from(2);
    }
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    match store.get_instance(&instance_id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            eprintln!("instance `{instance_id}` not found");
            return ExitCode::from(2);
        }
        Err(error) => return report_store_error("failed to load instance", error),
    }
    let payload_json = payload.to_string();
    let mut kernel = RuntimeKernel::new(store);
    let received = match kernel.ingest_external_event(
        &instance_id,
        &event_name,
        &payload_json,
        Some(&idempotency_key(&[
            &instance_id,
            "notify",
            &event_name,
            &payload_json,
        ])),
    ) {
        Ok(event) => event,
        Err(error) => return report_store_error("failed to record event", error),
    };
    let fact_event = match kernel.derive_fact(
        &instance_id,
        &event_name,
        &received.event_id,
        &payload_json,
        Some(&received.event_id),
        Some(&idempotency_key(&[
            &instance_id,
            "notify-fact",
            &received.event_id,
        ])),
    ) {
        Ok(event) => event,
        Err(error) => return report_store_error("failed to record event fact", error),
    };
    if options.json {
        emit_json(json!({
            "instance_id": instance_id,
            "event": event_name,
            "event_id": received.event_id,
            "fact_event_id": fact_event.event_id,
            "payload": payload,
        }))
    } else {
        println!(
            "notified {instance_id} with `{event_name}` at event #{}",
            fact_event.sequence
        );
        ExitCode::SUCCESS
    }
}

/// Coordination state is fully attributable and inspectable
/// (spec/coordination.md): who holds which slot, who spent the budget, who
/// wrote which entry.
fn coordination_list(options: &CliOptions, kind: &str) -> ExitCode {
    let store =
        match whipplescript_store::coordination::CoordinationStore::open(coordination_store_path())
        {
            Ok(store) => store,
            Err(error) => return report_store_error("failed to open coordination store", error),
        };
    let name = options
        .args
        .first()
        .filter(|arg| !arg.starts_with('-'))
        .map(String::as_str);
    match kind {
        "leases" => {
            let rows = match store.list_leases(name) {
                Ok(rows) => rows,
                Err(error) => return report_store_error("failed to list leases", error),
            };
            if options.json {
                return emit_json(Value::Array(
                    rows.iter()
                        .map(|row| {
                            json!({
                                "resource": row.resource,
                                "key": row.key,
                                "holder": row.holder,
                                "acquired_at": row.acquired_at,
                                "expires_at": row.expires_at,
                            })
                        })
                        .collect(),
                ));
            }
            if rows.is_empty() {
                println!("no leases held");
            }
            for row in rows {
                println!(
                    "{}/{} held by {} (expires {})",
                    row.resource, row.key, row.holder, row.expires_at
                );
            }
            ExitCode::SUCCESS
        }
        "ledger" => {
            let partition = options
                .args
                .iter()
                .position(|arg| arg == "--partition")
                .and_then(|index| options.args.get(index + 1))
                .map(String::as_str);
            let rows = match store.list_entries(name, partition) {
                Ok(rows) => rows,
                Err(error) => return report_store_error("failed to list ledger entries", error),
            };
            if options.json {
                return emit_json(Value::Array(
                    rows.iter()
                        .map(|row| {
                            json!({
                                "ledger": row.ledger,
                                "partition": row.partition,
                                "seq": row.seq,
                                "entry": json_from_str(&row.payload_json),
                                "appended_by": row.appended_by,
                                "appended_at": row.appended_at,
                            })
                        })
                        .collect(),
                ));
            }
            if rows.is_empty() {
                println!("no ledger entries");
            }
            for row in rows {
                println!(
                    "{}#{} [{}] by {}: {}",
                    row.ledger,
                    row.seq,
                    row.partition,
                    row.appended_by,
                    one_line(&row.payload_json)
                );
            }
            ExitCode::SUCCESS
        }
        _ => {
            let rows = match store.list_counters(name) {
                Ok(rows) => rows,
                Err(error) => return report_store_error("failed to list counters", error),
            };
            if options.json {
                return emit_json(Value::Array(
                    rows.iter()
                        .map(|row| {
                            json!({
                                "counter": row.counter,
                                "key": row.key,
                                "consumed": row.consumed,
                                "period": row.period,
                            })
                        })
                        .collect(),
                ));
            }
            if rows.is_empty() {
                println!("no counters");
            }
            for row in rows {
                println!(
                    "{}/{} consumed={} period={}",
                    row.counter, row.key, row.consumed, row.period
                );
            }
            ExitCode::SUCCESS
        }
    }
}

/// The observability ambassador (spec/observability.md C8): a cursor-tracked
/// exporter that tails the durable event log and emits OTLP/HTTP JSON traces.
/// The event log is the buffer — zero hot-path overhead, failure isolation,
/// and the cursor makes emission exactly-once across re-runs and replays.
/// Config is the standard OTel environment; with no endpoint and no
/// `--dry-run` it refuses rather than guessing. Plain-HTTP endpoints only:
/// the standard sidecar deployment posts to a local OpenTelemetry Collector,
/// which owns TLS and fan-out to backends.
fn otel_export(options: &CliOptions) -> ExitCode {
    let usage = "usage: whip otel-export <instance> [--dry-run]";
    let mut instance_id = None;
    let mut dry_run = false;
    for arg in &options.args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            other if other.starts_with('-') => {
                eprintln!("unknown otel-export option `{other}`");
                return ExitCode::from(2);
            }
            value if instance_id.is_none() => instance_id = Some(value.to_owned()),
            _ => {
                eprintln!("{usage}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(instance_id) = instance_id else {
        eprintln!("{usage}");
        return ExitCode::from(2);
    };
    let store = match open_store_or_exit(options) {
        Ok(store) => store,
        Err(code) => return code,
    };
    let runs = match store.list_runs(&instance_id) {
        Ok(runs) => runs,
        Err(error) => return report_store_error("failed to list runs", error),
    };
    let effects = match store.list_effects(&instance_id) {
        Ok(effects) => effects,
        Err(error) => return report_store_error("failed to list effects", error),
    };
    drop(store);

    // Emit-once cursor: runs already exported are skipped; a crash mid-export
    // resumes from the cursor without duplication.
    let cursor_path = options.store_path.with_extension("otel-cursor.json");
    let mut cursor: Value = fs::read_to_string(&cursor_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    let exported = cursor
        .get(&instance_id)
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();

    let trace_id = format!(
        "{:032x}",
        u128::from(stable_hash(&instance_id)) << 64 | u128::from(stable_hash("trace"))
    );
    let service_name = env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "whipplescript".to_owned());
    let mut spans = Vec::new();
    let mut newly_exported = Vec::new();
    for run in &runs {
        // Only terminal runs export (a span needs an end); running work
        // exports on a later pass.
        if run.status == "running" || exported.contains(&run.run_id) {
            continue;
        }
        let effect = effects
            .iter()
            .find(|effect| effect.effect_id == run.effect_id);
        // Spans are named after source constructs so traces read like the
        // workflow; content stays structural (ids, kinds, statuses) per the
        // export content policy.
        let name = effect
            .map(|effect| {
                let rule = json_from_str(&effect.input_json)
                    .get("rule")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_default();
                if rule.is_empty() {
                    effect.kind.clone()
                } else {
                    format!("{}.{}", rule, effect.kind)
                }
            })
            .unwrap_or_else(|| "run".to_owned());
        let mut attributes = vec![
            otel_attr("whipplescript.instance_id", &instance_id),
            otel_attr("whipplescript.effect_id", &run.effect_id),
            otel_attr("whipplescript.run_id", &run.run_id),
            otel_attr("whipplescript.provider", &run.provider),
            otel_attr("whipplescript.effect.status", &run.status),
        ];
        if let Some(effect) = effect {
            attributes.push(otel_attr("whipplescript.effect.kind", &effect.kind));
            // GenAI semantic conventions (version-pinned in the spec) for
            // model-backed spans, so fleets land in LLM dashboards natively.
            if effect.kind == "agent.tell" || effect.kind == "baml.coerce" {
                attributes.push(otel_attr("gen_ai.system", &run.provider));
            }
        }
        spans.push(json!({
            "traceId": trace_id,
            "spanId": format!("{:016x}", stable_hash(&run.run_id)),
            "name": name,
            "kind": 1,
            "startTimeUnixNano": otel_nanos(&run.started_at),
            "endTimeUnixNano": otel_nanos(run.completed_at.as_deref().unwrap_or(&run.started_at)),
            "status": {"code": if run.status == "completed" { 1 } else { 2 }},
            "attributes": attributes,
        }));
        newly_exported.push(run.run_id.clone());
    }
    if spans.is_empty() {
        println!("otel-export {instance_id}: nothing new to export");
        return ExitCode::SUCCESS;
    }
    let payload = json!({
        "resourceSpans": [{
            "resource": {"attributes": [otel_attr("service.name", &service_name)]},
            "scopeSpans": [{
                "scope": {"name": "whipplescript", "version": whipplescript_core::version()},
                "spans": spans,
            }],
        }],
    });

    if dry_run {
        println!("{payload:#}");
        return ExitCode::SUCCESS;
    }
    let endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4318".to_owned());
    if let Err(error) = otel_post(&endpoint, &payload.to_string()) {
        // Failure isolation: the log persists; the exporter catches up on the
        // next pass. Nothing was marked exported.
        eprintln!("otel-export failed (will catch up next pass): {error}");
        return ExitCode::FAILURE;
    }
    let mut all = exported;
    all.extend(newly_exported.iter().cloned());
    cursor[&instance_id] = json!(all.into_iter().collect::<Vec<_>>());
    if let Err(error) = fs::write(&cursor_path, cursor.to_string()) {
        eprintln!("failed to persist otel cursor: {error}");
        return ExitCode::FAILURE;
    }
    println!(
        "otel-export {instance_id}: exported {} span(s) to {endpoint}",
        newly_exported.len()
    );
    ExitCode::SUCCESS
}

fn otel_attr(key: &str, value: &str) -> Value {
    json!({"key": key, "value": {"stringValue": value}})
}

/// `YYYY-MM-DD HH:MM:SS` (store timestamps) to Unix nanoseconds, best-effort.
fn otel_nanos(timestamp: &str) -> String {
    let normalized = timestamp.replace(' ', "T");
    let seconds = iso_like_to_unix_seconds(&normalized).unwrap_or(0);
    format!("{}", (seconds as i128) * 1_000_000_000)
}

fn iso_like_to_unix_seconds(value: &str) -> Option<i64> {
    let date = value.get(0..10)?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    let time = value.get(11..19).unwrap_or("00:00:00");
    let mut parts = time.split(':');
    let hour: i64 = parts.next()?.parse().ok()?;
    let minute: i64 = parts.next()?.parse().ok()?;
    let second: i64 = parts.next()?.parse().ok()?;
    // Civil-days algorithm (Howard Hinnant), valid for the store's range.
    let years = if month <= 2 { year - 1 } else { year };
    let era = years.div_euclid(400);
    let yoe = years - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Minimal OTLP/HTTP POST over plain HTTP — the sidecar's peer is a local
/// OpenTelemetry Collector, which owns TLS and backend fan-out.
fn otel_post(endpoint: &str, body: &str) -> Result<(), String> {
    use std::io::{Read, Write};
    let stripped = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| format!("only http:// endpoints are supported (got `{endpoint}`); point at a local OpenTelemetry Collector"))?;
    let host_port = stripped.split('/').next().unwrap_or(stripped);
    let address = if host_port.contains(':') {
        host_port.to_owned()
    } else {
        format!("{host_port}:4318")
    };
    let mut stream = std::net::TcpStream::connect(&address)
        .map_err(|error| format!("connect {address}: {error}"))?;
    let request = format!(
        "POST /v1/traces HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("send: {error}"))?;
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("");
    if status.starts_with('2') {
        Ok(())
    } else {
        Err(format!("collector responded {status}"))
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
    let artifact_counts = match artifact_counts_for_runs(&store, &runs) {
        Ok(counts) => counts,
        Err(error) => return report_store_error("failed to load artifacts", error),
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
        "runs": runs.iter().map(|run| run_to_json_with_lifecycle_and_artifacts(run, &events, &artifact_counts)).collect::<Vec<_>>(),
        "native_lifecycle": native_lifecycle_events(&events),
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
                let status = trace_effect_status(
                    payload
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("failed"),
                );
                if let Some(run_id) = payload.get("run_id").and_then(Value::as_str) {
                    push_trace_record(
                        &mut records,
                        TraceEvent::EffectTerminal {
                            run_id: run_id.to_owned(),
                            effect_id: effect_id.to_owned(),
                            status,
                        },
                    );
                } else if matches!(status, EffectStatus::Cancelled) {
                    push_trace_record(
                        &mut records,
                        TraceEvent::EffectCancelled {
                            effect_id: effect_id.to_owned(),
                        },
                    );
                }
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
            "workflow.revision_activated" => {
                let Some(revision_id) = payload.get("revision_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(from_version_id) = payload.get("from_version_id").and_then(Value::as_str)
                else {
                    continue;
                };
                let Some(to_version_id) = payload.get("to_version_id").and_then(Value::as_str)
                else {
                    continue;
                };
                let Some(from_epoch) = payload.get("from_epoch").and_then(Value::as_i64) else {
                    continue;
                };
                let Some(to_epoch) = payload.get("to_epoch").and_then(Value::as_i64) else {
                    continue;
                };
                let Some(cancellation_policy) =
                    payload.get("cancellation_policy").and_then(Value::as_str)
                else {
                    continue;
                };
                push_trace_record(
                    &mut records,
                    TraceEvent::RevisionActivated {
                        revision_id: revision_id.to_owned(),
                        from_version_id: from_version_id.to_owned(),
                        to_version_id: to_version_id.to_owned(),
                        from_epoch,
                        to_epoch,
                        cancellation_policy: cancellation_policy.to_owned(),
                        terminal_cancel_effects: trace_string_array(
                            payload.get("terminal_cancel_effects"),
                        ),
                        request_cancel_effects: trace_string_array(
                            payload.get("request_cancel_effects"),
                        ),
                    },
                );
            }
            "effect.cancellation_requested" => {
                let Some(effect_id) = payload.get("effect_id").and_then(Value::as_str) else {
                    continue;
                };
                let revision_id = match payload.get("revision_id") {
                    Some(Value::Null) | None => None,
                    Some(value) => match value.as_str() {
                        Some(revision_id) => Some(revision_id.to_owned()),
                        None => continue,
                    },
                };
                let reason = match payload.get("reason") {
                    Some(Value::Null) | None => None,
                    Some(value) => match value.as_str() {
                        Some(reason) => Some(reason.to_owned()),
                        None => continue,
                    },
                };
                let Some(requested_by) = payload.get("requested_by").and_then(Value::as_str) else {
                    continue;
                };
                push_trace_record(
                    &mut records,
                    TraceEvent::EffectCancellationRequested {
                        effect_id: effect_id.to_owned(),
                        revision_id,
                        reason,
                        requested_by: requested_by.to_owned(),
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

fn trace_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
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
            harden_store_directory(parent).map_err(|error| {
                format!(
                    "failed to set private permissions on store directory `{}`: {error}",
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

#[cfg(unix)]
fn harden_store_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    if permissions.mode() & 0o1000 != 0 {
        return Ok(());
    }
    if permissions.mode() & 0o777 != 0o700 {
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn harden_store_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
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
    for warning in &compiled.warnings {
        eprint!(
            "{}",
            render_diagnostic_with_severity(path, &bundle.source, warning, "warning")
        );
    }
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

/// Compile for static validation (`check`/`compile`): on top of ordinary
/// compilation this enforces the workflow liveness lints.
fn compile_source_path_for_validation(
    path: &str,
    root: Option<&str>,
) -> Result<(String, IrProgram), CompileFailure> {
    let (source, ir) = compile_source_path_with_root(path, root)?;
    let liveness = lint_workflow_liveness(&ir);
    if !liveness.is_empty() {
        return Err(CompileFailure::Diagnostics {
            source,
            diagnostics: liveness,
        });
    }
    Ok((source, ir))
}

/// Static liveness lints: every workflow must be able to terminate, and every
/// rule must have a satisfiable read set. Escape hatches: tag a workflow
/// `@service` when it intentionally runs forever, and tag a rule `@external`
/// when its facts arrive from outside the workflow (plugins, fixtures,
/// external systems).
fn lint_workflow_liveness(ir: &IrProgram) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let service_tagged = ir
        .source_tags
        .iter()
        .any(|tag| tag.target_kind == "workflow" && tag.name == "service");
    let has_terminal = ir.rules.iter().any(|rule| {
        rule.body
            .lines()
            .map(str::trim)
            .any(|line| line.starts_with("complete ") || line.starts_with("fail "))
    });
    if !has_terminal && !service_tagged {
        diagnostics.push(Diagnostic {
            span: SourceSpan { start: 0, end: 0 },
            message: format!(
                "workflow `{}` has no rule that reaches `complete` or `fail`",
                ir.workflow
            ),
            suggestion: Some(
                "add a rule that runs `complete <output> { ... }` or `fail <failure> { ... }`, or tag the workflow `@service` if it intentionally runs forever"
                    .to_owned(),
            ),
        });
    }

    let produced = ir
        .rules
        .iter()
        .flat_map(|rule| rule.metadata.fact_writes.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    let input_schemas = ir
        .workflow_contracts
        .iter()
        .filter(|contract| contract.kind == IrWorkflowContractKind::Input)
        .filter_map(|contract| match &contract.ty {
            IrType::Ref(name) => Some(name.clone()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let has_human_ask = ir.rules.iter().any(|rule| {
        rule.metadata
            .effects
            .iter()
            .any(|effect| effect.kind == IrEffectKind::HumanAsk)
    });
    let has_tell = ir.rules.iter().any(|rule| {
        rule.metadata
            .effects
            .iter()
            .any(|effect| effect.kind == IrEffectKind::AgentTell)
    });

    for rule in &ir.rules {
        let external_tagged = ir.source_tags.iter().any(|tag| {
            tag.target_kind == "rule" && tag.target == rule.name && tag.name == "external"
        });
        if external_tagged {
            continue;
        }
        for when in &rule.whens {
            let pattern = when.pattern.as_str();
            if pattern == "started" || pattern.ends_with(" is available") {
                continue;
            }
            // General form: `when fact <name> as x`. Sugar phrases reduce to
            // the same runtime fact names.
            let general_name = pattern
                .strip_prefix("fact ")
                .and_then(|rest| rest.split_whitespace().next());
            let pattern = match general_name {
                Some("human.answer.received") => "human answered",
                Some("agent.turn.completed") => "worker completed turn",
                Some(name) if name.contains('.') => {
                    // Other dotted runtime facts arrive from outside the
                    // workflow's own rules; the lint cannot see producers.
                    continue;
                }
                Some(name) => {
                    // `fact ClassName` is an ordinary class match.
                    let read = format!("schema:{name}");
                    if !produced.contains(&read) && !input_schemas.contains(name) {
                        diagnostics.push(Diagnostic {
                            span: when.span,
                            message: format!(
                                "rule `{}` can never fire: nothing produces `{name}`",
                                rule.name
                            ),
                            suggestion: Some(format!(
                                "seed `{name}` from a table, record it in another rule, declare it as a workflow input, or tag the rule `@external` if it arrives from an external system"
                            )),
                        });
                    }
                    continue;
                }
                None => pattern,
            };
            if pattern.starts_with("human answered") {
                if !has_human_ask {
                    diagnostics.push(Diagnostic {
                        span: when.span,
                        message: format!(
                            "rule `{}` can never fire: no rule creates an `askHuman` request",
                            rule.name
                        ),
                        suggestion: Some(
                            "add an `askHuman` effect, or tag the rule `@external` if answers arrive from outside this workflow"
                                .to_owned(),
                        ),
                    });
                }
                continue;
            }
            if pattern.starts_with("worker completed turn") {
                if !has_tell {
                    diagnostics.push(Diagnostic {
                        span: when.span,
                        message: format!(
                            "rule `{}` can never fire: no rule creates an agent turn",
                            rule.name
                        ),
                        suggestion: Some(
                            "add a `tell` effect, or tag the rule `@external` if turns arrive from outside this workflow"
                                .to_owned(),
                        ),
                    });
                }
                continue;
            }
            let first = pattern.split_whitespace().next().unwrap_or_default();
            if !first.chars().next().is_some_and(char::is_uppercase) {
                // Remaining lowercase patterns (loft, manual review) are fed
                // by external systems the lint cannot see.
                continue;
            }
            let builtin = matches!(
                first,
                "AgentTurn"
                    | "LoftIssue"
                    | "LoftClaim"
                    | "WorkItem"
                    | "Evidence"
                    | "HumanAnswer"
                    | "TerminalFailed"
                    | "TerminalTimedOut"
                    | "TerminalCancelled"
            );
            if builtin
                || produced.contains(&format!("schema:{first}"))
                || input_schemas.contains(first)
            {
                continue;
            }
            diagnostics.push(Diagnostic {
                span: when.span,
                message: format!(
                    "rule `{}` can never fire: nothing produces `{first}`",
                    rule.name
                ),
                suggestion: Some(format!(
                    "seed `{first}` from a table, record it in another rule, declare it as a workflow input, or tag the rule `@external` if it arrives from an external system"
                )),
            });
        }
    }

    diagnostics
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
                "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  effect({upstream}, {terminal}) dep({upstream}, {predicate}, {downstream}) effect({downstream}, blocked)\n  =>*\n  effect({upstream}, {terminal}) dep({upstream}, {predicate}, {downstream}) effect({downstream}, running) .\n\n"
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
                    "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  effect({upstream}, {non_terminal}) dep({upstream}, {predicate}, {downstream}) effect({downstream}, blocked)\n  =>*\n  effect({upstream}, {non_terminal}) dep({upstream}, {predicate}, {downstream}) effect({downstream}, running) .\n\n"
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
        append_revision_model_searches(
            source,
            &mut output,
            &mut expected,
            rule,
            &rule_symbols,
            &fact_symbols,
            &graph_symbols,
            &effect_symbols,
        );
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

#[allow(clippy::too_many_arguments)]
fn append_revision_model_searches(
    source: &str,
    output: &mut String,
    expected: &mut Vec<ExpectedSearch>,
    rule: &IrRule,
    rule_symbols: &BTreeMap<String, String>,
    fact_symbols: &BTreeMap<String, String>,
    graph_symbols: &BTreeMap<String, String>,
    effect_symbols: &BTreeMap<String, String>,
) {
    let Some(first_when) = rule.whens.first() else {
        return;
    };
    let Some(first_effect) = rule.metadata.effects.first() else {
        return;
    };
    let Some(rule_symbol) = rule_symbols.get(&rule.name) else {
        return;
    };
    let fact_key = rule_fact_key(&rule.name, 0, &first_when.pattern);
    let Some(fact_symbol) = fact_symbols.get(&fact_key) else {
        return;
    };
    let graph_key = rule_graph_key(&rule.name, 0);
    let Some(graph_symbol) = graph_symbols.get(&graph_key) else {
        return;
    };
    let first_effect_key = effect_key(&rule.name, &first_effect.id);
    let Some(effect_symbol) = effect_symbols.get(&first_effect_key) else {
        return;
    };

    output.push_str(&format!(
        "--- {}: revision-scoped rule commits after activation on the active revision.\n",
        rule.name
    ));
    output.push_str(&format!(
        "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  instance(rootInstance, rootWorkflow, instRunning) activeRevision(rootInstance, version1, epoch0) nextEpoch(epoch0, epoch1) activateRevision(rootInstance, version1, version2, epoch0, epoch1, keep) fact({fact_symbol}) scopedRuleV(rootInstance, version2, epoch1, {rule_symbol}, {fact_symbol}, {graph_symbol}) ruleReady({rule_symbol}, {fact_symbol}, {graph_symbol}) graphV({graph_symbol}, version2, epoch1, {effect_symbol})\n  =>*\n  instance(rootInstance, rootWorkflow, instRunning) activeRevision(rootInstance, version2, epoch1) nextEpoch(epoch0, epoch1) revisionActivated(rootInstance, version1, version2, epoch1, keep) revisionCancellationPolicy(rootInstance, version1, epoch0, keep) event(revisionActivatedEvt) fact({fact_symbol}) scopedRuleV(rootInstance, version2, epoch1, {rule_symbol}, {fact_symbol}, {graph_symbol}) scopedRuleFired(rootInstance, {rule_symbol}, {fact_symbol}, {graph_symbol}) event(ruleCommitEvt) graphCommitted({graph_symbol}) effect({effect_symbol}, queued) effectVersion({effect_symbol}, version2, epoch1) .\n\n"
    ));
    expected.push(ExpectedSearch {
        outcome: ExpectedSearchResult::Solution,
        span: first_when.span,
        description: format!("{} active revision scoped rule commits", rule.name),
        upstream: rule.name.clone(),
        predicate: "revision-active-rule",
        downstream: first_effect.id.clone(),
    });

    output.push_str(&format!(
        "--- {}: stale revision-scoped rule cannot commit after a newer revision is active.\n",
        rule.name
    ));
    output.push_str(&format!(
        "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  instance(rootInstance, rootWorkflow, instRunning) activeRevision(rootInstance, version2, epoch1) fact({fact_symbol}) scopedRuleV(rootInstance, version1, epoch0, {rule_symbol}, {fact_symbol}, {graph_symbol}) ruleReady({rule_symbol}, {fact_symbol}, {graph_symbol}) graphV({graph_symbol}, version1, epoch0, {effect_symbol})\n  =>*\n  instance(rootInstance, rootWorkflow, instRunning) activeRevision(rootInstance, version2, epoch1) fact({fact_symbol}) scopedRuleV(rootInstance, version1, epoch0, {rule_symbol}, {fact_symbol}, {graph_symbol}) scopedRuleFired(rootInstance, {rule_symbol}, {fact_symbol}, {graph_symbol}) event(ruleCommitEvt) graphCommitted({graph_symbol}) effect({effect_symbol}, queued) effectVersion({effect_symbol}, version1, epoch0) .\n\n"
    ));
    expected.push(ExpectedSearch {
        outcome: ExpectedSearchResult::NoSolution,
        span: first_when.span,
        description: format!("{} stale revision scoped rule cannot commit", rule.name),
        upstream: rule.name.clone(),
        predicate: "revision-stale-rule",
        downstream: first_effect.id.clone(),
    });

    output.push_str(&format!(
        "--- {}: revision activation does not rewrite old effect attribution.\n",
        rule.name
    ));
    output.push_str(&format!(
        "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  instance(rootInstance, rootWorkflow, instRunning) activeRevision(rootInstance, version1, epoch0) nextEpoch(epoch0, epoch1) activateRevision(rootInstance, version1, version2, epoch0, epoch1, keep) effect({effect_symbol}, queued) effectVersion({effect_symbol}, version1, epoch0)\n  =>*\n  instance(rootInstance, rootWorkflow, instRunning) activeRevision(rootInstance, version2, epoch1) nextEpoch(epoch0, epoch1) revisionActivated(rootInstance, version1, version2, epoch1, keep) revisionCancellationPolicy(rootInstance, version1, epoch0, keep) event(revisionActivatedEvt) effect({effect_symbol}, queued) effectVersion({effect_symbol}, version2, epoch1) .\n\n"
    ));
    expected.push(ExpectedSearch {
        outcome: ExpectedSearchResult::NoSolution,
        span: first_effect.span,
        description: format!("{} old effect keeps revision attribution", first_effect.id),
        upstream: first_effect.id.clone(),
        predicate: "revision-effect-attribution",
        downstream: "effectVersion".to_owned(),
    });

    for dependency in &rule.metadata.dependencies {
        if !matches!(&dependency.predicate, IrDependencyPredicate::Completes) {
            continue;
        }
        let upstream_key = effect_key(&rule.name, &dependency.upstream);
        let downstream_key = effect_key(&rule.name, &dependency.downstream);
        let Some(upstream) = effect_symbols.get(&upstream_key) else {
            continue;
        };
        let Some(downstream) = effect_symbols.get(&downstream_key) else {
            continue;
        };
        let span = dependency_source_span(source, &dependency.upstream, "completes");
        output.push_str(&format!(
            "--- {}: revision-cancelled upstream releases `completes` dependencies.\n",
            rule.name
        ));
        output.push_str(&format!(
            "search [1] in WHIPPLESCRIPT-GENERATED-CHECK :\n  instance(rootInstance, rootWorkflow, instRunning) activeRevision(rootInstance, version1, epoch0) nextEpoch(epoch0, epoch1) activateRevision(rootInstance, version1, version2, epoch0, epoch1, cancelQueued) effect({upstream}, queued) effectVersion({upstream}, version1, epoch0) dep({upstream}, completes, {downstream}) effect({downstream}, blocked) effectVersion({downstream}, version1, epoch0)\n  =>*\n  instance(rootInstance, rootWorkflow, instRunning) activeRevision(rootInstance, version2, epoch1) nextEpoch(epoch0, epoch1) revisionActivated(rootInstance, version1, version2, epoch1, cancelQueued) revisionCancellationPolicy(rootInstance, version1, epoch0, cancelQueued) event(revisionActivatedEvt) effect({upstream}, cancelled) effectVersion({upstream}, version1, epoch0) dep({upstream}, completes, {downstream}) effect({downstream}, queued) effectVersion({downstream}, version1, epoch0) .\n\n"
        ));
        expected.push(ExpectedSearch {
            outcome: ExpectedSearchResult::Solution,
            span,
            description: format!(
                "{} --completes--> {} releases after revision cancellation",
                dependency.upstream, dependency.downstream
            ),
            upstream: dependency.upstream.clone(),
            predicate: "revision-completes-cancelled",
            downstream: dependency.downstream.clone(),
        });
    }
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
                maude_scalar_expr(left, context),
                maude_scalar_expr(right, context)
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

/// Applies `cancel <binding>` operations committed by a rule: pending
/// effects terminal-cancel; running effects get a cancellation request (a
/// request, not a result); already-terminal effects are a recorded no-op.
fn apply_rule_cancels(
    store_path: &Path,
    instance_id: &str,
    rule_name: &str,
    effect_ids: &[String],
    causation_event_id: &str,
) -> Result<(), StoreError> {
    for effect_id in effect_ids {
        let store = SqliteStore::open(store_path)?;
        let status = store
            .list_effects(instance_id)?
            .into_iter()
            .find(|effect| &effect.effect_id == effect_id)
            .map(|effect| effect.status);
        drop(store);
        match status.as_deref() {
            Some("running") => {
                let mut store = SqliteStore::open(store_path)?;
                let _ = store.request_effect_cancellation(EffectCancellationRequest {
                    instance_id,
                    effect_id,
                    revision_id: None,
                    reason: Some("cancelled by rule"),
                    requested_by: rule_name,
                    causation_event_id: Some(causation_event_id),
                    idempotency_key: Some(&idempotency_key(&[
                        instance_id,
                        effect_id,
                        rule_name,
                        "rule-cancel-request",
                    ])),
                });
            }
            Some("completed") | Some("failed") | Some("timed_out") | Some("cancelled") => {
                // No-op with evidence: cancelling settled work is legal.
                let store = SqliteStore::open(store_path)?;
                store.record_diagnostic(DiagnosticRecord {
                    instance_id: Some(instance_id),
                    program_id: None,
                    program_version_id: None,
                    severity: "info",
                    code: Some("cancel.noop"),
                    message: &format!(
                        "rule `{rule_name}` cancelled effect `{effect_id}` after it reached a terminal status"
                    ),
                    source_span_json: None,
                    subject_type: Some("effect"),
                    subject_id: Some(effect_id),
                    event_id: Some(causation_event_id),
                    effect_id: Some(effect_id),
                    run_id: None,
                    assertion_id: None,
                    evidence_ids_json: "[]",
                    artifact_ids_json: "[]",
                    causation_id: Some(causation_event_id),
                    correlation_id: None,
                    idempotency_key: Some(&idempotency_key(&[
                        instance_id,
                        effect_id,
                        rule_name,
                        "rule-cancel-noop",
                    ])),
                })?;
            }
            Some(_) => {
                let store = SqliteStore::open(store_path)?;
                let mut kernel = RuntimeKernel::new(store);
                kernel.cancel_effect(EffectCancellation {
                    instance_id,
                    effect_id,
                    reason: Some("cancelled by rule"),
                    idempotency_key: Some(&idempotency_key(&[
                        instance_id,
                        effect_id,
                        rule_name,
                        "rule-cancel",
                    ])),
                })?;
            }
            None => {}
        }
    }
    Ok(())
}

fn report_store_error(context: &str, error: StoreError) -> ExitCode {
    eprintln!("{context}: {}", store_error(error));
    ExitCode::FAILURE
}

fn store_error(error: StoreError) -> String {
    match error {
        StoreError::Io(error) => format!("store I/O error: {error}"),
        StoreError::Sqlite(error) => {
            format!("internal store error ({error}); this is a whip bug, please report it")
        }
        StoreError::Json(error) => format!(
            "internal store error (malformed stored JSON: {error}); this is a whip bug, please report it"
        ),
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
        TraceEvent::RevisionActivated {
            revision_id,
            from_version_id,
            to_version_id,
            from_epoch,
            to_epoch,
            cancellation_policy,
            terminal_cancel_effects,
            request_cancel_effects,
        } => json!({
            "type": "revision_activated",
            "revision_id": revision_id,
            "from_version_id": from_version_id,
            "to_version_id": to_version_id,
            "from_epoch": from_epoch,
            "to_epoch": to_epoch,
            "cancellation_policy": cancellation_policy,
            "terminal_cancel_effects": terminal_cancel_effects,
            "request_cancel_effects": request_cancel_effects,
        }),
        TraceEvent::EffectCancellationRequested {
            effect_id,
            revision_id,
            reason,
            requested_by,
        } => json!({
            "type": "effect_cancellation_requested",
            "effect_id": effect_id,
            "revision_id": revision_id,
            "reason": reason,
            "requested_by": requested_by,
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
        "program_version_id": fact.program_version_id,
        "revision_epoch": fact.revision_epoch,
        "name": fact.name,
        "key": fact.key,
        "value": json_from_str(&fact.value_json),
        "provenance_class": fact.provenance_class,
        "source_span": fact.source_span_json.as_deref().map(json_from_str),
    })
}

fn effect_to_json(effect: &EffectView) -> Value {
    json!({
        "effect_id": effect.effect_id,
        "kind": effect.kind,
        "target": effect.target,
        "provider_selection": effect_provider_selection_json(effect).unwrap_or(Value::Null),
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

fn effect_provider_selection_json(effect: &EffectView) -> Option<Value> {
    if effect.kind != "agent.tell" {
        return None;
    }
    let agent = effect.target.as_deref()?;
    let declared = serde_json::from_str::<Value>(&effect.declared_profiles_json).ok()?;
    if let Some(harness) = declared_agent_harness_in_value(&declared, agent) {
        let kind = declared_harness_kind_in_value(&declared, &harness)?;
        return Some(json!({
            "source_harness_id": harness,
            "provider_id": harness,
            "provider_kind": kind,
        }));
    }
    let provider = declared_agent_provider_in_value(&declared, agent)?;
    Some(json!({
        "source_harness_id": Value::Null,
        "provider_id": provider,
        "provider_kind": provider,
    }))
}

fn run_to_json(run: &RunView) -> Value {
    json!({
        "run_id": run.run_id,
        "effect_id": run.effect_id,
        "provider": run.provider,
        "provider_selection": run_provider_selection_json(run),
        "worker_id": run.worker_id,
        "status": run.status,
        "started_at": run.started_at,
        "completed_at": run.completed_at,
        "cancel_requested": run.cancel_requested,
    })
}

fn run_provider_selection_json(run: &RunView) -> Value {
    let metadata = json_from_str(&run.metadata_json);
    if let Some(selection) = metadata.get("provider_selection") {
        return selection.clone();
    }
    let native_provider = metadata.get("native_provider");
    if let Some(native_provider) = native_provider {
        return json!({
            "provider_id": native_provider
                .get("provider_id")
                .cloned()
                .unwrap_or_else(|| Value::String(run.provider.clone())),
            "provider_kind": native_provider
                .get("provider_kind")
                .cloned()
                .unwrap_or(Value::Null),
            "surface": native_provider
                .get("surface")
                .cloned()
                .unwrap_or(Value::Null),
        });
    }
    json!({
        "provider_id": run.provider,
    })
}

fn run_to_json_with_lifecycle_and_artifacts(
    run: &RunView,
    events: &[EventView],
    artifact_counts: &BTreeMap<String, usize>,
) -> Value {
    let mut value = run_to_json(run);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "native_lifecycle".to_owned(),
            latest_native_lifecycle_for_run(events, &run.run_id).unwrap_or(Value::Null),
        );
        object.insert(
            "artifact_count".to_owned(),
            json!(artifact_counts.get(&run.run_id).copied().unwrap_or(0)),
        );
    }
    value
}

fn artifact_counts_for_runs(
    store: &SqliteStore,
    runs: &[RunView],
) -> Result<BTreeMap<String, usize>, StoreError> {
    let mut counts = BTreeMap::new();
    for run in runs {
        counts.insert(
            run.run_id.clone(),
            store.list_artifacts_for_run(&run.run_id)?.len(),
        );
    }
    Ok(counts)
}

fn artifacts_for_runs(
    store: &SqliteStore,
    runs: &[RunView],
) -> Result<Vec<ArtifactView>, StoreError> {
    let mut artifacts = Vec::new();
    for run in runs {
        artifacts.extend(store.list_artifacts_for_run(&run.run_id)?);
    }
    Ok(artifacts)
}

fn latest_native_lifecycle_for_run(events: &[EventView], run_id: &str) -> Option<Value> {
    events.iter().rev().find_map(|event| {
        let lifecycle = native_lifecycle_event_to_json(event)?;
        (lifecycle.get("run_id").and_then(Value::as_str) == Some(run_id)).then_some(lifecycle)
    })
}

fn native_lifecycle_events(events: &[EventView]) -> Value {
    Value::Array(
        events
            .iter()
            .filter_map(native_lifecycle_event_to_json)
            .collect(),
    )
}

fn native_lifecycle_event_to_json(event: &EventView) -> Option<Value> {
    if !event.event_type.starts_with("agent.turn.") {
        return None;
    }
    let payload = json_from_str(&event.payload_json);
    let provider_event_type = payload.get("provider_event_type")?;
    Some(json!({
        "event_id": event.event_id,
        "sequence": event.sequence,
        "event_type": event.event_type,
        "run_id": payload.get("run_id").cloned().unwrap_or(Value::Null),
        "effect_id": payload.get("effect_id").cloned().unwrap_or(Value::Null),
        "provider": payload.get("provider").cloned().unwrap_or(Value::Null),
        "status": payload.get("status").cloned().unwrap_or(Value::Null),
        "terminal": payload.get("terminal").cloned().unwrap_or(Value::Bool(false)),
        "provider_event_type": provider_event_type.clone(),
        "provider_session_id": payload.get("provider_session_id").cloned().unwrap_or(Value::Null),
        "provider_turn_id": payload.get("provider_turn_id").cloned().unwrap_or(Value::Null),
        "evidence_id": payload.get("evidence_id").cloned().unwrap_or(Value::Null),
    }))
}

fn workflow_invocation_to_json(invocation: &WorkflowInvocationView) -> Value {
    json!({
        "invocation_id": invocation.invocation_id,
        "parent_instance_id": invocation.parent_instance_id,
        "parent_effect_id": invocation.parent_effect_id,
        "parent_program_version_id": invocation.parent_program_version_id,
        "parent_revision_epoch": invocation.parent_revision_epoch,
        "parent_active_program_version_id": invocation.parent_active_program_version_id,
        "parent_active_revision_epoch": invocation.parent_active_revision_epoch,
        "child_instance_id": invocation.child_instance_id,
        "child_program_version_id": invocation.child_program_version_id,
        "child_revision_epoch": invocation.child_revision_epoch,
        "child_active_program_version_id": invocation.child_active_program_version_id,
        "child_active_revision_epoch": invocation.child_active_revision_epoch,
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

fn revision_compatibility_to_json(report: &RevisionCompatibilityReport) -> Value {
    json!({
        "instance_id": report.instance_id,
        "active_version_id": report.active_version_id,
        "candidate_version_id": report.candidate_version_id,
        "compatible": report.compatible,
        "diagnostics": report
            .diagnostics
            .iter()
            .map(revision_compatibility_diagnostic_to_json)
            .collect::<Vec<_>>(),
    })
}

fn revision_compatibility_diagnostic_to_json(
    diagnostic: &RevisionCompatibilityDiagnostic,
) -> Value {
    json!({
        "code": diagnostic.code,
        "message": diagnostic.message,
        "subject": diagnostic.subject,
        "source_span": diagnostic.source_span_json.as_deref().map(json_from_str),
    })
}

fn revision_cancellation_impact_to_json(impact: &RevisionCancellationImpact) -> Value {
    json!({
        "instance_id": impact.instance_id,
        "active_version_id": impact.active_version_id,
        "active_revision_epoch": impact.active_revision_epoch,
        "cancellation_policy": impact.cancellation_policy,
        "terminal_cancel_effects": impact.terminal_cancel_effects,
        "request_cancel_effects": impact.request_cancel_effects,
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RevisionAgentImpact {
    removed_agents_affecting_effects: Vec<RevisionRemovedAgentImpact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RevisionRemovedAgentImpact {
    agent: String,
    effect_id: String,
    status: String,
    program_version_id: Option<String>,
    revision_epoch: i64,
}

fn revision_agent_impact(
    store: &SqliteStore,
    instance_id: &str,
    candidate: &IrProgram,
) -> Result<RevisionAgentImpact, StoreError> {
    let candidate_agents = candidate
        .agents
        .iter()
        .map(|agent| agent.name.as_str())
        .collect::<BTreeSet<_>>();
    let removed_agents_affecting_effects = store
        .list_effects(instance_id)?
        .into_iter()
        .filter(|effect| effect.kind == "agent.tell")
        .filter(|effect| !effect_status_is_terminal(&effect.status))
        .filter_map(|effect| {
            let agent = effect.target.as_deref()?;
            (!candidate_agents.contains(agent)).then(|| RevisionRemovedAgentImpact {
                agent: agent.to_owned(),
                effect_id: effect.effect_id,
                status: effect.status,
                program_version_id: effect.program_version_id,
                revision_epoch: effect.revision_epoch,
            })
        })
        .collect();
    Ok(RevisionAgentImpact {
        removed_agents_affecting_effects,
    })
}

fn effect_status_is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "timed_out" | "cancelled")
}

fn revision_agent_impact_to_json(impact: &RevisionAgentImpact) -> Value {
    json!({
        "removed_agents_affecting_effects": impact
            .removed_agents_affecting_effects
            .iter()
            .map(revision_removed_agent_impact_to_json)
            .collect::<Vec<_>>(),
    })
}

fn revision_removed_agent_impact_to_json(impact: &RevisionRemovedAgentImpact) -> Value {
    json!({
        "agent": impact.agent,
        "effect_id": impact.effect_id,
        "status": impact.status,
        "program_version_id": impact.program_version_id,
        "revision_epoch": impact.revision_epoch,
    })
}

#[allow(clippy::too_many_arguments)]
fn revision_report_json(
    dry_run: bool,
    revise_options: &ReviseOptions,
    ir: &IrProgram,
    source_hash: &str,
    ir_hash: &str,
    compatibility: &RevisionCompatibilityReport,
    impact: &RevisionCancellationImpact,
    agent_impact: &RevisionAgentImpact,
    revision: Option<&WorkflowRevisionView>,
) -> Value {
    json!({
        "dry_run": dry_run,
        "instance_id": revise_options.instance_id,
        "source_path": display_path(&revise_options.program_path),
        "root_workflow": ir.workflow,
        "candidate_source_hash": source_hash,
        "candidate_version_hash": ir_hash,
        "compatibility": revision_compatibility_to_json(compatibility),
        "cancellation": revision_cancellation_impact_to_json(impact),
        "agent_impact": revision_agent_impact_to_json(agent_impact),
        "would_create": revision_would_create_to_json(
            dry_run,
            revise_options,
            ir,
            source_hash,
            ir_hash,
            compatibility,
            impact,
            agent_impact
        ),
        "would_activate": dry_run && compatibility.compatible,
        "revision": revision.map(workflow_revision_to_json),
        "next": revision.map(|revision| {
            format!("whip status {} --json", revision.instance_id)
        }),
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_revision_dry_run(
    options: &CliOptions,
    revise_options: &ReviseOptions,
    ir: &IrProgram,
    source_hash: &str,
    ir_hash: &str,
    compatibility: &RevisionCompatibilityReport,
    impact: &RevisionCancellationImpact,
    agent_impact: &RevisionAgentImpact,
) -> ExitCode {
    emit_revision_report(
        options,
        revise_options,
        ir,
        source_hash,
        ir_hash,
        compatibility,
        impact,
        agent_impact,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_revision_report(
    options: &CliOptions,
    revise_options: &ReviseOptions,
    ir: &IrProgram,
    source_hash: &str,
    ir_hash: &str,
    compatibility: &RevisionCompatibilityReport,
    impact: &RevisionCancellationImpact,
    agent_impact: &RevisionAgentImpact,
    revision: Option<&WorkflowRevisionView>,
) -> ExitCode {
    let dry_run = revise_options.dry_run;
    if options.json {
        return emit_json(revision_report_json(
            dry_run,
            revise_options,
            ir,
            source_hash,
            ir_hash,
            compatibility,
            impact,
            agent_impact,
            revision,
        ));
    }

    if let Some(revision) = revision {
        println!(
            "revision {} activated epoch={} from={} to={}",
            revision.revision_id, revision.epoch, revision.from_version_id, revision.to_version_id
        );
        println!("next: whip status {} --json", revision.instance_id);
    } else {
        println!(
            "revision dry-run: {}",
            if compatibility.compatible {
                "compatible"
            } else {
                "blocked"
            }
        );
        println!(
            "active_version={} active_epoch={} candidate_hash={}",
            compatibility.active_version_id, impact.active_revision_epoch, ir_hash
        );
    }
    println!(
        "cancel_policy={} terminal_cancel={} request_cancel={}",
        impact.cancellation_policy,
        impact.terminal_cancel_effects.len(),
        impact.request_cancel_effects.len()
    );
    println!(
        "agent_impact removed_agents_affecting_effects={}",
        agent_impact.removed_agents_affecting_effects.len()
    );
    if dry_run {
        println!(
            "would_create diagnostics={} evidence=1",
            compatibility.diagnostics.len()
        );
    }
    for removed in &agent_impact.removed_agents_affecting_effects {
        println!(
            "removed_agent {} effect={} status={} version={} epoch={}",
            removed.agent,
            removed.effect_id,
            removed.status,
            removed.program_version_id.as_deref().unwrap_or("-"),
            removed.revision_epoch
        );
    }
    for diagnostic in &compatibility.diagnostics {
        println!("diagnostic {}: {}", diagnostic.code, diagnostic.message);
    }
    ExitCode::SUCCESS
}

#[allow(clippy::too_many_arguments)]
fn revision_would_create_to_json(
    dry_run: bool,
    revise_options: &ReviseOptions,
    ir: &IrProgram,
    source_hash: &str,
    ir_hash: &str,
    compatibility: &RevisionCompatibilityReport,
    impact: &RevisionCancellationImpact,
    agent_impact: &RevisionAgentImpact,
) -> Value {
    if !dry_run {
        return json!({
            "diagnostics": [],
            "evidence": [],
        });
    }

    json!({
        "diagnostics": compatibility
            .diagnostics
            .iter()
            .map(|diagnostic| {
                json!({
                    "severity": "error",
                    "code": diagnostic.code,
                    "message": diagnostic.message,
                    "source_span": diagnostic.source_span_json.as_deref().map(json_from_str),
                    "subject": diagnostic.subject,
                    "subject_type": "revision",
                    "subject_id": revise_options.instance_id,
                })
            })
            .collect::<Vec<_>>(),
        "evidence": [
            {
                "kind": "workflow.revision.dry_run",
                "subject_type": "instance",
                "subject_id": revise_options.instance_id,
                "summary": if compatibility.compatible {
                    "revision dry-run compatible"
                } else {
                    "revision dry-run blocked"
                },
                "metadata": {
                    "source_path": display_path(&revise_options.program_path),
                    "root_workflow": ir.workflow,
                    "candidate_source_hash": source_hash,
                    "candidate_version_hash": ir_hash,
                    "compatible": compatibility.compatible,
                    "cancellation_policy": impact.cancellation_policy,
                    "terminal_cancel_count": impact.terminal_cancel_effects.len(),
                    "request_cancel_count": impact.request_cancel_effects.len(),
                    "removed_agent_impact_count": agent_impact.removed_agents_affecting_effects.len(),
                },
            }
        ],
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

fn artifact_to_json(artifact: &ArtifactView) -> Value {
    json!({
        "artifact_id": artifact.artifact_id,
        "run_id": artifact.run_id,
        "kind": artifact.kind,
        "path": redact_cli_metadata(&artifact.path),
        "content_hash": artifact.content_hash.as_deref().map(redact_cli_metadata),
        "mime_type": artifact.mime_type,
        "created_at": artifact.created_at,
    })
}

fn redact_cli_metadata(value: &str) -> String {
    value
        .split_whitespace()
        .map(|token| {
            if token_has_secret_pattern(token) {
                "[REDACTED]".to_owned()
            } else if let Some((key, _)) = token.split_once('=') {
                if key_is_sensitive(key) {
                    format!("{key}=[REDACTED]")
                } else {
                    token.to_owned()
                }
            } else {
                token.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn key_is_sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "api_key",
        "apikey",
        "credential",
        "password",
        "secret",
        "token",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn token_has_secret_pattern(token: &str) -> bool {
    let token = token.trim_matches(|ch: char| {
        matches!(ch, '"' | '\'' | ',' | ';' | ':' | ')' | '(' | '[' | ']')
    });
    token.contains("github_pat_")
        || token
            .find("sk-")
            .is_some_and(|start| token[start..].len() >= 20)
        || token
            .find("ghp_")
            .is_some_and(|start| token[start..].len() >= 20)
        || token
            .find("AKIA")
            .is_some_and(|start| token[start..].len() >= 20)
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
        "native_lifecycle": native_lifecycle_events(&status.recent_events),
        "recent_events": status.recent_events.iter().map(event_to_json).collect::<Vec<_>>(),
    })
}

fn status_to_json_with_effects_and_runs(
    status: &StatusView,
    effects: &[EffectView],
    runs: &[RunView],
) -> Value {
    let mut value = status_to_json(status);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "effects".to_owned(),
            Value::Array(effects.iter().map(effect_to_json).collect()),
        );
        object.insert(
            "runs".to_owned(),
            Value::Array(runs.iter().map(run_to_json).collect()),
        );
    }
    value
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
    render_diagnostic_with_severity(path, source, diagnostic, "error")
}

fn render_diagnostic_with_severity(
    path: &str,
    source: &str,
    diagnostic: &Diagnostic,
    severity: &str,
) -> String {
    let location = locate_span(source, diagnostic.span);
    let gutter_width = location.line.to_string().len();
    let underline = underline_for_span(&location, diagnostic.span);
    let mut rendered = String::new();

    rendered.push_str(severity);
    rendered.push_str(": ");
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
            program_version_id: None,
            revision_epoch: 0,
            name: "Window".to_owned(),
            key: "fact-invalid-duration".to_owned(),
            value_json: r#"{"elapsed":"bad-duration","limit":"PT1H"}"#.to_owned(),
            provenance_class: "external".to_owned(),
            source_span_json: None,
        }];

        let assertions = eval_assertions(&ir, &facts, &[], None, &AssertionTagFilter::default());

        assert_eq!(assertions.len(), 1);
        assert_eq!(assertions[0].status, AssertionStatus::Error);
        assert!(assertions[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("invalid duration value `bad-duration`")));
        assert!(assertions[0]
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("invalid duration value `bad-duration`")));
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
            program_version_id: None,
            revision_epoch: 0,
            name: "Task".to_owned(),
            key: "Task:queued".to_owned(),
            value_json: r#"{"status":"queued"}"#.to_owned(),
            provenance_class: "rule".to_owned(),
            source_span_json: None,
        };
        let facts = vec![fact];
        let effects = Vec::new();
        let ready = ready_contexts(&ir, &ir.rules[0], &facts, &effects, None);
        assert_eq!(ready.contexts.len(), 1);

        let lowering = lower_rule(
            "ins_test",
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
    fn lowering_preserves_multiline_prompt_content_type_metadata() {
        let source = r#"
workflow PromptContentType

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start
  when started
=> {
  tell worker as turn """markdown
  Write a short report.
  """
}
"#;
        let ir = whipplescript_parser::compile_program(source)
            .ir
            .expect("compile");
        let lowering = lower_rule(
            "ins_test",
            &ir,
            &ir.rules[0],
            &RuleContext::default(),
            &[],
            &[],
            None,
        );

        assert_eq!(lowering.effects.len(), 1);
        let input = json_from_str(&lowering.effects[0].input_json);
        assert_eq!(
            input.get("prompt").and_then(Value::as_str),
            Some("Write a short report.")
        );
        assert_eq!(
            input.get("prompt_content_type").and_then(Value::as_str),
            Some("markdown")
        );
    }

    #[test]
    fn lowering_preserves_coerce_prompt_content_type_metadata() {
        let source = r#"
workflow CoercePromptContentType

class Review {
  accepted bool
}

coerce reviewArtifact() -> Review {
  prompt """markdown
  Review the artifact.
  """
}

rule start
  when started
=> {
  coerce reviewArtifact() as review
}
"#;
        let ir = whipplescript_parser::compile_program(source)
            .ir
            .expect("compile");
        let lowering = lower_rule(
            "ins_test",
            &ir,
            &ir.rules[0],
            &RuleContext::default(),
            &[],
            &[],
            None,
        );

        assert_eq!(lowering.effects.len(), 1);
        let input = json_from_str(&lowering.effects[0].input_json);
        assert_eq!(
            input.get("prompt_template").and_then(Value::as_str),
            Some("Review the artifact.")
        );
        assert_eq!(
            input.get("prompt_content_type").and_then(Value::as_str),
            Some("markdown")
        );
    }

    #[test]
    fn lowering_preserves_human_prompt_content_type_metadata() {
        let source = r#"
workflow HumanPromptContentType

rule start
  when started
=> {
  askHuman """application/json
  {
    "question": "Approve this release?"
  }
  """
}
"#;
        let ir = whipplescript_parser::compile_program(source)
            .ir
            .expect("compile");
        let lowering = lower_rule(
            "ins_test",
            &ir,
            &ir.rules[0],
            &RuleContext::default(),
            &[],
            &[],
            None,
        );

        assert_eq!(lowering.effects.len(), 1);
        let input = json_from_str(&lowering.effects[0].input_json);
        assert_eq!(
            input.get("prompt").and_then(Value::as_str),
            Some("{\n    \"question\": \"Approve this release?\"\n  }")
        );
        assert_eq!(
            input.get("prompt_content_type").and_then(Value::as_str),
            Some("application/json")
        );
    }

    #[test]
    fn multiline_prompt_single_word_opening_tail_stays_prompt_text() {
        let lines = ["tell worker \"\"\"hello", "world", "\"\"\""];
        let (prompt, cursor) = parse_prompt_from_lines(&lines, 0, lines[0]);

        assert_eq!(cursor, 2);
        assert_eq!(
            prompt,
            ParsedPrompt {
                text: "hello\nworld".to_owned(),
                content_type: None,
            }
        );
    }

    #[test]
    fn ready_contexts_match_queue_ready_item_alias_to_projected_fact() {
        let source = r#"
workflow QueueReadyAlias

queue backlog {
  tracker builtin
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule claim_ready
  when backlog has ready item as item
  when worker is available
=> {
  claim item as lease
}
"#;
        let ir = whipplescript_parser::compile_program(source)
            .ir
            .expect("compile");
        let fact = FactView {
            fact_id: "fact-queue-item".to_owned(),
            program_version_id: None,
            revision_epoch: 0,
            name: "queue.item.ready".to_owned(),
            key: "backlog:WS-1:gen".to_owned(),
            value_json: r#"{"queue":"backlog","id":"WS-1","title":"Ready item","body":"Do work"}"#
                .to_owned(),
            provenance_class: "queue".to_owned(),
            source_span_json: None,
        };
        let other_queue = FactView {
            fact_id: "fact-other-item".to_owned(),
            program_version_id: None,
            revision_epoch: 0,
            name: "queue.item.ready".to_owned(),
            key: "other:WS-2:gen".to_owned(),
            value_json: r#"{"queue":"other","id":"WS-2","title":"Other","body":""}"#.to_owned(),
            provenance_class: "queue".to_owned(),
            source_span_json: None,
        };
        let facts = vec![fact, other_queue];
        let effects = Vec::new();
        let ready = ready_contexts(&ir, &ir.rules[0], &facts, &effects, None);

        assert_eq!(ready.contexts.len(), 1);
        assert_eq!(ready.contexts[0].bindings[0].0, "item");
        assert_eq!(ready.contexts[0].bindings[0].1.key, "backlog:WS-1:gen");
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
    fn reconstructs_revision_trace_records_from_store_events() {
        let events = vec![
            event_view(
                1,
                "rule.committed",
                json!({
                    "rule": "dispatch",
                    "facts": [],
                    "effects": [{"effect_id": "running", "status": "queued"}],
                    "dependencies": []
                }),
            ),
            event_view(
                2,
                "effect.run_started",
                json!({"effect_id": "running", "run_id": "run_running"}),
            ),
            event_view(
                3,
                "workflow.revision_activated",
                json!({
                    "revision_id": "rev_1",
                    "instance_id": "ins_1",
                    "from_version_id": "ver_old",
                    "to_version_id": "ver_new",
                    "from_epoch": 0,
                    "to_epoch": 1,
                    "activation_policy": {},
                    "cancellation_policy": "request_running",
                    "terminal_cancel_effects": [],
                    "request_cancel_effects": ["running"]
                }),
            ),
            event_view(
                4,
                "effect.cancellation_requested",
                json!({
                    "request_id": "ecr_1",
                    "effect_id": "running",
                    "revision_id": "rev_1",
                    "reason": "workflow revision",
                    "requested_by": "workflow.revision"
                }),
            ),
            event_view(
                5,
                "effect.terminal",
                json!({
                    "effect_id": "running",
                    "run_id": "run_running",
                    "status": "completed"
                }),
            ),
        ];

        let records = reconstruct_trace_records(&events);

        assert_eq!(records.len(), 6);
        check_trace(&records).expect("revision trace conforms");
        match &records[3].event {
            TraceEvent::RevisionActivated {
                revision_id,
                to_epoch,
                request_cancel_effects,
                ..
            } => {
                assert_eq!(revision_id, "rev_1");
                assert_eq!(*to_epoch, 1);
                assert_eq!(request_cancel_effects, &vec!["running".to_owned()]);
            }
            event => panic!("expected revision activation record, got {event:?}"),
        }

        let rendered = trace_record_to_json(&records[4]);
        let event = rendered.get("event").expect("event");
        assert_eq!(
            event.get("type").and_then(Value::as_str),
            Some("effect_cancellation_requested")
        );
        assert_eq!(
            event.get("requested_by").and_then(Value::as_str),
            Some("workflow.revision")
        );
    }

    #[test]
    fn renders_revision_log_event_details() {
        let event = event_view(
            1,
            "workflow.revision_activated",
            json!({
                "revision_id": "rev_1",
                "from_version_id": "ver_old",
                "to_version_id": "ver_new",
                "from_epoch": 0,
                "to_epoch": 1,
                "cancellation_policy": "request_running",
                "terminal_cancel_effects": ["queued"],
                "request_cancel_effects": ["running"]
            }),
        );

        assert_eq!(
            log_event_details(&event).as_deref(),
            Some(
                "revision=rev_1 epoch=0->1 from=ver_old to=ver_new cancel=request_running terminal_cancel=1 request_cancel=1"
            )
        );
    }

    #[test]
    fn renders_cancellation_request_log_event_details() {
        let event = event_view(
            2,
            "effect.cancellation_requested",
            json!({
                "request_id": "ecr_1",
                "effect_id": "running",
                "revision_id": "rev_1",
                "reason": "workflow revision",
                "requested_by": "workflow.revision"
            }),
        );

        assert_eq!(
            log_event_details(&event).as_deref(),
            Some("effect=running revision=rev_1 by=workflow.revision reason=workflow revision")
        );
    }

    #[test]
    fn renders_run_cancellation_request_json() {
        let run = RunView {
            run_id: "run-1".to_owned(),
            effect_id: "effect-1".to_owned(),
            provider: "fixture".to_owned(),
            worker_id: "worker-1".to_owned(),
            status: "running".to_owned(),
            started_at: "2026-01-01T00:00:00Z".to_owned(),
            completed_at: None,
            metadata_json: "{}".to_owned(),
            cancel_requested: true,
        };

        let rendered = run_to_json(&run);
        assert_eq!(
            rendered.get("cancel_requested").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            rendered
                .pointer("/provider_selection/provider_id")
                .and_then(Value::as_str),
            Some("fixture")
        );
    }

    #[test]
    fn renders_run_provider_selection_metadata_json() {
        let run = RunView {
            run_id: "run-1".to_owned(),
            effect_id: "effect-1".to_owned(),
            provider: "runner".to_owned(),
            worker_id: "worker-1".to_owned(),
            status: "running".to_owned(),
            started_at: "2026-01-01T00:00:00Z".to_owned(),
            completed_at: None,
            metadata_json: json!({
                "provider_selection": {
                    "provider_id": "runner",
                    "provider_kind": "command",
                    "source_harness_id": "runner",
                    "surface": "command"
                }
            })
            .to_string(),
            cancel_requested: false,
        };

        let rendered = run_to_json(&run);
        assert_eq!(
            rendered
                .pointer("/provider_selection/provider_kind")
                .and_then(Value::as_str),
            Some("command")
        );
        assert_eq!(
            rendered
                .pointer("/provider_selection/source_harness_id")
                .and_then(Value::as_str),
            Some("runner")
        );
        assert_eq!(
            rendered
                .pointer("/provider_selection/surface")
                .and_then(Value::as_str),
            Some("command")
        );
    }

    #[test]
    fn renders_effect_harness_provider_selection_json() {
        let effect = EffectView {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("implementer".to_owned()),
            input_json: r#"{"prompt":"go"}"#.to_owned(),
            status: "queued".to_owned(),
            created_by_rule: "start".to_owned(),
            program_version_id: Some("version-1".to_owned()),
            revision_epoch: 0,
            profile: Some("repo-writer".to_owned()),
            required_capabilities_json: r#"["agent.tell"]"#.to_owned(),
            declared_profiles_json: json!({
                "harnesses": [{"name": "coder", "kind": "codex"}],
                "agents": [{"name": "implementer", "harness": "coder"}]
            })
            .to_string(),
            policy_block_reason: None,
            cancel_requested: false,
        };

        let rendered = effect_to_json(&effect);

        assert_eq!(
            rendered
                .pointer("/provider_selection/source_harness_id")
                .and_then(Value::as_str),
            Some("coder")
        );
        assert_eq!(
            rendered
                .pointer("/provider_selection/provider_kind")
                .and_then(Value::as_str),
            Some("codex")
        );
    }

    #[test]
    fn status_json_includes_effects_and_runs_provider_selection() {
        let status = StatusView {
            instance: InstanceView {
                instance_id: "ins-1".to_owned(),
                program_id: "prog-1".to_owned(),
                version_id: "ver-1".to_owned(),
                revision_epoch: 0,
                status: "running".to_owned(),
                input_json: "{}".to_owned(),
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                updated_at: "2026-01-01T00:00:00Z".to_owned(),
            },
            fact_count: 0,
            queued_effect_count: 1,
            blocked_effect_count: 0,
            active_run_count: 1,
            failure_count: 0,
            cancellation_request_count: 0,
            revisions: Vec::new(),
            parent_invocation: None,
            child_invocations: Vec::new(),
            recent_events: Vec::new(),
        };
        let effect = EffectView {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("implementer".to_owned()),
            input_json: r#"{"prompt":"go"}"#.to_owned(),
            status: "running".to_owned(),
            created_by_rule: "start".to_owned(),
            program_version_id: Some("ver-1".to_owned()),
            revision_epoch: 0,
            profile: Some("repo-writer".to_owned()),
            required_capabilities_json: r#"["agent.tell"]"#.to_owned(),
            declared_profiles_json: json!({
                "harnesses": [{"name": "coder", "kind": "codex"}],
                "agents": [{"name": "implementer", "harness": "coder"}]
            })
            .to_string(),
            policy_block_reason: None,
            cancel_requested: false,
        };
        let run = RunView {
            run_id: "run-1".to_owned(),
            effect_id: "eff-1".to_owned(),
            provider: "coder".to_owned(),
            worker_id: "worker-1".to_owned(),
            status: "running".to_owned(),
            started_at: "2026-01-01T00:00:00Z".to_owned(),
            completed_at: None,
            metadata_json: json!({
                "native_provider": {
                    "provider_id": "coder",
                    "provider_kind": "codex",
                    "surface": "codex_app_server"
                },
                "provider_selection": {
                    "provider_id": "coder",
                    "provider_kind": "codex",
                    "source_harness_id": "coder",
                    "surface": "codex_app_server"
                }
            })
            .to_string(),
            cancel_requested: false,
        };

        let rendered = status_to_json_with_effects_and_runs(&status, &[effect], &[run]);

        assert_eq!(
            rendered
                .pointer("/effects/0/provider_selection/provider_kind")
                .and_then(Value::as_str),
            Some("codex")
        );
        assert_eq!(
            rendered
                .pointer("/runs/0/provider_selection/source_harness_id")
                .and_then(Value::as_str),
            Some("coder")
        );
        assert_eq!(
            rendered
                .pointer("/runs/0/provider_selection/surface")
                .and_then(Value::as_str),
            Some("codex_app_server")
        );
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

    fn revision_generated_checks_source() -> &'static str {
        r#"
workflow RevisionGeneratedChecks

class Task {
  title string
}

class Classification {
  summary string
}

coerce classify(title string) -> Classification {
  prompt "Classify"
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule classify
  when Task as task
=> {
  coerce classify(task.title) as classification

  after classification completes {
    tell worker "summarize" as notify
  }
}
"#
    }

    #[test]
    fn generates_model_searches_for_effect_dependencies() {
        let source = include_str!("../../../examples/queue-worker-with-review.whip");
        let compiled = whipplescript_parser::compile_program(source);
        let ir = compiled.ir.expect("example compiles");
        let (_maude, expected) =
            generate_maude_model_search(source, &ir, Path::new("/tmp/kernel.maude"));

        assert_eq!(expected.len(), 18);
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.outcome == ExpectedSearchResult::Solution)
                .count(),
            6
        );
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "succeeds")
                .count(),
            9
        );
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "fails")
                .count(),
            6
        );
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "revision-active-rule")
                .count(),
            1
        );
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "revision-stale-rule")
                .count(),
            1
        );
        assert_eq!(
            expected
                .iter()
                .filter(|result| result.predicate == "revision-effect-attribution")
                .count(),
            1
        );
    }

    #[test]
    fn generates_revision_model_searches_for_effects_and_completes_dependencies() {
        let source = revision_generated_checks_source();
        let compiled = whipplescript_parser::compile_program(source);
        let ir = compiled
            .ir
            .unwrap_or_else(|| panic!("source compiles: {:?}", compiled.diagnostics));
        let (maude, expected) =
            generate_maude_model_search(source, &ir, Path::new("/tmp/kernel.maude"));

        assert!(maude.contains("scopedRuleV("));
        assert!(maude.contains("activeRevision("));
        assert!(maude.contains("effectVersion("));
        assert!(maude.contains("revisionCancellationPolicy("));
        assert!(expected.iter().any(|result| {
            result.predicate == "revision-active-rule"
                && result.outcome == ExpectedSearchResult::Solution
        }));
        assert!(expected.iter().any(|result| {
            result.predicate == "revision-stale-rule"
                && result.outcome == ExpectedSearchResult::NoSolution
        }));
        assert!(expected.iter().any(|result| {
            result.predicate == "revision-effect-attribution"
                && result.outcome == ExpectedSearchResult::NoSolution
        }));
        assert!(expected.iter().any(|result| {
            result.predicate == "revision-completes-cancelled"
                && result.outcome == ExpectedSearchResult::Solution
        }));
    }

    #[test]
    fn generated_ne_false_case_compares_left_and_right_operands() {
        let left = Expr::Literal(ExprLiteral::String("left".to_owned()));
        let right = Expr::Literal(ExprLiteral::String("right".to_owned()));
        let left_key = left.to_snapshot();
        let right_key = right.to_snapshot();
        let expr = Expr::Binary {
            op: BinaryOp::Ne,
            left: Box::new(left),
            right: Box::new(right),
        };
        let mut context = MaudeExprContext::default();

        let cases = maude_bool_cases(&expr, &mut context);

        let left_symbol = context
            .scalar_symbols
            .get(&left_key)
            .expect("left symbol exists");
        let right_symbol = context
            .scalar_symbols
            .get(&right_key)
            .expect("right symbol exists");
        assert_ne!(left_symbol, right_symbol);
        assert_eq!(
            cases.false_expr,
            format!("neExpr(scalar({left_symbol}), scalar({right_symbol}))")
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

assert count(Result) == 0
assert count(Result) == 0
assert count(Result where status == "accepted") >= 0
assert count(Result where status not in ["accepted", "queued"]) == 0
assert "urgent" in ["urgent", "later"]

rule accept
  when Task as task where task.status == "queued" && task.priority >= 1 && "urgent" in task.labels && task.metadata["phase"] == "kernel" && count(Result where metadata["phase"] == "done") == 0
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
        let source = include_str!("../../../examples/queue-worker-with-review.whip");
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

assert count(Result) == 0
assert count(Result) == 0

rule accept
  when Task as task where task.status == "queued" && count(Result) == 0
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
    fn generated_model_search_runs_revision_fixture() {
        if find_executable_in_path(&["maude"], &path_value()).is_none() {
            return;
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let kernel_path =
            fs::canonicalize(root.join("models/maude/kernel.maude")).expect("kernel path resolves");
        let source = revision_generated_checks_source();
        let compiled = whipplescript_parser::compile_program(source);
        let ir = compiled
            .ir
            .unwrap_or_else(|| panic!("source compiles: {:?}", compiled.diagnostics));
        let (maude, expected) = generate_maude_model_search(source, &ir, &kernel_path);
        assert!(expected.iter().any(|result| {
            result.predicate == "revision-completes-cancelled"
                && result.outcome == ExpectedSearchResult::Solution
        }));

        let output =
            run_maude_source("generated-revision-check-fixture", &maude).expect("runs Maude");
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

    #[test]
    fn parses_doctor_provider_config_option() {
        let options = DoctorOptions::parse(&[
            "--providers".to_owned(),
            "--provider-config".to_owned(),
            "providers.json".to_owned(),
            "--record-provider-evidence".to_owned(),
            "ins_123".to_owned(),
        ])
        .expect("doctor options parse");

        assert_eq!(
            options.provider_config_paths,
            vec![PathBuf::from("providers.json")]
        );
        assert_eq!(
            options.record_provider_evidence_instance_id.as_deref(),
            Some("ins_123")
        );
        assert!(options.providers);
    }

    #[test]
    fn parses_worker_and_dev_provider_config_options() {
        let worker = WorkerOptions::parse(&[
            "ins_123".to_owned(),
            "--provider".to_owned(),
            "fixture".to_owned(),
            "--provider-config".to_owned(),
            "providers-a.json".to_owned(),
            "--provider-config".to_owned(),
            "providers-b.json".to_owned(),
            "--once".to_owned(),
        ])
        .expect("worker options parse");

        assert_eq!(worker.instance_id, "ins_123");
        assert_eq!(
            worker.provider_config_paths,
            vec![
                PathBuf::from("providers-a.json"),
                PathBuf::from("providers-b.json")
            ]
        );

        let dev = DevOptions::parse(&[
            "workflow.whip".to_owned(),
            "--provider-config".to_owned(),
            "providers.json".to_owned(),
            "--until".to_owned(),
            "idle".to_owned(),
            "--stream".to_owned(),
            "ndjson".to_owned(),
        ])
        .expect("dev options parse");

        assert_eq!(dev.program_path, "workflow.whip");
        assert_eq!(
            dev.provider_config_paths,
            vec![PathBuf::from("providers.json")]
        );
        assert_eq!(dev.stream, Some(DevStreamFormat::Ndjson));
    }

    #[test]
    fn doctor_provider_config_validation_redacts_extra_values() {
        let results = validate_doctor_provider_config_json(
            r#"{
              "providers": [
                {
                  "provider_id": "codex-main",
                  "provider_kind": "codex",
                  "surface": "codex_app_server",
                  "credentials_ref": "secret:codex",
                  "cancellation_depth": "native_stop",
                  "api_key": "sk-should-not-appear"
                },
                {
                  "provider_id": "pi-main",
                  "provider_kind": "pi",
                  "surface": "pi_rpc",
                  "credentials_ref": "secret:pi",
                  "cancellation_depth": "native_stop"
                }
              ]
            }"#,
        );

        assert!(results.iter().any(|result| {
            result.status == whipplescript_kernel::provider::ProviderValidationStatus::Pass
                && result.provider == "codex-main"
                && result.code == "surface_supported"
        }));
        assert!(results.iter().any(|result| {
            result.status == whipplescript_kernel::provider::ProviderValidationStatus::Pass
                && result.provider == "pi-main"
                && result.code == "cancellation_supported"
        }));
        assert!(!json!(results
            .iter()
            .map(ProviderValidationResult::to_json)
            .collect::<Vec<_>>())
        .to_string()
        .contains("sk-should-not-appear"));
    }

    #[test]
    fn doctor_provider_config_validation_rejects_command_without_executable() {
        let results = validate_doctor_provider_config_json(
            r#"{
              "providers": [
                {
                  "provider_id": "runner",
                  "provider_kind": "command",
                  "surface": "command",
                  "workspace_policy": "read_only",
                  "cancellation_depth": "none",
                  "artifact_policy": "metadata"
                }
              ]
            }"#,
        );

        assert!(results.iter().any(|result| {
            result.status == whipplescript_kernel::provider::ProviderValidationStatus::Fail
                && result.provider == "runner"
                && result.code == "invalid_command_config"
                && result
                    .message
                    .contains("missing required command field `executable`")
        }));
    }

    #[test]
    fn native_lifecycle_summary_exposes_redacted_status_for_runs() {
        let events = vec![
            event_view(
                1,
                "agent.turn.streamed",
                json!({
                    "run_id": "run-1",
                    "effect_id": "tell",
                    "provider": "pi-main",
                    "status": "streamed",
                    "terminal": false,
                    "provider_event_type": "message_end",
                    "provider_payload_shape": {"type":"object","keys":2},
                    "evidence_id": "evd_stream",
                }),
            ),
            event_view(
                2,
                "agent.turn.cancelled",
                json!({
                    "run_id": "run-1",
                    "effect_id": "tell",
                    "provider": "pi-main",
                    "status": "cancelled",
                    "terminal": true,
                    "provider_event_type": "turn_end",
                    "provider_payload_shape": {"type":"object","keys":3},
                    "evidence_id": "evd_cancel",
                }),
            ),
            event_view(3, "workflow.completed", json!({"status":"completed"})),
        ];
        let lifecycle = native_lifecycle_events(&events);
        assert_eq!(lifecycle.as_array().expect("array").len(), 2);
        assert_eq!(
            lifecycle
                .pointer("/1/provider_event_type")
                .and_then(Value::as_str),
            Some("turn_end")
        );
        assert!(lifecycle.to_string().contains("evd_cancel"));
        assert!(!lifecycle.to_string().contains("provider_payload_shape"));

        let run = RunView {
            run_id: "run-1".to_owned(),
            effect_id: "tell".to_owned(),
            provider: "pi-main".to_owned(),
            worker_id: "worker-1".to_owned(),
            status: "running".to_owned(),
            started_at: "2026-01-01T00:00:00Z".to_owned(),
            completed_at: None,
            metadata_json: json!({
                "native_provider": {
                    "provider_id": "pi-main",
                    "provider_kind": "pi",
                    "surface": "pi_rpc"
                }
            })
            .to_string(),
            cancel_requested: true,
        };
        let run_json = run_to_json_with_lifecycle_and_artifacts(&run, &events, &BTreeMap::new());
        assert_eq!(
            run_json
                .pointer("/native_lifecycle/status")
                .and_then(Value::as_str),
            Some("cancelled")
        );
        assert_eq!(
            run_json
                .pointer("/native_lifecycle/evidence_id")
                .and_then(Value::as_str),
            Some("evd_cancel")
        );
        assert_eq!(
            run_json
                .pointer("/provider_selection/surface")
                .and_then(Value::as_str),
            Some("pi_rpc")
        );
    }

    #[test]
    fn provider_cancellation_policy_tracks_validated_native_shapes() {
        assert_eq!(
            provider_cancellation_policy("codex-main"),
            ProviderCancellationPolicy::NativeStop {
                acknowledgement_order: CancellationAcknowledgementOrder::BeforeTerminal,
            }
        );
        assert_eq!(
            provider_cancellation_policy("pi-main"),
            ProviderCancellationPolicy::NativeStop {
                acknowledgement_order: CancellationAcknowledgementOrder::AfterTerminalAllowed,
            }
        );
        assert_eq!(
            provider_cancellation_policy("fixture-cancellable"),
            ProviderCancellationPolicy::NativeStop {
                acknowledgement_order: CancellationAcknowledgementOrder::BeforeTerminal,
            }
        );
        assert_eq!(
            provider_cancellation_policy("claude-main"),
            ProviderCancellationPolicy::Unsupported
        );
    }

    #[test]
    fn agent_provider_selection_uses_bound_harness_metadata() {
        let effect = ClaimableEffect {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("implementer".to_owned()),
            profile: Some("repo-writer".to_owned()),
            input_json: "{}".to_owned(),
            required_capabilities_json: "[]".to_owned(),
            declared_profiles_json: json!({
                "harnesses": [
                    {"name": "coder", "kind": "codex"},
                    {"name": "reviewer", "kind": "claude"}
                ],
                "agents": [
                    {"name": "implementer", "harness": "coder", "profile": "repo-writer"},
                    {"name": "critic", "harness": "reviewer", "profile": "repo-reader"}
                ]
            })
            .to_string(),
        };

        let selection =
            agent_provider_selection_with_config_paths(&effect, "fixture", &[]).expect("selection");

        assert_eq!(
            selection,
            AgentProviderSelection {
                provider_id: "coder".to_owned(),
                kind: "fixture".to_owned(),
                source_harness_id: Some("coder".to_owned()),
                surface: None,
                provider_config: None,
                command_plan: None,
            }
        );
    }

    #[test]
    fn agent_provider_selection_supports_map_shaped_harness_metadata() {
        let effect = ClaimableEffect {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("implementer".to_owned()),
            profile: Some("repo-writer".to_owned()),
            input_json: "{}".to_owned(),
            required_capabilities_json: "[]".to_owned(),
            declared_profiles_json: json!({
                "harnesses": {
                    "coder": {"kind": "codex"}
                },
                "agents": {
                    "implementer": {"harness": "coder", "profile": "repo-writer"}
                }
            })
            .to_string(),
        };

        let selection =
            agent_provider_selection_with_config_paths(&effect, "fixture", &[]).expect("selection");

        assert_eq!(selection.provider_id, "coder");
        assert_eq!(selection.kind, "fixture");
        assert_eq!(selection.source_harness_id.as_deref(), Some("coder"));
    }

    #[test]
    fn agent_provider_selection_uses_direct_provider_metadata() {
        let effect = ClaimableEffect {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("implementer".to_owned()),
            profile: Some("repo-writer".to_owned()),
            input_json: "{}".to_owned(),
            required_capabilities_json: "[]".to_owned(),
            declared_profiles_json: json!({
                "agents": [
                    {"name": "implementer", "provider": "codex", "profile": "repo-writer"}
                ]
            })
            .to_string(),
        };

        let selection =
            agent_provider_selection_with_config_paths(&effect, "fixture", &[]).expect("selection");

        assert_eq!(
            selection,
            AgentProviderSelection {
                provider_id: "codex".to_owned(),
                kind: "fixture".to_owned(),
                source_harness_id: None,
                surface: None,
                provider_config: None,
                command_plan: None,
            }
        );
    }

    #[test]
    fn agent_provider_selection_uses_provider_config_for_harness_surface() {
        let config_path = std::env::temp_dir().join(format!(
            "whipplescript-harness-provider-config-{}.json",
            std::process::id()
        ));
        fs::write(
            &config_path,
            json!({
                "providers": [
                    {
                        "provider_id": "coder",
                        "provider_kind": "codex",
                        "surface": "codex_app_server",
                        "credentials_ref": "env:OPENAI_API_KEY",
                        "profile_ids": ["repo-writer"],
                        "workspace_policy": "read_only",
                        "cancellation_depth": "native_stop",
                        "artifact_policy": "metadata"
                    }
                ]
            })
            .to_string(),
        )
        .expect("config writes");
        let effect = ClaimableEffect {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("implementer".to_owned()),
            profile: Some("repo-writer".to_owned()),
            input_json: "{}".to_owned(),
            required_capabilities_json: "[]".to_owned(),
            declared_profiles_json: json!({
                "harnesses": [
                    {"name": "coder", "kind": "codex"}
                ],
                "agents": [
                    {"name": "implementer", "harness": "coder", "profile": "repo-writer"}
                ]
            })
            .to_string(),
        };

        let selection = agent_provider_selection_with_config_paths(
            &effect,
            "fixture",
            std::slice::from_ref(&config_path),
        )
        .expect("selection");

        assert_eq!(selection.provider_id, "coder");
        assert_eq!(selection.kind, "codex");
        assert_eq!(selection.source_harness_id.as_deref(), Some("coder"));
        assert_eq!(selection.surface.as_deref(), Some("codex_app_server"));
        assert!(selection.command_plan.is_none());
        let config = selection
            .provider_config
            .as_ref()
            .expect("provider config selected");
        assert_eq!(config.provider_kind, ProviderKind::Codex);
        assert_eq!(config.surface, AdapterSurface::CodexAppServer);
        assert_eq!(config.workspace_policy, "read_only");
        assert_eq!(
            config.credentials_ref.as_deref(),
            Some("env:OPENAI_API_KEY")
        );
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn agent_provider_selection_uses_command_provider_config_plan() {
        let config_path = std::env::temp_dir().join(format!(
            "whipplescript-command-harness-provider-config-{}.json",
            std::process::id()
        ));
        fs::write(
            &config_path,
            json!({
                "providers": [
                    {
                        "provider_id": "runner",
                        "provider_kind": "command",
                        "surface": "command",
                        "workspace_policy": "read_only",
                        "cancellation_depth": "none",
                        "artifact_policy": "metadata",
                        "executable": "sh",
                        "args": ["-c", "cat >/dev/null; echo command completed"],
                        "env": {"MODE": "test"},
                        "required_env": ["PATH"],
                        "required_commands": ["sh"],
                        "timeout_ms": 2500,
                        "require_stdout_json": true
                    }
                ]
            })
            .to_string(),
        )
        .expect("config writes");
        let effect = ClaimableEffect {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("worker".to_owned()),
            profile: Some("repo-reader".to_owned()),
            input_json: "{}".to_owned(),
            required_capabilities_json: "[]".to_owned(),
            declared_profiles_json: json!({
                "harnesses": [
                    {"name": "runner", "kind": "command"}
                ],
                "agents": [
                    {"name": "worker", "harness": "runner", "profile": "repo-reader"}
                ]
            })
            .to_string(),
        };

        let selection = agent_provider_selection_with_config_paths(
            &effect,
            "fixture",
            std::slice::from_ref(&config_path),
        )
        .expect("selection");

        let expected_plan = CommandLaunchPlan::new("runner", "sh")
            .arg("-c")
            .arg("cat >/dev/null; echo command completed")
            .env("MODE", "test")
            .require_env("PATH")
            .require_command("sh")
            .timeout(Duration::from_millis(2500))
            .require_stdout_json();
        assert_eq!(selection.provider_id, "runner");
        assert_eq!(selection.kind, "command");
        assert_eq!(selection.source_harness_id.as_deref(), Some("runner"));
        assert_eq!(selection.surface.as_deref(), Some("command"));
        assert_eq!(selection.command_plan.as_ref(), Some(&expected_plan));
        let config = selection
            .provider_config
            .as_ref()
            .expect("provider config selected");
        assert_eq!(config.provider_kind, ProviderKind::Command);
        assert_eq!(config.timeout_ms, Some(2500));
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn command_provider_config_requires_executable() {
        let config_path = std::env::temp_dir().join(format!(
            "whipplescript-command-harness-missing-executable-{}.json",
            std::process::id()
        ));
        fs::write(
            &config_path,
            json!({
                "providers": [
                    {
                        "provider_id": "runner",
                        "provider_kind": "command",
                        "surface": "command",
                        "workspace_policy": "read_only",
                        "cancellation_depth": "none"
                    }
                ]
            })
            .to_string(),
        )
        .expect("config writes");
        let effect = ClaimableEffect {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("worker".to_owned()),
            profile: Some("repo-reader".to_owned()),
            input_json: "{}".to_owned(),
            required_capabilities_json: "[]".to_owned(),
            declared_profiles_json: json!({
                "harnesses": [
                    {"name": "runner", "kind": "command"}
                ],
                "agents": [
                    {"name": "worker", "harness": "runner", "profile": "repo-reader"}
                ]
            })
            .to_string(),
        };

        let error = agent_provider_selection_with_config_paths(
            &effect,
            "fixture",
            std::slice::from_ref(&config_path),
        )
        .expect_err("missing executable rejects command config");

        match error {
            StoreError::Conflict(message) => {
                assert!(message.contains("missing required command field `executable`"));
            }
            error => panic!("expected command config conflict, got {error:?}"),
        }
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn agent_provider_selection_falls_back_without_harness_binding() {
        let effect = ClaimableEffect {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("worker".to_owned()),
            profile: Some("repo-writer".to_owned()),
            input_json: "{}".to_owned(),
            required_capabilities_json: "[]".to_owned(),
            declared_profiles_json: json!({
                "agents": [
                    {"name": "worker", "profile": "repo-writer"}
                ]
            })
            .to_string(),
        };

        let selection = agent_provider_selection_with_config_paths(&effect, "claude-main", &[])
            .expect("selection");

        assert_eq!(
            selection,
            AgentProviderSelection {
                provider_id: "claude-main".to_owned(),
                kind: "claude".to_owned(),
                source_harness_id: None,
                surface: None,
                provider_config: None,
                command_plan: None,
            }
        );
    }

    #[test]
    fn native_turn_request_applies_provider_config_fields() {
        let config_json = json!({
            "provider_id": "coder",
            "provider_kind": "codex",
            "surface": "codex_app_server",
            "credentials_ref": "env:OPENAI_API_KEY",
            "profile_ids": ["repo-writer"],
            "default_model": "gpt-5.4",
            "workspace_policy": "shared",
            "timeout_ms": 60000,
            "cancellation_depth": "hard_process_stop",
            "artifact_policy": "required",
            "health_checks": ["codex_cli"],
            "cwd": "/tmp/whip-coder"
        });
        let config =
            ProviderBindingConfig::from_value(&config_json).expect("provider config parses");
        let effect = ClaimableEffect {
            effect_id: "eff-1".to_owned(),
            kind: "agent.tell".to_owned(),
            target: Some("implementer".to_owned()),
            profile: Some("repo-writer".to_owned()),
            input_json: r#"{"prompt":"go"}"#.to_owned(),
            required_capabilities_json: r#"["agent.tell"]"#.to_owned(),
            declared_profiles_json: "{}".to_owned(),
        };
        let execution = AgentTurnExecution {
            instance_id: "ins-1",
            effect_id: "eff-1",
            run_id: "run-1",
            provider: "coder",
            worker_id: "worker-1",
            lease_id: "lease-1",
            lease_expires_at: "2030-01-01T00:00:00Z",
            agent: "implementer",
            profile: Some("repo-writer"),
            input_json: r#"{"prompt":"go"}"#,
            skill_names: &[],
        };

        let request =
            codex_native_turn_request(execution, &effect, r#"{"prompt":"go"}"#, Some(&config))
                .expect("request builds");

        assert_eq!(request.provider_id, "coder");
        assert_eq!(request.workspace_policy, "shared");
        assert_eq!(
            request.cancellation_depth,
            CancellationDepth::HardProcessStop
        );
        assert_eq!(request.artifact_policy, "required");
        assert_eq!(
            request.credential_ref.as_deref(),
            Some("env:OPENAI_API_KEY")
        );
        assert_eq!(
            request.provider_options.get("cwd").and_then(Value::as_str),
            Some("/tmp/whip-coder")
        );
        assert_eq!(
            request
                .provider_options
                .get("model")
                .and_then(Value::as_str),
            Some("gpt-5.4")
        );
        assert_eq!(
            request
                .provider_options
                .get("timeout_ms")
                .and_then(Value::as_u64),
            Some(60000)
        );
        assert_eq!(
            request
                .provider_options
                .get("profile_ids")
                .and_then(Value::as_array)
                .and_then(|values| values.first())
                .and_then(Value::as_str),
            Some("repo-writer")
        );

        let claude_config_json = json!({
            "provider_id": "reviewer",
            "provider_kind": "claude",
            "surface": "claude_agent_sdk",
            "credentials_ref": "env:ANTHROPIC_API_KEY",
            "default_model": "sonnet-4",
            "workspace_policy": "per_effect_worktree",
            "cancellation_depth": "cooperative_request",
            "artifact_policy": "metadata",
            "timeout_ms": 45000,
            "cwd": "/tmp/whip-reviewer"
        });
        let claude_config =
            ProviderBindingConfig::from_value(&claude_config_json).expect("claude config parses");
        let claude_request = claude_native_turn_request(
            AgentTurnExecution {
                provider: "reviewer",
                agent: "critic",
                profile: None,
                ..execution
            },
            &effect,
            r#"{"prompt":"review"}"#,
            Some(&claude_config),
        )
        .expect("claude request builds");

        assert_eq!(claude_request.provider_id, "reviewer");
        assert_eq!(claude_request.workspace_policy, "per_effect_worktree");
        assert_eq!(
            claude_request.cancellation_depth,
            CancellationDepth::CooperativeRequest
        );
        assert_eq!(
            claude_request.credential_ref.as_deref(),
            Some("env:ANTHROPIC_API_KEY")
        );
        assert_eq!(
            claude_request
                .provider_options
                .get("model")
                .and_then(Value::as_str),
            Some("sonnet-4")
        );
        assert_eq!(
            claude_request
                .provider_options
                .get("cwd")
                .and_then(Value::as_str),
            Some("/tmp/whip-reviewer")
        );

        let pi_config_json = json!({
            "provider_id": "pi-main",
            "provider_kind": "pi",
            "surface": "pi_rpc",
            "credentials_ref": "env:PI_API_KEY",
            "default_model": "pi-model",
            "workspace_policy": "shared",
            "cancellation_depth": "native_stop",
            "artifact_policy": "required",
            "model_provider": "openai-compatible"
        });
        let pi_config =
            ProviderBindingConfig::from_value(&pi_config_json).expect("pi config parses");
        let pi_request = pi_native_turn_request(
            AgentTurnExecution {
                provider: "pi-main",
                agent: "planner",
                profile: None,
                ..execution
            },
            &effect,
            r#"{"prompt":"plan"}"#,
            Some(&pi_config),
        )
        .expect("pi request builds");

        assert_eq!(pi_request.provider_id, "pi-main");
        assert_eq!(pi_request.workspace_policy, "shared");
        assert_eq!(pi_request.cancellation_depth, CancellationDepth::NativeStop);
        assert_eq!(pi_request.artifact_policy, "required");
        assert_eq!(pi_request.credential_ref.as_deref(), Some("env:PI_API_KEY"));
        assert_eq!(
            pi_request
                .provider_options
                .get("model")
                .and_then(Value::as_str),
            Some("pi-model")
        );
        assert_eq!(
            pi_request
                .provider_options
                .get("provider")
                .and_then(Value::as_str),
            Some("openai-compatible")
        );
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
