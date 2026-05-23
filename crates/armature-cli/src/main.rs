use armature_engine::queue::{EventStatus, WorkflowEvent};
use armature_engine::storage::WorkflowStore;
use armature_engine::WorkflowRuntime;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::process::Output;
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(name = "armature")]
#[command(about = "Armature statechart workflow CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse and statically validate a workflow file.
    Validate {
        file: PathBuf,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Parse and statically validate adapter manifest files.
    ValidateAdapter {
        manifests: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Parse and statically validate capability policy document files.
    ValidatePolicy {
        policies: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Add one event to a workflow queue without processing it.
    Emit {
        file: PathBuf,
        #[arg(long)]
        event: String,
        #[arg(long, default_value = "{}")]
        payload: String,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Run one queued event through a workflow.
    Run {
        file: PathBuf,
        #[arg(long)]
        event: Option<String>,
        #[arg(long, default_value = "{}")]
        payload: String,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show persisted workflow status.
    Status {
        file: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show a compact workflow overview for humans and agents.
    Overview {
        file: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show durable workflow events.
    Events {
        file: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Show the append-only workflow log.
    Log {
        file: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Run a bounded formal check for a workflow file.
    Check {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = CliModelTarget::Tla)]
        target: CliModelTarget,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Run stronger proof-oriented verification when available.
    Prove {
        file: PathBuf,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Compile a workflow into build artifacts.
    Build {
        file: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Emit a formal model from a workflow file.
    EmitModel {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = CliModelTarget::Tla)]
        target: CliModelTarget,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
    },
    /// Emit a checker config for a generated formal model.
    EmitConfig {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = CliModelTarget::Tla)]
        target: CliModelTarget,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliModelTarget {
    Tla,
    Apalache,
    Maude,
    Veil,
}

impl From<CliModelTarget> for armature_modelgen::ModelTarget {
    fn from(target: CliModelTarget) -> Self {
        match target {
            CliModelTarget::Tla => Self::Tla,
            CliModelTarget::Apalache => Self::Apalache,
            CliModelTarget::Maude => Self::Maude,
            CliModelTarget::Veil => Self::Veil,
        }
    }
}

#[derive(Debug, Error)]
enum CliError {
    #[error("failed to read `{path}`: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create `{path}`: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Source(#[from] armature_workflow::SourceError),
    #[error("workflow validation failed")]
    Validation {
        diagnostics: Vec<armature_workflow::Diagnostic>,
    },
    #[error("invalid event: {0}")]
    InvalidEvent(String),
    #[error("invalid adapter manifest input: {0}")]
    InvalidAdapterManifestInput(String),
    #[error("invalid policy document input: {0}")]
    InvalidPolicyDocumentInput(String),
    #[error("payload is not valid JSON: {0}")]
    Payload(#[from] serde_json::Error),
    #[error("failed to parse adapter manifest `{path}`: {source}")]
    AdapterManifest {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to parse policy document `{path}`: {source}")]
    PolicyDocument {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("adapter manifest validation failed: {message}")]
    AdapterManifestValidation {
        message: String,
        diagnostics: Vec<armature_workflow::Diagnostic>,
    },
    #[error("workflow contract validation failed: {message}")]
    WorkflowContractValidation {
        message: String,
        diagnostics: Vec<armature_workflow::Diagnostic>,
    },
    #[error("storage error: {0}")]
    Storage(#[from] armature_engine::storage::StorageError),
    #[error("runtime error: {0}")]
    Runtime(#[from] armature_engine::RuntimeError),
    #[error("model generation error: {0}")]
    Modelgen(#[from] armature_modelgen::ModelgenError),
    #[error("failed to write `{path}`: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("check tool `{0}` is not installed, not on PATH, and unavailable through nix")]
    CheckToolUnavailable(&'static str),
    #[error("failed to execute check tool `{tool}`: {source}")]
    CheckToolExecution {
        tool: &'static str,
        source: std::io::Error,
    },
    #[error("check failed with status {status}\nstdout:\n{stdout}\nstderr:\n{stderr}")]
    CheckFailed {
        status: std::process::ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("prove is not implemented yet; use `armature check --target tla` or `armature check --target maude` for bounded verification")]
    ProveUnavailable,
}

#[derive(Debug, Serialize)]
struct ValidateOutput {
    ok: bool,
    diagnostics: Vec<armature_workflow::Diagnostic>,
}

#[derive(Debug, Serialize)]
struct RunOutput {
    outcome: Option<armature_engine::EventProcessingOutcome>,
    status: armature_engine::status::WorkflowStatus,
}

#[derive(Debug, Serialize)]
struct EmitOutput {
    event: WorkflowEvent,
    status: armature_engine::status::WorkflowStatus,
}

#[derive(Debug, Serialize)]
struct EventsOutput {
    workflow_id: String,
    events: Vec<WorkflowEvent>,
}

#[derive(Debug, Serialize)]
struct LogOutput {
    workflow_id: String,
    records: Vec<armature_engine::log::WorkflowLogRecord>,
}

#[derive(Debug, Serialize)]
struct CheckOutput {
    ok: bool,
    target: String,
    stdout: String,
    stderr: String,
    artifacts: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct ProveOutput {
    ok: bool,
    available: bool,
    message: String,
    suggested_command: String,
}

#[derive(Debug, Serialize)]
struct BuildOutput {
    workflow_id: String,
    output_dir: PathBuf,
    ir_json: PathBuf,
    baml_src: PathBuf,
    tla_model: PathBuf,
    tla_config: PathBuf,
    maude_model: PathBuf,
    adapter_manifest_bundle: Option<PathBuf>,
    policy_document_bundle: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct OverviewOutput {
    validation: ValidateOutput,
    status: Option<armature_engine::status::WorkflowStatus>,
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        print_cli_error(&error);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Validate {
            file,
            adapter_manifests,
            policy_documents,
            json,
        } => {
            let source = load_source(&file)?;
            let parsed =
                armature_workflow::parse_syntax_with_file(&source, file.display().to_string());
            let mut diagnostics = parsed.diagnostics;
            if let Some(ir) = parsed.ir {
                diagnostics.extend(armature_workflow::validate_ir(&ir).diagnostics);
                let policies = load_policy_documents(&policy_documents)?;
                if !adapter_manifests.is_empty() {
                    let manifests = load_adapter_manifests(&adapter_manifests)?;
                    diagnostics.extend(armature_adapters::validate_adapter_manifests(&manifests));
                    diagnostics.extend(armature_adapters::validate_workflow_effects(
                        &ir, &manifests,
                    ));
                    diagnostics.extend(armature_adapters::validate_workflow_policy(
                        &ir, &manifests, &policies,
                    ));
                } else {
                    diagnostics.extend(armature_adapters::validate_policy_documents(&policies));
                }
            }
            let ok = !diagnostics_have_errors(&diagnostics);
            if json {
                print_json(&ValidateOutput {
                    ok,
                    diagnostics: diagnostics.clone(),
                })?;
            } else if ok {
                println!("ok");
            }

            if ok {
                Ok(())
            } else {
                Err(CliError::Validation {
                    diagnostics: if json { Vec::new() } else { diagnostics },
                })
            }
        }
        Command::ValidateAdapter { manifests, json } => {
            if manifests.is_empty() {
                return Err(CliError::InvalidAdapterManifestInput(
                    "validate-adapter requires at least one manifest path".to_string(),
                ));
            }
            let parsed_manifests = load_adapter_manifests(&manifests)?;
            let diagnostics = armature_adapters::validate_adapter_manifests(&parsed_manifests);
            let ok = !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == armature_workflow::Severity::Error);

            if json {
                print_json(&ValidateOutput {
                    ok,
                    diagnostics: diagnostics.clone(),
                })?;
            } else if ok {
                println!("ok");
            }

            if ok {
                Ok(())
            } else {
                Err(CliError::Validation {
                    diagnostics: if json { Vec::new() } else { diagnostics },
                })
            }
        }
        Command::ValidatePolicy { policies, json } => {
            if policies.is_empty() {
                return Err(CliError::InvalidPolicyDocumentInput(
                    "validate-policy requires at least one policy path".to_string(),
                ));
            }
            let parsed_policies = load_policy_documents(&policies)?;
            let diagnostics = armature_adapters::validate_policy_documents(&parsed_policies);
            let ok = !diagnostics_have_errors(&diagnostics);

            if json {
                print_json(&ValidateOutput {
                    ok,
                    diagnostics: diagnostics.clone(),
                })?;
            } else if ok {
                println!("ok");
            }

            if ok {
                Ok(())
            } else {
                Err(CliError::Validation {
                    diagnostics: if json { Vec::new() } else { diagnostics },
                })
            }
        }
        Command::Emit {
            file,
            event,
            payload,
            store,
            adapter_manifests,
            json,
        } => {
            let ir = load_valid_ir(&file)?;
            let manifests = load_valid_adapter_manifests(&adapter_manifests)?;
            let workflow_id = ir.workflow.name.clone();
            let store_path = store.unwrap_or_else(|| default_store_path(&file, &workflow_id));
            ensure_parent_dir(&store_path)?;
            let store = WorkflowStore::open(&store_path)?;
            let event = build_cli_event(&ir, &manifests, &workflow_id, event, payload)?;
            store.enqueue_event(&event)?;
            let status = stored_status(&ir, &store, &workflow_id)?;
            let output = EmitOutput { event, status };

            if json {
                print_json(&output)?;
            } else {
                println!("{}", output.status.pending_events);
            }
            Ok(())
        }
        Command::Run {
            file,
            event,
            payload,
            store,
            adapter_manifests,
            policy_documents,
            json,
        } => {
            let ir =
                load_valid_ir_with_adapter_manifests(&file, &adapter_manifests, &policy_documents)?;
            let workflow_id = ir.workflow.name.clone();
            let store_path = store.unwrap_or_else(|| default_store_path(&file, &workflow_id));
            ensure_parent_dir(&store_path)?;
            let store = WorkflowStore::open(&store_path)?;
            let manifests = load_valid_adapter_manifests(&adapter_manifests)?;
            let policies = load_valid_policy_documents(&policy_documents)?;
            let dispatcher = adapter_dispatcher_from_manifests(manifests.clone(), policies);
            let mut runtime = WorkflowRuntime::with_dispatcher(ir, store, dispatcher)?;
            if let Some(event) = event {
                let event = {
                    let ir = runtime.interpreter().workflow();
                    build_cli_event(ir, &manifests, &workflow_id, event, payload)?
                };
                runtime.enqueue_event(&event)?;
            }

            let outcome = runtime.process_next_event()?;
            let output = RunOutput {
                outcome,
                status: runtime.projected_status()?,
            };

            if json {
                print_json(&output)?;
            } else {
                println!("{}", output.status.current_state);
            }
            Ok(())
        }
        Command::Status { file, store, json } => {
            let status = load_status_for_file(&file, store)?;

            if json {
                print_json(&status)?;
            } else {
                println!("{}", status.current_state);
            }
            Ok(())
        }
        Command::Overview {
            file,
            store,
            adapter_manifests,
            policy_documents,
            json,
        } => {
            let overview =
                load_overview_for_file(&file, store, &adapter_manifests, &policy_documents)?;

            if json {
                print_json(&overview)?;
            } else {
                print_overview(&overview);
            }
            Ok(())
        }
        Command::Events {
            file,
            store,
            limit,
            json,
        } => {
            let ir = load_ir(&file)?;
            let workflow_id = ir.workflow.name.clone();
            let store_path = store.unwrap_or_else(|| default_store_path(&file, &workflow_id));
            let events = if store_path.exists() {
                WorkflowStore::open(&store_path)?.events(&workflow_id, limit)?
            } else {
                Vec::new()
            };
            let output = EventsOutput {
                workflow_id,
                events,
            };

            if json {
                print_json(&output)?;
            } else {
                for event in &output.events {
                    println!("{} {} {:?}", event.event_id, event.event_type, event.status);
                }
            }
            Ok(())
        }
        Command::Log {
            file,
            store,
            limit,
            json,
        } => {
            let ir = load_ir(&file)?;
            let workflow_id = ir.workflow.name.clone();
            let store_path = store.unwrap_or_else(|| default_store_path(&file, &workflow_id));
            let records = if store_path.exists() {
                WorkflowStore::open(&store_path)?.recent_log_records(&workflow_id, limit)?
            } else {
                Vec::new()
            };
            let output = LogOutput {
                workflow_id,
                records,
            };

            if json {
                print_json(&output)?;
            } else {
                for record in &output.records {
                    println!("{record:?}");
                }
            }
            Ok(())
        }
        Command::Check {
            file,
            target,
            adapter_manifests,
            policy_documents,
            json,
        } => {
            let ir =
                load_valid_ir_with_adapter_manifests(&file, &adapter_manifests, &policy_documents)?;
            let output = check_workflow(&ir, target)?;

            if json {
                print_json(&output)?;
            } else {
                print!("{}", output.stdout);
                eprint!("{}", output.stderr);
                if output.ok {
                    println!("ok");
                }
            }
            Ok(())
        }
        Command::Prove {
            file,
            adapter_manifests,
            policy_documents,
            json,
        } => {
            let _ir =
                load_valid_ir_with_adapter_manifests(&file, &adapter_manifests, &policy_documents)?;
            if json {
                print_json(&ProveOutput {
                    ok: false,
                    available: false,
                    message: "prove is not implemented yet".to_string(),
                    suggested_command: "armature check --target tla".to_string(),
                })?;
            }
            Err(CliError::ProveUnavailable)
        }
        Command::Build {
            file,
            out,
            adapter_manifests,
            policy_documents,
            json,
        } => {
            let ir = load_valid_ir(&file)?;
            let manifests = load_valid_adapter_manifests(&adapter_manifests)?;
            let policies = load_valid_policy_documents(&policy_documents)?;
            if !manifests.is_empty() || !policies.is_empty() {
                validate_ir_contracts(&ir, &manifests, &policies)?;
            }
            let output = build_workflow(&file, &ir, out, &manifests, &policies)?;

            if json {
                print_json(&output)?;
            } else {
                println!("{}", output.output_dir.display());
            }
            Ok(())
        }
        Command::EmitModel {
            file,
            target,
            adapter_manifests,
            policy_documents,
        } => {
            let ir =
                load_valid_ir_with_adapter_manifests(&file, &adapter_manifests, &policy_documents)?;

            println!("{}", armature_modelgen::emit_model(&ir, target.into())?);
            Ok(())
        }
        Command::EmitConfig {
            file,
            target,
            adapter_manifests,
            policy_documents,
        } => {
            let _ir =
                load_valid_ir_with_adapter_manifests(&file, &adapter_manifests, &policy_documents)?;

            println!("{}", emit_check_config(target)?);
            Ok(())
        }
    }
}

fn emit_check_config(target: CliModelTarget) -> Result<String, CliError> {
    Ok(armature_modelgen::emit_check_config(target.into())?)
}

fn load_ir(file: &PathBuf) -> Result<armature_workflow::WorkflowIr, CliError> {
    let source = load_source(file)?;
    Ok(armature_workflow::parse_source_with_file(
        &source,
        file.display().to_string(),
    )?)
}

fn load_valid_ir(file: &PathBuf) -> Result<armature_workflow::WorkflowIr, CliError> {
    let ir = load_ir(file)?;
    let report = armature_workflow::validate_ir(&ir);
    if report.is_ok() {
        Ok(ir)
    } else {
        Err(CliError::Validation {
            diagnostics: report.diagnostics,
        })
    }
}

fn load_valid_ir_with_adapter_manifests(
    file: &PathBuf,
    adapter_manifests: &[PathBuf],
    policy_documents: &[PathBuf],
) -> Result<armature_workflow::WorkflowIr, CliError> {
    let ir = load_valid_ir(file)?;
    let manifests = load_valid_adapter_manifests(adapter_manifests)?;
    let policies = load_valid_policy_documents(policy_documents)?;
    if !manifests.is_empty() || !policies.is_empty() {
        validate_ir_contracts(&ir, &manifests, &policies)?;
    }
    Ok(ir)
}

fn validate_ir_contracts(
    ir: &armature_workflow::WorkflowIr,
    manifests: &[armature_adapters::AdapterManifest],
    policies: &[armature_adapters::CapabilityPolicyDocument],
) -> Result<(), CliError> {
    let mut diagnostics = Vec::new();
    if !manifests.is_empty() {
        diagnostics.extend(armature_adapters::validate_workflow_effects(ir, manifests));
    }
    if !policies.is_empty() {
        diagnostics.extend(armature_adapters::validate_workflow_policy(
            ir, manifests, policies,
        ));
    }
    if diagnostics_have_errors(&diagnostics) {
        return Err(workflow_contract_validation_error(diagnostics));
    }
    Ok(())
}

fn load_status_for_file(
    file: &PathBuf,
    store: Option<PathBuf>,
) -> Result<armature_engine::status::WorkflowStatus, CliError> {
    let ir = load_ir(file)?;
    let workflow_id = ir.workflow.name.clone();
    let store_path = store.unwrap_or_else(|| default_store_path(file, &workflow_id));
    if store_path.exists() {
        let store = WorkflowStore::open(&store_path)?;
        stored_status(&ir, &store, &workflow_id)
    } else {
        Ok(armature_engine::status::WorkflowStatus {
            workflow_id: workflow_id.clone(),
            workflow_name: workflow_id,
            current_state: armature_engine::initial_state_name(&ir),
            blocked_reason: None,
            pending_events: 0,
            queued_events: Vec::new(),
            active_invocations: Vec::new(),
            recent_transition: None,
            recent_effects: Vec::new(),
            recent_failures: Vec::new(),
        })
    }
}

fn load_overview_for_file(
    file: &PathBuf,
    store: Option<PathBuf>,
    adapter_manifests: &[PathBuf],
    policy_documents: &[PathBuf],
) -> Result<OverviewOutput, CliError> {
    let source = load_source(file)?;
    let parsed = armature_workflow::parse_syntax_with_file(&source, file.display().to_string());
    let mut diagnostics = parsed.diagnostics;
    let status = if let Some(ir) = parsed.ir {
        diagnostics.extend(armature_workflow::validate_ir(&ir).diagnostics);
        let policies = load_policy_documents(policy_documents)?;
        if !adapter_manifests.is_empty() {
            let manifests = load_adapter_manifests(adapter_manifests)?;
            diagnostics.extend(armature_adapters::validate_adapter_manifests(&manifests));
            diagnostics.extend(armature_adapters::validate_workflow_effects(
                &ir, &manifests,
            ));
            diagnostics.extend(armature_adapters::validate_workflow_policy(
                &ir, &manifests, &policies,
            ));
        } else {
            diagnostics.extend(armature_adapters::validate_policy_documents(&policies));
        }
        let workflow_id = ir.workflow.name.clone();
        let store_path = store.unwrap_or_else(|| default_store_path(file, &workflow_id));
        Some(if store_path.exists() {
            let store = WorkflowStore::open(&store_path)?;
            stored_status(&ir, &store, &workflow_id)?
        } else {
            armature_engine::status::WorkflowStatus {
                workflow_id: workflow_id.clone(),
                workflow_name: workflow_id,
                current_state: armature_engine::initial_state_name(&ir),
                blocked_reason: None,
                pending_events: 0,
                queued_events: Vec::new(),
                active_invocations: Vec::new(),
                recent_transition: None,
                recent_effects: Vec::new(),
                recent_failures: Vec::new(),
            }
        })
    } else {
        None
    };
    let ok = !diagnostics_have_errors(&diagnostics);
    Ok(OverviewOutput {
        validation: ValidateOutput { ok, diagnostics },
        status,
    })
}

fn load_source(file: &PathBuf) -> Result<String, CliError> {
    fs::read_to_string(file).map_err(|source| CliError::Read {
        path: file.clone(),
        source,
    })
}

fn load_valid_adapter_manifests(
    manifests: &[PathBuf],
) -> Result<Vec<armature_adapters::AdapterManifest>, CliError> {
    let parsed_manifests = load_adapter_manifests(manifests)?;
    let diagnostics = armature_adapters::validate_adapter_manifests(&parsed_manifests);
    if diagnostics_have_errors(&diagnostics) {
        return Err(adapter_manifest_validation_error(diagnostics));
    }
    Ok(parsed_manifests)
}

fn load_valid_policy_documents(
    policies: &[PathBuf],
) -> Result<Vec<armature_adapters::CapabilityPolicyDocument>, CliError> {
    let parsed_policies = load_policy_documents(policies)?;
    let diagnostics = armature_adapters::validate_policy_documents(&parsed_policies);
    if diagnostics_have_errors(&diagnostics) {
        return Err(adapter_manifest_validation_error(diagnostics));
    }
    Ok(parsed_policies)
}

fn adapter_dispatcher_from_manifests(
    manifests: Vec<armature_adapters::AdapterManifest>,
    policies: Vec<armature_adapters::CapabilityPolicyDocument>,
) -> Box<dyn armature_engine::effects::EffectDispatcher> {
    if manifests.is_empty() {
        Box::new(armature_engine::effects::NoopEffectDispatcher)
    } else {
        Box::new(
            armature_adapters::ManifestEffectDispatcher::from_manifests(manifests)
                .with_policies(policies),
        )
    }
}

fn adapter_manifest_validation_error(diagnostics: Vec<armature_workflow::Diagnostic>) -> CliError {
    let message = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    CliError::AdapterManifestValidation {
        message,
        diagnostics,
    }
}

fn workflow_contract_validation_error(diagnostics: Vec<armature_workflow::Diagnostic>) -> CliError {
    let message = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == armature_workflow::Severity::Error)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    CliError::WorkflowContractValidation {
        message,
        diagnostics,
    }
}

fn load_adapter_manifests(
    manifests: &[PathBuf],
) -> Result<Vec<armature_adapters::AdapterManifest>, CliError> {
    let mut parsed_manifests = Vec::new();
    for path in manifests {
        let source = load_source(path)?;
        let manifest = serde_json::from_str::<armature_adapters::AdapterManifest>(&source)
            .map_err(|source| CliError::AdapterManifest {
                path: path.clone(),
                source,
            })?;
        parsed_manifests.push(manifest);
    }

    Ok(parsed_manifests)
}

fn load_policy_documents(
    policies: &[PathBuf],
) -> Result<Vec<armature_adapters::CapabilityPolicyDocument>, CliError> {
    let mut parsed_policies = Vec::new();
    for path in policies {
        let source = load_source(path)?;
        let policy = serde_json::from_str::<armature_adapters::CapabilityPolicyDocument>(&source)
            .map_err(|source| CliError::PolicyDocument {
            path: path.clone(),
            source,
        })?;
        parsed_policies.push(policy);
    }

    Ok(parsed_policies)
}

fn diagnostics_have_errors(diagnostics: &[armature_workflow::Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == armature_workflow::Severity::Error)
}

fn print_json(value: &impl Serialize) -> Result<(), CliError> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_cli_error(error: &CliError) {
    eprintln!("error: {error}");
    match error {
        CliError::Validation { diagnostics } => print_diagnostics(diagnostics),
        CliError::AdapterManifestValidation { diagnostics, .. } => print_diagnostics(diagnostics),
        CliError::WorkflowContractValidation { diagnostics, .. } => print_diagnostics(diagnostics),
        _ => {}
    }
}

fn print_diagnostics(diagnostics: &[armature_workflow::Diagnostic]) {
    for diagnostic in diagnostics {
        eprintln!("{}", format_diagnostic(diagnostic));
    }
}

fn format_diagnostic(diagnostic: &armature_workflow::Diagnostic) -> String {
    let severity = match diagnostic.severity {
        armature_workflow::Severity::Error => "error",
        armature_workflow::Severity::Warning => "warning",
        armature_workflow::Severity::Note => "note",
    };
    match &diagnostic.span {
        Some(span) => format!(
            "{}:{}:{}: {severity}: {}",
            span.file, span.start_line, span.start_column, diagnostic.message
        ),
        None => format!("{severity}: {}", diagnostic.message),
    }
}

fn print_overview(overview: &OverviewOutput) {
    println!(
        "validation: {}",
        if overview.validation.ok {
            "ok"
        } else {
            "failed"
        }
    );
    for diagnostic in &overview.validation.diagnostics {
        println!("  {}", format_diagnostic(diagnostic));
    }

    let Some(status) = &overview.status else {
        println!("runtime: unavailable");
        return;
    };

    println!("workflow: {}", status.workflow_name);
    println!("state: {}", status.current_state);
    println!("pending events: {}", status.pending_events);

    if status.active_invocations.is_empty() {
        println!("active: none");
    } else {
        println!("active:");
        for invocation in &status.active_invocations {
            match invocation.max {
                Some(max) => println!("  {}: {}/{}", invocation.agent, invocation.count, max),
                None => println!("  {}: {}", invocation.agent, invocation.count),
            }
        }
    }

    if status.queued_events.is_empty() {
        println!("queued events: none");
    } else {
        println!("queued events:");
        for event in &status.queued_events {
            println!(
                "  {} {} attempts={}",
                event.event_id, event.event_type, event.attempt_count
            );
        }
    }

    if let Some(transition) = &status.recent_transition {
        println!("latest transition: {transition}");
    } else {
        println!("latest transition: none");
    }

    if status.recent_effects.is_empty() {
        println!("latest effects: none");
    } else {
        println!("latest effects:");
        for effect in &status.recent_effects {
            let target = effect
                .target
                .as_deref()
                .map(|target| format!(" {target}"))
                .unwrap_or_default();
            let required_capabilities = if effect.required_capabilities.is_empty() {
                String::new()
            } else {
                format!(" requires={}", effect.required_capabilities.join(","))
            };
            let error = effect
                .error
                .as_deref()
                .map(|error| format!(" error={error}"))
                .unwrap_or_default();
            println!(
                "  {}{} {:?}{}{}",
                effect.effect, target, effect.status, required_capabilities, error
            );
        }
    }

    if status.recent_failures.is_empty() {
        println!("recent failures: none");
    } else {
        println!("recent failures:");
        for failure in &status.recent_failures {
            println!("  {failure}");
        }
    }
}

fn build_cli_event(
    ir: &armature_workflow::WorkflowIr,
    manifests: &[armature_adapters::AdapterManifest],
    workflow_id: &str,
    event_type: String,
    payload: String,
) -> Result<WorkflowEvent, CliError> {
    let payload: serde_json::Value = serde_json::from_str(&payload)?;
    let mut declared = false;
    let mut schema_matches = true;

    if let Some(event_schema) = ir.events.get(&event_type).map(|event| &event.payload) {
        declared = true;
        schema_matches &= event_schema.accepts_json_with_types(&payload, &ir.types);
    }

    for manifest in manifests {
        if let Some(event_schema) = manifest.events.get(&event_type) {
            declared = true;
            schema_matches &= event_schema.accepts_json_with_types(&payload, &manifest.types);
        }
    }

    if !declared {
        return Err(CliError::InvalidEvent(format!(
            "event `{event_type}` is not declared"
        )));
    }

    if !schema_matches {
        return Err(CliError::InvalidEvent(format!(
            "payload does not match schema for event `{event_type}`"
        )));
    }

    Ok(WorkflowEvent {
        event_id: format!("evt_cli_{}", ulid::Ulid::new()),
        workflow_id: workflow_id.to_string(),
        event_type,
        payload,
        source: None,
        occurred_at: None,
        enqueued_at: None,
        correlation_id: None,
        causation_id: None,
        dedupe_key: None,
        status: EventStatus::Queued,
        attempt_count: 0,
        last_error: None,
    })
}

fn stored_status(
    ir: &armature_workflow::WorkflowIr,
    store: &WorkflowStore,
    workflow_id: &str,
) -> Result<armature_engine::status::WorkflowStatus, CliError> {
    Ok(armature_engine::project_status(ir, store, workflow_id)?)
}

fn check_workflow(
    ir: &armature_workflow::WorkflowIr,
    target: CliModelTarget,
) -> Result<CheckOutput, CliError> {
    match target {
        CliModelTarget::Tla => check_tla_workflow(ir),
        CliModelTarget::Maude => check_maude_workflow(ir),
        CliModelTarget::Apalache | CliModelTarget::Veil => {
            let _ = armature_modelgen::emit_model(ir, target.into())?;
            unreachable!("unsupported targets currently return modelgen errors")
        }
    }
}

fn check_tla_workflow(ir: &armature_workflow::WorkflowIr) -> Result<CheckOutput, CliError> {
    let model = armature_modelgen::emit_model(ir, armature_modelgen::ModelTarget::Tla)?;
    let module_name = tla_module_name(&model).unwrap_or_else(|| "ArmatureModel".to_string());
    let base = std::env::temp_dir().join(format!("{module_name}_{}", ulid::Ulid::new()));
    fs::create_dir_all(&base).map_err(|source| CliError::CreateDir {
        path: base.clone(),
        source,
    })?;
    let model_path = base.join(format!("{module_name}.tla"));
    let config_path = base.join(format!("{module_name}.cfg"));
    fs::write(&model_path, model).map_err(|source| CliError::Write {
        path: model_path.clone(),
        source,
    })?;
    fs::write(&config_path, armature_modelgen::emit_tla_check_config()).map_err(|source| {
        CliError::Write {
            path: config_path.clone(),
            source,
        }
    })?;

    let output = run_check_tool(
        "tlc",
        &[
            OsString::from("-deadlock"),
            OsString::from("-config"),
            config_path.as_os_str().to_owned(),
            model_path.as_os_str().to_owned(),
        ],
    )?;

    let check_output = CheckOutput {
        ok: output.status.success(),
        target: "tla".to_string(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        artifacts: vec![model_path, config_path],
    };

    if check_output.ok {
        Ok(check_output)
    } else {
        Err(CliError::CheckFailed {
            status: output.status,
            stdout: check_output.stdout,
            stderr: check_output.stderr,
        })
    }
}

fn check_maude_workflow(ir: &armature_workflow::WorkflowIr) -> Result<CheckOutput, CliError> {
    let model = armature_modelgen::emit_model(ir, armature_modelgen::ModelTarget::Maude)?;
    let module_name = maude_module_name(&model).unwrap_or_else(|| "ARMATURE-MODEL".to_string());
    let base = std::env::temp_dir().join(format!("{module_name}_{}", ulid::Ulid::new()));
    fs::create_dir_all(&base).map_err(|source| CliError::CreateDir {
        path: base.clone(),
        source,
    })?;
    let model_path = base.join(format!("{module_name}.maude"));
    fs::write(&model_path, model).map_err(|source| CliError::Write {
        path: model_path.clone(),
        source,
    })?;

    let output = run_check_tool("maude", &[model_path.as_os_str().to_owned()])?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let ok = output.status.success() && stdout.contains("No solution.");
    let check_output = CheckOutput {
        ok,
        target: "maude".to_string(),
        stdout,
        stderr,
        artifacts: vec![model_path],
    };

    if check_output.ok {
        Ok(check_output)
    } else {
        Err(CliError::CheckFailed {
            status: output.status,
            stdout: check_output.stdout,
            stderr: check_output.stderr,
        })
    }
}

fn tla_module_name(model: &str) -> Option<String> {
    let line = model.lines().next()?;
    line.strip_prefix("---- MODULE ")?
        .strip_suffix(" ----")
        .map(str::to_string)
}

fn run_check_tool(tool: &'static str, args: &[OsString]) -> Result<Output, CliError> {
    match ProcessCommand::new(tool).args(args).output() {
        Ok(output) => Ok(output),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            run_check_tool_through_nix(tool, args)
        }
        Err(source) => Err(CliError::CheckToolExecution { tool, source }),
    }
}

fn run_check_tool_through_nix(tool: &'static str, args: &[OsString]) -> Result<Output, CliError> {
    let output = ProcessCommand::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "develop",
            "-c",
        ])
        .arg(tool)
        .args(args)
        .output()
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                CliError::CheckToolUnavailable(tool)
            } else {
                CliError::CheckToolExecution {
                    tool: "nix",
                    source,
                }
            }
        })?;

    Ok(output)
}

fn maude_module_name(model: &str) -> Option<String> {
    let line = model.lines().next()?;
    line.strip_prefix("mod ")?
        .strip_suffix(" is")
        .map(str::to_string)
}

fn build_workflow(
    file: &Path,
    ir: &armature_workflow::WorkflowIr,
    out: Option<PathBuf>,
    manifests: &[armature_adapters::AdapterManifest],
    policies: &[armature_adapters::CapabilityPolicyDocument],
) -> Result<BuildOutput, CliError> {
    let output_dir = out.unwrap_or_else(|| {
        file.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".armature")
            .join("build")
            .join("workflows")
            .join(&ir.workflow.name)
    });
    fs::create_dir_all(&output_dir).map_err(|source| CliError::CreateDir {
        path: output_dir.clone(),
        source,
    })?;

    let ir_json = output_dir.join("workflow-ir.json");
    let baml_dir = output_dir.join("baml_src");
    let baml_src = baml_dir.join("workflow.baml");
    let tla_source = armature_modelgen::emit_model(ir, armature_modelgen::ModelTarget::Tla)?;
    let tla_name = tla_module_name(&tla_source).unwrap_or_else(|| ir.workflow.name.clone());
    let tla_model = output_dir.join(format!("{tla_name}.tla"));
    let tla_config = output_dir.join(format!("{tla_name}.cfg"));
    let maude_source = armature_modelgen::emit_model(ir, armature_modelgen::ModelTarget::Maude)?;
    let maude_name = maude_module_name(&maude_source).unwrap_or_else(|| ir.workflow.name.clone());
    let maude_model = output_dir.join(format!("{maude_name}.maude"));
    let adapter_manifest_bundle =
        (!manifests.is_empty()).then(|| output_dir.join("adapter-manifests.json"));
    let policy_document_bundle =
        (!policies.is_empty()).then(|| output_dir.join("policy-documents.json"));
    let built_ir = ir_with_build_artifacts(ir, &baml_src);
    fs::create_dir_all(&baml_dir).map_err(|source| CliError::CreateDir {
        path: baml_dir.clone(),
        source,
    })?;
    fs::write(
        &ir_json,
        serde_json::to_string_pretty(&built_ir).expect("workflow IR serializes"),
    )
    .map_err(|source| CliError::Write {
        path: ir_json.clone(),
        source,
    })?;
    fs::write(&baml_src, emit_baml_source(ir)).map_err(|source| CliError::Write {
        path: baml_src.clone(),
        source,
    })?;
    fs::write(&tla_model, tla_source).map_err(|source| CliError::Write {
        path: tla_model.clone(),
        source,
    })?;
    fs::write(&tla_config, armature_modelgen::emit_tla_check_config()).map_err(|source| {
        CliError::Write {
            path: tla_config.clone(),
            source,
        }
    })?;
    fs::write(&maude_model, maude_source).map_err(|source| CliError::Write {
        path: maude_model.clone(),
        source,
    })?;
    if let Some(path) = &adapter_manifest_bundle {
        fs::write(
            path,
            serde_json::to_string_pretty(manifests).expect("adapter manifests serialize"),
        )
        .map_err(|source| CliError::Write {
            path: path.clone(),
            source,
        })?;
    }
    if let Some(path) = &policy_document_bundle {
        fs::write(
            path,
            serde_json::to_string_pretty(policies).expect("policy documents serialize"),
        )
        .map_err(|source| CliError::Write {
            path: path.clone(),
            source,
        })?;
    }

    Ok(BuildOutput {
        workflow_id: ir.workflow.name.clone(),
        output_dir,
        ir_json,
        baml_src,
        tla_model,
        tla_config,
        maude_model,
        adapter_manifest_bundle,
        policy_document_bundle,
    })
}

fn ir_with_build_artifacts(
    ir: &armature_workflow::WorkflowIr,
    baml_src: &Path,
) -> armature_workflow::WorkflowIr {
    let mut ir = ir.clone();
    let artifact = baml_src.to_string_lossy().to_string();
    for function in ir.coerce_functions.values_mut() {
        function.generated_baml_artifact = Some(artifact.clone());
    }
    ir
}

fn emit_baml_source(ir: &armature_workflow::WorkflowIr) -> String {
    let mut output = String::new();
    let emitted_types = baml_reachable_types(ir);

    for (name, schema) in &ir.types {
        if !emitted_types.contains(name) {
            continue;
        }
        match schema {
            armature_workflow::schema::Schema::Enum { values } => {
                output.push_str(&format!("enum {name} {{\n"));
                for value in values {
                    output.push_str(&format!("  {value}\n"));
                }
                output.push_str("}\n\n");
            }
            armature_workflow::schema::Schema::Record { fields } => {
                output.push_str(&format!("class {name} {{\n"));
                for field in fields {
                    output.push_str(&format!("  {} {}\n", field.name, baml_type(&field.schema)));
                }
                output.push_str("}\n\n");
            }
            _ => {}
        }
    }

    let mut model_clients = std::collections::BTreeMap::new();
    for function in ir.coerce_functions.values() {
        if let Some(model) = &function.model {
            let next_index = model_clients.len() + 1;
            model_clients
                .entry(model.clone())
                .or_insert_with(|| format!("ArmatureLLM{next_index}"));
        }
    }

    for (model, client_name) in &model_clients {
        output.push_str(&format!(
            "client<llm> {client_name} {{\n  provider \"openai\"\n  options {{\n    model \"{}\"\n    api_key env.OPENAI_API_KEY\n  }}\n}}\n\n",
            escape_baml_string(model)
        ));
    }

    for (name, function) in &ir.coerce_functions {
        let params = function
            .params
            .iter()
            .map(|param| format!("{}: {}", param.name, baml_type(&param.schema)))
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!(
            "function {name}({params}) -> {} {{\n",
            baml_type(&function.output)
        ));
        if let Some(model) = &function.model {
            if let Some(client_name) = model_clients.get(model) {
                output.push_str(&format!("  client {client_name}\n"));
            }
        }
        output.push_str("  prompt #\"");
        output.push('\n');
        output.push_str(function.prompt.as_deref().unwrap_or(""));
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("  \"#\n");
        output.push_str("}\n\n");
    }

    output
}

fn baml_reachable_types(ir: &armature_workflow::WorkflowIr) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for function in ir.coerce_functions.values() {
        for param in &function.params {
            collect_baml_type_refs(&param.schema, ir, &mut names);
        }
        collect_baml_type_refs(&function.output, ir, &mut names);
    }
    names
}

fn collect_baml_type_refs(
    schema: &armature_workflow::schema::Schema,
    ir: &armature_workflow::WorkflowIr,
    names: &mut BTreeSet<String>,
) {
    match schema {
        armature_workflow::schema::Schema::Ref { name } => {
            if names.insert(name.clone()) {
                if let Some(schema) = ir.types.get(name) {
                    collect_baml_type_refs(schema, ir, names);
                }
            }
        }
        armature_workflow::schema::Schema::Optional { inner }
        | armature_workflow::schema::Schema::List { inner }
        | armature_workflow::schema::Schema::Set { inner } => {
            collect_baml_type_refs(inner, ir, names);
        }
        armature_workflow::schema::Schema::Map { key, value } => {
            collect_baml_type_refs(key, ir, names);
            collect_baml_type_refs(value, ir, names);
        }
        armature_workflow::schema::Schema::Union { variants } => {
            for variant in variants {
                collect_baml_type_refs(variant, ir, names);
            }
        }
        armature_workflow::schema::Schema::Record { fields } => {
            for field in fields {
                collect_baml_type_refs(&field.schema, ir, names);
            }
        }
        armature_workflow::schema::Schema::String
        | armature_workflow::schema::Schema::Int
        | armature_workflow::schema::Schema::Float
        | armature_workflow::schema::Schema::Boolean
        | armature_workflow::schema::Schema::Null
        | armature_workflow::schema::Schema::Time
        | armature_workflow::schema::Schema::Duration
        | armature_workflow::schema::Schema::Agent
        | armature_workflow::schema::Schema::Literal { .. }
        | armature_workflow::schema::Schema::Enum { .. }
        | armature_workflow::schema::Schema::Json => {}
    }
}

fn baml_type(schema: &armature_workflow::schema::Schema) -> String {
    match schema {
        armature_workflow::schema::Schema::String
        | armature_workflow::schema::Schema::Time
        | armature_workflow::schema::Schema::Duration
        | armature_workflow::schema::Schema::Agent => "string".to_string(),
        armature_workflow::schema::Schema::Int => "int".to_string(),
        armature_workflow::schema::Schema::Float => "float".to_string(),
        armature_workflow::schema::Schema::Boolean => "bool".to_string(),
        armature_workflow::schema::Schema::Null => "null".to_string(),
        armature_workflow::schema::Schema::Literal { value } => match value {
            serde_json::Value::String(value) => format!("\"{}\"", escape_baml_string(value)),
            serde_json::Value::Bool(value) => value.to_string(),
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::Null => "null".to_string(),
            _ => "string".to_string(),
        },
        armature_workflow::schema::Schema::Enum { values } => values.join(" | "),
        armature_workflow::schema::Schema::Optional { inner } => format!("{}?", baml_type(inner)),
        armature_workflow::schema::Schema::List { inner }
        | armature_workflow::schema::Schema::Set { inner } => format!("{}[]", baml_type(inner)),
        armature_workflow::schema::Schema::Map { key, value } => {
            format!("map<{}, {}>", baml_type(key), baml_type(value))
        }
        armature_workflow::schema::Schema::Union { variants } => variants
            .iter()
            .map(baml_type)
            .collect::<Vec<_>>()
            .join(" | "),
        armature_workflow::schema::Schema::Record { .. }
        | armature_workflow::schema::Schema::Json => "string".to_string(),
        armature_workflow::schema::Schema::Ref { name } => name.clone(),
    }
}

fn escape_baml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn default_store_path(file: &Path, workflow_name: &str) -> PathBuf {
    let base = file.parent().unwrap_or_else(|| Path::new("."));
    base.join(".armature")
        .join("workflows")
        .join(format!("{workflow_name}.sqlite"))
}

fn ensure_parent_dir(path: &Path) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| CliError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}
