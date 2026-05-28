use whippletree_engine::queue::{EventStatus, WorkflowEvent};
use whippletree_engine::storage::WorkflowStore;
use whippletree_engine::WorkflowRuntime;
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::process::Output;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_INSPECTION_LIMIT: usize = 10_000;

#[derive(Debug, Parser)]
#[command(name = "whip")]
#[command(about = "Whippletree statechart workflow CLI")]
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
        /// Adapter manifest JSON file to include in validation.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to enforce during validation.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Harness profile policy JSON file to validate native provider authority.
        #[arg(long = "profile-policy")]
        profile_policy: Option<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
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
    /// Queue one typed workflow event without processing it.
    Emit {
        file: PathBuf,
        /// Workflow event type to enqueue.
        #[arg(long)]
        event: String,
        /// JSON payload for the event.
        #[arg(long, default_value = "{}")]
        payload: String,
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Adapter manifest JSON file providing extra event schemas.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to validate before enqueueing.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Supply built-in human-review response event schema for emit intake.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Supply built-in agent completion event schema for emit intake.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Process one queued event, or enqueue and process one event when --event is supplied.
    Run {
        file: PathBuf,
        /// Workflow event type to enqueue before processing.
        #[arg(long)]
        event: Option<String>,
        /// JSON payload for --event.
        #[arg(long, default_value = "{}")]
        payload: String,
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Adapter manifest JSON file to validate and dispatch effects.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to enforce while dispatching effects.
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
        #[arg(long = "agent-file", hide = true)]
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
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Adapter manifest JSON file to include in status validation.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to include in status validation.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
        /// Print a compact operator-oriented status summary.
        #[arg(long)]
        compact: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show a compact workflow overview for humans and agents.
    Overview {
        file: PathBuf,
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Adapter manifest JSON file to include in overview validation.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to include in overview validation.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show durable workflow events.
    Events {
        file: PathBuf,
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Adapter manifest JSON file to include in event-schema validation.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to validate before reading events.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
        /// Durable event status to filter by.
        #[arg(long, value_enum)]
        status: Option<CliEventStatus>,
        /// Maximum number of event records to print.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Retry a failed or dead-lettered workflow event.
    RetryEvent {
        file: PathBuf,
        /// Failed or dead-lettered event id to requeue.
        #[arg(long = "event-id")]
        event_id: String,
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Adapter manifest JSON file to include in validation.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to include in validation.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show the append-only workflow log.
    Log {
        file: PathBuf,
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Adapter manifest JSON file to include in validation.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to include in validation.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
        /// Maximum number of log records to print.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Run a bounded formal check for a workflow file.
    Check {
        file: PathBuf,
        /// Formal model backend to check.
        #[arg(long, value_enum, default_value_t = CliModelTarget::Tla)]
        target: CliModelTarget,
        /// Adapter manifest JSON file to include before checking.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to include before checking.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Run or inspect the native local agent harness.
    Harness {
        #[command(subcommand)]
        command: HarnessCommand,
    },
    /// Run stronger proof-oriented verification when available.
    Prove {
        file: PathBuf,
        /// Adapter manifest JSON file to include before proof checks.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to include before proof checks.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Compile a workflow into build artifacts.
    Build {
        file: PathBuf,
        /// Output directory for generated artifacts.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Adapter manifest JSON file to bundle with the build.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to bundle with the build.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest in validation/build metadata.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest in validation/build metadata.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest in validation/build metadata.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Emit a formal model from a workflow file.
    EmitModel {
        file: PathBuf,
        /// Formal model backend to emit.
        #[arg(long, value_enum, default_value_t = CliModelTarget::Tla)]
        target: CliModelTarget,
        /// Adapter manifest JSON file to include while emitting the model.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to include while emitting the model.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
    },
    /// Emit a checker config for a generated formal model.
    EmitConfig {
        file: PathBuf,
        /// Formal checker backend to configure.
        #[arg(long, value_enum, default_value_t = CliModelTarget::Tla)]
        target: CliModelTarget,
        /// Adapter manifest JSON file to include while emitting config.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to include while emitting config.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Include the built-in JSON plan adapter manifest for validation.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Include the built-in JSON human-review adapter manifest for validation.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Include the built-in JSON agent adapter manifest for validation.
        #[arg(long = "agent-file", hide = true)]
        agent_file: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum HarnessCommand {
    /// Claim and run at most one queued native agent invocation.
    Once {
        file: PathBuf,
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Harness provider config JSON file.
        #[arg(long)]
        config: PathBuf,
        /// Harness profile policy JSON file.
        #[arg(long = "profile-policy")]
        profile_policy: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Poll the native agent ledger and run queued invocations.
    Run {
        file: PathBuf,
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Harness provider config JSON file.
        #[arg(long)]
        config: PathBuf,
        /// Harness profile policy JSON file.
        #[arg(long = "profile-policy")]
        profile_policy: Option<PathBuf>,
        /// Also process queued workflow events between provider runs.
        #[arg(long = "drive-workflow")]
        drive_workflow: bool,
        /// Adapter manifest JSON file to validate and dispatch effects while driving workflow events.
        #[arg(long = "adapter-manifest")]
        adapter_manifests: Vec<PathBuf>,
        /// Capability policy JSON file to enforce while driving workflow events.
        #[arg(long = "policy")]
        policy_documents: Vec<PathBuf>,
        /// Testing: provide deterministic coerce output as NAME=JSON while driving workflow events.
        #[arg(long = "fake-coerce-output")]
        fake_coerce_outputs: Vec<String>,
        /// Testing: provide deterministic capability/expression call output as NAME=JSON while driving workflow events.
        #[arg(long = "fake-call-output")]
        fake_call_outputs: Vec<String>,
        /// Use a JSON file as the built-in plan adapter backing store while driving workflow events.
        #[arg(long = "plan-file")]
        plan_file: Option<PathBuf>,
        /// Use a JSON file as the built-in human-review obligation store while driving workflow events.
        #[arg(long = "review-file")]
        review_file: Option<PathBuf>,
        /// Call real BAML functions through an already-running baml-cli serve endpoint.
        #[arg(long = "baml-url")]
        baml_url: Option<String>,
        #[arg(long = "baml-timeout-ms", default_value_t = 30_000)]
        baml_timeout_ms: u64,
        /// Maximum loop iterations; omit for continuous operation.
        #[arg(long = "max-iterations")]
        max_iterations: Option<usize>,
        #[arg(long)]
        json: bool,
    },
    /// Show native harness ledger state.
    Status {
        file: PathBuf,
        /// SQLite workflow store path.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Harness profile policy JSON file.
        #[arg(long = "profile-policy")]
        profile_policy: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliModelTarget {
    Tla,
    Apalache,
    Maude,
    Veil,
}

impl From<CliModelTarget> for whippletree_modelgen::ModelTarget {
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
    #[error("invalid workflow name `{name}`; use a .whip identifier such as `Workflow` or `spec-implementation`")]
    InvalidWorkflowName { name: String },
    #[error("{0}")]
    Source(#[from] whippletree_workflow::SourceError),
    #[error("workflow validation failed")]
    Validation {
        diagnostics: Vec<whippletree_workflow::Diagnostic>,
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
    #[error("failed to parse harness config `{path}`: {source}")]
    HarnessConfig {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to parse harness profile policy `{path}`: {source}")]
    HarnessProfilePolicy {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("harness profile policy validation failed: {message}")]
    HarnessProfilePolicyValidation {
        message: String,
        diagnostics: Vec<whippletree_workflow::Diagnostic>,
    },
    #[error("harness profile `{profile}` for agent `{agent}` is not defined")]
    MissingHarnessProfile { agent: String, profile: String },
    #[error("harness profile policy has no default profile for agent `{agent}`")]
    MissingDefaultHarnessProfile { agent: String },
    #[error("harness profile `{profile}` for agent `{agent}` resolves to provider `{provider}`, but config for that agent uses provider `{configured_provider}`")]
    HarnessProfileProviderMismatch {
        agent: String,
        profile: String,
        provider: String,
        configured_provider: String,
    },
    #[error("harness profile `{profile}` for agent `{agent}` uses command provider but command provider is not allowed by policy")]
    HarnessCommandProviderDenied { agent: String, profile: String },
    #[error("harness config has no provider for agent `{0}`")]
    MissingHarnessProvider(String),
    #[error("harness provider `{provider}` for agent `{agent}` is not supported yet")]
    UnsupportedHarnessProvider { agent: String, provider: String },
    #[error("harness provider for agent `{agent}` has an empty command")]
    EmptyHarnessCommand { agent: String },
    #[error(
        "failed to collect harness provider output for invocation `{invocation_id}`: {source}"
    )]
    HarnessOutput {
        invocation_id: String,
        source: std::io::Error,
    },
    #[error("failed to execute harness provider for invocation `{invocation_id}`: {source}")]
    HarnessExecution {
        invocation_id: String,
        source: std::io::Error,
    },
    #[error("adapter manifest validation failed: {message}")]
    AdapterManifestValidation {
        message: String,
        diagnostics: Vec<whippletree_workflow::Diagnostic>,
    },
    #[error("policy document validation failed: {message}")]
    PolicyDocumentValidation {
        message: String,
        diagnostics: Vec<whippletree_workflow::Diagnostic>,
    },
    #[error("workflow contract validation failed: {message}")]
    WorkflowContractValidation {
        message: String,
        diagnostics: Vec<whippletree_workflow::Diagnostic>,
    },
    #[error("storage error: {0}")]
    Storage(#[from] whippletree_engine::storage::StorageError),
    #[error("runtime error: {0}")]
    Runtime(#[from] whippletree_engine::RuntimeError),
    #[error("adapter bridge error: {0}")]
    AdapterBridge(#[from] whippletree_engine::effects::EffectError),
    #[error("model generation error: {0}")]
    Modelgen(#[from] whippletree_modelgen::ModelgenError),
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
    diagnostics: Vec<whippletree_workflow::Diagnostic>,
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
    outcome: Option<whippletree_engine::EventProcessingOutcome>,
    status: whippletree_engine::status::WorkflowStatus,
}

#[derive(Debug, Serialize)]
struct EmitOutput {
    event: WorkflowEvent,
    status: whippletree_engine::status::WorkflowStatus,
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
    status: whippletree_engine::status::WorkflowStatus,
}

#[derive(Debug, Serialize)]
struct LogOutput {
    workflow_id: String,
    records: Vec<whippletree_engine::log::WorkflowLogRecord>,
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

#[derive(Debug, Deserialize)]
struct HarnessConfig {
    agents: BTreeMap<String, HarnessAgentConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HarnessAgentConfig {
    provider: String,
    #[serde(default)]
    command: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<PathBuf>,
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HarnessProfilePolicy {
    mode: HarnessProfileMode,
    #[serde(default)]
    default_profile: Option<String>,
    #[serde(default)]
    allow_command_provider: Option<bool>,
    #[serde(default)]
    profiles: BTreeMap<String, HarnessProfileDefinition>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum HarnessProfileMode {
    Permissive,
    Separated,
    Custom,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HarnessProfileDefinition {
    description: String,
    provider: String,
    #[serde(default)]
    command: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<PathBuf>,
    timeout_seconds: Option<u64>,
    #[serde(default)]
    filesystem: Option<String>,
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    allowed_env: Vec<String>,
    #[serde(default)]
    allowed_tools: Vec<String>,
    #[serde(default)]
    enforcement: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedHarnessProvider {
    profile: Option<String>,
    config: HarnessAgentConfig,
    requested_authority: serde_json::Value,
    enforced_authority: serde_json::Value,
    enforcement: String,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HarnessOnceOutput {
    workflow_id: String,
    invocation: Option<whippletree_engine::storage::AgentInvocationRecord>,
    completion_event: Option<WorkflowEvent>,
    provider_status: Option<i32>,
    provider_timed_out: bool,
}

#[derive(Debug, Serialize)]
struct HarnessStatusOutput {
    workflow_id: String,
    workflow_status: Option<whippletree_engine::status::WorkflowStatus>,
    invocations: Vec<whippletree_engine::storage::AgentInvocationRecord>,
    completions: Vec<whippletree_engine::storage::AgentCompletionRecord>,
    harness_events: Vec<whippletree_engine::storage::HarnessEventRecord>,
    recent_failures: Vec<whippletree_engine::storage::HarnessEventRecord>,
}

struct HarnessProviderOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: Option<i32>,
    success: bool,
    timed_out: bool,
}

#[derive(Debug, Serialize)]
struct OverviewOutput {
    validation: ValidateOutput,
    status: Option<whippletree_engine::status::WorkflowStatus>,
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
            profile_policy,
            plan_file,
            review_file,
            agent_file,
            json,
        } => {
            let source = load_source(&file)?;
            let parsed =
                whippletree_workflow::parse_syntax_with_file(&source, file.display().to_string());
            let mut diagnostics = parsed.diagnostics;
            if let Some(ir) = parsed.ir {
                diagnostics.extend(whippletree_workflow::validate_ir(&ir).diagnostics);
                if let Some(path) = &profile_policy {
                    let policy = load_harness_profile_policy(path)?;
                    diagnostics.extend(validate_harness_profile_policy(&ir, &policy));
                }
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
                    diagnostics.extend(whippletree_adapters::validate_adapter_manifests(&manifests));
                    diagnostics.extend(whippletree_adapters::validate_workflow_effects(
                        &ir, &manifests,
                    ));
                    diagnostics.extend(whippletree_adapters::validate_workflow_policy(
                        &ir, &manifests, &policies,
                    ));
                } else {
                    diagnostics.extend(whippletree_adapters::validate_policy_documents(&policies));
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
            let diagnostics = whippletree_adapters::validate_adapter_manifests(&parsed_manifests);
            let ok = !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == whippletree_workflow::Severity::Error);

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
            let diagnostics = whippletree_adapters::validate_policy_documents(&parsed_policies);
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
                    whippletree_adapters::record_human_review_response(review_file, &event.payload)?;
                }
            }
            if event.event_type == "finished" {
                if let Some(agent_file) = agent_file.as_deref() {
                    whippletree_adapters::record_agent_finished_event(agent_file, &event.payload)?;
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
                    let diagnostics = whippletree_adapters::validate_baml_http_policy(&policies, url);
                    if diagnostics_have_errors(&diagnostics) {
                        return Err(policy_document_validation_error(diagnostics));
                    }
                }
            }
            let ir = load_valid_ir_with_contracts(&file, &manifests, &policies)?;
            let store_baml_raw_response =
                whippletree_adapters::should_store_baml_raw_response(&policies);
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
            compact,
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
            } else if compact {
                print_compact_status(&status);
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
                return Err(whippletree_engine::storage::StorageError::EventNotFound {
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
        Command::Harness { command } => match command {
            HarnessCommand::Once {
                file,
                store,
                config,
                profile_policy,
                json,
            } => {
                let output = run_harness_once(&file, store, &config, profile_policy.as_deref())?;
                if json {
                    print_json(&output)?;
                } else if let Some(invocation) = output.invocation {
                    println!(
                        "{} {}",
                        invocation.invocation_id,
                        output
                            .completion_event
                            .as_ref()
                            .map(|event| event.event_id.as_str())
                            .unwrap_or("no-event")
                    );
                } else {
                    println!("idle");
                }
                Ok(())
            }
            HarnessCommand::Run {
                file,
                store,
                config,
                profile_policy,
                drive_workflow,
                adapter_manifests,
                policy_documents,
                fake_coerce_outputs,
                fake_call_outputs,
                plan_file,
                review_file,
                baml_url,
                baml_timeout_ms,
                max_iterations,
                json,
            } => {
                let mut outputs = Vec::new();
                let max_iterations = max_iterations.unwrap_or(usize::MAX);
                for _ in 0..max_iterations {
                    let mut did_work = false;
                    if drive_workflow {
                        let processed = match process_workflow_once(
                            &file,
                            store.clone(),
                            &adapter_manifests,
                            &policy_documents,
                            &fake_coerce_outputs,
                            &fake_call_outputs,
                            plan_file.clone(),
                            review_file.clone(),
                            baml_url.clone(),
                            baml_timeout_ms,
                        ) {
                            Ok(processed) => processed,
                            Err(error) => {
                                append_harness_observation_best_effort(
                                    &file,
                                    store.clone(),
                                    "workflow_validation_failed",
                                    serde_json::json!({"error": error.to_string()}),
                                );
                                return Err(error);
                            }
                        };
                        did_work |= processed;
                    }
                    let output =
                        run_harness_once(&file, store.clone(), &config, profile_policy.as_deref())?;
                    did_work |= output.invocation.is_some();
                    outputs.push(output);
                    if !did_work {
                        append_harness_observation_best_effort(
                            &file,
                            store.clone(),
                            "idle_without_work",
                            serde_json::json!({"driveWorkflow": drive_workflow}),
                        );
                        break;
                    }
                }
                if json {
                    print_json(&outputs)?;
                } else {
                    println!("{}", outputs.len());
                }
                Ok(())
            }
            HarnessCommand::Status {
                file,
                store,
                profile_policy,
                json,
            } => {
                let output = harness_status(&file, store, profile_policy.as_deref())?;
                if json {
                    print_json(&output)?;
                } else {
                    println!("invocations: {}", output.invocations.len());
                    for invocation in output.invocations {
                        println!(
                            "{} {} {:?}",
                            invocation.invocation_id, invocation.agent, invocation.status
                        );
                    }
                }
                Ok(())
            }
        },
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

            println!("{}", whippletree_modelgen::emit_model(&ir, target.into())?);
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
    Ok(whippletree_modelgen::emit_check_config(target.into())?)
}

fn load_ir(file: &PathBuf) -> Result<whippletree_workflow::WorkflowIr, CliError> {
    let source = load_source(file)?;
    Ok(whippletree_workflow::parse_source_with_file(
        &source,
        file.display().to_string(),
    )?)
}

fn load_valid_ir(file: &PathBuf) -> Result<whippletree_workflow::WorkflowIr, CliError> {
    let ir = load_ir(file)?;
    let report = whippletree_workflow::validate_ir(&ir);
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
        whippletree_workflow::WorkflowIr,
        Vec<whippletree_adapters::AdapterManifest>,
        Vec<whippletree_adapters::CapabilityPolicyDocument>,
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
    manifests: &[whippletree_adapters::AdapterManifest],
    policies: &[whippletree_adapters::CapabilityPolicyDocument],
) -> Result<whippletree_workflow::WorkflowIr, CliError> {
    let ir = load_valid_ir(file)?;
    if !manifests.is_empty() || !policies.is_empty() {
        validate_ir_contracts(&ir, manifests, policies)?;
    }
    Ok(ir)
}

fn validate_ir_contracts(
    ir: &whippletree_workflow::WorkflowIr,
    manifests: &[whippletree_adapters::AdapterManifest],
    policies: &[whippletree_adapters::CapabilityPolicyDocument],
) -> Result<(), CliError> {
    let mut diagnostics = Vec::new();
    if !manifests.is_empty() {
        diagnostics.extend(whippletree_adapters::validate_workflow_effects(ir, manifests));
    }
    if !policies.is_empty() {
        diagnostics.extend(whippletree_adapters::validate_workflow_policy(
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
) -> Result<whippletree_engine::status::WorkflowStatus, CliError> {
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
        let data = whippletree_engine::initial_context_from_ir(&ir);
        Ok(whippletree_engine::status::WorkflowStatus {
            workflow_id: workflow_id.clone(),
            workflow_name: workflow_id,
            current_state: whippletree_engine::initial_state_name(&ir),
            blocked_reason: None,
            data_summary: whippletree_engine::summarize_status_data(&data),
            data,
            pending_events: 0,
            queued_events: Vec::new(),
            active_invocations: Vec::new(),
            recent_transition: None,
            recent_effects: Vec::new(),
            latest_coerce_calls: Vec::new(),
            latest_coerce_failures: Vec::new(),
            current_coerce_failure: None,
            current_effect_failures: Vec::new(),
            policy_blockers: Vec::new(),
            current_blockers: Vec::new(),
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
    let parsed = whippletree_workflow::parse_syntax_with_file(&source, file.display().to_string());
    let mut diagnostics = parsed.diagnostics;
    let status = if let Some(ir) = parsed.ir {
        diagnostics.extend(whippletree_workflow::validate_ir(&ir).diagnostics);
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
            diagnostics.extend(whippletree_adapters::validate_adapter_manifests(&manifests));
            diagnostics.extend(whippletree_adapters::validate_workflow_effects(
                &ir, &manifests,
            ));
            diagnostics.extend(whippletree_adapters::validate_workflow_policy(
                &ir, &manifests, &policies,
            ));
        } else {
            diagnostics.extend(whippletree_adapters::validate_policy_documents(&policies));
        }
        let workflow_id = ir.workflow.name.clone();
        let store_path = store.unwrap_or_else(|| default_store_path(file, &workflow_id));
        Some(if store_path.exists() {
            let store = WorkflowStore::open(&store_path)?;
            stored_status(&ir, &store, &workflow_id)?
        } else {
            let data = whippletree_engine::initial_context_from_ir(&ir);
            whippletree_engine::status::WorkflowStatus {
                workflow_id: workflow_id.clone(),
                workflow_name: workflow_id,
                current_state: whippletree_engine::initial_state_name(&ir),
                blocked_reason: None,
                data_summary: whippletree_engine::summarize_status_data(&data),
                data,
                pending_events: 0,
                queued_events: Vec::new(),
                active_invocations: Vec::new(),
                recent_transition: None,
                recent_effects: Vec::new(),
                latest_coerce_calls: Vec::new(),
                latest_coerce_failures: Vec::new(),
                current_coerce_failure: None,
                current_effect_failures: Vec::new(),
                policy_blockers: Vec::new(),
                current_blockers: Vec::new(),
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

    let whippletree_dir = dir.join(".whip");
    let state_dir = whippletree_dir.join("state");
    fs::create_dir_all(&state_dir).map_err(|source| CliError::CreateDir {
        path: state_dir.clone(),
        source,
    })?;
    let workflow_store_dir = whippletree_dir.join("workflows");
    fs::create_dir_all(&workflow_store_dir).map_err(|source| CliError::CreateDir {
        path: workflow_store_dir.clone(),
        source,
    })?;

    let workflow = dir.join("workflow.whip");
    let policy = whippletree_dir.join("policy.json");
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
) -> Result<Vec<whippletree_adapters::AdapterManifest>, CliError> {
    let parsed_manifests = load_adapter_manifests(manifests)?;
    let diagnostics = whippletree_adapters::validate_adapter_manifests(&parsed_manifests);
    if diagnostics_have_errors(&diagnostics) {
        return Err(adapter_manifest_validation_error(diagnostics));
    }
    Ok(parsed_manifests)
}

fn load_valid_policy_documents(
    policies: &[PathBuf],
) -> Result<Vec<whippletree_adapters::CapabilityPolicyDocument>, CliError> {
    let parsed_policies = load_policy_documents(policies)?;
    let diagnostics = whippletree_adapters::validate_policy_documents(&parsed_policies);
    if diagnostics_have_errors(&diagnostics) {
        return Err(policy_document_validation_error(diagnostics));
    }
    Ok(parsed_policies)
}

fn adapter_dispatcher_from_manifests(
    manifests: Vec<whippletree_adapters::AdapterManifest>,
    policies: Vec<whippletree_adapters::CapabilityPolicyDocument>,
    plan_file: Option<PathBuf>,
    review_file: Option<PathBuf>,
    agent_file: Option<PathBuf>,
) -> Box<dyn whippletree_engine::effects::EffectDispatcher> {
    if manifests.is_empty() {
        Box::new(whippletree_engine::effects::NoopEffectDispatcher)
    } else {
        let dispatcher = whippletree_adapters::ManifestEffectDispatcher::from_manifests(manifests)
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

fn add_json_plan_manifest_if_needed(manifests: &mut Vec<whippletree_adapters::AdapterManifest>) {
    let has_plan_manifest = manifests.iter().any(|manifest| {
        manifest
            .effects
            .keys()
            .any(|effect| effect.starts_with("plan."))
    });
    if !has_plan_manifest {
        manifests.push(whippletree_adapters::json_plan_adapter_manifest());
    }
}

fn add_file_backed_manifests_if_needed(
    manifests: &mut Vec<whippletree_adapters::AdapterManifest>,
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
    manifests: &mut Vec<whippletree_adapters::AdapterManifest>,
) {
    let has_review_manifest = manifests
        .iter()
        .any(|manifest| manifest.effects.contains_key("askHuman"));
    if !has_review_manifest {
        manifests.push(whippletree_adapters::json_human_review_adapter_manifest());
    }
}

fn add_json_human_review_response_event_manifest_if_needed(
    manifests: &mut Vec<whippletree_adapters::AdapterManifest>,
) {
    let has_review_response_event = manifests
        .iter()
        .any(|manifest| manifest.events.contains_key("humanReview.responded"));
    if !has_review_response_event {
        manifests.push(whippletree_adapters::json_human_review_response_event_manifest());
    }
}

fn add_json_agent_manifest_if_needed(manifests: &mut Vec<whippletree_adapters::AdapterManifest>) {
    let has_agent_manifest = manifests.iter().any(|manifest| {
        manifest.effects.contains_key("start") || manifest.effects.contains_key("send")
    });
    if !has_agent_manifest {
        manifests.push(whippletree_adapters::json_agent_adapter_manifest());
    }
}

fn add_json_agent_finished_event_manifest_if_needed(
    manifests: &mut Vec<whippletree_adapters::AdapterManifest>,
) {
    let has_agent_finished_event = manifests
        .iter()
        .any(|manifest| manifest.events.contains_key("finished"));
    if !has_agent_finished_event {
        manifests.push(whippletree_adapters::json_agent_finished_event_manifest());
    }
}

fn coerce_executor_for_run(
    fake_coerce_outputs: BTreeMap<String, serde_json::Value>,
    baml_url: Option<String>,
    baml_timeout_ms: Option<u64>,
    ir: &whippletree_workflow::WorkflowIr,
    store_baml_raw_response: bool,
) -> Box<dyn whippletree_engine::coerce::CoerceExecutor> {
    if !fake_coerce_outputs.is_empty() {
        Box::new(whippletree_engine::coerce::FakeCoerceExecutor::new(
            fake_coerce_outputs,
        ))
    } else if let Some(url) = baml_url {
        Box::new(
            whippletree_engine::coerce::BamlHttpCoerceExecutor::new(url)
                .with_timeout_ms(baml_timeout_ms)
                .with_baml_src_hash(baml_source_hash(ir))
                .with_store_raw_response(store_baml_raw_response),
        )
    } else {
        Box::new(whippletree_engine::coerce::NoopCoerceExecutor)
    }
}

fn baml_source_hash(ir: &whippletree_workflow::WorkflowIr) -> Option<String> {
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

fn adapter_manifest_validation_error(diagnostics: Vec<whippletree_workflow::Diagnostic>) -> CliError {
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

fn policy_document_validation_error(diagnostics: Vec<whippletree_workflow::Diagnostic>) -> CliError {
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

fn harness_profile_policy_validation_error(
    diagnostics: Vec<whippletree_workflow::Diagnostic>,
) -> CliError {
    let message = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == whippletree_workflow::Severity::Error)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    CliError::HarnessProfilePolicyValidation {
        message,
        diagnostics,
    }
}

fn workflow_contract_validation_error(diagnostics: Vec<whippletree_workflow::Diagnostic>) -> CliError {
    let message = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == whippletree_workflow::Severity::Error)
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
) -> Result<Vec<whippletree_adapters::AdapterManifest>, CliError> {
    let mut parsed_manifests = Vec::new();
    for path in manifests {
        let source = load_source(path)?;
        let manifest = serde_json::from_str::<whippletree_adapters::AdapterManifest>(&source)
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
) -> Result<Vec<whippletree_adapters::CapabilityPolicyDocument>, CliError> {
    let mut parsed_policies = Vec::new();
    for path in policies {
        let source = load_source(path)?;
        let policy = serde_json::from_str::<whippletree_adapters::CapabilityPolicyDocument>(&source)
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

fn diagnostics_have_errors(diagnostics: &[whippletree_workflow::Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == whippletree_workflow::Severity::Error)
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
        CliError::HarnessProfilePolicyValidation { diagnostics, .. } => {
            print_diagnostics(diagnostics)
        }
        CliError::WorkflowContractValidation { diagnostics, .. } => print_diagnostics(diagnostics),
        _ => {}
    }
}

fn print_diagnostics(diagnostics: &[whippletree_workflow::Diagnostic]) {
    for diagnostic in diagnostics {
        eprintln!("{}", format_diagnostic(diagnostic));
    }
}

fn format_diagnostic(diagnostic: &whippletree_workflow::Diagnostic) -> String {
    let severity = match diagnostic.severity {
        whippletree_workflow::Severity::Error => "error",
        whippletree_workflow::Severity::Warning => "warning",
        whippletree_workflow::Severity::Note => "note",
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

    if status.current_effect_failures.is_empty() {
        println!("current effect failures: none");
    } else {
        println!("current effect failures:");
        for failure in &status.current_effect_failures {
            println!("  {failure}");
        }
    }

    if status.current_blockers.is_empty() {
        println!("current blockers: none");
    } else {
        println!("current blockers:");
        for blocker in &status.current_blockers {
            println!("  {blocker}");
        }
    }

    if status.recent_failures.is_empty() {
        println!("recent failures (history): none");
    } else {
        println!("recent failures (history):");
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

    if let Some(call) = &status.current_coerce_failure {
        let error = call
            .error
            .as_deref()
            .map(|error| format!(" error={error}"))
            .unwrap_or_default();
        println!(
            "current coerce failure: {} {:?} http={:?}{}",
            call.function_name, call.status, call.http_status, error
        );
    } else {
        println!("current coerce failure: none");
    }

    if status.latest_coerce_failures.is_empty() {
        println!("latest coerce failures (history): none");
    } else {
        println!("latest coerce failures (history):");
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

fn print_status(status: &whippletree_engine::status::WorkflowStatus) {
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

    if status.current_effect_failures.is_empty() {
        println!("current effect failures: none");
    } else {
        println!("current effect failures:");
        for failure in &status.current_effect_failures {
            println!("  {failure}");
        }
    }

    if status.current_blockers.is_empty() {
        println!("current blockers: none");
    } else {
        println!("current blockers:");
        for blocker in &status.current_blockers {
            println!("  {blocker}");
        }
    }

    if status.recent_failures.is_empty() {
        println!("recent failures (history): none");
    } else {
        println!("recent failures (history):");
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

    if let Some(call) = &status.current_coerce_failure {
        let error = call
            .error
            .as_deref()
            .map(|error| format!(" error={error}"))
            .unwrap_or_default();
        println!(
            "current coerce failure: {} {:?} http={:?}{}",
            call.function_name, call.status, call.http_status, error
        );
    } else {
        println!("current coerce failure: none");
    }

    if status.latest_coerce_failures.is_empty() {
        println!("latest coerce failures (history): none");
    } else {
        println!("latest coerce failures (history):");
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

fn print_compact_status(status: &whippletree_engine::status::WorkflowStatus) {
    println!("workflow: {}", status.workflow_name);
    println!("state: {}", status.current_state);
    println!("waiting: {}", status_waiting_reason(status));
    println!("pending events: {}", status.pending_events);
    if status.active_invocations.is_empty() {
        println!("active: none");
    } else {
        let active = status
            .active_invocations
            .iter()
            .map(|invocation| match invocation.max {
                Some(max) => format!("{}={}/{}", invocation.agent, invocation.count, max),
                None => format!("{}={}", invocation.agent, invocation.count),
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!("active: {active}");
    }
    if status.current_blockers.is_empty() {
        println!("current blockers: none");
    } else {
        println!("current blockers:");
        for blocker in &status.current_blockers {
            println!("  {blocker}");
        }
    }
    if !status.current_effect_failures.is_empty() {
        println!("current effect failures:");
        for failure in &status.current_effect_failures {
            println!("  {failure}");
        }
    }
    if let Some(transition) = &status.recent_transition {
        println!("latest transition: {transition}");
    } else {
        println!("latest transition: none");
    }
}

fn overview_waiting_reason(
    overview: &OverviewOutput,
    status: &whippletree_engine::status::WorkflowStatus,
) -> String {
    if !overview.validation.ok {
        return "validation failed; inspect diagnostics above".to_string();
    }
    status_waiting_reason(status)
}

fn status_waiting_reason(status: &whippletree_engine::status::WorkflowStatus) -> String {
    if let Some(reason) = &status.blocked_reason {
        return reason.clone();
    }
    if let Some(blocker) = status.policy_blockers.first() {
        return format!("policy blocked: {blocker}");
    }
    if let Some(blocker) = status.current_blockers.first() {
        return format!("current blocker: {blocker}");
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

#[allow(clippy::too_many_arguments)]
fn process_workflow_once(
    file: &PathBuf,
    store_path: Option<PathBuf>,
    adapter_manifests: &[PathBuf],
    policy_documents: &[PathBuf],
    fake_coerce_outputs: &[String],
    fake_call_outputs: &[String],
    plan_file: Option<PathBuf>,
    review_file: Option<PathBuf>,
    baml_url: Option<String>,
    baml_timeout_ms: u64,
) -> Result<bool, CliError> {
    let mut manifests = load_valid_adapter_manifests(adapter_manifests)?;
    if plan_file.is_some() {
        add_json_plan_manifest_if_needed(&mut manifests);
    }
    if review_file.is_some() {
        add_json_human_review_manifest_if_needed(&mut manifests);
    }
    let policies = load_valid_policy_documents(policy_documents)?;
    let parsed_fake_coerce_outputs = parse_fake_outputs(fake_coerce_outputs)?;
    if parsed_fake_coerce_outputs.is_empty() {
        if let Some(url) = &baml_url {
            let diagnostics = whippletree_adapters::validate_baml_http_policy(&policies, url);
            if diagnostics_have_errors(&diagnostics) {
                return Err(policy_document_validation_error(diagnostics));
            }
        }
    }
    let ir = load_valid_ir_with_contracts(file, &manifests, &policies)?;
    let workflow_id = ir.workflow.name.clone();
    let store_baml_raw_response = whippletree_adapters::should_store_baml_raw_response(&policies);
    let store_path = store_path.unwrap_or_else(|| default_store_path(file, &workflow_id));
    ensure_parent_dir(&store_path)?;
    let store = WorkflowStore::open(&store_path)?;
    let dispatcher =
        adapter_dispatcher_from_manifests(manifests, policies, plan_file, review_file, None);
    let coerce_executor = coerce_executor_for_run(
        parsed_fake_coerce_outputs,
        baml_url,
        Some(baml_timeout_ms),
        &ir,
        store_baml_raw_response,
    );
    let fake_call_outputs = parse_fake_outputs(fake_call_outputs)?;
    let mut runtime = WorkflowRuntime::with_dispatcher_and_coerce_executor(
        ir,
        store,
        dispatcher,
        coerce_executor,
    )?
    .with_fake_call_outputs(fake_call_outputs);
    runtime
        .process_next_event()
        .map(|outcome| outcome.is_some())
        .map_err(Into::into)
}

fn run_harness_once(
    file: &PathBuf,
    store_path: Option<PathBuf>,
    config_path: &Path,
    profile_policy_path: Option<&Path>,
) -> Result<HarnessOnceOutput, CliError> {
    let ir = load_valid_ir(file)?;
    let workflow_id = ir.workflow.name.clone();
    let store_path = store_path.unwrap_or_else(|| default_store_path(file, &workflow_id));
    ensure_parent_dir(&store_path)?;
    let store = WorkflowStore::open(&store_path)?;
    let config = load_harness_config(config_path)?;
    let profile_policy = profile_policy_path
        .map(load_harness_profile_policy)
        .transpose()?;
    if let Some(policy) = &profile_policy {
        let diagnostics = validate_harness_profile_policy(&ir, policy);
        if diagnostics_have_errors(&diagnostics) {
            return Err(harness_profile_policy_validation_error(diagnostics));
        }
    }
    recover_expired_harness_leases(&store, &workflow_id)?;
    let Some(invocation) = store
        .queued_agent_invocations(&workflow_id, 1)?
        .into_iter()
        .next()
    else {
        return Ok(HarnessOnceOutput {
            workflow_id,
            invocation: None,
            completion_event: None,
            provider_status: None,
            provider_timed_out: false,
        });
    };

    let resolved_provider =
        resolve_harness_provider(&ir, &config, profile_policy.as_ref(), &invocation).map_err(
            |error| {
                let kind = if matches!(error, CliError::MissingHarnessProvider(_)) {
                    "unknown_agent"
                } else {
                    "profile_resolution_failed"
                };
                let _ = append_harness_event(
                    &store,
                    &workflow_id,
                    Some(&invocation.invocation_id),
                    kind,
                    serde_json::json!({
                        "agent": invocation.agent,
                        "configuredAgents": config.agents.keys().cloned().collect::<Vec<_>>(),
                        "error": error.to_string(),
                    }),
                );
                error
            },
        )?;
    let agent_config = &resolved_provider.config;
    let provider_command = resolve_harness_provider_command(agent_config, &invocation)?;

    let worker_id = format!("whippletree-cli-{}", std::process::id());
    let lease_ms = agent_config
        .timeout_seconds
        .unwrap_or(300)
        .saturating_add(60)
        .saturating_mul(1_000);
    let lease_until = (current_unix_millis().saturating_add(lease_ms)).to_string();
    let Some(claimed) =
        store.claim_agent_invocation(&invocation.invocation_id, &worker_id, &lease_until)?
    else {
        return Ok(HarnessOnceOutput {
            workflow_id,
            invocation: None,
            completion_event: None,
            provider_status: None,
            provider_timed_out: false,
        });
    };

    let run_dir = store_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("runs")
        .join(&claimed.invocation_id);
    fs::create_dir_all(&run_dir).map_err(|source| CliError::CreateDir {
        path: run_dir.clone(),
        source,
    })?;
    let stdout_path = run_dir.join("stdout.log");
    let stderr_path = run_dir.join("stderr.log");
    let run_dir_display = run_dir.display().to_string();
    let stdout_path_display = stdout_path.display().to_string();
    let stderr_path_display = stderr_path.display().to_string();
    store.mark_agent_invocation_started(
        whippletree_engine::storage::AgentInvocationStartedUpdate {
            invocation_id: &claimed.invocation_id,
            provider: Some(&agent_config.provider),
            provider_run_id: Some(&claimed.invocation_id),
            resolved_profile: resolved_provider.profile.as_deref(),
            profile_enforcement: Some(&resolved_provider.enforcement),
            run_dir: Some(&run_dir_display),
            stdout_path: Some(&stdout_path_display),
            stderr_path: Some(&stderr_path_display),
        },
    )?;
    append_harness_event(
        &store,
        &workflow_id,
        Some(&claimed.invocation_id),
        "provider_started",
        serde_json::json!({
            "agent": claimed.agent,
            "requestedProfile": &claimed.requested_profile,
            "resolvedProfile": &resolved_provider.profile,
            "provider": agent_config.provider,
            "command": provider_command,
            "requestedAuthority": &resolved_provider.requested_authority,
            "enforcedAuthority": &resolved_provider.enforced_authority,
            "enforcement": &resolved_provider.enforcement,
            "warnings": &resolved_provider.warnings,
        }),
    )?;

    let output = run_harness_command(
        agent_config,
        &provider_command,
        &claimed,
        &workflow_id,
        &run_dir,
    )?;
    fs::write(&stdout_path, &output.stdout).map_err(|source| CliError::Write {
        path: stdout_path.clone(),
        source,
    })?;
    fs::write(&stderr_path, &output.stderr).map_err(|source| CliError::Write {
        path: stderr_path.clone(),
        source,
    })?;

    let exit_code = output.exit_code;
    let succeeded = output.success;
    let completion_status = if output.timed_out {
        "timed_out"
    } else if succeeded {
        "succeeded"
    } else {
        "failed"
    };
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let summary = if succeeded {
        tail_text(&stdout_text, 500)
    } else {
        tail_text(&stderr_text, 500)
    };
    let invocation_status = if output.timed_out {
        whippletree_engine::storage::AgentInvocationStatus::TimedOut
    } else if succeeded {
        whippletree_engine::storage::AgentInvocationStatus::Succeeded
    } else {
        whippletree_engine::storage::AgentInvocationStatus::Failed
    };
    store.mark_agent_invocation_exited(
        &claimed.invocation_id,
        invocation_status,
        exit_code,
        (!succeeded).then_some(summary.as_str()),
    )?;
    append_harness_event(
        &store,
        &workflow_id,
        Some(&claimed.invocation_id),
        "provider_exited",
        serde_json::json!({
            "status": completion_status,
            "exitCode": exit_code,
            "timedOut": output.timed_out,
            "stdoutPath": stdout_path,
            "stderrPath": stderr_path,
        }),
    )?;
    if output.timed_out {
        append_harness_event(
            &store,
            &workflow_id,
            Some(&claimed.invocation_id),
            "provider_timed_out",
            serde_json::json!({
                "timeoutSeconds": agent_config.timeout_seconds,
                "command": provider_command,
            }),
        )?;
    } else if !succeeded {
        append_harness_event(
            &store,
            &workflow_id,
            Some(&claimed.invocation_id),
            "provider_command_failed",
            serde_json::json!({
                "exitCode": exit_code,
                "stderr": tail_text(&stderr_text, 500),
                "command": provider_command,
            }),
        )?;
    }

    let payload = serde_json::json!({
        "id": claimed.invocation_id,
        "name": claimed.agent,
        "status": completion_status,
        "summary": summary,
        "exitCode": exit_code,
    });
    let mut completion_event = match build_cli_event(
        &ir,
        &[],
        &workflow_id,
        "finished".to_string(),
        serde_json::to_string(&payload)?,
    ) {
        Ok(event) => event,
        Err(error) => {
            store.mark_agent_invocation_exited(
                &claimed.invocation_id,
                whippletree_engine::storage::AgentInvocationStatus::CompletionRejected,
                exit_code,
                Some(&error.to_string()),
            )?;
            append_harness_event(
                &store,
                &workflow_id,
                Some(&claimed.invocation_id),
                "completion_schema_mismatch",
                serde_json::json!({"error": error.to_string(), "payload": payload}),
            )?;
            return Err(error);
        }
    };
    completion_event.source = Some(whippletree_engine::queue::EventSource {
        kind: "harness".to_string(),
        name: Some(agent_config.provider.clone()),
    });
    let completion = whippletree_engine::storage::AgentCompletionRecord {
        workflow_id: workflow_id.clone(),
        completion_id: format!("cmp_{}", ulid::Ulid::new()),
        invocation_id: claimed.invocation_id.clone(),
        agent: claimed.agent.clone(),
        status: completion_status.to_string(),
        summary: Some(summary),
        exit_code,
        event_id: Some(completion_event.event_id.clone()),
        payload,
        created_at: current_unix_millis().to_string(),
    };
    let inserted_completion = store.record_agent_completion(&completion, &completion_event)?;
    if inserted_completion {
        append_harness_event(
            &store,
            &workflow_id,
            Some(&claimed.invocation_id),
            "completion_enqueued",
            serde_json::json!({"eventId": completion_event.event_id}),
        )?;
    } else {
        append_harness_event(
            &store,
            &workflow_id,
            Some(&claimed.invocation_id),
            "duplicate_completion_ignored",
            serde_json::json!({"eventId": completion_event.event_id}),
        )?;
    }

    Ok(HarnessOnceOutput {
        workflow_id,
        invocation: Some(claimed),
        completion_event: Some(completion_event),
        provider_status: exit_code,
        provider_timed_out: output.timed_out,
    })
}

fn harness_status(
    file: &PathBuf,
    store_path: Option<PathBuf>,
    profile_policy_path: Option<&Path>,
) -> Result<HarnessStatusOutput, CliError> {
    let ir = load_valid_ir(file)?;
    if let Some(path) = profile_policy_path {
        let policy = load_harness_profile_policy(path)?;
        let diagnostics = validate_harness_profile_policy(&ir, &policy);
        if diagnostics_have_errors(&diagnostics) {
            return Err(harness_profile_policy_validation_error(diagnostics));
        }
    }
    let workflow_id = ir.workflow.name.clone();
    let store_path = store_path.unwrap_or_else(|| default_store_path(file, &workflow_id));
    if !store_path.exists() {
        return Ok(HarnessStatusOutput {
            workflow_id,
            workflow_status: None,
            invocations: Vec::new(),
            completions: Vec::new(),
            harness_events: Vec::new(),
            recent_failures: Vec::new(),
        });
    }
    let store = WorkflowStore::open(&store_path)?;
    recover_expired_harness_leases(&store, &workflow_id)?;
    let harness_events = store.recent_harness_events(&workflow_id, 50)?;
    let recent_failures = harness_events
        .iter()
        .filter(|event| {
            matches!(
                event.kind.as_str(),
                "provider_command_failed"
                    | "provider_timed_out"
                    | "workflow_validation_failed"
                    | "unknown_agent"
                    | "profile_resolution_failed"
                    | "completion_schema_mismatch"
                    | "lease_expired"
            )
        })
        .cloned()
        .collect();
    Ok(HarnessStatusOutput {
        invocations: store.recent_agent_invocations(&workflow_id, 50)?,
        completions: store.recent_agent_completions(&workflow_id, 50)?,
        workflow_status: Some(stored_status(&ir, &store, &workflow_id)?),
        harness_events,
        recent_failures,
        workflow_id,
    })
}

fn recover_expired_harness_leases(
    store: &WorkflowStore,
    workflow_id: &str,
) -> Result<Vec<whippletree_engine::storage::AgentInvocationRecord>, CliError> {
    let now = current_unix_millis().to_string();
    let recovered = store.recover_expired_agent_leases(workflow_id, &now)?;
    for invocation in &recovered {
        append_harness_event(
            store,
            workflow_id,
            Some(&invocation.invocation_id),
            "lease_expired",
            serde_json::json!({
                "agent": invocation.agent,
                "previousStatus": invocation.status,
                "claimedBy": invocation.claimed_by,
                "claimExpiresAt": invocation.claim_expires_at,
            }),
        )?;
    }
    Ok(recovered)
}

fn append_harness_observation_best_effort(
    file: &PathBuf,
    store_path: Option<PathBuf>,
    kind: &str,
    payload: serde_json::Value,
) {
    let Ok(source) = load_source(file) else {
        return;
    };
    let parsed = whippletree_workflow::parse_syntax_with_file(&source, file.display().to_string());
    let workflow_id = parsed
        .ir
        .as_ref()
        .map(|ir| ir.workflow.name.clone())
        .or_else(|| {
            file.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string());
    let store_path = match store_path {
        Some(path) => path,
        None if parsed.ir.is_some() => default_store_path(file, &workflow_id),
        None => return,
    };
    let Ok(store) = WorkflowStore::open(&store_path) else {
        return;
    };
    let _ = append_harness_event(&store, &workflow_id, None, kind, payload);
}

fn load_harness_config(path: &Path) -> Result<HarnessConfig, CliError> {
    let contents = fs::read_to_string(path).map_err(|source| CliError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&contents).map_err(|source| CliError::HarnessConfig {
        path: path.to_path_buf(),
        source,
    })
}

fn load_harness_profile_policy(path: &Path) -> Result<HarnessProfilePolicy, CliError> {
    let contents = fs::read_to_string(path).map_err(|source| CliError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&contents).map_err(|source| CliError::HarnessProfilePolicy {
        path: path.to_path_buf(),
        source,
    })
}

fn validate_harness_profile_policy(
    ir: &whippletree_workflow::WorkflowIr,
    policy: &HarnessProfilePolicy,
) -> Vec<whippletree_workflow::Diagnostic> {
    let mut diagnostics = Vec::new();
    for (name, profile) in &policy.profiles {
        if !valid_profile_name(name) {
            diagnostics.push(profile_policy_error(format!(
                "harness profile `{name}` must be non-empty and contain no whitespace/control characters"
            )));
        }
        if profile.description.trim().is_empty() {
            diagnostics.push(profile_policy_error(format!(
                "harness profile `{name}` must include a non-empty description"
            )));
        }
        validate_harness_provider_name(&mut diagnostics, name, &profile.provider);
        validate_authority_value(
            &mut diagnostics,
            name,
            "filesystem",
            profile.filesystem.as_deref(),
            &["provider_default", "none", "read_only", "workspace_write"],
        );
        validate_authority_value(
            &mut diagnostics,
            name,
            "network",
            profile.network.as_deref(),
            &["provider_default", "denied", "allowed"],
        );
        validate_authority_value(
            &mut diagnostics,
            name,
            "enforcement",
            profile.enforcement.as_deref(),
            &["native", "best_effort", "external", "native_or_best_effort"],
        );
    }

    if matches!(policy.mode, HarnessProfileMode::Custom) {
        if policy.default_profile.is_none() {
            diagnostics.push(profile_policy_error(
                "custom harness profile policy must declare defaultProfile".to_string(),
            ));
        }
        for builtin in [
            "permissive",
            "research",
            "repo-reader",
            "repo-writer",
            "human-review",
        ] {
            let _ = builtin;
        }
    }

    if let Some(default_profile) = &policy.default_profile {
        if !profile_exists(policy, default_profile) {
            diagnostics.push(profile_policy_error(format!(
                "default harness profile `{default_profile}` is not defined"
            )));
        }
    }

    for (agent_name, agent) in &ir.agents {
        let Some(profile) = agent.profile.as_ref() else {
            continue;
        };
        if !valid_profile_name(profile) {
            diagnostics.push(profile_policy_error(format!(
                "agent `{agent_name}` references invalid harness profile `{profile}`"
            )));
        } else if !profile_exists(policy, profile) {
            diagnostics.push(profile_policy_error(format!(
                "agent `{agent_name}` references undefined harness profile `{profile}`"
            )));
        }
    }

    diagnostics
}

fn validate_harness_provider_name(
    diagnostics: &mut Vec<whippletree_workflow::Diagnostic>,
    profile: &str,
    provider: &str,
) {
    if !matches!(provider, "command" | "codex" | "claude" | "pi") {
        diagnostics.push(profile_policy_error(format!(
            "harness profile `{profile}` uses unsupported provider `{provider}`"
        )));
    }
}

fn validate_authority_value(
    diagnostics: &mut Vec<whippletree_workflow::Diagnostic>,
    profile: &str,
    field: &str,
    value: Option<&str>,
    allowed: &[&str],
) {
    let Some(value) = value else {
        return;
    };
    if !allowed.contains(&value) {
        diagnostics.push(profile_policy_error(format!(
            "harness profile `{profile}` has invalid {field} `{value}`"
        )));
    }
}

fn profile_policy_error(message: String) -> whippletree_workflow::Diagnostic {
    whippletree_workflow::Diagnostic {
        severity: whippletree_workflow::Severity::Error,
        message,
        span: None,
    }
}

fn valid_profile_name(name: &str) -> bool {
    !name.trim().is_empty()
        && !name
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

fn profile_exists(policy: &HarnessProfilePolicy, profile: &str) -> bool {
    policy.profiles.contains_key(profile)
        || !matches!(policy.mode, HarnessProfileMode::Custom)
            && builtin_harness_profile(profile).is_some()
}

fn builtin_harness_profile(profile: &str) -> Option<HarnessProfileDefinition> {
    let definition = match profile {
        "research" => HarnessProfileDefinition {
            description:
                "Use for external documentation and web research. Do not edit repository files."
                    .to_string(),
            provider: "codex".to_string(),
            command: Vec::new(),
            args: Vec::new(),
            cwd: None,
            timeout_seconds: Some(1200),
            filesystem: Some("read_only".to_string()),
            network: Some("allowed".to_string()),
            allowed_env: vec!["OPENAI_API_KEY".to_string()],
            allowed_tools: vec!["read".to_string(), "web".to_string()],
            enforcement: Some("native_or_best_effort".to_string()),
        },
        "repo-reader" => HarnessProfileDefinition {
            description: "Use for repository inspection without edits.".to_string(),
            provider: "codex".to_string(),
            command: Vec::new(),
            args: Vec::new(),
            cwd: None,
            timeout_seconds: Some(1200),
            filesystem: Some("read_only".to_string()),
            network: Some("denied".to_string()),
            allowed_env: vec!["OPENAI_API_KEY".to_string()],
            allowed_tools: vec!["read".to_string()],
            enforcement: Some("native_or_best_effort".to_string()),
        },
        "repo-writer" => HarnessProfileDefinition {
            description: "Use for implementation work after the task is clear.".to_string(),
            provider: "codex".to_string(),
            command: Vec::new(),
            args: Vec::new(),
            cwd: None,
            timeout_seconds: Some(1800),
            filesystem: Some("workspace_write".to_string()),
            network: Some("denied".to_string()),
            allowed_env: vec!["OPENAI_API_KEY".to_string()],
            allowed_tools: vec!["read".to_string(), "edit".to_string(), "test".to_string()],
            enforcement: Some("native_or_best_effort".to_string()),
        },
        "human-review" => HarnessProfileDefinition {
            description: "Use for structured approval or decision collection.".to_string(),
            provider: "command".to_string(),
            command: Vec::new(),
            args: Vec::new(),
            cwd: None,
            timeout_seconds: Some(300),
            filesystem: Some("none".to_string()),
            network: Some("denied".to_string()),
            allowed_env: Vec::new(),
            allowed_tools: Vec::new(),
            enforcement: Some("native".to_string()),
        },
        _ => return None,
    };
    Some(definition)
}

fn resolve_harness_provider_command(
    config: &HarnessAgentConfig,
    invocation: &whippletree_engine::storage::AgentInvocationRecord,
) -> Result<Vec<String>, CliError> {
    if !matches!(
        config.provider.as_str(),
        "command" | "codex" | "claude" | "pi"
    ) {
        return Err(CliError::UnsupportedHarnessProvider {
            agent: invocation.agent.clone(),
            provider: config.provider.clone(),
        });
    }
    let mut command = if !config.command.is_empty() {
        config.command.clone()
    } else {
        match config.provider.as_str() {
            "command" => {
                return Err(CliError::EmptyHarnessCommand {
                    agent: invocation.agent.clone(),
                });
            }
            "codex" => vec![
                "codex".to_string(),
                "exec".to_string(),
                "{{prompt}}".to_string(),
            ],
            "claude" => vec![
                "claude".to_string(),
                "-p".to_string(),
                "{{prompt}}".to_string(),
            ],
            "pi" => vec![
                "pi".to_string(),
                "run".to_string(),
                "{{prompt}}".to_string(),
            ],
            provider => {
                return Err(CliError::UnsupportedHarnessProvider {
                    agent: invocation.agent.clone(),
                    provider: provider.to_string(),
                });
            }
        }
    };
    command.extend(config.args.clone());
    if command.is_empty() {
        return Err(CliError::EmptyHarnessCommand {
            agent: invocation.agent.clone(),
        });
    }
    Ok(command)
}

fn resolve_harness_provider(
    ir: &whippletree_workflow::WorkflowIr,
    config: &HarnessConfig,
    policy: Option<&HarnessProfilePolicy>,
    invocation: &whippletree_engine::storage::AgentInvocationRecord,
) -> Result<ResolvedHarnessProvider, CliError> {
    let Some(policy) = policy else {
        let config = config
            .agents
            .get(&invocation.agent)
            .ok_or_else(|| CliError::MissingHarnessProvider(invocation.agent.clone()))?;
        return Ok(ResolvedHarnessProvider {
            profile: None,
            config: config.clone(),
            requested_authority: serde_json::json!({"mode": "legacy_config"}),
            enforced_authority: serde_json::json!({"mode": "legacy_config"}),
            enforcement: "legacy_config".to_string(),
            warnings: Vec::new(),
        });
    };

    let agent_profile = invocation.requested_profile.clone().or_else(|| {
        ir.agents
            .get(&invocation.agent)
            .and_then(|agent| agent.profile.clone())
    });
    let profile_name = match agent_profile.or_else(|| policy.default_profile.clone()) {
        Some(profile) => profile,
        None if matches!(policy.mode, HarnessProfileMode::Permissive) => "permissive".to_string(),
        None => {
            return Err(CliError::MissingDefaultHarnessProfile {
                agent: invocation.agent.clone(),
            });
        }
    };

    if profile_name == "permissive" && !policy.profiles.contains_key("permissive") {
        let config = config
            .agents
            .get(&invocation.agent)
            .ok_or_else(|| CliError::MissingHarnessProvider(invocation.agent.clone()))?;
        return Ok(ResolvedHarnessProvider {
            profile: Some(profile_name),
            config: config.clone(),
            requested_authority: serde_json::json!({
                "filesystem": "provider_default",
                "network": "provider_default"
            }),
            enforced_authority: serde_json::json!({
                "filesystem": "provider_default",
                "network": "provider_default"
            }),
            enforcement: "best_effort".to_string(),
            warnings: vec![
                "permissive profile uses concrete harness config provider defaults".to_string(),
            ],
        });
    }

    let profile = policy
        .profiles
        .get(&profile_name)
        .cloned()
        .or_else(|| {
            (!matches!(policy.mode, HarnessProfileMode::Custom))
                .then(|| builtin_harness_profile(&profile_name))
                .flatten()
        })
        .ok_or_else(|| CliError::MissingHarnessProfile {
            agent: invocation.agent.clone(),
            profile: profile_name.clone(),
        })?;

    let command_provider_allowed = policy
        .allow_command_provider
        .unwrap_or(matches!(policy.mode, HarnessProfileMode::Permissive));
    if profile.provider == "command" && !command_provider_allowed {
        return Err(CliError::HarnessCommandProviderDenied {
            agent: invocation.agent.clone(),
            profile: profile_name,
        });
    }

    let configured_agent = config.agents.get(&invocation.agent);
    if let Some(configured_agent) = configured_agent {
        if configured_agent.provider != profile.provider {
            return Err(CliError::HarnessProfileProviderMismatch {
                agent: invocation.agent.clone(),
                profile: profile_name,
                provider: profile.provider.clone(),
                configured_provider: configured_agent.provider.clone(),
            });
        }
    }

    let mut resolved_config = configured_agent.cloned().unwrap_or(HarnessAgentConfig {
        provider: profile.provider.clone(),
        command: Vec::new(),
        args: Vec::new(),
        cwd: None,
        timeout_seconds: None,
    });
    resolved_config.provider = profile.provider.clone();
    if !profile.command.is_empty() {
        resolved_config.command = profile.command.clone();
    }
    if !profile.args.is_empty() {
        resolved_config.args = profile.args.clone();
    }
    if profile.cwd.is_some() {
        resolved_config.cwd = profile.cwd.clone();
    }
    if profile.timeout_seconds.is_some() {
        resolved_config.timeout_seconds = profile.timeout_seconds;
    }

    let requested_authority = serde_json::json!({
        "filesystem": profile.filesystem.as_deref().unwrap_or("provider_default"),
        "network": profile.network.as_deref().unwrap_or("provider_default"),
        "allowedEnv": profile.allowed_env,
        "allowedTools": profile.allowed_tools,
    });
    let enforcement = profile
        .enforcement
        .clone()
        .unwrap_or_else(|| "best_effort".to_string());
    let mut warnings = Vec::new();
    if matches!(
        enforcement.as_str(),
        "best_effort" | "native_or_best_effort"
    ) {
        warnings.push(format!(
            "provider `{}` restrictions are recorded as `{}` until provider-specific sandbox flags are mapped",
            resolved_config.provider, enforcement
        ));
    }
    let enforced_authority = serde_json::json!({
        "provider": resolved_config.provider,
        "enforcement": enforcement,
        "filesystem": profile.filesystem.as_deref().unwrap_or("provider_default"),
        "network": profile.network.as_deref().unwrap_or("provider_default"),
    });

    Ok(ResolvedHarnessProvider {
        profile: Some(profile_name),
        config: resolved_config,
        requested_authority,
        enforced_authority,
        enforcement,
        warnings,
    })
}

fn expand_harness_arg(
    arg: &str,
    invocation: &whippletree_engine::storage::AgentInvocationRecord,
    run_dir: &Path,
) -> Result<String, CliError> {
    let input_json = serde_json::to_string(&invocation.input)?;
    let prompt = invocation
        .input
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| input_json.clone());
    Ok(arg
        .replace("{{prompt}}", &prompt)
        .replace("{{inputJson}}", &input_json)
        .replace("{{invocationId}}", &invocation.invocation_id)
        .replace("{{agent}}", &invocation.agent)
        .replace("{{runDir}}", &run_dir.display().to_string()))
}

fn run_harness_command(
    config: &HarnessAgentConfig,
    provider_command: &[String],
    invocation: &whippletree_engine::storage::AgentInvocationRecord,
    workflow_id: &str,
    run_dir: &Path,
) -> Result<HarnessProviderOutput, CliError> {
    let expanded_command = provider_command
        .iter()
        .map(|arg| expand_harness_arg(arg, invocation, run_dir))
        .collect::<Result<Vec<_>, _>>()?;
    let Some((program, args)) = expanded_command.split_first() else {
        return Err(CliError::EmptyHarnessCommand {
            agent: invocation.agent.clone(),
        });
    };
    let mut command = ProcessCommand::new(program);
    command.args(args);
    if let Some(cwd) = &config.cwd {
        command.current_dir(cwd);
    }
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.env("WHIPPLETREE_WORKFLOW_ID", workflow_id);
    command.env("WHIPPLETREE_INVOCATION_ID", &invocation.invocation_id);
    command.env("WHIPPLETREE_AGENT", &invocation.agent);
    command.env(
        "WHIPPLETREE_INPUT_JSON",
        serde_json::to_string(&invocation.input)?,
    );
    command.env("WHIPPLETREE_RUN_DIR", run_dir);
    if let Some(message) = invocation
        .input
        .get("message")
        .and_then(serde_json::Value::as_str)
    {
        command.env("WHIPPLETREE_PROMPT", message);
    } else {
        command.env("WHIPPLETREE_PROMPT", serde_json::to_string(&invocation.input)?);
    }
    let mut child = command
        .spawn()
        .map_err(|source| CliError::HarnessExecution {
            invocation_id: invocation.invocation_id.clone(),
            source,
        })?;
    let stdout_reader = child.stdout.take().map(|pipe| {
        thread::spawn(move || {
            let mut pipe = pipe;
            let mut output = Vec::new();
            pipe.read_to_end(&mut output).map(|_| output)
        })
    });
    let stderr_reader = child.stderr.take().map(|pipe| {
        thread::spawn(move || {
            let mut pipe = pipe;
            let mut output = Vec::new();
            pipe.read_to_end(&mut output).map(|_| output)
        })
    });
    let deadline = config
        .timeout_seconds
        .map(|seconds| Instant::now() + Duration::from_secs(seconds));
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|source| CliError::HarnessExecution {
                invocation_id: invocation.invocation_id.clone(),
                source,
            })?
        {
            break status;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            timed_out = true;
            child.kill().map_err(|source| CliError::HarnessExecution {
                invocation_id: invocation.invocation_id.clone(),
                source,
            })?;
            break child.wait().map_err(|source| CliError::HarnessExecution {
                invocation_id: invocation.invocation_id.clone(),
                source,
            })?;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let stdout = match stdout_reader {
        Some(reader) => reader
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("stdout reader panicked")))
            .map_err(|source| CliError::HarnessOutput {
                invocation_id: invocation.invocation_id.clone(),
                source,
            })?,
        None => Vec::new(),
    };
    let mut stderr = match stderr_reader {
        Some(reader) => reader
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("stderr reader panicked")))
            .map_err(|source| CliError::HarnessOutput {
                invocation_id: invocation.invocation_id.clone(),
                source,
            })?,
        None => Vec::new(),
    };
    if timed_out && stderr.is_empty() {
        stderr.extend_from_slice(b"provider timed out");
    }
    Ok(HarnessProviderOutput {
        stdout,
        stderr,
        exit_code: status.code(),
        success: status.success() && !timed_out,
        timed_out,
    })
}

fn append_harness_event(
    store: &WorkflowStore,
    workflow_id: &str,
    invocation_id: Option<&str>,
    kind: &str,
    payload: serde_json::Value,
) -> Result<(), CliError> {
    store.append_harness_event(&whippletree_engine::storage::HarnessEventRecord {
        workflow_id: workflow_id.to_string(),
        event_id: format!("harness_{}", ulid::Ulid::new()),
        invocation_id: invocation_id.map(str::to_string),
        kind: kind.to_string(),
        payload,
        created_at: current_unix_millis().to_string(),
    })?;
    Ok(())
}

fn tail_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars().rev().take(max_chars).collect::<Vec<_>>();
    chars.reverse();
    chars.into_iter().collect::<String>().trim().to_string()
}

fn current_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn build_cli_event(
    ir: &whippletree_workflow::WorkflowIr,
    manifests: &[whippletree_adapters::AdapterManifest],
    workflow_id: &str,
    event_type: String,
    payload: String,
) -> Result<WorkflowEvent, CliError> {
    let payload: serde_json::Value = serde_json::from_str(&payload)?;
    let mut declared = false;
    let mut mismatch = None;

    if let Some(event_schema) = ir.events.get(&event_type).map(|event| &event.payload) {
        declared = true;
        mismatch = event_schema.explain_json_mismatch(&payload, &ir.types);
    }

    for manifest in manifests {
        if let Some(event_schema) = manifest.events.get(&event_type) {
            declared = true;
            if mismatch.is_none() {
                mismatch = event_schema.explain_json_mismatch(&payload, &manifest.types);
            }
        }
    }

    if !declared {
        return Err(CliError::InvalidEvent(format!(
            "event `{event_type}` is not declared"
        )));
    }

    if let Some(reason) = mismatch {
        return Err(CliError::InvalidEvent(format!(
            "payload does not match schema for event `{event_type}`: {reason}"
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
    ir: &whippletree_workflow::WorkflowIr,
    store: &WorkflowStore,
    workflow_id: &str,
) -> Result<whippletree_engine::status::WorkflowStatus, CliError> {
    Ok(whippletree_engine::project_status(ir, store, workflow_id)?)
}

fn check_workflow(
    ir: &whippletree_workflow::WorkflowIr,
    target: CliModelTarget,
) -> Result<CheckOutput, CliError> {
    match target {
        CliModelTarget::Tla => check_tla_workflow(ir),
        CliModelTarget::Maude => check_maude_workflow(ir),
        CliModelTarget::Apalache | CliModelTarget::Veil => {
            let _ = whippletree_modelgen::emit_model(ir, target.into())?;
            unreachable!("unsupported targets currently return modelgen errors")
        }
    }
}

fn prove_workflow(ir: &whippletree_workflow::WorkflowIr) -> Result<ProveOutput, CliError> {
    let checks = vec![
        check_workflow(ir, CliModelTarget::Tla)?,
        check_workflow(ir, CliModelTarget::Maude)?,
    ];

    Ok(ProveOutput {
        ok: checks.iter().all(|check| check.ok),
        available: true,
        message: "bounded generated proof checks passed for TLA+ and Maude".to_string(),
        suggested_command: "whip check --target tla; whip check --target maude".to_string(),
        checks,
    })
}

fn check_tla_workflow(ir: &whippletree_workflow::WorkflowIr) -> Result<CheckOutput, CliError> {
    let model = whippletree_modelgen::emit_model(ir, whippletree_modelgen::ModelTarget::Tla)?;
    let module_name = tla_module_name(&model).unwrap_or_else(|| "WhippletreeModel".to_string());
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
    fs::write(&config_path, whippletree_modelgen::emit_tla_check_config()).map_err(|source| {
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

fn check_maude_workflow(ir: &whippletree_workflow::WorkflowIr) -> Result<CheckOutput, CliError> {
    let model = whippletree_modelgen::emit_model(ir, whippletree_modelgen::ModelTarget::Maude)?;
    let module_name = maude_module_name(&model).unwrap_or_else(|| "WHIPPLETREE-MODEL".to_string());
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
    ir: &whippletree_workflow::WorkflowIr,
    out: Option<PathBuf>,
    manifests: &[whippletree_adapters::AdapterManifest],
    policies: &[whippletree_adapters::CapabilityPolicyDocument],
) -> Result<BuildOutput, CliError> {
    let output_dir = out.unwrap_or_else(|| {
        file.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".whip")
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
    let tla_source = whippletree_modelgen::emit_model(ir, whippletree_modelgen::ModelTarget::Tla)?;
    let tla_name = tla_module_name(&tla_source).unwrap_or_else(|| ir.workflow.name.clone());
    let tla_model = output_dir.join(format!("{tla_name}.tla"));
    let tla_config = output_dir.join(format!("{tla_name}.cfg"));
    let maude_source = whippletree_modelgen::emit_model(ir, whippletree_modelgen::ModelTarget::Maude)?;
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
    fs::write(&tla_config, whippletree_modelgen::emit_tla_check_config()).map_err(|source| {
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
    ir: &whippletree_workflow::WorkflowIr,
    baml_src: &Path,
) -> whippletree_workflow::WorkflowIr {
    let mut ir = ir.clone();
    let artifact = baml_src.to_string_lossy().to_string();
    for function in ir.coerce_functions.values_mut() {
        function.generated_baml_artifact = Some(artifact.clone());
    }
    ir
}

fn emit_baml_source(ir: &whippletree_workflow::WorkflowIr) -> String {
    let mut output = String::new();
    let emitted_types = baml_reachable_types(ir);

    for (name, schema) in &ir.types {
        if !emitted_types.contains(name) {
            continue;
        }
        match schema {
            whippletree_workflow::schema::Schema::Enum { values } => {
                output.push_str(&format!("enum {name} {{\n"));
                for value in values {
                    output.push_str(&format!("  {value}\n"));
                }
                output.push_str("}\n\n");
            }
            whippletree_workflow::schema::Schema::Record { fields } => {
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
                .or_insert_with(|| format!("WhippletreeLLM{next_index}"));
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
        output.push_str("  prompt #\"\n");
        output.push_str(&normalize_baml_prompt(
            function.prompt.as_deref().unwrap_or(""),
        ));
        output.push_str("\n\"#\n");
        output.push_str("}\n\n");
    }

    output
}

fn normalize_baml_prompt(prompt: &str) -> String {
    let mut lines = prompt.lines().collect::<Vec<_>>();
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let common_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.as_bytes()
                .iter()
                .take_while(|byte| **byte == b' ' || **byte == b'\t')
                .count()
        })
        .min()
        .unwrap_or(0);

    lines
        .into_iter()
        .map(|line| line.get(common_indent..).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn baml_reachable_types(ir: &whippletree_workflow::WorkflowIr) -> BTreeSet<String> {
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
    schema: &whippletree_workflow::schema::Schema,
    ir: &whippletree_workflow::WorkflowIr,
    names: &mut BTreeSet<String>,
) {
    match schema {
        whippletree_workflow::schema::Schema::Ref { name } => {
            if names.insert(name.clone()) {
                if let Some(schema) = ir.types.get(name) {
                    collect_baml_type_refs(schema, ir, names);
                }
            }
        }
        whippletree_workflow::schema::Schema::Optional { inner }
        | whippletree_workflow::schema::Schema::List { inner }
        | whippletree_workflow::schema::Schema::Set { inner } => {
            collect_baml_type_refs(inner, ir, names);
        }
        whippletree_workflow::schema::Schema::Map { key, value } => {
            collect_baml_type_refs(key, ir, names);
            collect_baml_type_refs(value, ir, names);
        }
        whippletree_workflow::schema::Schema::Union { variants } => {
            for variant in variants {
                collect_baml_type_refs(variant, ir, names);
            }
        }
        whippletree_workflow::schema::Schema::Record { fields } => {
            for field in fields {
                collect_baml_type_refs(&field.schema, ir, names);
            }
        }
        whippletree_workflow::schema::Schema::String
        | whippletree_workflow::schema::Schema::Int
        | whippletree_workflow::schema::Schema::Float
        | whippletree_workflow::schema::Schema::Boolean
        | whippletree_workflow::schema::Schema::Null
        | whippletree_workflow::schema::Schema::Time
        | whippletree_workflow::schema::Schema::Duration
        | whippletree_workflow::schema::Schema::Agent
        | whippletree_workflow::schema::Schema::Literal { .. }
        | whippletree_workflow::schema::Schema::Enum { .. }
        | whippletree_workflow::schema::Schema::Json => {}
    }
}

fn baml_type(schema: &whippletree_workflow::schema::Schema) -> String {
    match schema {
        whippletree_workflow::schema::Schema::String
        | whippletree_workflow::schema::Schema::Time
        | whippletree_workflow::schema::Schema::Duration
        | whippletree_workflow::schema::Schema::Agent => "string".to_string(),
        whippletree_workflow::schema::Schema::Int => "int".to_string(),
        whippletree_workflow::schema::Schema::Float => "float".to_string(),
        whippletree_workflow::schema::Schema::Boolean => "bool".to_string(),
        whippletree_workflow::schema::Schema::Null => "null".to_string(),
        whippletree_workflow::schema::Schema::Literal { value } => match value {
            serde_json::Value::String(value) => format!("\"{}\"", escape_baml_string(value)),
            serde_json::Value::Bool(value) => value.to_string(),
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::Null => "null".to_string(),
            _ => "string".to_string(),
        },
        whippletree_workflow::schema::Schema::Enum { values } => values.join(" | "),
        whippletree_workflow::schema::Schema::Optional { inner } => format!("{}?", baml_type(inner)),
        whippletree_workflow::schema::Schema::List { inner }
        | whippletree_workflow::schema::Schema::Set { inner } => format!("{}[]", baml_type(inner)),
        whippletree_workflow::schema::Schema::Map { key, value } => {
            format!("map<{}, {}>", baml_type(key), baml_type(value))
        }
        whippletree_workflow::schema::Schema::Union { variants } => variants
            .iter()
            .map(baml_type)
            .collect::<Vec<_>>()
            .join(" | "),
        whippletree_workflow::schema::Schema::Record { .. }
        | whippletree_workflow::schema::Schema::Json => "string".to_string(),
        whippletree_workflow::schema::Schema::Ref { name } => name.clone(),
    }
}

fn escape_baml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn default_store_path(file: &Path, workflow_name: &str) -> PathBuf {
    let base = file.parent().unwrap_or_else(|| Path::new("."));
    base.join(".whip")
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
