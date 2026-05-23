use armature_engine::queue::{EventStatus, WorkflowEvent};
use armature_engine::storage::WorkflowStore;
use armature_engine::WorkflowRuntime;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::process::Output;
use thiserror::Error;

const MAX_INSPECTION_LIMIT: usize = 10_000;

#[derive(Debug, Parser)]
#[command(name = "armature")]
#[command(about = "Armature statechart workflow CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliEventStatus {
    Queued,
    Processing,
    Processed,
    Ignored,
    Failed,
    #[value(name = "dead_lettered", alias = "dead-lettered")]
    DeadLettered,
}

impl From<CliEventStatus> for EventStatus {
    fn from(value: CliEventStatus) -> Self {
        match value {
            CliEventStatus::Queued => EventStatus::Queued,
            CliEventStatus::Processing => EventStatus::Processing,
            CliEventStatus::Processed => EventStatus::Processed,
            CliEventStatus::Ignored => EventStatus::Ignored,
            CliEventStatus::Failed => EventStatus::Failed,
            CliEventStatus::DeadLettered => EventStatus::DeadLettered,
        }
    }
}

fn cli_event_status_name(status: EventStatus) -> &'static str {
    match status {
        EventStatus::Queued => "queued",
        EventStatus::Processing => "processing",
        EventStatus::Processed => "processed",
        EventStatus::Ignored => "ignored",
        EventStatus::Failed => "failed",
        EventStatus::DeadLettered => "dead_lettered",
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a minimal local workflow project.
    Init {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Machine name for the generated workflow file.
        #[arg(long, default_value = "Workflow")]
        name: String,
        /// Overwrite existing scaffold files.
        #[arg(long)]
        force: bool,
        #[arg(long)]
        json: bool,
    },
    /// Parse and statically validate a workflow file.
    Validate {
        file: PathBuf,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
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
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Supply built-in human-review response event schema for emit intake.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Supply built-in agent completion event schema for emit intake.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
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
        /// Testing: provide deterministic coerce output as NAME=JSON.
        #[arg(long = "fake-coerce-output")]
        fake_coerce_outputs: Vec<String>,
        /// Testing: provide deterministic capability/expression call output as NAME=JSON.
        #[arg(long = "fake-call-output")]
        fake_call_outputs: Vec<String>,
        /// Use a JSON file as the built-in plan adapter backing store.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Use a JSON file as the built-in human-review obligation store.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Use a JSON file as the built-in agent invocation/message store.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
        /// Call real BAML functions through an already-running baml-cli serve endpoint.
        #[arg(long = "baml-url")]
        baml_url: Option<String>,
        #[arg(long = "baml-timeout-ms", default_value_t = 30_000)]
        baml_timeout_ms: u64,
        #[arg(long)]
        json: bool,
    },
    /// Show persisted workflow status.
    Status {
        file: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
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
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show durable workflow events.
    Events {
        file: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
        #[arg(long, value_enum)]
        status: Option<CliEventStatus>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Retry a failed or dead-lettered workflow event.
    RetryEvent {
        file: PathBuf,
        #[arg(long = "event-id")]
        event_id: String,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show the append-only workflow log.
    Log {
        file: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
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
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
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
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
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
        /// Include the built-in JSON plan adapter manifest in validation/build metadata.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest in validation/build metadata.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest in validation/build metadata.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
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
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
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
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file")]
        agent_file: Option<PathBuf>,
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
    #[error("refusing to overwrite existing `{path}`; pass --force to replace scaffold files")]
    InitPathExists { path: PathBuf },
    #[error("invalid workflow name `{name}`; use a .armature identifier such as `Workflow` or `spec-implementation`")]
    InvalidWorkflowName { name: String },
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
    #[error("invalid fake output `{0}`; expected NAME=JSON")]
    InvalidFakeOutput(String),
    #[error("duplicate fake output `{0}`")]
    DuplicateFakeOutput(String),
    #[error("limit for {command} must be <= {max}; got {limit}")]
    InvalidLimit {
        command: &'static str,
        limit: usize,
        max: usize,
    },
    #[error("fake output `{name}` is not valid JSON: {source}")]
    FakeOutputJson {
        name: String,
        source: serde_json::Error,
    },
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
    #[error("policy document validation failed: {message}")]
    PolicyDocumentValidation {
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
    #[error("adapter bridge error: {0}")]
    AdapterBridge(#[from] armature_engine::effects::EffectError),
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
}

#[derive(Debug, Serialize)]
struct ValidateOutput {
    ok: bool,
    diagnostics: Vec<armature_workflow::Diagnostic>,
}

#[derive(Debug, Serialize)]
struct InitOutput {
    root: PathBuf,
    workflow_name: String,
    workflow: PathBuf,
    policy: PathBuf,
    state_dir: PathBuf,
    workflow_store_dir: PathBuf,
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
struct RetryEventOutput {
    workflow_id: String,
    event: WorkflowEvent,
    status: armature_engine::status::WorkflowStatus,
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
    checks: Vec<CheckOutput>,
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
    artifact_hashes_json: PathBuf,
    artifact_hashes: BTreeMap<String, String>,
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
        Command::Init {
            dir,
            name,
            force,
            json,
        } => {
            let output = init_project(&dir, &name, force)?;
            if json {
                print_json(&output)?;
            } else {
                println!("created {}", output.workflow.display());
                println!("created {}", output.policy.display());
                println!("created {}", output.state_dir.display());
                println!("created {}", output.workflow_store_dir.display());
            }
            Ok(())
        }
        Command::Validate {
            file,
            adapter_manifests,
            policy_documents,
            plan_file,
            review_file,
            agent_file,
            json,
        } => {
            let source = load_source(&file)?;
            let parsed =
                armature_workflow::parse_syntax_with_file(&source, file.display().to_string());
            let mut diagnostics = parsed.diagnostics;
            if let Some(ir) = parsed.ir {
                diagnostics.extend(armature_workflow::validate_ir(&ir).diagnostics);
                let policies = load_policy_documents(&policy_documents)?;
                if !adapter_manifests.is_empty()
                    || plan_file.is_some()
                    || review_file.is_some()
                    || agent_file.is_some()
                {
                    let mut manifests = load_adapter_manifests(&adapter_manifests)?;
                    add_file_backed_manifests_if_needed(
                        &mut manifests,
                        plan_file.is_some(),
                        review_file.is_some(),
                        agent_file.is_some(),
                    );
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
            policy_documents,
            review_file,
            agent_file,
            json,
        } => {
            let ir = load_valid_ir(&file)?;
            let mut manifests = load_valid_adapter_manifests(&adapter_manifests)?;
            if review_file.is_some() {
                add_json_human_review_response_event_manifest_if_needed(&mut manifests);
            }
            if agent_file.is_some() {
                add_json_agent_finished_event_manifest_if_needed(&mut manifests);
            }
            let _policies = load_valid_policy_documents(&policy_documents)?;
            let workflow_id = ir.workflow.name.clone();
            let store_path = store.unwrap_or_else(|| default_store_path(&file, &workflow_id));
            ensure_parent_dir(&store_path)?;
            let store = WorkflowStore::open(&store_path)?;
            let event = build_cli_event(&ir, &manifests, &workflow_id, event, payload)?;
            if event.event_type == "humanReview.responded" {
                if let Some(review_file) = review_file.as_deref() {
                    armature_adapters::record_human_review_response(review_file, &event.payload)?;
                }
            }
            if event.event_type == "finished" {
                if let Some(agent_file) = agent_file.as_deref() {
                    armature_adapters::record_agent_finished_event(agent_file, &event.payload)?;
                }
            }
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
            fake_coerce_outputs,
            fake_call_outputs,
            plan_file,
            review_file,
            agent_file,
            baml_url,
            baml_timeout_ms,
            json,
        } => {
            let mut manifests = load_valid_adapter_manifests(&adapter_manifests)?;
            if plan_file.is_some() {
                add_json_plan_manifest_if_needed(&mut manifests);
            }
            if review_file.is_some() {
                add_json_human_review_manifest_if_needed(&mut manifests);
            }
            if agent_file.is_some() {
                add_json_agent_manifest_if_needed(&mut manifests);
            }
            let policies = load_valid_policy_documents(&policy_documents)?;
            let fake_coerce_outputs = parse_fake_outputs(&fake_coerce_outputs)?;
            if fake_coerce_outputs.is_empty() {
                if let Some(url) = &baml_url {
                    let diagnostics = armature_adapters::validate_baml_http_policy(&policies, url);
                    if diagnostics_have_errors(&diagnostics) {
                        return Err(policy_document_validation_error(diagnostics));
                    }
                }
            }
            let ir = load_valid_ir_with_contracts(&file, &manifests, &policies)?;
            let store_baml_raw_response =
                armature_adapters::should_store_baml_raw_response(&policies);
            let workflow_id = ir.workflow.name.clone();
            let store_path = store.unwrap_or_else(|| default_store_path(&file, &workflow_id));
            ensure_parent_dir(&store_path)?;
            let store = WorkflowStore::open(&store_path)?;
            let dispatcher = adapter_dispatcher_from_manifests(
                manifests.clone(),
                policies,
                plan_file.clone(),
                review_file.clone(),
                agent_file.clone(),
            );
            let fake_call_outputs = parse_fake_outputs(&fake_call_outputs)?;
            let coerce_executor = coerce_executor_for_run(
                fake_coerce_outputs,
                baml_url,
                Some(baml_timeout_ms),
                &ir,
                store_baml_raw_response,
            );
            let mut runtime = WorkflowRuntime::with_dispatcher_and_coerce_executor(
                ir,
                store,
                dispatcher,
                coerce_executor,
            )?
            .with_fake_call_outputs(fake_call_outputs);
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
        Command::Status {
            file,
            store,
            adapter_manifests,
            policy_documents,
            plan_file,
            review_file,
            agent_file,
            json,
        } => {
            let status = load_status_for_file(
                &file,
                store,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;

            if json {
                print_json(&status)?;
            } else {
                print_status(&status);
            }
            Ok(())
        }
        Command::Overview {
            file,
            store,
            adapter_manifests,
            policy_documents,
            plan_file,
            review_file,
            agent_file,
            json,
        } => {
            let overview = load_overview_for_file(
                &file,
                store,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;

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
            adapter_manifests,
            policy_documents,
            plan_file,
            review_file,
            agent_file,
            status,
            limit,
            json,
        } => {
            validate_inspection_limit("events", limit)?;
            let (ir, _manifests, _policies) = load_valid_ir_with_contract_shortcuts(
                &file,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;
            let workflow_id = ir.workflow.name.clone();
            let store_path = store.unwrap_or_else(|| default_store_path(&file, &workflow_id));
            let events = if store_path.exists() {
                let store = WorkflowStore::open(&store_path)?;
                if let Some(status) = status {
                    store.events_by_status(&workflow_id, status.into(), limit)?
                } else {
                    store.events(&workflow_id, limit)?
                }
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
                    print!(
                        "{} {} {}",
                        event.event_id,
                        event.event_type,
                        cli_event_status_name(event.status)
                    );
                    if event.attempt_count > 0 {
                        print!(" attempts={}", event.attempt_count);
                    }
                    if let Some(last_error) = &event.last_error {
                        print!(" error={}", last_error);
                    }
                    println!();
                }
            }
            Ok(())
        }
        Command::RetryEvent {
            file,
            event_id,
            store,
            adapter_manifests,
            policy_documents,
            plan_file,
            review_file,
            agent_file,
            json,
        } => {
            let (ir, _manifests, _policies) = load_valid_ir_with_contract_shortcuts(
                &file,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;
            let workflow_id = ir.workflow.name.clone();
            let store_path = store.unwrap_or_else(|| default_store_path(&file, &workflow_id));
            if !store_path.exists() {
                return Err(armature_engine::storage::StorageError::EventNotFound {
                    workflow_id,
                    event_id,
                }
                .into());
            }
            let store = WorkflowStore::open(&store_path)?;
            let event = store.retry_event(&workflow_id, &event_id)?;
            let status = stored_status(&ir, &store, &workflow_id)?;
            let output = RetryEventOutput {
                workflow_id,
                event,
                status,
            };

            if json {
                print_json(&output)?;
            } else {
                println!(
                    "retried {} status={} pending_events={}",
                    output.event.event_id,
                    cli_event_status_name(output.event.status),
                    output.status.pending_events
                );
            }
            Ok(())
        }
        Command::Log {
            file,
            store,
            adapter_manifests,
            policy_documents,
            plan_file,
            review_file,
            agent_file,
            limit,
            json,
        } => {
            validate_inspection_limit("log", limit)?;
            let (ir, _manifests, _policies) = load_valid_ir_with_contract_shortcuts(
                &file,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;
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
            plan_file,
            review_file,
            agent_file,
            json,
        } => {
            let (ir, _manifests, _policies) = load_valid_ir_with_contract_shortcuts(
                &file,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;
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
            plan_file,
            review_file,
            agent_file,
            json,
        } => {
            let (ir, _manifests, _policies) = load_valid_ir_with_contract_shortcuts(
                &file,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;
            let output = prove_workflow(&ir)?;

            if json {
                print_json(&output)?;
            } else {
                for check in &output.checks {
                    println!("== {} ==", check.target);
                    print!("{}", check.stdout);
                    eprint!("{}", check.stderr);
                }
                if output.ok {
                    println!("ok");
                }
            }
            Ok(())
        }
        Command::Build {
            file,
            out,
            adapter_manifests,
            policy_documents,
            plan_file,
            review_file,
            agent_file,
            json,
        } => {
            let (ir, manifests, policies) = load_valid_ir_with_contract_shortcuts(
                &file,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;
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
            plan_file,
            review_file,
            agent_file,
        } => {
            let (ir, _manifests, _policies) = load_valid_ir_with_contract_shortcuts(
                &file,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;

            println!("{}", armature_modelgen::emit_model(&ir, target.into())?);
            Ok(())
        }
        Command::EmitConfig {
            file,
            target,
            adapter_manifests,
            policy_documents,
            plan_file,
            review_file,
            agent_file,
        } => {
            let (_ir, _manifests, _policies) = load_valid_ir_with_contract_shortcuts(
                &file,
                &adapter_manifests,
                &policy_documents,
                plan_file.is_some(),
                review_file.is_some(),
                agent_file.is_some(),
            )?;

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

fn load_valid_ir_with_contract_shortcuts(
    file: &PathBuf,
    adapter_manifests: &[PathBuf],
    policy_documents: &[PathBuf],
    include_plan_manifest: bool,
    include_review_manifest: bool,
    include_agent_manifest: bool,
) -> Result<
    (
        armature_workflow::WorkflowIr,
        Vec<armature_adapters::AdapterManifest>,
        Vec<armature_adapters::CapabilityPolicyDocument>,
    ),
    CliError,
> {
    let mut manifests = load_valid_adapter_manifests(adapter_manifests)?;
    add_file_backed_manifests_if_needed(
        &mut manifests,
        include_plan_manifest,
        include_review_manifest,
        include_agent_manifest,
    );
    let policies = load_valid_policy_documents(policy_documents)?;
    let ir = load_valid_ir_with_contracts(file, &manifests, &policies)?;
    Ok((ir, manifests, policies))
}

fn load_valid_ir_with_contracts(
    file: &PathBuf,
    manifests: &[armature_adapters::AdapterManifest],
    policies: &[armature_adapters::CapabilityPolicyDocument],
) -> Result<armature_workflow::WorkflowIr, CliError> {
    let ir = load_valid_ir(file)?;
    if !manifests.is_empty() || !policies.is_empty() {
        validate_ir_contracts(&ir, manifests, policies)?;
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
    adapter_manifests: &[PathBuf],
    policy_documents: &[PathBuf],
    include_plan_manifest: bool,
    include_review_manifest: bool,
    include_agent_manifest: bool,
) -> Result<armature_engine::status::WorkflowStatus, CliError> {
    let mut manifests = load_valid_adapter_manifests(adapter_manifests)?;
    add_file_backed_manifests_if_needed(
        &mut manifests,
        include_plan_manifest,
        include_review_manifest,
        include_agent_manifest,
    );
    let policies = load_valid_policy_documents(policy_documents)?;
    let ir = load_valid_ir_with_contracts(file, &manifests, &policies)?;
    let workflow_id = ir.workflow.name.clone();
    let store_path = store.unwrap_or_else(|| default_store_path(file, &workflow_id));
    if store_path.exists() {
        let store = WorkflowStore::open(&store_path)?;
        stored_status(&ir, &store, &workflow_id)
    } else {
        let data = armature_engine::initial_context_from_ir(&ir);
        Ok(armature_engine::status::WorkflowStatus {
            workflow_id: workflow_id.clone(),
            workflow_name: workflow_id,
            current_state: armature_engine::initial_state_name(&ir),
            blocked_reason: None,
            data_summary: armature_engine::summarize_status_data(&data),
            data,
            pending_events: 0,
            queued_events: Vec::new(),
            active_invocations: Vec::new(),
            recent_transition: None,
            recent_effects: Vec::new(),
            latest_coerce_calls: Vec::new(),
            latest_coerce_failures: Vec::new(),
            policy_blockers: Vec::new(),
            recent_failures: Vec::new(),
        })
    }
}

fn load_overview_for_file(
    file: &PathBuf,
    store: Option<PathBuf>,
    adapter_manifests: &[PathBuf],
    policy_documents: &[PathBuf],
    include_plan_manifest: bool,
    include_review_manifest: bool,
    include_agent_manifest: bool,
) -> Result<OverviewOutput, CliError> {
    let source = load_source(file)?;
    let parsed = armature_workflow::parse_syntax_with_file(&source, file.display().to_string());
    let mut diagnostics = parsed.diagnostics;
    let status = if let Some(ir) = parsed.ir {
        diagnostics.extend(armature_workflow::validate_ir(&ir).diagnostics);
        let policies = load_policy_documents(policy_documents)?;
        if !adapter_manifests.is_empty()
            || include_plan_manifest
            || include_review_manifest
            || include_agent_manifest
        {
            let mut manifests = load_adapter_manifests(adapter_manifests)?;
            add_file_backed_manifests_if_needed(
                &mut manifests,
                include_plan_manifest,
                include_review_manifest,
                include_agent_manifest,
            );
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
            let data = armature_engine::initial_context_from_ir(&ir);
            armature_engine::status::WorkflowStatus {
                workflow_id: workflow_id.clone(),
                workflow_name: workflow_id,
                current_state: armature_engine::initial_state_name(&ir),
                blocked_reason: None,
                data_summary: armature_engine::summarize_status_data(&data),
                data,
                pending_events: 0,
                queued_events: Vec::new(),
                active_invocations: Vec::new(),
                recent_transition: None,
                recent_effects: Vec::new(),
                latest_coerce_calls: Vec::new(),
                latest_coerce_failures: Vec::new(),
                policy_blockers: Vec::new(),
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

const INIT_WORKFLOW_TEMPLATE: &str = r#"machine {name}
initial waiting

event start {
  message string
}

state waiting {
  on start as evt {
    goto done
  }
}

state done {
  final
}
"#;

const INIT_POLICY_SOURCE: &str = r#"{
  "mode": "local",
  "allowed_capabilities": [],
  "denied_capabilities": [],
  "allow_baml_network": false,
  "allowed_baml_urls": [],
  "allow_managed_baml_server": false,
  "allowed_models": [],
  "allowed_env_vars": [],
  "store_baml_raw_responses": false
}
"#;

fn init_project(dir: &Path, name: &str, force: bool) -> Result<InitOutput, CliError> {
    if !valid_workflow_ident(name) {
        return Err(CliError::InvalidWorkflowName {
            name: name.to_string(),
        });
    }

    fs::create_dir_all(dir).map_err(|source| CliError::CreateDir {
        path: dir.to_path_buf(),
        source,
    })?;

    let armature_dir = dir.join(".armature");
    let state_dir = armature_dir.join("state");
    fs::create_dir_all(&state_dir).map_err(|source| CliError::CreateDir {
        path: state_dir.clone(),
        source,
    })?;
    let workflow_store_dir = armature_dir.join("workflows");
    fs::create_dir_all(&workflow_store_dir).map_err(|source| CliError::CreateDir {
        path: workflow_store_dir.clone(),
        source,
    })?;

    let workflow = dir.join("workflow.armature");
    let policy = armature_dir.join("policy.json");
    let workflow_source = INIT_WORKFLOW_TEMPLATE.replace("{name}", name);
    write_init_file(&workflow, &workflow_source, force)?;
    write_init_file(&policy, INIT_POLICY_SOURCE, force)?;

    Ok(InitOutput {
        root: dir.to_path_buf(),
        workflow_name: name.to_string(),
        workflow,
        policy,
        state_dir,
        workflow_store_dir,
    })
}

fn valid_workflow_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn write_init_file(path: &Path, content: &str, force: bool) -> Result<(), CliError> {
    if path.exists() && !force {
        return Err(CliError::InitPathExists {
            path: path.to_path_buf(),
        });
    }
    fs::write(path, content).map_err(|source| CliError::Write {
        path: path.to_path_buf(),
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
        return Err(policy_document_validation_error(diagnostics));
    }
    Ok(parsed_policies)
}

fn adapter_dispatcher_from_manifests(
    manifests: Vec<armature_adapters::AdapterManifest>,
    policies: Vec<armature_adapters::CapabilityPolicyDocument>,
    plan_file: Option<PathBuf>,
    review_file: Option<PathBuf>,
    agent_file: Option<PathBuf>,
) -> Box<dyn armature_engine::effects::EffectDispatcher> {
    if manifests.is_empty() {
        Box::new(armature_engine::effects::NoopEffectDispatcher)
    } else {
        let dispatcher = armature_adapters::ManifestEffectDispatcher::from_manifests(manifests)
            .with_policies(policies);
        let dispatcher = if let Some(plan_file) = plan_file {
            dispatcher.with_json_plan_file(plan_file)
        } else {
            dispatcher
        };
        let dispatcher = if let Some(review_file) = review_file {
            dispatcher.with_human_review_file(review_file)
        } else {
            dispatcher
        };
        let dispatcher = if let Some(agent_file) = agent_file {
            dispatcher.with_agent_file(agent_file)
        } else {
            dispatcher
        };
        Box::new(dispatcher)
    }
}

fn add_json_plan_manifest_if_needed(manifests: &mut Vec<armature_adapters::AdapterManifest>) {
    let has_plan_manifest = manifests.iter().any(|manifest| {
        manifest
            .effects
            .keys()
            .any(|effect| effect.starts_with("plan."))
    });
    if !has_plan_manifest {
        manifests.push(armature_adapters::json_plan_adapter_manifest());
    }
}

fn add_file_backed_manifests_if_needed(
    manifests: &mut Vec<armature_adapters::AdapterManifest>,
    include_plan_manifest: bool,
    include_review_manifest: bool,
    include_agent_manifest: bool,
) {
    if include_plan_manifest {
        add_json_plan_manifest_if_needed(manifests);
    }
    if include_review_manifest {
        add_json_human_review_manifest_if_needed(manifests);
    }
    if include_agent_manifest {
        add_json_agent_manifest_if_needed(manifests);
    }
}

fn add_json_human_review_manifest_if_needed(
    manifests: &mut Vec<armature_adapters::AdapterManifest>,
) {
    let has_review_manifest = manifests
        .iter()
        .any(|manifest| manifest.effects.contains_key("askHuman"));
    if !has_review_manifest {
        manifests.push(armature_adapters::json_human_review_adapter_manifest());
    }
}

fn add_json_human_review_response_event_manifest_if_needed(
    manifests: &mut Vec<armature_adapters::AdapterManifest>,
) {
    let has_review_response_event = manifests
        .iter()
        .any(|manifest| manifest.events.contains_key("humanReview.responded"));
    if !has_review_response_event {
        manifests.push(armature_adapters::json_human_review_response_event_manifest());
    }
}

fn add_json_agent_manifest_if_needed(manifests: &mut Vec<armature_adapters::AdapterManifest>) {
    let has_agent_manifest = manifests.iter().any(|manifest| {
        manifest.effects.contains_key("start") || manifest.effects.contains_key("send")
    });
    if !has_agent_manifest {
        manifests.push(armature_adapters::json_agent_adapter_manifest());
    }
}

fn add_json_agent_finished_event_manifest_if_needed(
    manifests: &mut Vec<armature_adapters::AdapterManifest>,
) {
    let has_agent_finished_event = manifests
        .iter()
        .any(|manifest| manifest.events.contains_key("finished"));
    if !has_agent_finished_event {
        manifests.push(armature_adapters::json_agent_finished_event_manifest());
    }
}

fn coerce_executor_for_run(
    fake_coerce_outputs: BTreeMap<String, serde_json::Value>,
    baml_url: Option<String>,
    baml_timeout_ms: Option<u64>,
    ir: &armature_workflow::WorkflowIr,
    store_baml_raw_response: bool,
) -> Box<dyn armature_engine::coerce::CoerceExecutor> {
    if !fake_coerce_outputs.is_empty() {
        Box::new(armature_engine::coerce::FakeCoerceExecutor::new(
            fake_coerce_outputs,
        ))
    } else if let Some(url) = baml_url {
        Box::new(
            armature_engine::coerce::BamlHttpCoerceExecutor::new(url)
                .with_timeout_ms(baml_timeout_ms)
                .with_baml_src_hash(baml_source_hash(ir))
                .with_store_raw_response(store_baml_raw_response),
        )
    } else {
        Box::new(armature_engine::coerce::NoopCoerceExecutor)
    }
}

fn baml_source_hash(ir: &armature_workflow::WorkflowIr) -> Option<String> {
    if ir.coerce_functions.is_empty() {
        return None;
    }
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(emit_baml_source(ir).as_bytes());
    let digest = hasher.finalize();
    Some(format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
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

fn policy_document_validation_error(diagnostics: Vec<armature_workflow::Diagnostic>) -> CliError {
    let message = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    CliError::PolicyDocumentValidation {
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

fn parse_fake_outputs(outputs: &[String]) -> Result<BTreeMap<String, serde_json::Value>, CliError> {
    let mut parsed = BTreeMap::new();
    for output in outputs {
        let Some((name, json)) = output.split_once('=') else {
            return Err(CliError::InvalidFakeOutput(output.clone()));
        };
        if name.is_empty()
            || name
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(CliError::InvalidFakeOutput(output.clone()));
        }
        let value = serde_json::from_str(json).map_err(|source| CliError::FakeOutputJson {
            name: name.to_string(),
            source,
        })?;
        if parsed.insert(name.to_string(), value).is_some() {
            return Err(CliError::DuplicateFakeOutput(name.to_string()));
        }
    }
    Ok(parsed)
}

fn validate_inspection_limit(command: &'static str, limit: usize) -> Result<(), CliError> {
    if limit > MAX_INSPECTION_LIMIT {
        return Err(CliError::InvalidLimit {
            command,
            limit,
            max: MAX_INSPECTION_LIMIT,
        });
    }
    Ok(())
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
        CliError::PolicyDocumentValidation { diagnostics, .. } => print_diagnostics(diagnostics),
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
    println!("waiting: {}", overview_waiting_reason(overview, status));
    if status.data.is_empty() {
        println!("data: none");
    } else {
        println!(
            "data: {}",
            serde_json::to_string(&status.data).unwrap_or_else(|_| "<unavailable>".to_string())
        );
    }
    if status.data_summary.is_empty() {
        println!("data summary: none");
    } else {
        println!(
            "data summary: {}",
            serde_json::to_string(&status.data_summary)
                .unwrap_or_else(|_| "<unavailable>".to_string())
        );
    }
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

    if status.policy_blockers.is_empty() {
        println!("policy blockers: none");
    } else {
        println!("policy blockers:");
        for blocker in &status.policy_blockers {
            println!("  {blocker}");
        }
    }

    if status.latest_coerce_calls.is_empty() {
        println!("latest coerce: none");
    } else {
        println!("latest coerce:");
        for call in &status.latest_coerce_calls {
            let error = call
                .error
                .as_deref()
                .map(|error| format!(" error={error}"))
                .unwrap_or_default();
            println!(
                "  {} {:?} http={:?}{}",
                call.function_name, call.status, call.http_status, error
            );
        }
    }

    if status.latest_coerce_failures.is_empty() {
        println!("latest coerce failures: none");
    } else {
        println!("latest coerce failures:");
        for call in &status.latest_coerce_failures {
            let error = call
                .error
                .as_deref()
                .map(|error| format!(" error={error}"))
                .unwrap_or_default();
            println!(
                "  {} {:?} http={:?}{}",
                call.function_name, call.status, call.http_status, error
            );
        }
    }
}

fn print_status(status: &armature_engine::status::WorkflowStatus) {
    println!("workflow: {}", status.workflow_name);
    println!("state: {}", status.current_state);
    println!("waiting: {}", status_waiting_reason(status));
    if status.data.is_empty() {
        println!("data: none");
    } else {
        println!(
            "data: {}",
            serde_json::to_string(&status.data).unwrap_or_else(|_| "<unavailable>".to_string())
        );
    }
    if status.data_summary.is_empty() {
        println!("data summary: none");
    } else {
        println!(
            "data summary: {}",
            serde_json::to_string(&status.data_summary)
                .unwrap_or_else(|_| "<unavailable>".to_string())
        );
    }
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

    if status.policy_blockers.is_empty() {
        println!("policy blockers: none");
    } else {
        println!("policy blockers:");
        for blocker in &status.policy_blockers {
            println!("  {blocker}");
        }
    }

    if status.latest_coerce_calls.is_empty() {
        println!("latest coerce: none");
    } else {
        println!("latest coerce:");
        for call in &status.latest_coerce_calls {
            let error = call
                .error
                .as_deref()
                .map(|error| format!(" error={error}"))
                .unwrap_or_default();
            println!(
                "  {} {:?} http={:?}{}",
                call.function_name, call.status, call.http_status, error
            );
        }
    }

    if status.latest_coerce_failures.is_empty() {
        println!("latest coerce failures: none");
    } else {
        println!("latest coerce failures:");
        for call in &status.latest_coerce_failures {
            let error = call
                .error
                .as_deref()
                .map(|error| format!(" error={error}"))
                .unwrap_or_default();
            println!(
                "  {} {:?} http={:?}{}",
                call.function_name, call.status, call.http_status, error
            );
        }
    }
}

fn overview_waiting_reason(
    overview: &OverviewOutput,
    status: &armature_engine::status::WorkflowStatus,
) -> String {
    if !overview.validation.ok {
        return "validation failed; inspect diagnostics above".to_string();
    }
    status_waiting_reason(status)
}

fn status_waiting_reason(status: &armature_engine::status::WorkflowStatus) -> String {
    if let Some(reason) = &status.blocked_reason {
        return reason.clone();
    }
    if let Some(blocker) = status.policy_blockers.first() {
        return format!("policy blocked: {blocker}");
    }
    if let Some(failure) = status.recent_failures.first() {
        return format!("recent failure: {failure}");
    }
    if status.pending_events > 0 {
        return format!("{} queued event(s) ready to process", status.pending_events);
    }
    if !status.active_invocations.is_empty() {
        let active = status
            .active_invocations
            .iter()
            .map(|invocation| format!("{}={}", invocation.agent, invocation.count))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("waiting for active invocation(s): {active}");
    }
    "idle; no queued events or active invocations".to_string()
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

fn prove_workflow(ir: &armature_workflow::WorkflowIr) -> Result<ProveOutput, CliError> {
    let checks = vec![
        check_workflow(ir, CliModelTarget::Tla)?,
        check_workflow(ir, CliModelTarget::Maude)?,
    ];

    Ok(ProveOutput {
        ok: checks.iter().all(|check| check.ok),
        available: true,
        message: "bounded generated proof checks passed for TLA+ and Maude".to_string(),
        suggested_command: "armature check --target tla; armature check --target maude".to_string(),
        checks,
    })
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
    let artifact_hashes = build_artifact_hashes(
        &ir_json,
        &baml_src,
        &tla_model,
        &tla_config,
        &maude_model,
        adapter_manifest_bundle.as_deref(),
        policy_document_bundle.as_deref(),
    )?;
    let artifact_hashes_json = output_dir.join("artifact-hashes.json");
    fs::write(
        &artifact_hashes_json,
        serde_json::to_string_pretty(&artifact_hashes).expect("artifact hashes serialize"),
    )
    .map_err(|source| CliError::Write {
        path: artifact_hashes_json.clone(),
        source,
    })?;

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
        artifact_hashes_json,
        artifact_hashes,
    })
}

fn build_artifact_hashes(
    ir_json: &Path,
    baml_src: &Path,
    tla_model: &Path,
    tla_config: &Path,
    maude_model: &Path,
    adapter_manifest_bundle: Option<&Path>,
    policy_document_bundle: Option<&Path>,
) -> Result<BTreeMap<String, String>, CliError> {
    let mut hashes = BTreeMap::new();
    for (name, path) in [
        ("workflow-ir.json", ir_json),
        ("baml_src/workflow.baml", baml_src),
        (
            tla_model
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workflow.tla"),
            tla_model,
        ),
        (
            tla_config
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workflow.cfg"),
            tla_config,
        ),
        (
            maude_model
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workflow.maude"),
            maude_model,
        ),
    ] {
        hashes.insert(name.to_string(), sha256_file(path)?);
    }
    if let Some(path) = adapter_manifest_bundle {
        hashes.insert("adapter-manifests.json".to_string(), sha256_file(path)?);
    }
    if let Some(path) = policy_document_bundle {
        hashes.insert("policy-documents.json".to_string(), sha256_file(path)?);
    }
    Ok(hashes)
}

fn sha256_file(path: &Path) -> Result<String, CliError> {
    use sha2::Digest;
    let contents = fs::read(path).map_err(|source| CliError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let digest = sha2::Sha256::digest(&contents);
    Ok(format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
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
