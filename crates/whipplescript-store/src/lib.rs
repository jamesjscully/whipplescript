//! Durable SQLite store for event logs, facts, effects, and evidence.

pub mod content;
pub mod coordination;
pub mod files;
pub mod items;
#[cfg(feature = "native")]
pub mod native_stores;
pub mod skill_frontmatter;

use std::result;
#[cfg(feature = "native")]
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use whipplescript_core::Severity;

#[cfg(feature = "native")]
use rusqlite::{params, Connection, OptionalExtension};
#[cfg(feature = "native")]
use serde_json::{json, Value};

pub type StoreResult<T> = result::Result<T, StoreError>;

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    #[cfg(feature = "native")]
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Conflict(String),
    PolicyBlocked {
        effect_id: String,
        reason: String,
    },
    CapacityBlocked {
        effect_id: String,
        reason: String,
    },
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(feature = "native")]
impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(feature = "native")]
pub struct SqliteStore {
    connection: Connection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredEvent {
    pub event_id: String,
    pub sequence: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewEvent<'a> {
    pub instance_id: &'a str,
    pub event_type: &'a str,
    pub payload_json: &'a str,
    pub source: &'a str,
    pub causation_id: Option<&'a str>,
    pub correlation_id: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewProgramVersion<'a> {
    pub program_name: &'a str,
    pub source_hash: &'a str,
    pub ir_hash: &'a str,
    pub compiler_version: &'a str,
    pub declared_capabilities_json: &'a str,
    pub declared_profiles_json: &'a str,
    pub declared_skills_json: &'a str,
    pub declared_schemas_json: &'a str,
    pub analysis_summary_json: &'a str,
    pub generated_artifacts_json: &'a str,
    pub artifact_root: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramVersionRecord {
    pub program_id: String,
    pub version_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramVersionView {
    pub program_id: String,
    pub program_name: String,
    pub version_id: String,
    pub source_hash: String,
    pub ir_hash: String,
    pub compiler_version: String,
    pub analysis_summary_json: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewInstance<'a> {
    pub program_id: &'a str,
    pub version_id: &'a str,
    pub input_json: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewInstanceAuthority<'a> {
    pub workflow_principal: &'a str,
    pub effective_authority_json: &'a str,
}

impl<'a> NewInstanceAuthority<'a> {
    pub fn empty() -> Self {
        Self {
            workflow_principal: "",
            effective_authority_json: "[]",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceRecord {
    pub instance_id: String,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceView {
    pub instance_id: String,
    pub program_id: String,
    pub version_id: String,
    pub revision_epoch: i64,
    pub workflow_principal: String,
    pub effective_authority_json: String,
    pub status: String,
    pub input_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuleCommit<'a> {
    pub instance_id: &'a str,
    pub rule: &'a str,
    pub trigger_event_id: Option<&'a str>,
    pub facts: &'a [NewFact<'a>],
    pub consumed_fact_ids: &'a [&'a str],
    pub effects: &'a [NewEffect<'a>],
    pub dependencies: &'a [NewEffectDependency<'a>],
    pub terminal: Option<WorkflowTerminal<'a>>,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuleCommitRevisionGuard<'a> {
    pub program_version_id: &'a str,
    pub revision_epoch: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkflowTerminal<'a> {
    pub kind: WorkflowTerminalKind,
    pub name: &'a str,
    pub payload_json: &'a str,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowTerminalKind {
    Completed,
    Failed,
}

impl WorkflowTerminalKind {
    pub fn event_type(self) -> &'static str {
        match self {
            Self::Completed => "workflow.completed",
            Self::Failed => "workflow.failed",
        }
    }

    pub fn instance_status(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn action(self) -> &'static str {
        match self {
            Self::Completed => "complete",
            Self::Failed => "fail",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewFact<'a> {
    pub fact_id: &'a str,
    pub name: &'a str,
    pub key: &'a str,
    pub value_json: &'a str,
    pub schema_id: Option<&'a str>,
    pub provenance_class: &'a str,
    pub correlation_id: Option<&'a str>,
    pub source_span_json: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DerivedFact<'a> {
    pub instance_id: &'a str,
    pub fact: NewFact<'a>,
    pub source: &'a str,
    pub causation_id: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

/// One row of a typed fact batch (spec/admission-and-idempotency.md). `fact_id`
/// is the row's derived per-row admission key (the caller computes
/// `H(effect_key, row_index)` or `H(effect_key, natural_key)`); `key` is the
/// fact's natural key/index recorded on the fact row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FactBatchRow<'a> {
    pub fact_id: &'a str,
    pub key: &'a str,
    pub value_json: &'a str,
}

/// A typed fact batch admitted atomically from one validated effect outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FactBatch<'a> {
    pub instance_id: &'a str,
    pub source: &'a str,
    pub causation_id: Option<&'a str>,
    pub correlation_id: Option<&'a str>,
    /// The imported row schema name; each admitted fact carries it as its name.
    pub schema_name: &'a str,
    pub schema_id: Option<&'a str>,
    pub rows: &'a [FactBatchRow<'a>],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FactBatchOutcome {
    pub admitted: usize,
    pub skipped: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewEffect<'a> {
    pub effect_id: &'a str,
    pub kind: &'a str,
    pub target: Option<&'a str>,
    pub input_json: &'a str,
    pub status: &'a str,
    pub idempotency_key: &'a str,
    pub required_capabilities_json: &'a str,
    pub profile: Option<&'a str>,
    pub correlation_id: Option<&'a str>,
    pub source_span_json: Option<&'a str>,
    /// Creation-anchored deadline in seconds; for `timer.wait` effects this
    /// is the timer duration.
    pub timeout_seconds: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilitySchemaRegistration<'a> {
    pub capability: &'a str,
    pub description: &'a str,
    pub schema_json: &'a str,
    pub registered_by_package_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectProviderRegistration<'a> {
    pub provider_id: &'a str,
    pub effect_kind: &'a str,
    pub provider: &'a str,
    pub capability: &'a str,
    pub config_json: &'a str,
    pub registered_by_package_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileRegistration<'a> {
    pub profile_id: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub enforcement_mode: &'a str,
    pub allowed_capabilities_json: &'a str,
    pub config_json: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredProfilePolicy {
    pub enforcement_mode: String,
    pub allowed_capabilities: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageRegistration<'a> {
    pub package_id: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub manifest_json: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityBinding<'a> {
    pub binding_id: &'a str,
    pub program_id: Option<&'a str>,
    pub capability: &'a str,
    pub provider: &'a str,
    pub config_json: &'a str,
}

/// One project-instruction document (AGENTS.md / CLAUDE.md) registered for the
/// durable object's store-backed context resolution (context-assembly Phase 3
/// item 4). `position` is the injection order (root-most first, nearest-cwd
/// last); `content_hash` is the body's hash so it is content-addressed and
/// replay-stable, mirroring skills.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectContextDoc {
    pub position: i64,
    pub path: String,
    pub content_hash: String,
    pub body: String,
}

/// One recorded delta-kernel result (compute plane P8-1): the cached outcome
/// of a hermetic Class-A invocation, keyed by its content key (script hash +
/// environment hash + input hashes). Workspace-wide — not instance-scoped —
/// so a validator/judge on untouched inputs never runs twice. The source ids
/// record provenance (which run populated the entry).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeCachedResult {
    pub content_key: String,
    pub effect_kind: String,
    pub result_json: String,
    pub source_instance_id: String,
    pub source_effect_id: String,
    pub created_at: String,
}

/// Registration input for one delta-kernel cache entry. Insertion is
/// first-writer-wins (`INSERT OR IGNORE`): a content key is immutable once
/// recorded, which is what makes served-from-cache results canonical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComputeResultRegistration<'a> {
    pub content_key: &'a str,
    pub effect_kind: &'a str,
    pub result_json: &'a str,
    pub source_instance_id: &'a str,
    pub source_effect_id: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkillRegistration<'a> {
    pub skill_id: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub source: &'a str,
    pub source_path: &'a str,
    /// The full SKILL.md bytes the model reads on activation. `content_hash` is
    /// this body's hash (Decision 3), so it is content-addressed and replay-stable.
    pub body: &'a str,
    pub description: &'a str,
    pub required_capabilities_json: &'a str,
    pub metadata_json: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkillAttachment<'a> {
    pub attachment_id: &'a str,
    pub scope_type: &'a str,
    pub scope_id: &'a str,
    pub skill_name: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkillEvidence<'a> {
    pub instance_id: &'a str,
    pub run_id: &'a str,
    pub effect_id: &'a str,
    pub skill_names: &'a [&'a str],
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillView {
    pub skill_id: String,
    pub name: String,
    pub version: String,
    pub source: String,
    pub source_path: String,
    pub content_hash: String,
    pub description: String,
    pub required_capabilities_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillAttachmentView {
    pub attachment_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub skill: SkillView,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvidenceRecord<'a> {
    pub instance_id: &'a str,
    pub kind: &'a str,
    pub subject_type: &'a str,
    pub subject_id: &'a str,
    pub causation_id: Option<&'a str>,
    pub correlation_id: Option<&'a str>,
    pub summary: Option<&'a str>,
    pub metadata_json: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceView {
    pub evidence_id: String,
    pub instance_id: String,
    pub kind: String,
    pub subject_type: String,
    pub subject_id: String,
    pub causation_id: Option<String>,
    pub correlation_id: Option<String>,
    pub summary: Option<String>,
    pub metadata_json: String,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvidenceLink<'a> {
    pub evidence_id: &'a str,
    pub instance_id: &'a str,
    pub target_type: &'a str,
    pub target_id: &'a str,
    pub relation: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceLinkView {
    pub evidence_id: String,
    pub target_type: String,
    pub target_id: String,
    pub relation: String,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderValidationEvidence<'a> {
    pub instance_id: &'a str,
    pub provider_id: &'a str,
    pub provider_kind: &'a str,
    pub surface: &'a str,
    pub status: &'a str,
    pub config_json: &'a str,
    pub capability_json: &'a str,
    pub validation_results_json: &'a str,
    pub source_path: Option<&'a str>,
    pub correlation_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodexAppServerEvidence<'a> {
    pub instance_id: &'a str,
    pub provider_id: &'a str,
    pub thread_id: &'a str,
    pub turn_id: &'a str,
    pub metadata_json: &'a str,
    pub correlation_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClaudeAgentSdkEvidence<'a> {
    pub instance_id: &'a str,
    pub provider_id: &'a str,
    pub session_id: &'a str,
    pub run_id: &'a str,
    pub metadata_json: &'a str,
    pub correlation_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PiRpcEvidence<'a> {
    pub instance_id: &'a str,
    pub provider_id: &'a str,
    pub session_id: &'a str,
    pub run_id: &'a str,
    pub metadata_json: &'a str,
    pub correlation_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactRecord<'a> {
    pub run_id: &'a str,
    pub kind: &'a str,
    pub path: &'a str,
    pub content_hash: Option<&'a str>,
    pub mime_type: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactView {
    pub artifact_id: String,
    pub run_id: String,
    pub kind: String,
    pub path: String,
    pub content_hash: Option<String>,
    pub mime_type: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceRecord<'a> {
    pub instance_id: Option<&'a str>,
    pub effect_id: Option<&'a str>,
    pub run_id: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub policy: &'a str,
    pub uri: &'a str,
    pub status: &'a str,
    pub metadata_json: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceView {
    pub workspace_id: String,
    pub instance_id: Option<String>,
    pub effect_id: Option<String>,
    pub run_id: Option<String>,
    pub provider: Option<String>,
    pub policy: String,
    pub uri: String,
    pub status: String,
    pub metadata_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticRecord<'a> {
    pub instance_id: Option<&'a str>,
    pub program_id: Option<&'a str>,
    pub program_version_id: Option<&'a str>,
    pub severity: Severity,
    pub code: Option<&'a str>,
    pub message: &'a str,
    pub source_span_json: Option<&'a str>,
    pub subject_type: Option<&'a str>,
    pub subject_id: Option<&'a str>,
    pub event_id: Option<&'a str>,
    pub effect_id: Option<&'a str>,
    pub run_id: Option<&'a str>,
    pub assertion_id: Option<&'a str>,
    pub evidence_ids_json: &'a str,
    pub artifact_ids_json: &'a str,
    pub causation_id: Option<&'a str>,
    pub correlation_id: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalDiagnosticRecord {
    pub program_id: Option<String>,
    pub program_version_id: Option<String>,
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
    pub source_span_json: Option<String>,
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
    pub assertion_id: Option<String>,
    pub evidence_ids_json: String,
    pub artifact_ids_json: String,
    pub causation_id: Option<String>,
    pub correlation_id: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticView {
    pub diagnostic_id: String,
    pub instance_id: Option<String>,
    pub program_id: Option<String>,
    pub program_version_id: Option<String>,
    pub severity: String,
    pub code: Option<String>,
    pub message: String,
    pub source_span_json: Option<String>,
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
    pub event_id: Option<String>,
    pub effect_id: Option<String>,
    pub run_id: Option<String>,
    pub assertion_id: Option<String>,
    pub evidence_ids_json: String,
    pub artifact_ids_json: String,
    pub causation_id: Option<String>,
    pub correlation_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewInboxItem<'a> {
    pub inbox_item_id: &'a str,
    pub instance_id: &'a str,
    pub effect_id: Option<&'a str>,
    pub status: &'a str,
    pub prompt: &'a str,
    pub choices_json: &'a str,
    pub freeform_allowed: bool,
    pub severity: &'a str,
    pub related_effects_json: &'a str,
    pub related_artifacts_json: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxItemView {
    pub inbox_item_id: String,
    pub instance_id: String,
    pub effect_id: Option<String>,
    pub status: String,
    pub prompt: String,
    pub choices_json: String,
    pub freeform_allowed: bool,
    pub severity: String,
    pub related_effects_json: String,
    pub related_artifacts_json: String,
    pub answer_json: Option<String>,
    pub answered_by: Option<String>,
    pub created_at: String,
    pub answered_at: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanAnswer<'a> {
    pub inbox_item_id: &'a str,
    pub answer_json: &'a str,
    pub answered_by: &'a str,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewEffectDependency<'a> {
    pub dependency_id: &'a str,
    pub upstream_effect_id: &'a str,
    pub downstream_effect_id: &'a str,
    pub predicate: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewWorkflowInvocation<'a> {
    pub invocation_id: &'a str,
    pub parent_instance_id: &'a str,
    pub parent_effect_id: &'a str,
    pub child_instance_id: &'a str,
    pub target_workflow: &'a str,
    pub input_json: &'a str,
    pub source_span_json: Option<&'a str>,
    pub idempotency_key: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowInvocationView {
    pub invocation_id: String,
    pub parent_instance_id: String,
    pub parent_effect_id: String,
    pub parent_program_version_id: Option<String>,
    pub parent_revision_epoch: i64,
    pub parent_active_program_version_id: Option<String>,
    pub parent_active_revision_epoch: Option<i64>,
    pub child_instance_id: String,
    pub child_program_version_id: Option<String>,
    pub child_revision_epoch: Option<i64>,
    pub child_active_program_version_id: Option<String>,
    pub child_active_revision_epoch: Option<i64>,
    pub target_workflow: String,
    pub input_json: String,
    pub status: String,
    pub terminal_event_id: Option<String>,
    pub source_span_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectCompletion<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub run_id: &'a str,
    pub provider: &'a str,
    pub worker_id: &'a str,
    pub status: &'a str,
    pub exit_code: Option<i64>,
    pub summary: Option<&'a str>,
    pub metadata_json: &'a str,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimableEffect {
    pub effect_id: String,
    pub kind: String,
    pub target: Option<String>,
    pub profile: Option<String>,
    pub input_json: String,
    pub required_capabilities_json: String,
    pub declared_profiles_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventView {
    pub event_id: String,
    pub sequence: i64,
    pub event_type: String,
    pub payload_json: String,
    pub source: String,
    pub occurred_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactView {
    pub fact_id: String,
    pub program_version_id: Option<String>,
    pub revision_epoch: i64,
    pub name: String,
    pub key: String,
    pub value_json: String,
    pub provenance_class: String,
    pub source_span_json: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectView {
    pub effect_id: String,
    pub kind: String,
    pub target: Option<String>,
    pub input_json: String,
    pub status: String,
    pub created_by_rule: String,
    pub program_version_id: Option<String>,
    pub revision_epoch: i64,
    pub profile: Option<String>,
    pub required_capabilities_json: String,
    pub declared_profiles_json: String,
    pub policy_block_reason: Option<String>,
    /// For binding-time blocks (status `blocked`), the categorized reason
    /// (`provider_config`/`credentials`/`enforcement`/`provider_health`). Scheduling
    /// blocks encode their category in the `blocked_by_*` status string instead.
    pub policy_block_category: Option<String>,
    pub cancel_requested: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DueTimeEffect {
    pub effect_id: String,
    pub kind: String,
    pub status: String,
    pub timeout_seconds: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunView {
    pub run_id: String,
    pub effect_id: String,
    pub provider: String,
    pub worker_id: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub metadata_json: String,
    pub cancel_requested: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusView {
    pub instance: InstanceView,
    pub fact_count: i64,
    pub queued_effect_count: i64,
    pub blocked_effect_count: i64,
    pub active_run_count: i64,
    pub failure_count: i64,
    pub cancellation_request_count: i64,
    pub revisions: Vec<WorkflowRevisionView>,
    pub parent_invocation: Option<WorkflowInvocationView>,
    pub child_invocations: Vec<WorkflowInvocationView>,
    pub recent_events: Vec<EventView>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionActivation<'a> {
    pub instance_id: &'a str,
    pub from_version_id: &'a str,
    pub to_version_id: &'a str,
    pub activation_policy_json: &'a str,
    pub cancellation_policy: &'a str,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowRevisionView {
    pub revision_id: String,
    pub instance_id: String,
    pub epoch: i64,
    pub from_version_id: String,
    pub to_version_id: String,
    pub activated_by_event_id: String,
    pub activation_policy_json: String,
    pub cancellation_policy: String,
    pub status: String,
    pub idempotency_key: Option<String>,
    pub created_at: String,
    pub activated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionCancellationImpact {
    pub instance_id: String,
    pub active_version_id: String,
    pub active_revision_epoch: i64,
    pub cancellation_policy: String,
    pub terminal_cancel_effects: Vec<String>,
    pub request_cancel_effects: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionCompatibilityReport {
    pub instance_id: String,
    pub active_version_id: String,
    pub candidate_version_id: String,
    pub compatible: bool,
    pub diagnostics: Vec<RevisionCompatibilityDiagnostic>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionCandidate<'a> {
    pub candidate_version_id: &'a str,
    pub program_name: &'a str,
    pub analysis_summary_json: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionCompatibilityDiagnostic {
    pub code: String,
    pub message: String,
    pub subject: Option<String>,
    pub source_span_json: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectCancellationRequest<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub revision_id: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub requested_by: &'a str,
    pub causation_event_id: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectCancellationRequestView {
    pub request_id: String,
    pub instance_id: String,
    pub effect_id: String,
    pub revision_id: Option<String>,
    pub reason: Option<String>,
    pub requested_by: String,
    pub causation_event_id: Option<String>,
    pub status: String,
    pub idempotency_key: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub resolved_by_event_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunStart<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub run_id: &'a str,
    pub provider: &'a str,
    pub worker_id: &'a str,
    pub lease_id: &'a str,
    pub lease_expires_at: &'a str,
    pub metadata_json: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstanceTransition<'a> {
    pub instance_id: &'a str,
    pub status: &'a str,
    pub reason: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectCancellation<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub reason: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseRenewal<'a> {
    pub instance_id: &'a str,
    pub lease_id: &'a str,
    pub run_id: &'a str,
    pub new_expires_at: &'a str,
    pub idempotency_key: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpiredLease {
    pub lease_id: String,
    pub run_id: String,
    pub effect_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryEffect<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub retry_after: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

#[cfg(feature = "native")]
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

#[cfg(feature = "native")]
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "runtime-store-schema",
    sql: include_str!("../migrations/0001_runtime_store.sql"),
}];

/// Stage marker retained for the CLI/kernel scaffold.
pub fn store_stage() -> &'static str {
    whipplescript_core::IMPLEMENTATION_STAGE
}

#[cfg(feature = "native")]
impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref();
        let mut connection = Connection::open(path)?;
        // A whip worker executes the ready set of effects concurrently (a bounded
        // thread pool), and several worker processes may run against one store.
        // WAL lets a writer and readers coexist; `busy_timeout` makes a contended
        // write wait briefly instead of failing with SQLITE_BUSY. The durable
        // lease + per-row idempotency machinery is what makes the concurrent
        // execution itself safe (see models/tla AtMostOneRunExecutingEffect).
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        apply_migrations(&mut connection)?;
        if path.to_string_lossy() != ":memory:" {
            harden_store_file_permissions(path)?;
        }
        Ok(Self { connection })
    }

    pub fn open_in_memory() -> StoreResult<Self> {
        let mut connection = Connection::open_in_memory()?;
        // WAL does not apply to an in-memory database; busy_timeout is harmless
        // and keeps behavior uniform with the file-backed store.
        connection.execute_batch("PRAGMA busy_timeout=5000;")?;
        apply_migrations(&mut connection)?;
        Ok(Self { connection })
    }

    pub fn schema_version(&self) -> StoreResult<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn append_event(&self, event: NewEvent<'_>) -> StoreResult<StoredEvent> {
        append_event_on(&self.connection, event)
    }

    pub fn create_program_version(
        &mut self,
        version: NewProgramVersion<'_>,
    ) -> StoreResult<ProgramVersionRecord> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            r#"
            INSERT INTO programs (program_id, name)
            VALUES ('prg_' || lower(hex(randomblob(16))), ?1)
            ON CONFLICT(name) DO NOTHING
            "#,
            [version.program_name],
        )?;
        let program_id = tx.query_row(
            "SELECT program_id FROM programs WHERE name = ?1",
            [version.program_name],
            |row| row.get::<_, String>(0),
        )?;
        tx.execute(
            r#"
            INSERT INTO program_versions (
                version_id,
                program_id,
                source_hash,
                ir_hash,
                compiler_version,
                declared_capabilities,
                declared_profiles,
                declared_skills,
                declared_schemas,
                analysis_summary,
                generated_artifacts,
                artifact_root
            )
            VALUES (
                'ver_' || lower(hex(randomblob(16))),
                ?1,
                ?2,
                ?3,
                ?4,
                ?5,
                ?6,
                ?7,
                ?8,
                ?9,
                ?10,
                ?11
            )
            ON CONFLICT(program_id, source_hash, ir_hash) DO NOTHING
            "#,
            params![
                &program_id,
                version.source_hash,
                version.ir_hash,
                version.compiler_version,
                version.declared_capabilities_json,
                version.declared_profiles_json,
                version.declared_skills_json,
                version.declared_schemas_json,
                version.analysis_summary_json,
                version.generated_artifacts_json,
                version.artifact_root,
            ],
        )?;
        let version_id = tx.query_row(
            r#"
            SELECT version_id
            FROM program_versions
            WHERE program_id = ?1
              AND source_hash = ?2
              AND ir_hash = ?3
            "#,
            params![&program_id, version.source_hash, version.ir_hash],
            |row| row.get::<_, String>(0),
        )?;
        tx.commit()?;

        Ok(ProgramVersionRecord {
            program_id,
            version_id,
        })
    }

    pub fn get_program_version(&self, version_id: &str) -> StoreResult<Option<ProgramVersionView>> {
        self.connection
            .query_row(
                r#"
                SELECT
                    program_versions.program_id,
                    programs.name,
                    program_versions.version_id,
                    program_versions.source_hash,
                    program_versions.ir_hash,
                    program_versions.compiler_version,
                    program_versions.analysis_summary
                FROM program_versions
                JOIN programs ON programs.program_id = program_versions.program_id
                WHERE program_versions.version_id = ?1
                "#,
                [version_id],
                |row| {
                    Ok(ProgramVersionView {
                        program_id: row.get(0)?,
                        program_name: row.get(1)?,
                        version_id: row.get(2)?,
                        source_hash: row.get(3)?,
                        ir_hash: row.get(4)?,
                        compiler_version: row.get(5)?,
                        analysis_summary_json: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn create_instance(&self, instance: NewInstance<'_>) -> StoreResult<InstanceRecord> {
        self.create_instance_with_authority(instance, NewInstanceAuthority::empty())
    }

    pub fn create_instance_with_authority(
        &self,
        instance: NewInstance<'_>,
        authority: NewInstanceAuthority<'_>,
    ) -> StoreResult<InstanceRecord> {
        self.connection
            .query_row(
                r#"
                INSERT INTO instances (
                    instance_id,
                    program_id,
                    version_id,
                    workflow_principal,
                    effective_authority,
                    status,
                    input_json,
                    started_at
                )
                VALUES (
                    'ins_' || lower(hex(randomblob(16))),
                    ?1,
                    ?2,
                    ?3,
                    ?4,
                    'running',
                    ?5,
                    CURRENT_TIMESTAMP
                )
                RETURNING instance_id, status
                "#,
                params![
                    instance.program_id,
                    instance.version_id,
                    authority.workflow_principal,
                    authority.effective_authority_json,
                    instance.input_json,
                ],
                |row| {
                    Ok(InstanceRecord {
                        instance_id: row.get(0)?,
                        status: row.get(1)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub fn list_instance_revisions(
        &self,
        instance_id: &str,
    ) -> StoreResult<Vec<WorkflowRevisionView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                revision_id,
                instance_id,
                epoch,
                from_version_id,
                to_version_id,
                activated_by_event_id,
                activation_policy_json,
                cancellation_policy,
                status,
                idempotency_key,
                created_at,
                activated_at
            FROM instance_revisions
            WHERE instance_id = ?1
            ORDER BY epoch
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], workflow_revision_from_row)?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn revision_cancellation_impact(
        &self,
        instance_id: &str,
        cancellation_policy: &str,
    ) -> StoreResult<RevisionCancellationImpact> {
        let cancellation_policy = normalize_cancellation_policy(cancellation_policy)?;
        let (active_version_id, active_revision_epoch, status) = self
            .connection
            .query_row(
                r#"
                SELECT version_id, revision_epoch, status
                FROM instances
                WHERE instance_id = ?1
                "#,
                [instance_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::Conflict("instance does not exist".to_owned()))?;
        if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
            return Err(StoreError::Conflict(format!(
                "instance is {status}; revision impact requires a non-terminal instance"
            )));
        }
        let terminal_cancel_effects = if cancellation_policy == "keep" {
            Vec::new()
        } else {
            revision_policy_effects_on(&self.connection, instance_id, false)?
        };
        let request_cancel_effects = if cancellation_policy == "request_running" {
            revision_policy_effects_on(&self.connection, instance_id, true)?
        } else {
            Vec::new()
        };

        Ok(RevisionCancellationImpact {
            instance_id: instance_id.to_owned(),
            active_version_id,
            active_revision_epoch,
            cancellation_policy: cancellation_policy.to_owned(),
            terminal_cancel_effects,
            request_cancel_effects,
        })
    }

    pub fn analyze_revision_compatibility(
        &self,
        instance_id: &str,
        candidate_version_id: &str,
    ) -> StoreResult<RevisionCompatibilityReport> {
        let context = revision_instance_context_on(&self.connection, instance_id)?;
        let (active_program_id, active_summary) =
            program_version_analysis_on(&self.connection, &context.active_version_id)?;
        let (candidate_program_id, candidate_summary) =
            program_version_analysis_on(&self.connection, candidate_version_id)?;

        let mut diagnostics = Vec::new();
        add_instance_revision_diagnostics(&context, &mut diagnostics);
        if active_program_id != context.program_id || candidate_program_id != context.program_id {
            diagnostics.push(revision_compatibility_diagnostic(
                "revision.program_mismatch",
                "candidate version belongs to a different program".to_owned(),
                Some(candidate_version_id),
            ));
        }
        compare_revision_summaries(&active_summary, &candidate_summary, &mut diagnostics);
        add_active_fact_schema_diagnostics(
            &self.connection,
            instance_id,
            &active_summary,
            &candidate_summary,
            &mut diagnostics,
        )?;

        Ok(RevisionCompatibilityReport {
            instance_id: instance_id.to_owned(),
            active_version_id: context.active_version_id,
            candidate_version_id: candidate_version_id.to_owned(),
            compatible: diagnostics.is_empty(),
            diagnostics,
        })
    }

    pub fn analyze_revision_candidate(
        &self,
        instance_id: &str,
        candidate: RevisionCandidate<'_>,
    ) -> StoreResult<RevisionCompatibilityReport> {
        let context = revision_instance_context_on(&self.connection, instance_id)?;
        let (_active_program_id, active_summary) =
            program_version_analysis_on(&self.connection, &context.active_version_id)?;
        let candidate_summary = serde_json::from_str::<Value>(candidate.analysis_summary_json)?;

        let mut diagnostics = Vec::new();
        add_instance_revision_diagnostics(&context, &mut diagnostics);
        if candidate.program_name != context.program_name {
            diagnostics.push(revision_compatibility_diagnostic(
                "revision.program_mismatch",
                format!(
                    "candidate program `{}` does not match active program `{}`",
                    candidate.program_name, context.program_name
                ),
                Some(candidate.program_name),
            ));
        }
        compare_revision_summaries(&active_summary, &candidate_summary, &mut diagnostics);
        add_active_fact_schema_diagnostics(
            &self.connection,
            instance_id,
            &active_summary,
            &candidate_summary,
            &mut diagnostics,
        )?;

        Ok(RevisionCompatibilityReport {
            instance_id: instance_id.to_owned(),
            active_version_id: context.active_version_id,
            candidate_version_id: candidate.candidate_version_id.to_owned(),
            compatible: diagnostics.is_empty(),
            diagnostics,
        })
    }

    pub fn activate_revision(
        &mut self,
        activation: RevisionActivation<'_>,
    ) -> StoreResult<WorkflowRevisionView> {
        let cancellation_policy = normalize_cancellation_policy(activation.cancellation_policy)?;
        let activation_policy: Value = serde_json::from_str(activation.activation_policy_json)?;

        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(idempotency_key) = activation.idempotency_key {
            if let Some(existing) =
                revision_by_idempotency_on(&tx, activation.instance_id, idempotency_key)?
            {
                ensure_revision_idempotency_matches(
                    &existing,
                    &activation,
                    &activation_policy,
                    cancellation_policy,
                )?;
                return Ok(existing);
            }
        }

        let (program_id, program_name, current_version_id, current_epoch, status) = tx
            .query_row(
                r#"
                SELECT instances.program_id, programs.name, instances.version_id, instances.revision_epoch, instances.status
                FROM instances
                JOIN programs ON programs.program_id = instances.program_id
                WHERE instances.instance_id = ?1
                "#,
                [activation.instance_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::Conflict("instance does not exist".to_owned()))?;
        if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
            return Err(StoreError::Conflict(format!(
                "instance is {status}; revisions require a non-terminal instance"
            )));
        }
        if current_version_id != activation.from_version_id {
            return Err(StoreError::Conflict(format!(
                "active version is {}; expected {}",
                current_version_id, activation.from_version_id
            )));
        }
        let to_program_id = tx
            .query_row(
                "SELECT program_id FROM program_versions WHERE version_id = ?1",
                [activation.to_version_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::Conflict("target program version does not exist".to_owned())
            })?;
        if to_program_id != program_id {
            return Err(StoreError::Conflict(
                "target version belongs to a different program".to_owned(),
            ));
        }
        let (_active_program_id, active_summary) =
            program_version_analysis_on(&tx, &current_version_id)?;
        let (_candidate_program_id, candidate_summary) =
            program_version_analysis_on(&tx, activation.to_version_id)?;
        let mut compatibility_diagnostics = Vec::new();
        let context = RevisionInstanceContext {
            program_id: program_id.clone(),
            program_name,
            active_version_id: current_version_id.clone(),
            status: status.clone(),
        };
        add_instance_revision_diagnostics(&context, &mut compatibility_diagnostics);
        compare_revision_summaries(
            &active_summary,
            &candidate_summary,
            &mut compatibility_diagnostics,
        );
        add_active_fact_schema_diagnostics(
            &tx,
            activation.instance_id,
            &active_summary,
            &candidate_summary,
            &mut compatibility_diagnostics,
        )?;
        if !compatibility_diagnostics.is_empty() {
            let codes = compatibility_diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(StoreError::Conflict(format!(
                "revision candidate is incompatible: {codes}"
            )));
        }

        let next_epoch = current_epoch + 1;
        let revision_id = random_id_on(&tx, "rev")?;
        let queued_effects = revision_policy_effects_on(&tx, activation.instance_id, false)?;
        let running_effects = revision_policy_effects_on(&tx, activation.instance_id, true)?;
        let queued_effects_for_policy = if cancellation_policy == "keep" {
            Vec::new()
        } else {
            queued_effects
        };
        let running_effects_for_policy = if cancellation_policy == "request_running" {
            running_effects
        } else {
            Vec::new()
        };
        let payload = json!({
            "revision_id": &revision_id,
            "instance_id": activation.instance_id,
            "from_version_id": activation.from_version_id,
            "to_version_id": activation.to_version_id,
            "from_epoch": current_epoch,
            "to_epoch": next_epoch,
            "activation_policy": activation_policy,
            "cancellation_policy": cancellation_policy,
            "terminal_cancel_effects": &queued_effects_for_policy,
            "request_cancel_effects": &running_effects_for_policy,
        })
        .to_string();
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: activation.instance_id,
                event_type: "workflow.revision_activated",
                payload_json: &payload,
                source: "kernel",
                causation_id: None,
                correlation_id: Some(&revision_id),
                idempotency_key: activation.idempotency_key,
            },
        )?;

        tx.execute(
            r#"
            INSERT INTO instance_revisions (
                revision_id,
                instance_id,
                epoch,
                from_version_id,
                to_version_id,
                activated_by_event_id,
                activation_policy_json,
                cancellation_policy,
                status,
                idempotency_key
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9)
            "#,
            params![
                &revision_id,
                activation.instance_id,
                next_epoch,
                activation.from_version_id,
                activation.to_version_id,
                event.event_id,
                activation.activation_policy_json,
                cancellation_policy,
                activation.idempotency_key,
            ],
        )?;
        tx.execute(
            r#"
            UPDATE instances
            SET version_id = ?1,
                revision_epoch = ?2,
                last_event_id = ?3,
                updated_at = CURRENT_TIMESTAMP
            WHERE instance_id = ?4
            "#,
            params![
                activation.to_version_id,
                next_epoch,
                event.event_id,
                activation.instance_id,
            ],
        )?;

        for effect_id in &queued_effects_for_policy {
            let cancel_payload = json!({
                "effect_id": effect_id,
                "status": "cancelled",
                "revision_id": &revision_id,
                "reason": "workflow revision",
            })
            .to_string();
            let cancel_idempotency_key = format!("revision-cancel:{revision_id}:{effect_id}");
            let cancel_event = append_event_on(
                &tx,
                NewEvent {
                    instance_id: activation.instance_id,
                    event_type: "effect.terminal",
                    payload_json: &cancel_payload,
                    source: "kernel",
                    causation_id: Some(&event.event_id),
                    correlation_id: Some(&revision_id),
                    idempotency_key: Some(&cancel_idempotency_key),
                },
            )?;
            tx.execute(
                r#"
                UPDATE effects
                SET status = 'cancelled',
                    updated_at = CURRENT_TIMESTAMP
                WHERE instance_id = ?1
                  AND effect_id = ?2
                  AND status IN (
                      'queued',
                      'blocked',
                      'blocked_by_dependency',
                      'blocked_by_capacity',
                      'blocked_by_capability',
                      'blocked_by_profile'
                  )
                "#,
                params![activation.instance_id, effect_id],
            )?;
            mark_cancellation_requests_terminal_on(
                &tx,
                activation.instance_id,
                effect_id,
                &cancel_event.event_id,
            )?;
        }
        if !queued_effects_for_policy.is_empty() {
            satisfy_dependencies_on(&tx, activation.instance_id)?;
        }
        let mut cancellation_request_ids = Vec::new();
        for effect_id in &running_effects_for_policy {
            let request_idempotency_key =
                format!("revision-request-cancel:{revision_id}:{effect_id}");
            let request = insert_effect_cancellation_request_on(
                &tx,
                EffectCancellationRequest {
                    instance_id: activation.instance_id,
                    effect_id,
                    revision_id: Some(&revision_id),
                    reason: Some("workflow revision"),
                    requested_by: "workflow.revision",
                    causation_event_id: Some(&event.event_id),
                    idempotency_key: Some(&request_idempotency_key),
                },
            )?;
            cancellation_request_ids.push((effect_id.clone(), request.request_id));
        }
        if !queued_effects_for_policy.is_empty() {
            satisfy_dependencies_on(&tx, activation.instance_id)?;
        }
        let revision_evidence_metadata = json!({
            "revision_id": &revision_id,
            "event_id": event.event_id,
            "from_version_id": activation.from_version_id,
            "to_version_id": activation.to_version_id,
            "from_epoch": current_epoch,
            "to_epoch": next_epoch,
            "cancellation_policy": cancellation_policy,
            "terminal_cancel_effects": &queued_effects_for_policy,
            "request_cancel_effects": &running_effects_for_policy,
            "cancellation_request_ids": cancellation_request_ids
                .iter()
                .map(|(_, request_id)| request_id.as_str())
                .collect::<Vec<_>>(),
        })
        .to_string();
        let revision_evidence_id = insert_evidence_on(
            &tx,
            EvidenceRecord {
                instance_id: activation.instance_id,
                kind: "workflow.revision.activated",
                subject_type: "workflow_revision",
                subject_id: &revision_id,
                causation_id: Some(&event.event_id),
                correlation_id: Some(&revision_id),
                summary: Some("workflow revision activated"),
                metadata_json: &revision_evidence_metadata,
            },
        )?;
        insert_evidence_link_on(
            &tx,
            EvidenceLink {
                evidence_id: &revision_evidence_id,
                instance_id: activation.instance_id,
                target_type: "event",
                target_id: &event.event_id,
                relation: "activated",
            },
        )?;
        insert_evidence_link_on(
            &tx,
            EvidenceLink {
                evidence_id: &revision_evidence_id,
                instance_id: activation.instance_id,
                target_type: "program_version",
                target_id: activation.from_version_id,
                relation: "from_version",
            },
        )?;
        insert_evidence_link_on(
            &tx,
            EvidenceLink {
                evidence_id: &revision_evidence_id,
                instance_id: activation.instance_id,
                target_type: "program_version",
                target_id: activation.to_version_id,
                relation: "to_version",
            },
        )?;
        for effect_id in &queued_effects_for_policy {
            insert_evidence_link_on(
                &tx,
                EvidenceLink {
                    evidence_id: &revision_evidence_id,
                    instance_id: activation.instance_id,
                    target_type: "effect",
                    target_id: effect_id,
                    relation: "terminal_cancelled",
                },
            )?;
        }
        for (effect_id, request_id) in &cancellation_request_ids {
            insert_evidence_link_on(
                &tx,
                EvidenceLink {
                    evidence_id: &revision_evidence_id,
                    instance_id: activation.instance_id,
                    target_type: "effect",
                    target_id: effect_id,
                    relation: "cancellation_requested",
                },
            )?;
            insert_evidence_link_on(
                &tx,
                EvidenceLink {
                    evidence_id: &revision_evidence_id,
                    instance_id: activation.instance_id,
                    target_type: "effect_cancellation_request",
                    target_id: request_id,
                    relation: "created",
                },
            )?;
        }
        let view = revision_by_id_on(&tx, &revision_id)?
            .ok_or_else(|| StoreError::Conflict("revision was not recorded".to_owned()))?;
        tx.commit()?;
        Ok(view)
    }

    pub fn request_effect_cancellation(
        &mut self,
        request: EffectCancellationRequest<'_>,
    ) -> StoreResult<EffectCancellationRequestView> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let status = tx
            .query_row(
                "SELECT status FROM effects WHERE instance_id = ?1 AND effect_id = ?2",
                params![request.instance_id, request.effect_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::Conflict("effect does not exist".to_owned()))?;
        if status != "running" {
            return Err(StoreError::Conflict(format!(
                "effect is {status}; cancellation requests require running work"
            )));
        }
        let view = insert_effect_cancellation_request_on(&tx, request)?;
        tx.commit()?;
        Ok(view)
    }

    pub fn effect_has_open_cancellation_request(
        &self,
        instance_id: &str,
        effect_id: &str,
    ) -> StoreResult<bool> {
        effect_has_open_cancellation_request_on(&self.connection, instance_id, effect_id)
    }

    pub fn list_effect_cancellation_requests(
        &self,
        instance_id: &str,
    ) -> StoreResult<Vec<EffectCancellationRequestView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                request_id,
                instance_id,
                effect_id,
                revision_id,
                reason,
                requested_by,
                causation_event_id,
                status,
                idempotency_key,
                created_at,
                updated_at,
                resolved_by_event_id
            FROM effect_cancellation_requests
            WHERE instance_id = ?1
            ORDER BY created_at, request_id
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], effect_cancellation_request_from_row)?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn record_workflow_invocation(
        &self,
        invocation: NewWorkflowInvocation<'_>,
    ) -> StoreResult<()> {
        serde_json::from_str::<Value>(invocation.input_json)?;
        let (parent_program_version_id, parent_revision_epoch) = self
            .connection
            .query_row(
                r#"
                SELECT program_version_id, revision_epoch
                FROM effects
                WHERE instance_id = ?1
                  AND effect_id = ?2
                "#,
                params![invocation.parent_instance_id, invocation.parent_effect_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::Conflict("parent workflow invoke effect does not exist".to_owned())
            })?;
        let (child_program_version_id, child_revision_epoch) = self
            .connection
            .query_row(
                r#"
                SELECT version_id, revision_epoch
                FROM instances
                WHERE instance_id = ?1
                "#,
                [invocation.child_instance_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::Conflict("child workflow instance does not exist".to_owned())
            })?;
        self.connection.execute(
            r#"
            INSERT INTO workflow_invocations (
                invocation_id,
                parent_instance_id,
                parent_effect_id,
                parent_program_version_id,
                parent_revision_epoch,
                child_instance_id,
                child_program_version_id,
                child_revision_epoch,
                target_workflow,
                input_json,
                source_span_json,
                idempotency_key
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(idempotency_key) DO NOTHING
            "#,
            params![
                invocation.invocation_id,
                invocation.parent_instance_id,
                invocation.parent_effect_id,
                parent_program_version_id,
                parent_revision_epoch,
                invocation.child_instance_id,
                child_program_version_id,
                child_revision_epoch,
                invocation.target_workflow,
                invocation.input_json,
                invocation.source_span_json,
                invocation.idempotency_key,
            ],
        )?;
        Ok(())
    }

    pub fn get_workflow_invocation(
        &self,
        parent_instance_id: &str,
        parent_effect_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>> {
        self.connection
            .query_row(
                r#"
                SELECT
                    invocation_id,
                    parent_instance_id,
                    parent_effect_id,
                    parent_program_version_id,
                    parent_revision_epoch,
                    parent_instance.version_id,
                    parent_instance.revision_epoch,
                    child_instance_id,
                    child_program_version_id,
                    child_revision_epoch,
                    child_instance.version_id,
                    child_instance.revision_epoch,
                    workflow_invocations.target_workflow,
                    workflow_invocations.input_json,
                    CASE
                        WHEN parent_effect.status IN ('completed', 'failed', 'timed_out', 'cancelled')
                        THEN parent_effect.status
                        ELSE workflow_invocations.status
                    END,
                    workflow_invocations.terminal_event_id,
                    workflow_invocations.source_span_json,
                    workflow_invocations.created_at,
                    COALESCE(workflow_invocations.updated_at, workflow_invocations.created_at)
                FROM workflow_invocations
                LEFT JOIN instances AS parent_instance
                  ON parent_instance.instance_id = workflow_invocations.parent_instance_id
                LEFT JOIN instances AS child_instance
                  ON child_instance.instance_id = workflow_invocations.child_instance_id
                LEFT JOIN effects AS parent_effect
                  ON parent_effect.instance_id = workflow_invocations.parent_instance_id
                 AND parent_effect.effect_id = workflow_invocations.parent_effect_id
                WHERE workflow_invocations.parent_instance_id = ?1
                  AND workflow_invocations.parent_effect_id = ?2
                ORDER BY workflow_invocations.created_at DESC, invocation_id DESC
                LIMIT 1
                "#,
                params![parent_instance_id, parent_effect_id],
                workflow_invocation_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_child_workflow_invocations(
        &self,
        parent_instance_id: &str,
    ) -> StoreResult<Vec<WorkflowInvocationView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                invocation_id,
                parent_instance_id,
                parent_effect_id,
                parent_program_version_id,
                parent_revision_epoch,
                parent_instance.version_id,
                parent_instance.revision_epoch,
                child_instance_id,
                child_program_version_id,
                child_revision_epoch,
                child_instance.version_id,
                child_instance.revision_epoch,
                workflow_invocations.target_workflow,
                workflow_invocations.input_json,
                CASE
                    WHEN parent_effect.status IN ('completed', 'failed', 'timed_out', 'cancelled')
                    THEN parent_effect.status
                    ELSE workflow_invocations.status
                END,
                workflow_invocations.terminal_event_id,
                workflow_invocations.source_span_json,
                workflow_invocations.created_at,
                COALESCE(workflow_invocations.updated_at, workflow_invocations.created_at)
            FROM workflow_invocations
            LEFT JOIN instances AS parent_instance
              ON parent_instance.instance_id = workflow_invocations.parent_instance_id
            LEFT JOIN instances AS child_instance
              ON child_instance.instance_id = workflow_invocations.child_instance_id
            LEFT JOIN effects AS parent_effect
              ON parent_effect.instance_id = workflow_invocations.parent_instance_id
             AND parent_effect.effect_id = workflow_invocations.parent_effect_id
            WHERE workflow_invocations.parent_instance_id = ?1
            ORDER BY workflow_invocations.created_at, invocation_id
            "#,
        )?;
        let rows = statement
            .query_map([parent_instance_id], workflow_invocation_from_row)?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_parent_workflow_invocation(
        &self,
        child_instance_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>> {
        self.connection
            .query_row(
                r#"
                SELECT
                    invocation_id,
                    parent_instance_id,
                    parent_effect_id,
                    parent_program_version_id,
                    parent_revision_epoch,
                    parent_instance.version_id,
                    parent_instance.revision_epoch,
                    child_instance_id,
                    child_program_version_id,
                    child_revision_epoch,
                    child_instance.version_id,
                    child_instance.revision_epoch,
                    workflow_invocations.target_workflow,
                    workflow_invocations.input_json,
                    CASE
                        WHEN parent_effect.status IN ('completed', 'failed', 'timed_out', 'cancelled')
                        THEN parent_effect.status
                        ELSE workflow_invocations.status
                    END,
                    workflow_invocations.terminal_event_id,
                    workflow_invocations.source_span_json,
                    workflow_invocations.created_at,
                    COALESCE(workflow_invocations.updated_at, workflow_invocations.created_at)
                FROM workflow_invocations
                LEFT JOIN instances AS parent_instance
                  ON parent_instance.instance_id = workflow_invocations.parent_instance_id
                LEFT JOIN instances AS child_instance
                  ON child_instance.instance_id = workflow_invocations.child_instance_id
                LEFT JOIN effects AS parent_effect
                  ON parent_effect.instance_id = workflow_invocations.parent_instance_id
                 AND parent_effect.effect_id = workflow_invocations.parent_effect_id
                WHERE workflow_invocations.child_instance_id = ?1
                ORDER BY workflow_invocations.created_at DESC, invocation_id DESC
                LIMIT 1
                "#,
                [child_instance_id],
                workflow_invocation_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn commit_rule(&mut self, commit: RuleCommit<'_>) -> StoreResult<StoredEvent> {
        self.commit_rule_inner(commit, None)
    }

    pub fn commit_rule_with_revision_guard(
        &mut self,
        commit: RuleCommit<'_>,
        guard: RuleCommitRevisionGuard<'_>,
    ) -> StoreResult<StoredEvent> {
        self.commit_rule_inner(commit, Some(guard))
    }

    fn commit_rule_inner(
        &mut self,
        commit: RuleCommit<'_>,
        guard: Option<RuleCommitRevisionGuard<'_>>,
    ) -> StoreResult<StoredEvent> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(status) = instance_status_on(&tx, commit.instance_id)? {
            if status != "running" {
                return Err(StoreError::Conflict(format!(
                    "instance is {status}; rule commits require a running instance"
                )));
            }
        }
        let (program_version_id, revision_epoch) = active_revision_on(&tx, commit.instance_id)?;
        if let Some(guard) = guard {
            if program_version_id.as_deref() != Some(guard.program_version_id)
                || revision_epoch != guard.revision_epoch
            {
                return Err(StoreError::Conflict(format!(
                    "active revision changed before rule commit (expected version {} epoch {}, got version {} epoch {})",
                    guard.program_version_id,
                    guard.revision_epoch,
                    program_version_id.as_deref().unwrap_or("<none>"),
                    revision_epoch
                )));
            }
        }
        let payload = rule_commit_payload(commit, program_version_id.as_deref(), revision_epoch)?;
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: commit.instance_id,
                event_type: "rule.committed",
                payload_json: &payload,
                source: "kernel",
                causation_id: commit.trigger_event_id,
                correlation_id: None,
                idempotency_key: commit.idempotency_key,
            },
        )?;

        for fact in commit.facts {
            insert_fact(
                &tx,
                commit.instance_id,
                commit.rule,
                &event.event_id,
                program_version_id.as_deref(),
                revision_epoch,
                fact,
            )?;
        }
        consume_facts(&tx, commit.instance_id, commit.consumed_fact_ids)?;
        for effect in commit.effects {
            insert_effect(
                &tx,
                commit.instance_id,
                commit.rule,
                &event.event_id,
                program_version_id.as_deref(),
                revision_epoch,
                effect,
            )?;
        }
        for dependency in commit.dependencies {
            insert_effect_dependency(&tx, commit.instance_id, commit.rule, dependency)?;
        }
        if let Some(terminal) = commit.terminal {
            let terminal_payload = workflow_terminal_payload(commit, terminal)?;
            let terminal_event = append_event_on(
                &tx,
                NewEvent {
                    instance_id: commit.instance_id,
                    event_type: terminal.kind.event_type(),
                    payload_json: &terminal_payload,
                    source: "kernel",
                    causation_id: Some(&event.event_id),
                    correlation_id: Some(commit.rule),
                    idempotency_key: terminal.idempotency_key,
                },
            )?;
            tx.execute(
                r#"
                UPDATE instances
                SET status = ?1,
                    last_event_id = ?2,
                    last_error = CASE WHEN ?1 = 'failed' THEN ?3 ELSE last_error END,
                    updated_at = CURRENT_TIMESTAMP,
                    completed_at = CURRENT_TIMESTAMP
                WHERE instance_id = ?4
                "#,
                params![
                    terminal.kind.instance_status(),
                    terminal_event.event_id,
                    terminal.name,
                    commit.instance_id,
                ],
            )?;
        }
        let evidence_metadata = json!({
            "rule": commit.rule,
            "trigger_event_id": commit.trigger_event_id,
            "event_id": event.event_id,
            "program_version_id": program_version_id,
            "revision_epoch": revision_epoch,
            "facts": commit.facts.iter().map(|fact| fact.fact_id).collect::<Vec<_>>(),
            "consumed_facts": commit.consumed_fact_ids,
            "effects": commit.effects.iter().map(|effect| effect.effect_id).collect::<Vec<_>>(),
            "terminal": commit.terminal.map(|terminal| json!({
                "action": terminal.kind.action(),
                "name": terminal.name,
                "payload": serde_json::from_str::<Value>(terminal.payload_json).unwrap_or(Value::Null),
            })),
            "dependencies": commit
                .dependencies
                .iter()
                .map(|dependency| dependency.dependency_id)
                .collect::<Vec<_>>(),
        })
        .to_string();
        let evidence_id = insert_evidence_on(
            &tx,
            EvidenceRecord {
                instance_id: commit.instance_id,
                kind: "rule.committed",
                subject_type: "rule_commit",
                subject_id: &event.event_id,
                causation_id: commit.trigger_event_id,
                correlation_id: Some(commit.rule),
                summary: Some("rule committed facts and effects"),
                metadata_json: &evidence_metadata,
            },
        )?;
        insert_evidence_link_on(
            &tx,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: commit.instance_id,
                target_type: "event",
                target_id: &event.event_id,
                relation: "emitted",
            },
        )?;
        insert_evidence_link_on(
            &tx,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: commit.instance_id,
                target_type: "rule",
                target_id: commit.rule,
                relation: "committed",
            },
        )?;
        for fact in commit.facts {
            insert_evidence_link_on(
                &tx,
                EvidenceLink {
                    evidence_id: &evidence_id,
                    instance_id: commit.instance_id,
                    target_type: "fact",
                    target_id: fact.fact_id,
                    relation: "recorded",
                },
            )?;
        }
        for fact_id in commit.consumed_fact_ids {
            insert_evidence_link_on(
                &tx,
                EvidenceLink {
                    evidence_id: &evidence_id,
                    instance_id: commit.instance_id,
                    target_type: "fact",
                    target_id: fact_id,
                    relation: "consumed",
                },
            )?;
        }
        for effect in commit.effects {
            insert_evidence_link_on(
                &tx,
                EvidenceLink {
                    evidence_id: &evidence_id,
                    instance_id: commit.instance_id,
                    target_type: "effect",
                    target_id: effect.effect_id,
                    relation: "queued",
                },
            )?;
        }
        for dependency in commit.dependencies {
            insert_evidence_link_on(
                &tx,
                EvidenceLink {
                    evidence_id: &evidence_id,
                    instance_id: commit.instance_id,
                    target_type: "effect_dependency",
                    target_id: dependency.dependency_id,
                    relation: "created",
                },
            )?;
        }

        tx.commit()?;
        Ok(event)
    }

    pub fn derive_fact(&mut self, derived: DerivedFact<'_>) -> StoreResult<StoredEvent> {
        let payload = json!({
            "fact_id": derived.fact.fact_id,
            "name": derived.fact.name,
            "key": derived.fact.key,
            "value": serde_json::from_str::<Value>(derived.fact.value_json)?,
            "schema_id": derived.fact.schema_id,
            "provenance_class": derived.fact.provenance_class,
            "correlation_id": derived.fact.correlation_id,
        })
        .to_string();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: derived.instance_id,
                event_type: "fact.derived",
                payload_json: &payload,
                source: derived.source,
                causation_id: derived.causation_id,
                correlation_id: derived.fact.correlation_id,
                idempotency_key: derived.idempotency_key,
            },
        )?;
        let (program_version_id, revision_epoch) = active_revision_on(&tx, derived.instance_id)?;
        insert_fact(
            &tx,
            derived.instance_id,
            derived.source,
            &event.event_id,
            program_version_id.as_deref(),
            revision_epoch,
            &derived.fact,
        )?;
        tx.commit()?;
        Ok(event)
    }

    /// Admit a typed fact batch atomically (spec/admission-and-idempotency.md):
    /// from one validated effect outcome, record N facts in a single transaction.
    /// Each row carries its derived per-row admission key as `fact_id`
    /// (the caller computes `H(effect_key, row_index)` or `H(effect_key,
    /// natural_key)`); a row whose key already has a fact is skipped, so
    /// re-admission is idempotent. The whole batch commits or rolls back as a
    /// unit, so a mid-batch failure admits nothing. Realizes the Maude
    /// `importRow` admission model (models/maude/admission.maude).
    pub fn admit_fact_batch(&mut self, batch: FactBatch<'_>) -> StoreResult<FactBatchOutcome> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (program_version_id, revision_epoch) = active_revision_on(&tx, batch.instance_id)?;
        let mut admitted = 0usize;
        let mut skipped = 0usize;
        for row in batch.rows {
            // Idempotent skip: a row whose derived key already produced a fact is
            // absorbed (the admitted-set membership guard in the model).
            let exists = tx
                .query_row(
                    "SELECT 1 FROM facts WHERE fact_id = ?1 AND instance_id = ?2",
                    params![row.fact_id, batch.instance_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if exists {
                skipped += 1;
                continue;
            }
            let value: Value = serde_json::from_str(row.value_json)?;
            let payload = json!({
                "fact_id": row.fact_id,
                "name": batch.schema_name,
                "key": row.key,
                "value": value,
                "schema_id": batch.schema_id,
                "provenance_class": "import",
                "correlation_id": batch.correlation_id,
            })
            .to_string();
            let event = append_event_on(
                &tx,
                NewEvent {
                    instance_id: batch.instance_id,
                    event_type: "fact.derived",
                    payload_json: &payload,
                    source: batch.source,
                    causation_id: batch.causation_id,
                    correlation_id: batch.correlation_id,
                    idempotency_key: Some(row.fact_id),
                },
            )?;
            insert_fact(
                &tx,
                batch.instance_id,
                batch.source,
                &event.event_id,
                program_version_id.as_deref(),
                revision_epoch,
                &NewFact {
                    fact_id: row.fact_id,
                    name: batch.schema_name,
                    key: row.key,
                    value_json: row.value_json,
                    schema_id: batch.schema_id,
                    provenance_class: "import",
                    correlation_id: batch.correlation_id,
                    source_span_json: None,
                },
            )?;
            admitted += 1;
        }
        tx.commit()?;
        Ok(FactBatchOutcome { admitted, skipped })
    }

    pub fn complete_effect(
        &mut self,
        completion: EffectCompletion<'_>,
    ) -> StoreResult<StoredEvent> {
        self.complete_effect_with_terminal_diagnostic(completion, None)
    }

    pub fn complete_effect_with_terminal_diagnostic(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent> {
        let run_status = completion.status;
        self.complete_effect_terminal_inner(completion, diagnostic, run_status)
    }

    /// Recovery resolution: record a distinct `uncertain` run status while the
    /// effect itself becomes `failed` (a Failed subkind). Realizes the TLA+
    /// `ResolveUncertainRun` action and the admission-and-idempotency.md
    /// exactly-once recovery rule (started-without-terminal, no idempotent
    /// re-query → one `uncertain` terminal, never re-executed).
    pub fn resolve_effect_uncertain(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent> {
        self.complete_effect_terminal_inner(completion, diagnostic, "uncertain")
    }

    fn complete_effect_terminal_inner(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
        run_status: &str,
    ) -> StoreResult<StoredEvent> {
        let payload = effect_completion_payload(completion, diagnostic.as_ref());
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: completion.instance_id,
                event_type: "effect.terminal",
                payload_json: &payload,
                source: "kernel",
                causation_id: Some(completion.effect_id),
                correlation_id: None,
                idempotency_key: completion.idempotency_key,
            },
        )?;

        let updated_run = tx.execute(
            r#"
            UPDATE runs
            SET status = ?1,
                completed_at = CURRENT_TIMESTAMP,
                exit_code = ?2,
                summary = ?3,
                metadata_json = ?4
            WHERE run_id = ?5
              AND effect_id = ?6
              AND instance_id = ?7
              AND status = 'running'
            "#,
            params![
                run_status,
                completion.exit_code,
                completion.summary,
                completion.metadata_json,
                completion.run_id,
                completion.effect_id,
                completion.instance_id,
            ],
        )?;
        if updated_run == 0 {
            let terminal_exists = tx
                .query_row(
                    r#"
                    SELECT 1
                    FROM runs
                    WHERE run_id = ?1
                      AND effect_id = ?2
                      AND instance_id = ?3
                      AND status IN ('completed', 'failed', 'timed_out', 'cancelled', 'uncertain')
                    "#,
                    params![
                        completion.run_id,
                        completion.effect_id,
                        completion.instance_id,
                    ],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if terminal_exists {
                return Err(StoreError::Conflict(
                    "run already has a terminal completion".to_owned(),
                ));
            }

            return Err(StoreError::Conflict("run is not running".to_owned()));
        }
        tx.execute(
            r#"
            UPDATE leases
            SET status = 'released',
                released_at = CURRENT_TIMESTAMP
            WHERE run_id = ?1
              AND effect_id = ?2
              AND instance_id = ?3
              AND status = 'active'
            "#,
            params![
                completion.run_id,
                completion.effect_id,
                completion.instance_id,
            ],
        )?;
        tx.execute(
            "UPDATE effects SET status = ?1, updated_at = CURRENT_TIMESTAMP WHERE effect_id = ?2 AND instance_id = ?3",
            params![completion.status, completion.effect_id, completion.instance_id],
        )?;
        mark_cancellation_requests_terminal_on(
            &tx,
            completion.instance_id,
            completion.effect_id,
            &event.event_id,
        )?;
        satisfy_dependencies_on(&tx, completion.instance_id)?;
        if let Some(diagnostic) = diagnostic {
            insert_diagnostic_on(
                &tx,
                DiagnosticRecord {
                    instance_id: Some(completion.instance_id),
                    program_id: diagnostic.program_id.as_deref(),
                    program_version_id: diagnostic.program_version_id.as_deref(),
                    severity: diagnostic.severity,
                    code: diagnostic.code.as_deref(),
                    message: &diagnostic.message,
                    source_span_json: diagnostic.source_span_json.as_deref(),
                    subject_type: diagnostic.subject_type.as_deref(),
                    subject_id: diagnostic.subject_id.as_deref(),
                    event_id: Some(&event.event_id),
                    effect_id: Some(completion.effect_id),
                    run_id: Some(completion.run_id),
                    assertion_id: diagnostic.assertion_id.as_deref(),
                    evidence_ids_json: &diagnostic.evidence_ids_json,
                    artifact_ids_json: &diagnostic.artifact_ids_json,
                    causation_id: diagnostic.causation_id.as_deref(),
                    correlation_id: diagnostic.correlation_id.as_deref(),
                    idempotency_key: diagnostic.idempotency_key.as_deref(),
                },
            )?;
        }

        tx.commit()?;
        Ok(event)
    }

    /// Distinct kinds of effects still awaiting resolution (any non-terminal
    /// status). Used by the store-open guard to detect effects left behind by a
    /// kind rename (M4): after a one-way rename a persisted effect keeps its old
    /// kind string, which no dispatcher matches, so it would loop forever. The
    /// guard turns that silent hang into a loud, actionable error.
    pub fn pending_effect_kinds(&self) -> StoreResult<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT kind FROM effects \
             WHERE status NOT IN ('completed', 'failed', 'timed_out', 'cancelled')",
        )?;
        let kinds = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(kinds)
    }

    pub fn claimable_effects(&self, instance_id: &str) -> StoreResult<Vec<ClaimableEffect>> {
        if let Some(status) = instance_status_on(&self.connection, instance_id)? {
            if status != "running" {
                return Ok(Vec::new());
            }
        }
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                candidate.effect_id,
                candidate.kind,
                candidate.target,
                candidate.profile,
                candidate.input_json,
                candidate.required_capabilities,
                COALESCE(effect_versions.declared_profiles, active_versions.declared_profiles, '[]')
            FROM effects AS candidate
            LEFT JOIN instances ON instances.instance_id = candidate.instance_id
            LEFT JOIN program_versions AS active_versions
              ON active_versions.version_id = instances.version_id
            LEFT JOIN program_versions AS effect_versions
              ON effect_versions.version_id = candidate.program_version_id
            WHERE candidate.instance_id = ?1
              AND candidate.kind != 'timer.wait'
              AND (
                  candidate.status IN ('queued', 'blocked', 'blocked_by_dependency', 'blocked_by_capacity')
                  OR (candidate.kind = 'workflow.invoke' AND candidate.status = 'running')
              )
              AND NOT EXISTS (
                  SELECT 1
                  FROM effect_cancellation_requests AS request
                  WHERE request.instance_id = candidate.instance_id
                    AND request.effect_id = candidate.effect_id
                    AND request.status = 'requested'
              )
              AND NOT EXISTS (
                  SELECT 1
                  FROM effect_dependencies AS dependency
                  JOIN effects AS upstream
                    ON upstream.effect_id = dependency.upstream_effect_id
                   AND upstream.instance_id = dependency.instance_id
                  WHERE dependency.instance_id = candidate.instance_id
                    AND dependency.downstream_effect_id = candidate.effect_id
                    AND NOT (
                        (dependency.predicate = 'succeeds' AND upstream.status = 'completed')
                        OR (dependency.predicate = 'fails' AND upstream.status IN ('failed', 'timed_out'))
                        OR (dependency.predicate = 'timed_out' AND upstream.status = 'timed_out')
                        OR (dependency.predicate = 'cancelled' AND upstream.status = 'cancelled')
                        OR (dependency.predicate = 'completes' AND upstream.status IN ('completed', 'failed', 'timed_out', 'cancelled'))
                    )
              )
            ORDER BY candidate.created_at, candidate.effect_id
            "#,
        )?;
        let effects = statement
            .query_map([instance_id], |row| {
                Ok(ClaimableEffect {
                    effect_id: row.get(0)?,
                    kind: row.get(1)?,
                    target: row.get(2)?,
                    profile: row.get(3)?,
                    input_json: row.get(4)?,
                    required_capabilities_json: row.get(5)?,
                    declared_profiles_json: row.get(6)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        let mut claimable = Vec::new();
        for effect in effects {
            if policy_block_on(&self.connection, instance_id, &effect.effect_id)?.is_some() {
                continue;
            }
            if capacity_block_on(&self.connection, instance_id, &effect.effect_id)?.is_some() {
                continue;
            }
            claimable.push(effect);
        }
        Ok(claimable)
    }

    pub fn fact_exists(&self, instance_id: &str, fact_name: &str) -> StoreResult<bool> {
        self.connection
            .query_row(
                "SELECT 1 FROM facts WHERE instance_id = ?1 AND name = ?2 LIMIT 1",
                params![instance_id, fact_name],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(Into::into)
    }

    pub fn register_package(&self, package: PackageRegistration<'_>) -> StoreResult<()> {
        serde_json::from_str::<Value>(package.manifest_json)?;
        self.connection.execute(
            r#"
            INSERT INTO package_registrations (package_id, name, version, manifest_json)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(package_id) DO UPDATE SET
                name = excluded.name,
                version = excluded.version,
                manifest_json = excluded.manifest_json
            "#,
            params![
                package.package_id,
                package.name,
                package.version,
                package.manifest_json,
            ],
        )?;
        Ok(())
    }

    pub fn register_package_manifest(&self, manifest_json: &str) -> StoreResult<String> {
        let manifest: Value = serde_json::from_str(manifest_json)?;
        let package_id = required_manifest_string(&manifest, &["package_id", "plugin_id"])?;
        let name = required_manifest_string(&manifest, &["name"])?;
        let version = required_manifest_string(&manifest, &["version"])?;

        self.register_package(PackageRegistration {
            package_id: &package_id,
            name: &name,
            version: &version,
            manifest_json,
        })?;

        for capability in manifest
            .get("capabilities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let schema_json = capability
                .get("schema")
                .map(Value::to_string)
                .unwrap_or_else(|| "{}".to_owned());
            self.register_capability_schema(CapabilitySchemaRegistration {
                capability: &required_manifest_string(capability, &["capability", "id"])?,
                description: capability
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                schema_json: &schema_json,
                registered_by_package_id: Some(&package_id),
            })?;
        }

        for provider in manifest
            .get("providers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let config_json = provider
                .get("config")
                .map(Value::to_string)
                .unwrap_or_else(|| "{}".to_owned());
            self.register_effect_provider(EffectProviderRegistration {
                provider_id: &required_manifest_string(provider, &["provider_id", "id"])?,
                effect_kind: &manifest_effect_kind(provider),
                provider: &required_manifest_string(
                    provider,
                    &["provider", "provider_kind", "kind"],
                )?,
                capability: &required_manifest_string(
                    provider,
                    &["capability", "effect_contract", "effect_contract_id"],
                )?,
                config_json: &config_json,
                registered_by_package_id: Some(&package_id),
            })?;
        }

        for profile in manifest
            .get("profiles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let config_json = profile
                .get("config")
                .map(Value::to_string)
                .unwrap_or_else(|| "{}".to_owned());
            let allowed_json = profile
                .get("allowed_capabilities")
                .map(Value::to_string)
                .unwrap_or_else(|| "[]".to_owned());
            self.register_profile(ProfileRegistration {
                profile_id: &required_manifest_string(profile, &["profile_id", "id"])?,
                name: &required_string(profile, "name"),
                description: profile
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                enforcement_mode: profile
                    .get("enforcement_mode")
                    .and_then(Value::as_str)
                    .unwrap_or("enforce"),
                allowed_capabilities_json: &allowed_json,
                config_json: &config_json,
            })?;
        }

        for binding in manifest
            .get("bindings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let config_json = binding
                .get("config")
                .map(Value::to_string)
                .unwrap_or_else(|| "{}".to_owned());
            self.bind_capability(CapabilityBinding {
                binding_id: &required_manifest_string(binding, &["binding_id", "id"])?,
                program_id: binding.get("program_id").and_then(Value::as_str),
                capability: &required_manifest_string(binding, &["capability"])?,
                provider: &required_manifest_string(binding, &["provider", "provider_kind"])?,
                config_json: &config_json,
            })?;
        }

        Ok(package_id)
    }

    pub fn load_package_manifests_from_dir(
        &self,
        directory: impl AsRef<Path>,
    ) -> StoreResult<Vec<String>> {
        let mut loaded = Vec::new();
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }

            let manifest_json = fs::read_to_string(&path)?;
            loaded.push(self.register_package_manifest(&manifest_json)?);
        }
        loaded.sort();
        Ok(loaded)
    }

    pub fn register_capability_schema(
        &self,
        capability: CapabilitySchemaRegistration<'_>,
    ) -> StoreResult<()> {
        serde_json::from_str::<Value>(capability.schema_json)?;
        self.connection.execute(
            r#"
            INSERT INTO capability_schemas (
                capability,
                description,
                schema_json,
                registered_by_package_id
            )
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(capability) DO UPDATE SET
                description = excluded.description,
                schema_json = excluded.schema_json,
                registered_by_package_id = excluded.registered_by_package_id
            "#,
            params![
                capability.capability,
                capability.description,
                capability.schema_json,
                capability.registered_by_package_id,
            ],
        )?;
        Ok(())
    }

    pub fn register_effect_provider(
        &self,
        provider: EffectProviderRegistration<'_>,
    ) -> StoreResult<()> {
        serde_json::from_str::<Value>(provider.config_json)?;
        self.connection.execute(
            r#"
            INSERT INTO effect_providers (
                provider_id,
                effect_kind,
                provider,
                capability,
                config_json,
                registered_by_package_id
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(effect_kind, provider) DO UPDATE SET
                capability = excluded.capability,
                config_json = excluded.config_json,
                registered_by_package_id = excluded.registered_by_package_id
            "#,
            params![
                provider.provider_id,
                provider.effect_kind,
                provider.provider,
                provider.capability,
                provider.config_json,
                provider.registered_by_package_id,
            ],
        )?;
        Ok(())
    }

    pub fn register_profile(&self, profile: ProfileRegistration<'_>) -> StoreResult<()> {
        serde_json::from_str::<Value>(profile.allowed_capabilities_json)?;
        serde_json::from_str::<Value>(profile.config_json)?;
        self.connection.execute(
            r#"
            INSERT INTO profiles (
                profile_id,
                name,
                description,
                enforcement_mode,
                allowed_capabilities,
                config_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(name) DO UPDATE SET
                description = excluded.description,
                enforcement_mode = excluded.enforcement_mode,
                allowed_capabilities = excluded.allowed_capabilities,
                config_json = excluded.config_json
            "#,
            params![
                profile.profile_id,
                profile.name,
                profile.description,
                profile.enforcement_mode,
                profile.allowed_capabilities_json,
                profile.config_json,
            ],
        )?;
        Ok(())
    }

    pub fn registered_profile_policy(
        &self,
        profile: &str,
    ) -> StoreResult<Option<RegisteredProfilePolicy>> {
        Ok(profile_policy(&self.connection, profile)?.map(
            |(enforcement_mode, allowed_capabilities)| RegisteredProfilePolicy {
                enforcement_mode,
                allowed_capabilities,
            },
        ))
    }

    /// The provider name bound to `capability` for the program running
    /// `instance_id`, mirroring `capability_bound`'s program-scoped-wins-over-global
    /// scoping. `None` when no binding row exists (or its `provider` column is NULL).
    /// This is the registry-driven `CapabilityProvider` selection seam (S5 → first
    /// real provider): the native worker maps the returned name to a concrete impl.
    pub fn capability_bound_provider(
        &self,
        instance_id: &str,
        capability: &str,
    ) -> StoreResult<Option<String>> {
        let program_id = self
            .connection
            .query_row(
                "SELECT program_id FROM instances WHERE instance_id = ?1",
                [instance_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_default();
        capability_bound_provider(&self.connection, &program_id, capability)
    }

    pub fn bind_capability(&self, binding: CapabilityBinding<'_>) -> StoreResult<()> {
        serde_json::from_str::<Value>(binding.config_json)?;
        self.connection.execute(
            r#"
            INSERT INTO capability_bindings (
                binding_id,
                program_id,
                capability,
                provider,
                config_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(binding_id) DO UPDATE SET
                program_id = excluded.program_id,
                capability = excluded.capability,
                provider = excluded.provider,
                config_json = excluded.config_json
            "#,
            params![
                binding.binding_id,
                binding.program_id,
                binding.capability,
                binding.provider,
                binding.config_json,
            ],
        )?;
        Ok(())
    }

    pub fn register_skill(&self, skill: SkillRegistration<'_>) -> StoreResult<()> {
        serde_json::from_str::<Value>(skill.required_capabilities_json)?;
        serde_json::from_str::<Value>(skill.metadata_json)?;
        // Content-address the body (Decision 3): the hash tracks the exact bytes
        // the model reads, so it moves iff the SKILL.md body changes.
        let content_hash = stable_hash_hex(skill.body);
        self.connection.execute(
            r#"
            INSERT INTO skills (
                skill_id,
                name,
                version,
                source,
                source_path,
                content_hash,
                body,
                description,
                required_capabilities,
                metadata_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(name) DO UPDATE SET
                version = excluded.version,
                source = excluded.source,
                source_path = excluded.source_path,
                content_hash = excluded.content_hash,
                body = excluded.body,
                description = excluded.description,
                required_capabilities = excluded.required_capabilities,
                metadata_json = excluded.metadata_json
            "#,
            params![
                skill.skill_id,
                skill.name,
                skill.version,
                skill.source,
                skill.source_path,
                content_hash,
                skill.body,
                skill.description,
                skill.required_capabilities_json,
                skill.metadata_json,
            ],
        )?;
        Ok(())
    }

    pub fn attach_skill(&self, attachment: SkillAttachment<'_>) -> StoreResult<()> {
        let skill_id = self.connection.query_row(
            "SELECT skill_id FROM skills WHERE name = ?1",
            [attachment.skill_name],
            |row| row.get::<_, String>(0),
        )?;
        self.connection.execute(
            r#"
            INSERT INTO skill_attachments (
                attachment_id,
                scope_type,
                scope_id,
                skill_id
            )
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(scope_type, scope_id, skill_id) DO NOTHING
            "#,
            params![
                attachment.attachment_id,
                attachment.scope_type,
                attachment.scope_id,
                skill_id,
            ],
        )?;
        Ok(())
    }

    /// The content-addressed body of the skill registered at `source_path`, or
    /// `None` if no skill is registered there (context-assembly Phase 2, activation
    /// read — Decision 3). Resolving through the registry means the model reads the
    /// exact registered bytes, identically on native and the durable object.
    pub fn skill_body(&self, source_path: &str) -> StoreResult<Option<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT body FROM skills WHERE source_path = ?1")?;
        let mut rows = statement.query(params![source_path])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get::<_, String>(0)?)),
            None => Ok(None),
        }
    }

    /// Register (or replace) one project-instruction document at `position`
    /// (context-assembly Phase 3 item 4). Content-addressed like skills: the
    /// stored hash is the body's hash.
    pub fn register_project_context_doc(
        &self,
        position: i64,
        path: &str,
        body: &str,
    ) -> StoreResult<()> {
        let content_hash = stable_hash_hex(body);
        self.connection.execute(
            "INSERT OR REPLACE INTO project_context_docs (position, path, content_hash, body) \
             VALUES (?1, ?2, ?3, ?4)",
            params![position, path, content_hash, body],
        )?;
        Ok(())
    }

    /// The registered project-instruction documents in injection order.
    pub fn list_project_context_docs(&self) -> StoreResult<Vec<ProjectContextDoc>> {
        let mut statement = self.connection.prepare(
            "SELECT position, path, content_hash, body FROM project_context_docs \
             ORDER BY position",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProjectContextDoc {
                position: row.get(0)?,
                path: row.get(1)?,
                content_hash: row.get(2)?,
                body: row.get(3)?,
            })
        })?;
        rows.collect::<result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Record one delta-kernel result under its content key (compute plane
    /// P8-1). First-writer-wins: returns `true` when this call inserted the
    /// entry, `false` when the key was already recorded (the existing entry is
    /// canonical and left untouched).
    pub fn record_compute_result(
        &self,
        registration: ComputeResultRegistration<'_>,
    ) -> StoreResult<bool> {
        let inserted = self.connection.execute(
            "INSERT OR IGNORE INTO compute_result_cache \
             (content_key, effect_kind, result_json, source_instance_id, source_effect_id) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                registration.content_key,
                registration.effect_kind,
                registration.result_json,
                registration.source_instance_id,
                registration.source_effect_id,
            ],
        )?;
        Ok(inserted > 0)
    }

    /// Look up a delta-kernel result by content key (compute plane P8-1).
    pub fn lookup_compute_result(
        &self,
        content_key: &str,
    ) -> StoreResult<Option<ComputeCachedResult>> {
        self.connection
            .query_row(
                "SELECT content_key, effect_kind, result_json, source_instance_id, \
                 source_effect_id, created_at FROM compute_result_cache WHERE content_key = ?1",
                params![content_key],
                |row| {
                    Ok(ComputeCachedResult {
                        content_key: row.get(0)?,
                        effect_kind: row.get(1)?,
                        result_json: row.get(2)?,
                        source_instance_id: row.get(3)?,
                        source_effect_id: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn list_skills(&self) -> StoreResult<Vec<SkillView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                skill_id,
                name,
                version,
                source,
                source_path,
                content_hash,
                description,
                required_capabilities
            FROM skills
            ORDER BY name
            "#,
        )?;
        let rows = statement
            .query_map([], skill_view_from_row)?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_skill_attachments(
        &self,
        scope_type: &str,
        scope_id: &str,
    ) -> StoreResult<Vec<SkillAttachmentView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                attachment.attachment_id,
                attachment.scope_type,
                attachment.scope_id,
                skill.skill_id,
                skill.name,
                skill.version,
                skill.source,
                skill.source_path,
                skill.content_hash,
                skill.description,
                skill.required_capabilities
            FROM skill_attachments AS attachment
            JOIN skills AS skill ON skill.skill_id = attachment.skill_id
            WHERE attachment.scope_type = ?1
              AND attachment.scope_id = ?2
            ORDER BY skill.name
            "#,
        )?;
        let rows = statement
            .query_map(params![scope_type, scope_id], |row| {
                Ok(SkillAttachmentView {
                    attachment_id: row.get(0)?,
                    scope_type: row.get(1)?,
                    scope_id: row.get(2)?,
                    skill: SkillView {
                        skill_id: row.get(3)?,
                        name: row.get(4)?,
                        version: row.get(5)?,
                        source: row.get(6)?,
                        source_path: row.get(7)?,
                        content_hash: row.get(8)?,
                        description: row.get(9)?,
                        required_capabilities_json: row.get(10)?,
                    },
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn record_evidence(&self, evidence: EvidenceRecord<'_>) -> StoreResult<String> {
        insert_evidence_on(&self.connection, evidence)
    }

    pub fn record_provider_validation_evidence(
        &self,
        evidence: ProviderValidationEvidence<'_>,
    ) -> StoreResult<String> {
        let config = serde_json::from_str::<Value>(evidence.config_json)?;
        let capability = serde_json::from_str::<Value>(evidence.capability_json)?;
        let validation_results = serde_json::from_str::<Value>(evidence.validation_results_json)?;
        let metadata = json!({
            "provider_id": evidence.provider_id,
            "provider_kind": evidence.provider_kind,
            "surface": evidence.surface,
            "status": evidence.status,
            "source_path": evidence.source_path,
            "config": config,
            "capability": capability,
            "validation_results": validation_results,
        })
        .to_string();
        let summary = format!(
            "provider `{}` validation {} on {}",
            evidence.provider_id, evidence.status, evidence.surface
        );
        let evidence_id = insert_evidence_on(
            &self.connection,
            EvidenceRecord {
                instance_id: evidence.instance_id,
                kind: "provider.validation",
                subject_type: "provider_config",
                subject_id: evidence.provider_id,
                causation_id: None,
                correlation_id: evidence.correlation_id,
                summary: Some(&summary),
                metadata_json: &metadata,
            },
        )?;
        insert_evidence_link_on(
            &self.connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "provider",
                target_id: evidence.provider_id,
                relation: "validates",
            },
        )?;
        insert_evidence_link_on(
            &self.connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "provider_capability",
                target_id: &format!("{}:{}", evidence.provider_kind, evidence.surface),
                relation: "uses",
            },
        )?;
        Ok(evidence_id)
    }

    pub fn record_codex_app_server_evidence(
        &self,
        evidence: CodexAppServerEvidence<'_>,
    ) -> StoreResult<String> {
        let metadata = serde_json::from_str::<Value>(evidence.metadata_json)?;
        let metadata = json!({
            "provider_id": evidence.provider_id,
            "thread_id": evidence.thread_id,
            "turn_id": evidence.turn_id,
            "evidence": metadata,
        })
        .to_string();
        let summary = format!(
            "Codex app-server evidence for provider `{}` turn `{}`",
            evidence.provider_id, evidence.turn_id
        );
        let evidence_id = insert_evidence_on(
            &self.connection,
            EvidenceRecord {
                instance_id: evidence.instance_id,
                kind: "codex.app_server.evidence",
                subject_type: "provider_turn",
                subject_id: evidence.turn_id,
                causation_id: None,
                correlation_id: evidence.correlation_id,
                summary: Some(&summary),
                metadata_json: &metadata,
            },
        )?;
        insert_evidence_link_on(
            &self.connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "provider",
                target_id: evidence.provider_id,
                relation: "observes",
            },
        )?;
        insert_evidence_link_on(
            &self.connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "provider_thread",
                target_id: evidence.thread_id,
                relation: "observes",
            },
        )?;
        Ok(evidence_id)
    }

    pub fn record_claude_agent_sdk_evidence(
        &self,
        evidence: ClaudeAgentSdkEvidence<'_>,
    ) -> StoreResult<String> {
        let metadata = serde_json::from_str::<Value>(evidence.metadata_json)?;
        let metadata = json!({
            "provider_id": evidence.provider_id,
            "session_id": evidence.session_id,
            "run_id": evidence.run_id,
            "evidence": metadata,
        })
        .to_string();
        let summary = format!(
            "Claude Agent SDK evidence for provider `{}` session `{}`",
            evidence.provider_id, evidence.session_id
        );
        let evidence_id = insert_evidence_on(
            &self.connection,
            EvidenceRecord {
                instance_id: evidence.instance_id,
                kind: "claude.agent_sdk.evidence",
                subject_type: "provider_session",
                subject_id: evidence.session_id,
                causation_id: Some(evidence.run_id),
                correlation_id: evidence.correlation_id,
                summary: Some(&summary),
                metadata_json: &metadata,
            },
        )?;
        insert_evidence_link_on(
            &self.connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "provider",
                target_id: evidence.provider_id,
                relation: "observes",
            },
        )?;
        insert_evidence_link_on(
            &self.connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "provider_run",
                target_id: evidence.run_id,
                relation: "observes",
            },
        )?;
        Ok(evidence_id)
    }

    pub fn record_pi_rpc_evidence(&self, evidence: PiRpcEvidence<'_>) -> StoreResult<String> {
        let metadata = serde_json::from_str::<Value>(evidence.metadata_json)?;
        let metadata = json!({
            "provider_id": evidence.provider_id,
            "session_id": evidence.session_id,
            "run_id": evidence.run_id,
            "evidence": metadata,
        })
        .to_string();
        let summary = format!(
            "Pi RPC evidence for provider `{}` session `{}`",
            evidence.provider_id, evidence.session_id
        );
        let evidence_id = insert_evidence_on(
            &self.connection,
            EvidenceRecord {
                instance_id: evidence.instance_id,
                kind: "pi.rpc.evidence",
                subject_type: "provider_session",
                subject_id: evidence.session_id,
                causation_id: Some(evidence.run_id),
                correlation_id: evidence.correlation_id,
                summary: Some(&summary),
                metadata_json: &metadata,
            },
        )?;
        insert_evidence_link_on(
            &self.connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "provider",
                target_id: evidence.provider_id,
                relation: "observes",
            },
        )?;
        insert_evidence_link_on(
            &self.connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "provider_run",
                target_id: evidence.run_id,
                relation: "observes",
            },
        )?;
        Ok(evidence_id)
    }

    pub fn link_evidence(&self, link: EvidenceLink<'_>) -> StoreResult<()> {
        insert_evidence_link_on(&self.connection, link)
    }

    pub fn record_artifact(&self, artifact: ArtifactRecord<'_>) -> StoreResult<String> {
        self.connection
            .query_row(
                r#"
                INSERT INTO artifacts (
                    artifact_id,
                    run_id,
                    kind,
                    path,
                    content_hash,
                    mime_type
                )
                VALUES (
                    'art_' || lower(hex(randomblob(16))),
                    ?1,
                    ?2,
                    ?3,
                    ?4,
                    ?5
                )
                RETURNING artifact_id
                "#,
                params![
                    artifact.run_id,
                    artifact.kind,
                    artifact.path,
                    artifact.content_hash,
                    artifact.mime_type,
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(Into::into)
    }

    pub fn list_artifacts_for_run(&self, run_id: &str) -> StoreResult<Vec<ArtifactView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                artifact_id,
                run_id,
                kind,
                path,
                content_hash,
                mime_type,
                created_at
            FROM artifacts
            WHERE run_id = ?1
            ORDER BY created_at, artifact_id
            "#,
        )?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok(ArtifactView {
                    artifact_id: row.get(0)?,
                    run_id: row.get(1)?,
                    kind: row.get(2)?,
                    path: row.get(3)?,
                    content_hash: row.get(4)?,
                    mime_type: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn record_workspace(&self, workspace: WorkspaceRecord<'_>) -> StoreResult<String> {
        validate_workspace_policy(workspace.policy)?;
        validate_workspace_status(workspace.status)?;
        serde_json::from_str::<Value>(workspace.metadata_json)?;
        self.connection
            .query_row(
                r#"
                INSERT INTO workspaces (
                    workspace_id,
                    instance_id,
                    effect_id,
                    run_id,
                    provider,
                    policy,
                    uri,
                    status,
                    metadata_json,
                    updated_at
                )
                VALUES (
                    'wsp_' || lower(hex(randomblob(16))),
                    ?1,
                    ?2,
                    ?3,
                    ?4,
                    ?5,
                    ?6,
                    ?7,
                    ?8,
                    CURRENT_TIMESTAMP
                )
                ON CONFLICT(instance_id, effect_id, run_id, policy)
                DO UPDATE SET
                    provider = excluded.provider,
                    uri = excluded.uri,
                    status = excluded.status,
                    metadata_json = excluded.metadata_json,
                    updated_at = CURRENT_TIMESTAMP
                RETURNING workspace_id
                "#,
                params![
                    workspace.instance_id,
                    workspace.effect_id,
                    workspace.run_id,
                    workspace.provider,
                    workspace.policy,
                    workspace.uri,
                    workspace.status,
                    workspace.metadata_json,
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(Into::into)
    }

    pub fn get_workspace(&self, workspace_id: &str) -> StoreResult<Option<WorkspaceView>> {
        let sql = workspace_select_sql("WHERE workspace_id = ?1");
        self.connection
            .query_row(&sql, [workspace_id], workspace_from_row)
            .optional()
            .map_err(Into::into)
    }

    pub fn list_workspaces_for_instance(
        &self,
        instance_id: &str,
    ) -> StoreResult<Vec<WorkspaceView>> {
        let sql = workspace_select_sql("WHERE instance_id = ?1 ORDER BY created_at, workspace_id");
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement
            .query_map([instance_id], workspace_from_row)?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn record_diagnostic(&self, diagnostic: DiagnosticRecord<'_>) -> StoreResult<String> {
        insert_diagnostic_on(&self.connection, diagnostic)
    }

    pub fn list_diagnostics(&self, instance_id: Option<&str>) -> StoreResult<Vec<DiagnosticView>> {
        let mut sql = r#"
            SELECT
                diagnostic_id,
                instance_id,
                program_id,
                program_version_id,
                severity,
                code,
                message,
                source_span_json,
                subject_type,
                subject_id,
                event_id,
                effect_id,
                run_id,
                assertion_id,
                evidence_ids_json,
                artifact_ids_json,
                causation_id,
                correlation_id,
                idempotency_key,
                created_at
            FROM diagnostics
        "#
        .to_owned();
        if instance_id.is_some() {
            sql.push_str(" WHERE instance_id = ?1");
        }
        sql.push_str(" ORDER BY created_at, diagnostic_id");

        let mut statement = self.connection.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(DiagnosticView {
                diagnostic_id: row.get(0)?,
                instance_id: row.get(1)?,
                program_id: row.get(2)?,
                program_version_id: row.get(3)?,
                severity: row.get(4)?,
                code: row.get(5)?,
                message: row.get(6)?,
                source_span_json: row.get(7)?,
                subject_type: row.get(8)?,
                subject_id: row.get(9)?,
                event_id: row.get(10)?,
                effect_id: row.get(11)?,
                run_id: row.get(12)?,
                assertion_id: row.get(13)?,
                evidence_ids_json: row.get(14)?,
                artifact_ids_json: row.get(15)?,
                causation_id: row.get(16)?,
                correlation_id: row.get(17)?,
                idempotency_key: row.get(18)?,
                created_at: row.get(19)?,
            })
        };
        let rows = if let Some(instance_id) = instance_id {
            statement
                .query_map([instance_id], map_row)?
                .collect::<result::Result<Vec<_>, _>>()?
        } else {
            statement
                .query_map([], map_row)?
                .collect::<result::Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    pub fn list_diagnostics_from_events(
        &self,
        instance_id: &str,
    ) -> StoreResult<Vec<DiagnosticView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT event_id, payload_json, occurred_at
            FROM events
            WHERE instance_id = ?1
              AND event_type = 'effect.terminal'
            ORDER BY sequence
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], |row| {
                let event_id: String = row.get(0)?;
                let payload_json: String = row.get(1)?;
                let occurred_at: String = row.get(2)?;
                Ok((event_id, payload_json, occurred_at))
            })?
            .collect::<result::Result<Vec<_>, _>>()?;

        let mut diagnostics = Vec::new();
        for (event_id, payload_json, occurred_at) in rows {
            let payload = serde_json::from_str::<Value>(&payload_json)?;
            let Some(diagnostic) = payload.get("diagnostic").filter(|value| !value.is_null())
            else {
                continue;
            };
            diagnostics.push(DiagnosticView {
                diagnostic_id: format!("dia_event_{}", stable_hash_hex(&event_id)),
                instance_id: Some(instance_id.to_owned()),
                program_id: optional_string(diagnostic.get("program_id")),
                program_version_id: optional_string(diagnostic.get("program_version_id")),
                severity: optional_string(diagnostic.get("severity"))
                    .unwrap_or_else(|| "error".to_owned()),
                code: optional_string(diagnostic.get("code")),
                message: optional_string(diagnostic.get("message")).unwrap_or_default(),
                source_span_json: diagnostic.get("source_span").and_then(|value| {
                    if value.is_null() {
                        None
                    } else {
                        Some(value.to_string())
                    }
                }),
                subject_type: optional_string(diagnostic.get("subject_type")),
                subject_id: optional_string(diagnostic.get("subject_id")),
                event_id: Some(event_id.clone()),
                effect_id: optional_string(payload.get("effect_id")),
                run_id: optional_string(payload.get("run_id")),
                assertion_id: optional_string(diagnostic.get("assertion_id")),
                evidence_ids_json: diagnostic
                    .get("evidence_ids")
                    .cloned()
                    .unwrap_or_else(|| json!([]))
                    .to_string(),
                artifact_ids_json: diagnostic
                    .get("artifact_ids")
                    .cloned()
                    .unwrap_or_else(|| json!([]))
                    .to_string(),
                causation_id: optional_string(diagnostic.get("causation_id")),
                correlation_id: optional_string(diagnostic.get("correlation_id")),
                idempotency_key: optional_string(diagnostic.get("idempotency_key")),
                created_at: occurred_at,
            });
        }
        Ok(diagnostics)
    }

    pub fn effect_source_span_json(
        &self,
        instance_id: &str,
        effect_id: &str,
    ) -> StoreResult<Option<String>> {
        let payload_json = self
            .connection
            .query_row(
                r#"
                SELECT events.payload_json
                FROM effects
                JOIN events ON events.event_id = effects.created_by_event_id
                WHERE effects.instance_id = ?1
                  AND effects.effect_id = ?2
                "#,
                params![instance_id, effect_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(payload_json) = payload_json else {
            return Ok(None);
        };
        let payload = serde_json::from_str::<Value>(&payload_json)?;
        let span = payload
            .get("effects")
            .and_then(Value::as_array)
            .and_then(|effects| {
                effects.iter().find_map(|effect| {
                    (effect.get("effect_id").and_then(Value::as_str) == Some(effect_id))
                        .then(|| effect.get("source_span"))
                        .flatten()
                        .filter(|value| !value.is_null())
                        .map(Value::to_string)
                })
            });
        Ok(span)
    }

    pub fn create_inbox_item(&self, item: NewInboxItem<'_>) -> StoreResult<()> {
        serde_json::from_str::<Value>(item.choices_json)?;
        serde_json::from_str::<Value>(item.related_effects_json)?;
        serde_json::from_str::<Value>(item.related_artifacts_json)?;
        self.connection.execute(
            r#"
            INSERT INTO inbox_items (
                inbox_item_id,
                instance_id,
                effect_id,
                status,
                prompt,
                choices_json,
                freeform_allowed,
                severity,
                related_effects_json,
                related_artifacts_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                item.inbox_item_id,
                item.instance_id,
                item.effect_id,
                item.status,
                item.prompt,
                item.choices_json,
                if item.freeform_allowed { 1 } else { 0 },
                item.severity,
                item.related_effects_json,
                item.related_artifacts_json,
            ],
        )?;
        Ok(())
    }

    pub fn list_inbox_items(&self, status: Option<&str>) -> StoreResult<Vec<InboxItemView>> {
        let mut sql = r#"
            SELECT
                inbox_item_id,
                instance_id,
                effect_id,
                status,
                prompt,
                choices_json,
                freeform_allowed,
                severity,
                related_effects_json,
                related_artifacts_json,
                answer_json,
                answered_by,
                created_at,
                answered_at
            FROM inbox_items
        "#
        .to_owned();
        if status.is_some() {
            sql.push_str(" WHERE status = ?1");
        }
        sql.push_str(" ORDER BY created_at, inbox_item_id");

        let mut statement = self.connection.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(InboxItemView {
                inbox_item_id: row.get(0)?,
                instance_id: row.get(1)?,
                effect_id: row.get(2)?,
                status: row.get(3)?,
                prompt: row.get(4)?,
                choices_json: row.get(5)?,
                freeform_allowed: row.get::<_, i64>(6)? != 0,
                severity: row.get(7)?,
                related_effects_json: row.get(8)?,
                related_artifacts_json: row.get(9)?,
                answer_json: row.get(10)?,
                answered_by: row.get(11)?,
                created_at: row.get(12)?,
                answered_at: row.get(13)?,
            })
        };
        let rows = if let Some(status) = status {
            statement
                .query_map([status], map_row)?
                .collect::<result::Result<Vec<_>, _>>()?
        } else {
            statement
                .query_map([], map_row)?
                .collect::<result::Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    pub fn get_inbox_item(&self, inbox_item_id: &str) -> StoreResult<Option<InboxItemView>> {
        self.connection
            .query_row(
                r#"
                SELECT
                    inbox_item_id,
                    instance_id,
                    effect_id,
                    status,
                    prompt,
                    choices_json,
                    freeform_allowed,
                    severity,
                    related_effects_json,
                    related_artifacts_json,
                    answer_json,
                    answered_by,
                    created_at,
                    answered_at
                FROM inbox_items
                WHERE inbox_item_id = ?1
                "#,
                [inbox_item_id],
                |row| {
                    Ok(InboxItemView {
                        inbox_item_id: row.get(0)?,
                        instance_id: row.get(1)?,
                        effect_id: row.get(2)?,
                        status: row.get(3)?,
                        prompt: row.get(4)?,
                        choices_json: row.get(5)?,
                        freeform_allowed: row.get::<_, i64>(6)? != 0,
                        severity: row.get(7)?,
                        related_effects_json: row.get(8)?,
                        related_artifacts_json: row.get(9)?,
                        answer_json: row.get(10)?,
                        answered_by: row.get(11)?,
                        created_at: row.get(12)?,
                        answered_at: row.get(13)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn answer_inbox_item(&mut self, answer: HumanAnswer<'_>) -> StoreResult<StoredEvent> {
        let answer_value = serde_json::from_str::<Value>(answer.answer_json)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let item = tx
            .query_row(
                r#"
                SELECT instance_id, effect_id, prompt, status
                FROM inbox_items
                WHERE inbox_item_id = ?1
                "#,
                [answer.inbox_item_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::Conflict("inbox item was not found".to_owned()))?;
        if item.3 != "pending" {
            return Err(StoreError::Conflict(format!(
                "inbox item `{}` is not pending",
                answer.inbox_item_id
            )));
        }

        tx.execute(
            r#"
            UPDATE inbox_items
            SET status = 'answered',
                answer_json = ?2,
                answered_by = ?3,
                answered_at = CURRENT_TIMESTAMP
            WHERE inbox_item_id = ?1
              AND status = 'pending'
            "#,
            params![answer.inbox_item_id, answer.answer_json, answer.answered_by],
        )?;
        let choice = answer_value
            .get("choice")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let text = answer_value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let payload = json!({
            "inbox_item_id": answer.inbox_item_id,
            "effect_id": item.1,
            "prompt": item.2,
            "answered_by": answer.answered_by,
            "choice": choice,
            "text": text,
            "answer": answer_value,
        })
        .to_string();
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: &item.0,
                event_type: "human.answer.received",
                payload_json: &payload,
                source: "human",
                causation_id: Some(answer.inbox_item_id),
                correlation_id: item.1.as_deref(),
                idempotency_key: answer.idempotency_key,
            },
        )?;
        let fact_id = stable_hash_hex(&format!("{}:human-answer", answer.inbox_item_id));
        let fact = NewFact {
            fact_id: &fact_id,
            name: "human.answer.received",
            key: answer.inbox_item_id,
            value_json: &payload,
            schema_id: Some("HumanAnswer"),
            provenance_class: "human",
            correlation_id: item.1.as_deref(),
            source_span_json: None,
        };
        let (program_version_id, revision_epoch) = active_revision_on(&tx, &item.0)?;
        insert_fact(
            &tx,
            &item.0,
            "human",
            &event.event_id,
            program_version_id.as_deref(),
            revision_epoch,
            &fact,
        )?;
        tx.commit()?;
        Ok(event)
    }

    /// Holder-lifetime cleanup: when an instance reaches a terminal, its still
    /// `pending` human asks are moot. Mark them `cancelled` so they drop out of
    /// the inbox and can no longer be answered (`answer_inbox_item` rejects any
    /// non-`pending` item) — otherwise an operator could waste a decision on a
    /// dead instance. `inbox_items` is an authoritative table (not rebuilt by
    /// `rebuild_projections`), so a direct status update is durable. Returns the
    /// number of items cancelled.
    pub fn cancel_pending_inbox_for_instance(&mut self, instance_id: &str) -> StoreResult<usize> {
        let changed = self.connection.execute(
            r#"
            UPDATE inbox_items
            SET status = 'cancelled'
            WHERE instance_id = ?1 AND status = 'pending'
            "#,
            [instance_id],
        )?;
        Ok(changed)
    }

    pub fn record_skill_evidence(&self, evidence: SkillEvidence<'_>) -> StoreResult<String> {
        let skills = self.skills_by_name(evidence.skill_names)?;
        let metadata = json!({
            "effect_id": evidence.effect_id,
            "skills": skills.iter().map(skill_to_json).collect::<Vec<_>>(),
        })
        .to_string();
        let summary = if skills.is_empty() {
            "no skills injected".to_owned()
        } else {
            format!(
                "injected skills: {}",
                skills
                    .iter()
                    .map(|skill| format!("{}@{}", skill.name, skill.version))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        self.record_evidence(EvidenceRecord {
            instance_id: evidence.instance_id,
            kind: "skills.injected",
            subject_type: "run",
            subject_id: evidence.run_id,
            causation_id: Some(evidence.effect_id),
            correlation_id: evidence.idempotency_key,
            summary: Some(&summary),
            metadata_json: &metadata,
        })
    }

    pub fn list_evidence(&self, instance_id: &str) -> StoreResult<Vec<EvidenceView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                evidence_id,
                instance_id,
                kind,
                subject_type,
                subject_id,
                causation_id,
                correlation_id,
                summary,
                metadata_json,
                created_at
            FROM evidence
            WHERE instance_id = ?1
            ORDER BY created_at, evidence_id
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], |row| {
                Ok(EvidenceView {
                    evidence_id: row.get(0)?,
                    instance_id: row.get(1)?,
                    kind: row.get(2)?,
                    subject_type: row.get(3)?,
                    subject_id: row.get(4)?,
                    causation_id: row.get(5)?,
                    correlation_id: row.get(6)?,
                    summary: row.get(7)?,
                    metadata_json: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_evidence_for_subject(
        &self,
        subject_type: &str,
        subject_id: &str,
    ) -> StoreResult<Vec<EvidenceView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                evidence_id,
                instance_id,
                kind,
                subject_type,
                subject_id,
                causation_id,
                correlation_id,
                summary,
                metadata_json,
                created_at
            FROM evidence
            WHERE subject_type = ?1
              AND subject_id = ?2
            ORDER BY created_at, evidence_id
            "#,
        )?;
        let rows = statement
            .query_map(params![subject_type, subject_id], |row| {
                Ok(EvidenceView {
                    evidence_id: row.get(0)?,
                    instance_id: row.get(1)?,
                    kind: row.get(2)?,
                    subject_type: row.get(3)?,
                    subject_id: row.get(4)?,
                    causation_id: row.get(5)?,
                    correlation_id: row.get(6)?,
                    summary: row.get(7)?,
                    metadata_json: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_evidence_links(&self, instance_id: &str) -> StoreResult<Vec<EvidenceLinkView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT evidence_id, target_type, target_id, relation, created_at
            FROM evidence_links
            WHERE instance_id = ?1
            ORDER BY created_at, evidence_id, target_type, target_id, relation
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], |row| {
                Ok(EvidenceLinkView {
                    evidence_id: row.get(0)?,
                    target_type: row.get(1)?,
                    target_id: row.get(2)?,
                    relation: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn skills_by_name(&self, skill_names: &[&str]) -> StoreResult<Vec<SkillView>> {
        let mut skills = Vec::new();
        for name in skill_names {
            let skill = self.connection.query_row(
                r#"
                SELECT
                    skill_id,
                    name,
                    version,
                    source,
                    source_path,
                    content_hash,
                    description,
                    required_capabilities
                FROM skills
                WHERE name = ?1
                "#,
                [name],
                skill_view_from_row,
            )?;
            skills.push(skill);
        }
        skills.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(skills)
    }

    pub fn list_instances(&self) -> StoreResult<Vec<InstanceView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                instance_id,
                program_id,
                version_id,
                revision_epoch,
                workflow_principal,
                effective_authority,
                status,
                input_json,
                created_at,
                updated_at
            FROM instances
            ORDER BY created_at, instance_id
            "#,
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(InstanceView {
                    instance_id: row.get(0)?,
                    program_id: row.get(1)?,
                    version_id: row.get(2)?,
                    revision_epoch: row.get(3)?,
                    workflow_principal: row.get(4)?,
                    effective_authority_json: row.get(5)?,
                    status: row.get(6)?,
                    input_json: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_instance(&self, instance_id: &str) -> StoreResult<Option<InstanceView>> {
        self.connection
            .query_row(
                r#"
                SELECT
                    instance_id,
                    program_id,
                    version_id,
                    revision_epoch,
                    workflow_principal,
                    effective_authority,
                    status,
                    input_json,
                    created_at,
                    updated_at
                FROM instances
                WHERE instance_id = ?1
                "#,
                [instance_id],
                |row| {
                    Ok(InstanceView {
                        instance_id: row.get(0)?,
                        program_id: row.get(1)?,
                        version_id: row.get(2)?,
                        revision_epoch: row.get(3)?,
                        workflow_principal: row.get(4)?,
                        effective_authority_json: row.get(5)?,
                        status: row.get(6)?,
                        input_json: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_events(&self, instance_id: &str) -> StoreResult<Vec<EventView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT event_id, sequence, event_type, payload_json, source, occurred_at
            FROM events
            WHERE instance_id = ?1
            ORDER BY sequence
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], |row| {
                Ok(EventView {
                    event_id: row.get(0)?,
                    sequence: row.get(1)?,
                    event_type: row.get(2)?,
                    payload_json: row.get(3)?,
                    source: row.get(4)?,
                    occurred_at: row.get(5)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_facts(&self, instance_id: &str) -> StoreResult<Vec<FactView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT fact_id, program_version_id, revision_epoch, name, key, value_json, provenance_class, source_span_json
            FROM facts
            WHERE instance_id = ?1
              AND consumed_at IS NULL
            ORDER BY name, key
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], |row| {
                Ok(FactView {
                    fact_id: row.get(0)?,
                    program_version_id: row.get(1)?,
                    revision_epoch: row.get(2)?,
                    name: row.get(3)?,
                    key: row.get(4)?,
                    value_json: row.get(5)?,
                    provenance_class: row.get(6)?,
                    source_span_json: row.get(7)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_facts_including_consumed(&self, instance_id: &str) -> StoreResult<Vec<FactView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT fact_id, program_version_id, revision_epoch, name, key, value_json, provenance_class, source_span_json
            FROM facts
            WHERE instance_id = ?1
            ORDER BY name, key
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], |row| {
                Ok(FactView {
                    fact_id: row.get(0)?,
                    program_version_id: row.get(1)?,
                    revision_epoch: row.get(2)?,
                    name: row.get(3)?,
                    key: row.get(4)?,
                    value_json: row.get(5)?,
                    provenance_class: row.get(6)?,
                    source_span_json: row.get(7)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_effects(&self, instance_id: &str) -> StoreResult<Vec<EffectView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                effects.effect_id,
                effects.kind,
                effects.target,
                effects.input_json,
                effects.status,
                effects.created_by_rule,
                effects.program_version_id,
                effects.revision_epoch,
                effects.profile,
                effects.required_capabilities,
                effects.policy_block_reason,
                effects.policy_block_category,
                COALESCE(effect_versions.declared_profiles, active_versions.declared_profiles, '[]'),
                EXISTS (
                    SELECT 1
                    FROM effect_cancellation_requests AS request
                    WHERE request.instance_id = effects.instance_id
                      AND request.effect_id = effects.effect_id
                      AND request.status = 'requested'
                ) AS cancel_requested
            FROM effects
            LEFT JOIN instances ON instances.instance_id = effects.instance_id
            LEFT JOIN program_versions AS active_versions
              ON active_versions.version_id = instances.version_id
            LEFT JOIN program_versions AS effect_versions
              ON effect_versions.version_id = effects.program_version_id
            WHERE effects.instance_id = ?1
            ORDER BY effects.created_at, effects.effect_id
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], |row| {
                Ok(EffectView {
                    effect_id: row.get(0)?,
                    kind: row.get(1)?,
                    target: row.get(2)?,
                    input_json: row.get(3)?,
                    status: row.get(4)?,
                    created_by_rule: row.get(5)?,
                    program_version_id: row.get(6)?,
                    revision_epoch: row.get(7)?,
                    profile: row.get(8)?,
                    required_capabilities_json: row.get(9)?,
                    policy_block_reason: row.get(10)?,
                    policy_block_category: row.get(11)?,
                    declared_profiles_json: row.get(12)?,
                    cancel_requested: row.get(13)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_runs(&self, instance_id: &str) -> StoreResult<Vec<RunView>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                run_id,
                effect_id,
                provider,
                worker_id,
                status,
                started_at,
                completed_at,
                metadata_json,
                EXISTS (
                    SELECT 1
                    FROM effect_cancellation_requests AS request
                    WHERE request.instance_id = runs.instance_id
                      AND request.effect_id = runs.effect_id
                      AND request.status = 'requested'
                ) AS cancel_requested
            FROM runs
            WHERE runs.instance_id = ?1
            ORDER BY started_at, run_id
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], |row| {
                Ok(RunView {
                    run_id: row.get(0)?,
                    effect_id: row.get(1)?,
                    provider: row.get(2)?,
                    worker_id: row.get(3)?,
                    status: row.get(4)?,
                    started_at: row.get(5)?,
                    completed_at: row.get(6)?,
                    metadata_json: row.get(7)?,
                    cancel_requested: row.get(8)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn status(&self, instance_id: &str) -> StoreResult<Option<StatusView>> {
        let Some(instance) = self.get_instance(instance_id)? else {
            return Ok(None);
        };
        let fact_count = count_where(&self.connection, "facts", instance_id, None)?;
        let queued_effect_count = count_where(
            &self.connection,
            "effects",
            instance_id,
            Some("status IN ('queued', 'blocked_by_dependency')"),
        )?;
        let blocked_effect_count = count_where(
            &self.connection,
            "effects",
            instance_id,
            Some(
                "status IN ('blocked_by_capability', 'blocked_by_profile', 'blocked_by_capacity')",
            ),
        )?;
        let active_run_count = count_where(
            &self.connection,
            "runs",
            instance_id,
            Some("status = 'running'"),
        )?;
        let failure_count = count_where(
            &self.connection,
            "effects",
            instance_id,
            Some("status IN ('failed', 'timed_out')"),
        )?;
        let cancellation_request_count = count_where(
            &self.connection,
            "effect_cancellation_requests",
            instance_id,
            Some("status = 'requested'"),
        )?;
        let mut recent_events = self.list_events(instance_id)?;
        if recent_events.len() > 5 {
            recent_events = recent_events.split_off(recent_events.len() - 5);
        }
        let revisions = self.list_instance_revisions(instance_id)?;
        let parent_invocation = self.get_parent_workflow_invocation(instance_id)?;
        let child_invocations = self.list_child_workflow_invocations(instance_id)?;

        Ok(Some(StatusView {
            instance,
            fact_count,
            queued_effect_count,
            blocked_effect_count,
            active_run_count,
            failure_count,
            cancellation_request_count,
            revisions,
            parent_invocation,
            child_invocations,
            recent_events,
        }))
    }

    pub fn satisfy_dependencies(&self, instance_id: &str) -> StoreResult<usize> {
        satisfy_dependencies_on(&self.connection, instance_id)
    }

    pub fn start_run(&mut self, run: RunStart<'_>) -> StoreResult<StoredEvent> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(status) = instance_status_on(&tx, run.instance_id)? {
            if status != "running" {
                return Err(StoreError::Conflict(format!(
                    "instance is {status}; provider runs require a running instance"
                )));
            }
        }
        if let Some(block) = policy_block_on(&tx, run.instance_id, run.effect_id)? {
            let payload = json!({
                "effect_id": run.effect_id,
                "status": block.status,
                "reason": block.reason,
            })
            .to_string();
            append_event_on(
                &tx,
                NewEvent {
                    instance_id: run.instance_id,
                    event_type: "effect.blocked",
                    payload_json: &payload,
                    source: "kernel",
                    causation_id: Some(run.effect_id),
                    correlation_id: None,
                    idempotency_key: Some(&format!(
                        "policy-block:{}:{}",
                        run.effect_id, run.run_id
                    )),
                },
            )?;
            tx.execute(
                r#"
                UPDATE effects
                SET status = ?1,
                    policy_block_reason = ?2,
                    updated_at = CURRENT_TIMESTAMP
                WHERE instance_id = ?3
                  AND effect_id = ?4
                  AND status IN ('queued', 'blocked', 'blocked_by_dependency', 'blocked_by_capacity')
                "#,
                params![block.status, block.reason, run.instance_id, run.effect_id],
            )?;
            tx.commit()?;
            return Err(StoreError::PolicyBlocked {
                effect_id: run.effect_id.to_owned(),
                reason: block.reason,
            });
        }
        let claimable = tx.query_row(
            r#"
            SELECT NOT EXISTS (
                SELECT 1
                FROM effect_dependencies AS dependency
                JOIN effects AS upstream
                  ON upstream.effect_id = dependency.upstream_effect_id
                 AND upstream.instance_id = dependency.instance_id
                WHERE dependency.instance_id = ?1
                  AND dependency.downstream_effect_id = ?2
                  AND NOT (
                      (dependency.predicate = 'succeeds' AND upstream.status = 'completed')
                      OR (dependency.predicate = 'fails' AND upstream.status IN ('failed', 'timed_out'))
                      OR (dependency.predicate = 'timed_out' AND upstream.status = 'timed_out')
                      OR (dependency.predicate = 'cancelled' AND upstream.status = 'cancelled')
                      OR (dependency.predicate = 'completes' AND upstream.status IN ('completed', 'failed', 'timed_out', 'cancelled'))
                  )
            )
            "#,
            params![run.instance_id, run.effect_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !claimable {
            tx.execute(
                "UPDATE effects SET status = 'blocked_by_dependency', updated_at = CURRENT_TIMESTAMP WHERE instance_id = ?1 AND effect_id = ?2 AND status = 'queued'",
                params![run.instance_id, run.effect_id],
            )?;
            return Err(StoreError::Conflict(
                "effect dependencies are not satisfied".to_owned(),
            ));
        }
        if effect_has_open_cancellation_request_on(&tx, run.instance_id, run.effect_id)? {
            return Err(StoreError::Conflict(
                "effect cancellation has been requested".to_owned(),
            ));
        }
        if let Some(reason) = capacity_block_on(&tx, run.instance_id, run.effect_id)? {
            let payload = json!({
                "effect_id": run.effect_id,
                "status": "blocked_by_capacity",
                "reason": reason,
            })
            .to_string();
            append_event_on(
                &tx,
                NewEvent {
                    instance_id: run.instance_id,
                    event_type: "effect.blocked",
                    payload_json: &payload,
                    source: "kernel",
                    causation_id: Some(run.effect_id),
                    correlation_id: None,
                    idempotency_key: Some(&format!(
                        "capacity-block:{}:{}",
                        run.effect_id, run.run_id
                    )),
                },
            )?;
            tx.execute(
                r#"
                UPDATE effects
                SET status = 'blocked_by_capacity',
                    policy_block_reason = ?1,
                    updated_at = CURRENT_TIMESTAMP
                WHERE instance_id = ?2
                  AND effect_id = ?3
                  AND status IN ('queued', 'blocked', 'blocked_by_dependency', 'blocked_by_capacity')
                "#,
                params![reason, run.instance_id, run.effect_id],
            )?;
            tx.commit()?;
            return Err(StoreError::CapacityBlocked {
                effect_id: run.effect_id.to_owned(),
                reason,
            });
        }

        let fingerprint_salt = fingerprint_salt_from_metadata(run.metadata_json);
        let fingerprint = execution_fingerprint_on(
            &tx,
            run.instance_id,
            run.effect_id,
            fingerprint_salt.as_deref(),
        )?;
        let run_metadata = inject_execution_fingerprint(run.metadata_json, &fingerprint);
        let payload = run_start_payload(run, &run_metadata);
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: run.instance_id,
                event_type: "effect.run_started",
                payload_json: &payload,
                source: "kernel",
                causation_id: Some(run.effect_id),
                correlation_id: None,
                idempotency_key: Some(run.run_id),
            },
        )?;
        let changed = tx.execute(
            r#"
            UPDATE effects
            SET status = 'running',
                policy_block_reason = NULL,
                policy_block_category = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE instance_id = ?1
              AND effect_id = ?2
              AND status IN ('queued', 'blocked', 'blocked_by_dependency', 'blocked_by_capacity')
            "#,
            params![run.instance_id, run.effect_id],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict("effect is not claimable".to_owned()));
        }
        tx.execute(
            r#"
            INSERT INTO runs (
                run_id,
                effect_id,
                instance_id,
                provider,
                worker_id,
                status,
                metadata_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)
            "#,
            params![
                run.run_id,
                run.effect_id,
                run.instance_id,
                run.provider,
                run.worker_id,
                run_metadata,
            ],
        )?;
        tx.execute(
            r#"
            INSERT INTO leases (
                lease_id,
                run_id,
                effect_id,
                instance_id,
                worker_id,
                status,
                expires_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6)
            "#,
            params![
                run.lease_id,
                run.run_id,
                run.effect_id,
                run.instance_id,
                run.worker_id,
                run.lease_expires_at,
            ],
        )?;

        tx.commit()?;
        Ok(event)
    }

    /// Block an effect at provider-binding time (DR-0020): the worker found the
    /// provider unbindable (missing config/credentials/enforcement/healthy
    /// binding) *before* provider execution. Transitions the effect to the
    /// recoverable, non-terminal `blocked` status with a categorized reason and
    /// returns the recorded `effect.blocked` event.
    ///
    /// Idempotent: if the effect is already `blocked` with the same category this
    /// is a no-op that returns the existing block event (no new event), so a
    /// re-claimed effect that stays unbindable does not grow the event log — the
    /// effect simply waits, blocked, until the binding becomes available and the
    /// next worker pass runs it.
    pub fn block_effect_binding(
        &mut self,
        instance_id: &str,
        effect_id: &str,
        category: &str,
        detail: &str,
    ) -> StoreResult<StoredEvent> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT status, policy_block_category FROM effects \
                 WHERE instance_id = ?1 AND effect_id = ?2",
                params![instance_id, effect_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let already_blocked = matches!(
            &current,
            Some((status, cat)) if status == "blocked" && cat.as_deref() == Some(category)
        );
        if already_blocked {
            // Return the existing block event without recording a new one.
            let event = tx.query_row(
                "SELECT event_id, sequence FROM events \
                 WHERE instance_id = ?1 AND event_type = 'effect.blocked' AND causation_id = ?2 \
                 ORDER BY sequence DESC LIMIT 1",
                params![instance_id, effect_id],
                |row| {
                    Ok(StoredEvent {
                        event_id: row.get(0)?,
                        sequence: row.get(1)?,
                    })
                },
            )?;
            tx.commit()?;
            return Ok(event);
        }
        let payload = json!({
            "effect_id": effect_id,
            "status": "blocked",
            "category": category,
            "reason": detail,
        })
        .to_string();
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id,
                event_type: "effect.blocked",
                payload_json: &payload,
                source: "kernel",
                causation_id: Some(effect_id),
                correlation_id: None,
                idempotency_key: Some(&format!(
                    "binding-block:{instance_id}:{effect_id}:{category}"
                )),
            },
        )?;
        tx.execute(
            r#"
            UPDATE effects
            SET status = 'blocked',
                policy_block_reason = ?1,
                policy_block_category = ?2,
                updated_at = CURRENT_TIMESTAMP
            WHERE instance_id = ?3
              AND effect_id = ?4
              AND status IN ('queued', 'blocked', 'blocked_by_dependency', 'blocked_by_capacity')
            "#,
            params![detail, category, instance_id, effect_id],
        )?;
        tx.commit()?;
        Ok(event)
    }

    pub fn transition_instance(
        &mut self,
        transition: InstanceTransition<'_>,
    ) -> StoreResult<StoredEvent> {
        fn transition_allowed(current: &str, next: &str) -> bool {
            matches!(
                (current, next),
                ("running", "paused")
                    | ("paused", "running")
                    | ("running", "cancelled")
                    | ("paused", "cancelled")
                    | ("blocked", "cancelled")
                    // Generic internal failure terminal (flow auto-fail): an
                    // unhandled effect failure fails the instance directly,
                    // without an author-typed `failure` payload.
                    | ("running", "failed")
                    | ("paused", "failed")
                    | ("blocked", "failed")
            )
        }

        let payload = json!({
            "instance_id": transition.instance_id,
            "status": transition.status,
            "reason": transition.reason,
        })
        .to_string();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current_status = instance_status_on(&tx, transition.instance_id)?
            .ok_or_else(|| StoreError::Conflict("instance does not exist".to_owned()))?;
        if !transition_allowed(&current_status, transition.status) {
            return Err(StoreError::Conflict(format!(
                "cannot transition instance from {current_status} to {}",
                transition.status
            )));
        }
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: transition.instance_id,
                event_type: "instance.transitioned",
                payload_json: &payload,
                source: "kernel",
                causation_id: None,
                correlation_id: None,
                idempotency_key: transition.idempotency_key,
            },
        )?;
        tx.execute(
            r#"
            UPDATE instances
            SET status = ?1,
                last_event_id = ?2,
                last_error = ?3,
                updated_at = CURRENT_TIMESTAMP,
                completed_at = CASE
                    WHEN ?1 IN ('completed', 'cancelled', 'failed') THEN CURRENT_TIMESTAMP
                    ELSE completed_at
                END
            WHERE instance_id = ?4
            "#,
            params![
                transition.status,
                event.event_id,
                transition.reason,
                transition.instance_id,
            ],
        )?;
        tx.commit()?;
        Ok(event)
    }

    /// Effects whose creation-anchored time has arrived: due `timer.wait`
    /// effects (dependency-cleared) and effects whose `timeout` deadline has
    /// expired. Resolved on worker passes; rule evaluation never reads the
    /// clock.
    pub fn due_time_effects(
        &self,
        instance_id: &str,
        now: &str,
    ) -> StoreResult<Vec<DueTimeEffect>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                candidate.effect_id,
                candidate.kind,
                candidate.status,
                COALESCE(candidate.timeout_seconds, 0)
            FROM effects AS candidate
            WHERE candidate.instance_id = ?1
              AND candidate.status NOT IN ('completed', 'failed', 'timed_out', 'cancelled')
              AND (
                  -- relative deadline: created + timeout_seconds. `?2` is the
                  -- evaluation clock — `'now'` (wall clock) for dev/worker, or a
                  -- virtual timestamp a test injects via `given clock`.
                  (candidate.timeout_seconds IS NOT NULL
                   AND (strftime('%s', ?2) - strftime('%s', candidate.created_at)) >= candidate.timeout_seconds)
                  -- absolute deadline (timer until): input_json.deadline_at
                  -- (cast to integer: strftime returns text, and `>=` on text
                  -- compares lexicographically)
                  OR (json_extract(candidate.input_json, '$.deadline_at') IS NOT NULL
                      AND CAST(strftime('%s', ?2) AS INTEGER)
                          >= CAST(strftime('%s', json_extract(candidate.input_json, '$.deadline_at')) AS INTEGER))
              )
              AND (
                  candidate.kind != 'timer.wait'
                  OR NOT EXISTS (
                      SELECT 1
                      FROM effect_dependencies AS dependency
                      JOIN effects AS upstream
                        ON upstream.effect_id = dependency.upstream_effect_id
                       AND upstream.instance_id = dependency.instance_id
                      WHERE dependency.instance_id = candidate.instance_id
                        AND dependency.downstream_effect_id = candidate.effect_id
                        AND NOT (
                            (dependency.predicate = 'succeeds' AND upstream.status = 'completed')
                            OR (dependency.predicate = 'fails' AND upstream.status IN ('failed', 'timed_out'))
                            OR (dependency.predicate = 'timed_out' AND upstream.status = 'timed_out')
                            OR (dependency.predicate = 'cancelled' AND upstream.status = 'cancelled')
                            OR (dependency.predicate = 'completes' AND upstream.status IN ('completed', 'failed', 'timed_out', 'cancelled'))
                        )
                  )
              )
            ORDER BY candidate.created_at, candidate.effect_id
            "#,
        )?;
        let rows = statement
            .query_map([instance_id, now], |row| {
                Ok(DueTimeEffect {
                    effect_id: row.get(0)?,
                    kind: row.get(1)?,
                    status: row.get(2)?,
                    timeout_seconds: row.get(3)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Scheduled instants for an `every <interval_seconds>` clock source that are
    /// due at `now`, strictly after `after_scheduled` (the cursor — pass the source
    /// start when no occurrence has been admitted yet), in ascending ISO-8601
    /// order. Pure interval arithmetic via SQLite `strftime` (no timezone, no date
    /// library): the first occurrence is `cursor + interval`. `now` is the
    /// evaluation clock (`'now'` for dev/worker, or an injected virtual timestamp).
    /// Bounded to 100000 occurrences per pass as a runaway guard.
    pub fn due_interval_occurrences(
        &self,
        after_scheduled: &str,
        interval_seconds: i64,
        now: &str,
    ) -> StoreResult<Vec<String>> {
        if interval_seconds <= 0 {
            return Ok(Vec::new());
        }
        let mut statement = self.connection.prepare(
            r#"
            WITH RECURSIVE occurrence(scheduled_epoch) AS (
                SELECT CAST(strftime('%s', ?1) AS INTEGER) + ?2
                UNION ALL
                SELECT scheduled_epoch + ?2 FROM occurrence
                WHERE scheduled_epoch + ?2 <= CAST(strftime('%s', ?3) AS INTEGER)
                  AND scheduled_epoch < CAST(strftime('%s', ?1) AS INTEGER) + (?2 * 100000)
            )
            SELECT strftime('%Y-%m-%dT%H:%M:%SZ', scheduled_epoch, 'unixepoch')
            FROM occurrence
            WHERE scheduled_epoch <= CAST(strftime('%s', ?3) AS INTEGER)
            ORDER BY scheduled_epoch
            "#,
        )?;
        let rows = statement
            .query_map(params![after_scheduled, interval_seconds, now], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Resolve an evaluation-clock token (`'now'` or an injected virtual
    /// timestamp) to a concrete ISO-8601 instant, so an admitted occurrence records
    /// a fixed `observed_at` (replay reads the recorded value rather than the clock).
    pub fn resolve_clock(&self, now: &str) -> StoreResult<String> {
        let resolved: Option<String> = self
            .connection
            .query_row(
                "SELECT strftime('%Y-%m-%dT%H:%M:%SZ', ?1)",
                params![now],
                |row| row.get(0),
            )
            .optional()?;
        resolved.ok_or_else(|| StoreError::Conflict(format!("unparseable clock instant `{now}`")))
    }

    /// The latest `scheduled_at` already admitted for a clock signal, read from the
    /// append-only event log (NOT active facts — a consumed occurrence must not let
    /// the cursor regress and re-admit). `None` when no occurrence has been admitted
    /// yet, so the caller seeds the cursor from the source start.
    pub fn last_clock_occurrence(
        &self,
        instance_id: &str,
        signal: &str,
    ) -> StoreResult<Option<String>> {
        let latest: Option<String> = self
            .connection
            .query_row(
                "SELECT MAX(json_extract(payload_json, '$.scheduled_at')) \
                 FROM events WHERE instance_id = ?1 AND event_type = ?2",
                params![instance_id, signal],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(latest)
    }

    /// Pending time obligations (queued timers, unexpired deadlines) for
    /// status reporting.
    pub fn pending_time_effects(&self, instance_id: &str) -> StoreResult<Vec<DueTimeEffect>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT effect_id, kind, status, timeout_seconds
            FROM effects
            WHERE instance_id = ?1
              AND timeout_seconds IS NOT NULL
              AND status NOT IN ('completed', 'failed', 'timed_out', 'cancelled')
            ORDER BY created_at, effect_id
            "#,
        )?;
        let rows = statement
            .query_map([instance_id], |row| {
                Ok(DueTimeEffect {
                    effect_id: row.get(0)?,
                    kind: row.get(1)?,
                    status: row.get(2)?,
                    timeout_seconds: row.get(3)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Marks a never-run effect `timed_out` when its creation-anchored
    /// deadline expires. Running effects go through run-level timeout plus a
    /// cancellation request instead.
    pub fn expire_effect(
        &mut self,
        instance_id: &str,
        effect_id: &str,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        let payload = json!({
            "effect_id": effect_id,
            "status": "timed_out",
            "reason": "deadline exceeded",
        })
        .to_string();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id,
                event_type: "effect.terminal",
                payload_json: &payload,
                source: "kernel",
                causation_id: Some(effect_id),
                correlation_id: None,
                idempotency_key,
            },
        )?;
        let changed = tx.execute(
            r#"
            UPDATE effects
            SET status = 'timed_out',
                updated_at = CURRENT_TIMESTAMP
            WHERE instance_id = ?1
              AND effect_id = ?2
              AND status NOT IN ('completed', 'failed', 'timed_out', 'cancelled')
            "#,
            params![instance_id, effect_id],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict("effect cannot expire".to_owned()));
        }
        tx.commit()?;
        Ok(event)
    }

    /// Marks a projection fact consumed outside a rule commit (used when a
    /// projected work item stops being ready).
    pub fn retire_fact(&mut self, instance_id: &str, fact_id: &str) -> StoreResult<()> {
        self.connection.execute(
            r#"
            UPDATE facts
            SET consumed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
            WHERE instance_id = ?1 AND fact_id = ?2 AND consumed_at IS NULL
            "#,
            params![instance_id, fact_id],
        )?;
        Ok(())
    }

    pub fn cancel_effect(
        &mut self,
        cancellation: EffectCancellation<'_>,
    ) -> StoreResult<StoredEvent> {
        let payload = json!({
            "effect_id": cancellation.effect_id,
            "status": "cancelled",
            "reason": cancellation.reason,
        })
        .to_string();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: cancellation.instance_id,
                event_type: "effect.terminal",
                payload_json: &payload,
                source: "kernel",
                causation_id: Some(cancellation.effect_id),
                correlation_id: None,
                idempotency_key: cancellation.idempotency_key,
            },
        )?;
        let changed = tx.execute(
            r#"
            UPDATE effects
            SET status = 'cancelled',
                updated_at = CURRENT_TIMESTAMP
            WHERE instance_id = ?1
              AND effect_id = ?2
              AND status NOT IN ('completed', 'failed', 'timed_out', 'cancelled')
            "#,
            params![cancellation.instance_id, cancellation.effect_id],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict(
                "effect cannot be cancelled".to_owned(),
            ));
        }
        mark_cancellation_requests_terminal_on(
            &tx,
            cancellation.instance_id,
            cancellation.effect_id,
            &event.event_id,
        )?;
        satisfy_dependencies_on(&tx, cancellation.instance_id)?;
        tx.commit()?;
        Ok(event)
    }

    pub fn renew_lease(&mut self, renewal: LeaseRenewal<'_>) -> StoreResult<StoredEvent> {
        let payload = json!({
            "lease_id": renewal.lease_id,
            "run_id": renewal.run_id,
            "new_expires_at": renewal.new_expires_at,
        })
        .to_string();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: renewal.instance_id,
                event_type: "lease.renewed",
                payload_json: &payload,
                source: "kernel",
                causation_id: Some(renewal.run_id),
                correlation_id: None,
                idempotency_key: renewal.idempotency_key,
            },
        )?;
        let changed = tx.execute(
            r#"
            UPDATE leases
            SET expires_at = ?1
            WHERE instance_id = ?2
              AND lease_id = ?3
              AND run_id = ?4
              AND status = 'active'
            "#,
            params![
                renewal.new_expires_at,
                renewal.instance_id,
                renewal.lease_id,
                renewal.run_id,
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict("lease cannot be renewed".to_owned()));
        }
        tx.commit()?;
        Ok(event)
    }

    pub fn expire_leases(
        &mut self,
        instance_id: &str,
        now: &str,
    ) -> StoreResult<Vec<ExpiredLease>> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let expired = {
            let mut statement = tx.prepare(
                r#"
                SELECT lease_id, run_id, effect_id
                FROM leases
                WHERE instance_id = ?1
                  AND status = 'active'
                  AND expires_at <= ?2
                ORDER BY expires_at, lease_id
                "#,
            )?;
            let rows = statement
                .query_map(params![instance_id, now], |row| {
                    Ok(ExpiredLease {
                        lease_id: row.get(0)?,
                        run_id: row.get(1)?,
                        effect_id: row.get(2)?,
                    })
                })?
                .collect::<result::Result<Vec<_>, _>>()?;
            rows
        };

        for lease in &expired {
            let payload = json!({
                "lease_id": lease.lease_id,
                "run_id": lease.run_id,
                "effect_id": lease.effect_id,
                "expired_at": now,
            })
            .to_string();
            append_event_on(
                &tx,
                NewEvent {
                    instance_id,
                    event_type: "lease.expired",
                    payload_json: &payload,
                    source: "kernel",
                    causation_id: Some(&lease.run_id),
                    correlation_id: None,
                    idempotency_key: Some(&format!("lease-expired:{}", lease.lease_id)),
                },
            )?;
            tx.execute(
                r#"
                UPDATE leases
                SET status = 'expired',
                    released_at = CURRENT_TIMESTAMP
                WHERE lease_id = ?1
                "#,
                [&lease.lease_id],
            )?;
            tx.execute(
                r#"
                UPDATE runs
                SET status = 'lease_expired',
                    completed_at = CURRENT_TIMESTAMP
                WHERE run_id = ?1
                  AND status = 'running'
                "#,
                [&lease.run_id],
            )?;
            tx.execute(
                r#"
                UPDATE effects
                SET status = 'queued',
                    updated_at = CURRENT_TIMESTAMP
                WHERE instance_id = ?1
                  AND effect_id = ?2
                  AND status = 'running'
                "#,
                params![instance_id, lease.effect_id],
            )?;
        }

        tx.commit()?;
        Ok(expired)
    }

    pub fn retry_effect(&mut self, retry: RetryEffect<'_>) -> StoreResult<StoredEvent> {
        let payload = json!({
            "effect_id": retry.effect_id,
            "retry_after": retry.retry_after,
        })
        .to_string();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let event = append_event_on(
            &tx,
            NewEvent {
                instance_id: retry.instance_id,
                event_type: "effect.retried",
                payload_json: &payload,
                source: "kernel",
                causation_id: Some(retry.effect_id),
                correlation_id: None,
                idempotency_key: retry.idempotency_key,
            },
        )?;
        let changed = tx.execute(
            r#"
            UPDATE effects
            SET status = 'queued',
                updated_at = CURRENT_TIMESTAMP
            WHERE instance_id = ?1
              AND effect_id = ?2
              AND status IN ('failed', 'timed_out')
            "#,
            params![retry.instance_id, retry.effect_id],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict("effect is not retryable".to_owned()));
        }
        tx.commit()?;
        Ok(event)
    }

    pub fn rebuild_projections(&mut self, instance_id: &str) -> StoreResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let artifact_run_links = {
            let mut statement = tx.prepare(
                r#"
                SELECT artifact_id, run_id
                FROM artifacts
                WHERE run_id IN (
                    SELECT run_id FROM runs WHERE instance_id = ?1
                )
                ORDER BY artifact_id
                "#,
            )?;
            let rows = statement
                .query_map([instance_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<result::Result<Vec<_>, _>>()?;
            rows
        };
        for (artifact_id, _) in &artifact_run_links {
            tx.execute(
                "UPDATE artifacts SET run_id = NULL WHERE artifact_id = ?1",
                [artifact_id],
            )?;
        }
        tx.execute(
            "DELETE FROM effect_cancellation_requests WHERE instance_id = ?1",
            [instance_id],
        )?;
        tx.execute("DELETE FROM leases WHERE instance_id = ?1", [instance_id])?;
        tx.execute("DELETE FROM runs WHERE instance_id = ?1", [instance_id])?;
        tx.execute(
            "DELETE FROM instance_revisions WHERE instance_id = ?1",
            [instance_id],
        )?;
        tx.execute(
            "DELETE FROM effect_dependencies WHERE instance_id = ?1",
            [instance_id],
        )?;
        tx.execute("DELETE FROM effects WHERE instance_id = ?1", [instance_id])?;
        tx.execute("DELETE FROM facts WHERE instance_id = ?1", [instance_id])?;

        let events = {
            let mut statement = tx.prepare(
                r#"
                SELECT event_id, event_type, payload_json, idempotency_key, causation_id, source
                FROM events
                WHERE instance_id = ?1
                  AND event_type IN (
                      'rule.committed',
                      'fact.derived',
                      'workflow.completed',
                      'workflow.failed',
                      'instance.transitioned',
                      'workflow.revision_activated',
                      'effect.run_started',
                      'effect.terminal',
                      'effect.cancelled',
                      'effect.cancellation_requested',
                      'lease.expired'
                  )
                ORDER BY sequence
                "#,
            )?;
            let rows = statement
                .query_map([instance_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?
                .collect::<result::Result<Vec<_>, _>>()?;
            rows
        };

        for (event_id, event_type, payload_json, idempotency_key, causation_id, source) in events {
            match event_type.as_str() {
                "rule.committed" => replay_rule_commit(&tx, instance_id, &event_id, &payload_json)?,
                "fact.derived" => {
                    replay_fact_derived(&tx, instance_id, &event_id, &source, &payload_json)?
                }
                "workflow.completed" | "workflow.failed" => replay_workflow_terminal(
                    &tx,
                    instance_id,
                    &event_id,
                    &event_type,
                    &payload_json,
                )?,
                "instance.transitioned" => {
                    replay_instance_transition(&tx, instance_id, &event_id, &payload_json)?
                }
                "workflow.revision_activated" => replay_revision_activation(
                    &tx,
                    instance_id,
                    &event_id,
                    &payload_json,
                    idempotency_key.as_deref(),
                )?,
                "effect.run_started" => replay_run_started(&tx, instance_id, &payload_json)?,
                "effect.terminal" => {
                    replay_effect_terminal(&tx, instance_id, &event_id, &payload_json)?
                }
                "effect.cancelled" => {
                    replay_effect_cancelled(&tx, instance_id, &event_id, &payload_json)?
                }
                "effect.cancellation_requested" => replay_cancellation_request(
                    &tx,
                    instance_id,
                    &event_id,
                    &payload_json,
                    idempotency_key.as_deref(),
                    causation_id.as_deref(),
                )?,
                "lease.expired" => replay_lease_expired(&tx, instance_id, &payload_json)?,
                _ => {}
            }
        }

        for (artifact_id, run_id) in artifact_run_links {
            let run_exists = tx
                .query_row(
                    "SELECT 1 FROM runs WHERE run_id = ?1",
                    [&run_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if run_exists {
                tx.execute(
                    "UPDATE artifacts SET run_id = ?1 WHERE artifact_id = ?2",
                    params![run_id, artifact_id],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn table_exists(&self, table: &str) -> StoreResult<bool> {
        self.connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(Into::into)
    }
}

/// The durable runtime store as a backend-agnostic, object-safe trait — the
/// sans-IO store seam (DR-0033 Phase 3). A durable-object SQLite backend
/// implements the same event / fact / effect / instance / registry operations
/// the native rusqlite `SqliteStore` provides, so the DO host can back the
/// runtime without the language changing. The native impl forwards to the
/// inherent methods (delegation via inherent-method precedence, guarded by
/// `unconditional_recursion`).
///
/// Excluded from the seam (kept inherent-only): constructors (`open`/
/// `open_in_memory`) and native-filesystem convenience methods that take an
/// `impl AsRef<Path>` and read the local FS — the DO host does not load from a
/// directory. Excluded: `load_package_manifests_from_dir`.
#[allow(clippy::too_many_arguments)]
pub trait RuntimeStore {
    fn schema_version(&self) -> StoreResult<i64>;
    fn append_event(&self, event: NewEvent<'_>) -> StoreResult<StoredEvent>;
    fn create_program_version(
        &mut self,
        version: NewProgramVersion<'_>,
    ) -> StoreResult<ProgramVersionRecord>;
    fn get_program_version(&self, version_id: &str) -> StoreResult<Option<ProgramVersionView>>;
    fn create_instance(&self, instance: NewInstance<'_>) -> StoreResult<InstanceRecord>;
    fn create_instance_with_authority(
        &self,
        instance: NewInstance<'_>,
        authority: NewInstanceAuthority<'_>,
    ) -> StoreResult<InstanceRecord>;
    fn list_instance_revisions(&self, instance_id: &str) -> StoreResult<Vec<WorkflowRevisionView>>;
    fn revision_cancellation_impact(
        &self,
        instance_id: &str,
        cancellation_policy: &str,
    ) -> StoreResult<RevisionCancellationImpact>;
    fn analyze_revision_compatibility(
        &self,
        instance_id: &str,
        candidate_version_id: &str,
    ) -> StoreResult<RevisionCompatibilityReport>;
    fn analyze_revision_candidate(
        &self,
        instance_id: &str,
        candidate: RevisionCandidate<'_>,
    ) -> StoreResult<RevisionCompatibilityReport>;
    fn activate_revision(
        &mut self,
        activation: RevisionActivation<'_>,
    ) -> StoreResult<WorkflowRevisionView>;
    fn request_effect_cancellation(
        &mut self,
        request: EffectCancellationRequest<'_>,
    ) -> StoreResult<EffectCancellationRequestView>;
    fn effect_has_open_cancellation_request(
        &self,
        instance_id: &str,
        effect_id: &str,
    ) -> StoreResult<bool>;
    fn list_effect_cancellation_requests(
        &self,
        instance_id: &str,
    ) -> StoreResult<Vec<EffectCancellationRequestView>>;
    fn record_workflow_invocation(&self, invocation: NewWorkflowInvocation<'_>) -> StoreResult<()>;
    fn get_workflow_invocation(
        &self,
        parent_instance_id: &str,
        parent_effect_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>>;
    fn list_child_workflow_invocations(
        &self,
        parent_instance_id: &str,
    ) -> StoreResult<Vec<WorkflowInvocationView>>;
    fn get_parent_workflow_invocation(
        &self,
        child_instance_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>>;
    fn commit_rule(&mut self, commit: RuleCommit<'_>) -> StoreResult<StoredEvent>;
    fn commit_rule_with_revision_guard(
        &mut self,
        commit: RuleCommit<'_>,
        guard: RuleCommitRevisionGuard<'_>,
    ) -> StoreResult<StoredEvent>;
    fn derive_fact(&mut self, derived: DerivedFact<'_>) -> StoreResult<StoredEvent>;
    fn admit_fact_batch(&mut self, batch: FactBatch<'_>) -> StoreResult<FactBatchOutcome>;
    fn complete_effect(&mut self, completion: EffectCompletion<'_>) -> StoreResult<StoredEvent>;
    fn complete_effect_with_terminal_diagnostic(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent>;
    fn resolve_effect_uncertain(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent>;
    fn claimable_effects(&self, instance_id: &str) -> StoreResult<Vec<ClaimableEffect>>;
    fn fact_exists(&self, instance_id: &str, fact_name: &str) -> StoreResult<bool>;
    fn register_package(&self, package: PackageRegistration<'_>) -> StoreResult<()>;
    fn register_package_manifest(&self, manifest_json: &str) -> StoreResult<String>;
    fn register_capability_schema(
        &self,
        capability: CapabilitySchemaRegistration<'_>,
    ) -> StoreResult<()>;
    fn register_effect_provider(&self, provider: EffectProviderRegistration<'_>)
        -> StoreResult<()>;
    fn register_profile(&self, profile: ProfileRegistration<'_>) -> StoreResult<()>;
    fn registered_profile_policy(
        &self,
        profile: &str,
    ) -> StoreResult<Option<RegisteredProfilePolicy>>;
    fn bind_capability(&self, binding: CapabilityBinding<'_>) -> StoreResult<()>;
    fn register_skill(&self, skill: SkillRegistration<'_>) -> StoreResult<()>;
    fn register_project_context_doc(
        &self,
        position: i64,
        path: &str,
        body: &str,
    ) -> StoreResult<()>;
    fn list_project_context_docs(&self) -> StoreResult<Vec<ProjectContextDoc>>;
    fn record_compute_result(
        &self,
        registration: ComputeResultRegistration<'_>,
    ) -> StoreResult<bool>;
    fn lookup_compute_result(&self, content_key: &str) -> StoreResult<Option<ComputeCachedResult>>;
    fn attach_skill(&self, attachment: SkillAttachment<'_>) -> StoreResult<()>;
    fn list_skills(&self) -> StoreResult<Vec<SkillView>>;
    fn list_skill_attachments(
        &self,
        scope_type: &str,
        scope_id: &str,
    ) -> StoreResult<Vec<SkillAttachmentView>>;
    fn record_evidence(&self, evidence: EvidenceRecord<'_>) -> StoreResult<String>;
    fn record_provider_validation_evidence(
        &self,
        evidence: ProviderValidationEvidence<'_>,
    ) -> StoreResult<String>;
    fn record_codex_app_server_evidence(
        &self,
        evidence: CodexAppServerEvidence<'_>,
    ) -> StoreResult<String>;
    fn record_claude_agent_sdk_evidence(
        &self,
        evidence: ClaudeAgentSdkEvidence<'_>,
    ) -> StoreResult<String>;
    fn record_pi_rpc_evidence(&self, evidence: PiRpcEvidence<'_>) -> StoreResult<String>;
    fn link_evidence(&self, link: EvidenceLink<'_>) -> StoreResult<()>;
    fn record_artifact(&self, artifact: ArtifactRecord<'_>) -> StoreResult<String>;
    fn list_artifacts_for_run(&self, run_id: &str) -> StoreResult<Vec<ArtifactView>>;
    fn record_workspace(&self, workspace: WorkspaceRecord<'_>) -> StoreResult<String>;
    fn get_workspace(&self, workspace_id: &str) -> StoreResult<Option<WorkspaceView>>;
    fn list_workspaces_for_instance(&self, instance_id: &str) -> StoreResult<Vec<WorkspaceView>>;
    fn record_diagnostic(&self, diagnostic: DiagnosticRecord<'_>) -> StoreResult<String>;
    fn list_diagnostics(&self, instance_id: Option<&str>) -> StoreResult<Vec<DiagnosticView>>;
    fn list_diagnostics_from_events(&self, instance_id: &str) -> StoreResult<Vec<DiagnosticView>>;
    fn effect_source_span_json(
        &self,
        instance_id: &str,
        effect_id: &str,
    ) -> StoreResult<Option<String>>;
    fn create_inbox_item(&self, item: NewInboxItem<'_>) -> StoreResult<()>;
    fn list_inbox_items(&self, status: Option<&str>) -> StoreResult<Vec<InboxItemView>>;
    fn get_inbox_item(&self, inbox_item_id: &str) -> StoreResult<Option<InboxItemView>>;
    fn answer_inbox_item(&mut self, answer: HumanAnswer<'_>) -> StoreResult<StoredEvent>;
    fn cancel_pending_inbox_for_instance(&mut self, instance_id: &str) -> StoreResult<usize>;
    fn record_skill_evidence(&self, evidence: SkillEvidence<'_>) -> StoreResult<String>;
    fn list_evidence(&self, instance_id: &str) -> StoreResult<Vec<EvidenceView>>;
    fn list_evidence_for_subject(
        &self,
        subject_type: &str,
        subject_id: &str,
    ) -> StoreResult<Vec<EvidenceView>>;
    fn list_evidence_links(&self, instance_id: &str) -> StoreResult<Vec<EvidenceLinkView>>;
    fn list_instances(&self) -> StoreResult<Vec<InstanceView>>;
    fn get_instance(&self, instance_id: &str) -> StoreResult<Option<InstanceView>>;
    fn list_events(&self, instance_id: &str) -> StoreResult<Vec<EventView>>;
    fn list_facts(&self, instance_id: &str) -> StoreResult<Vec<FactView>>;
    fn list_facts_including_consumed(&self, instance_id: &str) -> StoreResult<Vec<FactView>>;
    fn list_effects(&self, instance_id: &str) -> StoreResult<Vec<EffectView>>;
    fn list_runs(&self, instance_id: &str) -> StoreResult<Vec<RunView>>;
    fn status(&self, instance_id: &str) -> StoreResult<Option<StatusView>>;
    fn satisfy_dependencies(&self, instance_id: &str) -> StoreResult<usize>;
    fn start_run(&mut self, run: RunStart<'_>) -> StoreResult<StoredEvent>;
    fn block_effect_binding(
        &mut self,
        instance_id: &str,
        effect_id: &str,
        category: &str,
        detail: &str,
    ) -> StoreResult<StoredEvent>;
    fn transition_instance(
        &mut self,
        transition: InstanceTransition<'_>,
    ) -> StoreResult<StoredEvent>;
    fn due_time_effects(&self, instance_id: &str, now: &str) -> StoreResult<Vec<DueTimeEffect>>;
    fn due_interval_occurrences(
        &self,
        after_scheduled: &str,
        interval_seconds: i64,
        now: &str,
    ) -> StoreResult<Vec<String>>;
    fn resolve_clock(&self, now: &str) -> StoreResult<String>;
    fn last_clock_occurrence(&self, instance_id: &str, signal: &str)
        -> StoreResult<Option<String>>;
    fn pending_time_effects(&self, instance_id: &str) -> StoreResult<Vec<DueTimeEffect>>;
    fn expire_effect(
        &mut self,
        instance_id: &str,
        effect_id: &str,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent>;
    fn retire_fact(&mut self, instance_id: &str, fact_id: &str) -> StoreResult<()>;
    fn cancel_effect(&mut self, cancellation: EffectCancellation<'_>) -> StoreResult<StoredEvent>;
    fn renew_lease(&mut self, renewal: LeaseRenewal<'_>) -> StoreResult<StoredEvent>;
    fn expire_leases(&mut self, instance_id: &str, now: &str) -> StoreResult<Vec<ExpiredLease>>;
    fn retry_effect(&mut self, retry: RetryEffect<'_>) -> StoreResult<StoredEvent>;
    fn rebuild_projections(&mut self, instance_id: &str) -> StoreResult<()>;
    fn table_exists(&self, table: &str) -> StoreResult<bool>;
}

#[allow(clippy::too_many_arguments)]
#[cfg(feature = "native")]
impl RuntimeStore for SqliteStore {
    fn schema_version(&self) -> StoreResult<i64> {
        self.schema_version()
    }
    fn append_event(&self, event: NewEvent<'_>) -> StoreResult<StoredEvent> {
        self.append_event(event)
    }
    fn create_program_version(
        &mut self,
        version: NewProgramVersion<'_>,
    ) -> StoreResult<ProgramVersionRecord> {
        self.create_program_version(version)
    }
    fn get_program_version(&self, version_id: &str) -> StoreResult<Option<ProgramVersionView>> {
        self.get_program_version(version_id)
    }
    fn create_instance(&self, instance: NewInstance<'_>) -> StoreResult<InstanceRecord> {
        self.create_instance(instance)
    }
    fn create_instance_with_authority(
        &self,
        instance: NewInstance<'_>,
        authority: NewInstanceAuthority<'_>,
    ) -> StoreResult<InstanceRecord> {
        self.create_instance_with_authority(instance, authority)
    }
    fn list_instance_revisions(&self, instance_id: &str) -> StoreResult<Vec<WorkflowRevisionView>> {
        self.list_instance_revisions(instance_id)
    }
    fn revision_cancellation_impact(
        &self,
        instance_id: &str,
        cancellation_policy: &str,
    ) -> StoreResult<RevisionCancellationImpact> {
        self.revision_cancellation_impact(instance_id, cancellation_policy)
    }
    fn analyze_revision_compatibility(
        &self,
        instance_id: &str,
        candidate_version_id: &str,
    ) -> StoreResult<RevisionCompatibilityReport> {
        self.analyze_revision_compatibility(instance_id, candidate_version_id)
    }
    fn analyze_revision_candidate(
        &self,
        instance_id: &str,
        candidate: RevisionCandidate<'_>,
    ) -> StoreResult<RevisionCompatibilityReport> {
        self.analyze_revision_candidate(instance_id, candidate)
    }
    fn activate_revision(
        &mut self,
        activation: RevisionActivation<'_>,
    ) -> StoreResult<WorkflowRevisionView> {
        self.activate_revision(activation)
    }
    fn request_effect_cancellation(
        &mut self,
        request: EffectCancellationRequest<'_>,
    ) -> StoreResult<EffectCancellationRequestView> {
        self.request_effect_cancellation(request)
    }
    fn effect_has_open_cancellation_request(
        &self,
        instance_id: &str,
        effect_id: &str,
    ) -> StoreResult<bool> {
        self.effect_has_open_cancellation_request(instance_id, effect_id)
    }
    fn list_effect_cancellation_requests(
        &self,
        instance_id: &str,
    ) -> StoreResult<Vec<EffectCancellationRequestView>> {
        self.list_effect_cancellation_requests(instance_id)
    }
    fn record_workflow_invocation(&self, invocation: NewWorkflowInvocation<'_>) -> StoreResult<()> {
        self.record_workflow_invocation(invocation)
    }
    fn get_workflow_invocation(
        &self,
        parent_instance_id: &str,
        parent_effect_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>> {
        self.get_workflow_invocation(parent_instance_id, parent_effect_id)
    }
    fn list_child_workflow_invocations(
        &self,
        parent_instance_id: &str,
    ) -> StoreResult<Vec<WorkflowInvocationView>> {
        self.list_child_workflow_invocations(parent_instance_id)
    }
    fn get_parent_workflow_invocation(
        &self,
        child_instance_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>> {
        self.get_parent_workflow_invocation(child_instance_id)
    }
    fn commit_rule(&mut self, commit: RuleCommit<'_>) -> StoreResult<StoredEvent> {
        self.commit_rule(commit)
    }
    fn commit_rule_with_revision_guard(
        &mut self,
        commit: RuleCommit<'_>,
        guard: RuleCommitRevisionGuard<'_>,
    ) -> StoreResult<StoredEvent> {
        self.commit_rule_with_revision_guard(commit, guard)
    }
    fn derive_fact(&mut self, derived: DerivedFact<'_>) -> StoreResult<StoredEvent> {
        self.derive_fact(derived)
    }
    fn admit_fact_batch(&mut self, batch: FactBatch<'_>) -> StoreResult<FactBatchOutcome> {
        self.admit_fact_batch(batch)
    }
    fn complete_effect(&mut self, completion: EffectCompletion<'_>) -> StoreResult<StoredEvent> {
        self.complete_effect(completion)
    }
    fn complete_effect_with_terminal_diagnostic(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent> {
        self.complete_effect_with_terminal_diagnostic(completion, diagnostic)
    }
    fn resolve_effect_uncertain(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent> {
        self.resolve_effect_uncertain(completion, diagnostic)
    }
    fn claimable_effects(&self, instance_id: &str) -> StoreResult<Vec<ClaimableEffect>> {
        self.claimable_effects(instance_id)
    }
    fn fact_exists(&self, instance_id: &str, fact_name: &str) -> StoreResult<bool> {
        self.fact_exists(instance_id, fact_name)
    }
    fn register_package(&self, package: PackageRegistration<'_>) -> StoreResult<()> {
        self.register_package(package)
    }
    fn register_package_manifest(&self, manifest_json: &str) -> StoreResult<String> {
        self.register_package_manifest(manifest_json)
    }
    fn register_capability_schema(
        &self,
        capability: CapabilitySchemaRegistration<'_>,
    ) -> StoreResult<()> {
        self.register_capability_schema(capability)
    }
    fn register_effect_provider(
        &self,
        provider: EffectProviderRegistration<'_>,
    ) -> StoreResult<()> {
        self.register_effect_provider(provider)
    }
    fn register_profile(&self, profile: ProfileRegistration<'_>) -> StoreResult<()> {
        self.register_profile(profile)
    }
    fn registered_profile_policy(
        &self,
        profile: &str,
    ) -> StoreResult<Option<RegisteredProfilePolicy>> {
        self.registered_profile_policy(profile)
    }
    fn bind_capability(&self, binding: CapabilityBinding<'_>) -> StoreResult<()> {
        self.bind_capability(binding)
    }
    fn register_project_context_doc(
        &self,
        position: i64,
        path: &str,
        body: &str,
    ) -> StoreResult<()> {
        SqliteStore::register_project_context_doc(self, position, path, body)
    }

    fn list_project_context_docs(&self) -> StoreResult<Vec<ProjectContextDoc>> {
        SqliteStore::list_project_context_docs(self)
    }

    fn record_compute_result(
        &self,
        registration: ComputeResultRegistration<'_>,
    ) -> StoreResult<bool> {
        SqliteStore::record_compute_result(self, registration)
    }

    fn lookup_compute_result(&self, content_key: &str) -> StoreResult<Option<ComputeCachedResult>> {
        SqliteStore::lookup_compute_result(self, content_key)
    }

    fn register_skill(&self, skill: SkillRegistration<'_>) -> StoreResult<()> {
        self.register_skill(skill)
    }
    fn attach_skill(&self, attachment: SkillAttachment<'_>) -> StoreResult<()> {
        self.attach_skill(attachment)
    }
    fn list_skills(&self) -> StoreResult<Vec<SkillView>> {
        self.list_skills()
    }
    fn list_skill_attachments(
        &self,
        scope_type: &str,
        scope_id: &str,
    ) -> StoreResult<Vec<SkillAttachmentView>> {
        self.list_skill_attachments(scope_type, scope_id)
    }
    fn record_evidence(&self, evidence: EvidenceRecord<'_>) -> StoreResult<String> {
        self.record_evidence(evidence)
    }
    fn record_provider_validation_evidence(
        &self,
        evidence: ProviderValidationEvidence<'_>,
    ) -> StoreResult<String> {
        self.record_provider_validation_evidence(evidence)
    }
    fn record_codex_app_server_evidence(
        &self,
        evidence: CodexAppServerEvidence<'_>,
    ) -> StoreResult<String> {
        self.record_codex_app_server_evidence(evidence)
    }
    fn record_claude_agent_sdk_evidence(
        &self,
        evidence: ClaudeAgentSdkEvidence<'_>,
    ) -> StoreResult<String> {
        self.record_claude_agent_sdk_evidence(evidence)
    }
    fn record_pi_rpc_evidence(&self, evidence: PiRpcEvidence<'_>) -> StoreResult<String> {
        self.record_pi_rpc_evidence(evidence)
    }
    fn link_evidence(&self, link: EvidenceLink<'_>) -> StoreResult<()> {
        self.link_evidence(link)
    }
    fn record_artifact(&self, artifact: ArtifactRecord<'_>) -> StoreResult<String> {
        self.record_artifact(artifact)
    }
    fn list_artifacts_for_run(&self, run_id: &str) -> StoreResult<Vec<ArtifactView>> {
        self.list_artifacts_for_run(run_id)
    }
    fn record_workspace(&self, workspace: WorkspaceRecord<'_>) -> StoreResult<String> {
        self.record_workspace(workspace)
    }
    fn get_workspace(&self, workspace_id: &str) -> StoreResult<Option<WorkspaceView>> {
        self.get_workspace(workspace_id)
    }
    fn list_workspaces_for_instance(&self, instance_id: &str) -> StoreResult<Vec<WorkspaceView>> {
        self.list_workspaces_for_instance(instance_id)
    }
    fn record_diagnostic(&self, diagnostic: DiagnosticRecord<'_>) -> StoreResult<String> {
        self.record_diagnostic(diagnostic)
    }
    fn list_diagnostics(&self, instance_id: Option<&str>) -> StoreResult<Vec<DiagnosticView>> {
        self.list_diagnostics(instance_id)
    }
    fn list_diagnostics_from_events(&self, instance_id: &str) -> StoreResult<Vec<DiagnosticView>> {
        self.list_diagnostics_from_events(instance_id)
    }
    fn effect_source_span_json(
        &self,
        instance_id: &str,
        effect_id: &str,
    ) -> StoreResult<Option<String>> {
        self.effect_source_span_json(instance_id, effect_id)
    }
    fn create_inbox_item(&self, item: NewInboxItem<'_>) -> StoreResult<()> {
        self.create_inbox_item(item)
    }
    fn list_inbox_items(&self, status: Option<&str>) -> StoreResult<Vec<InboxItemView>> {
        self.list_inbox_items(status)
    }
    fn get_inbox_item(&self, inbox_item_id: &str) -> StoreResult<Option<InboxItemView>> {
        self.get_inbox_item(inbox_item_id)
    }
    fn answer_inbox_item(&mut self, answer: HumanAnswer<'_>) -> StoreResult<StoredEvent> {
        self.answer_inbox_item(answer)
    }
    fn cancel_pending_inbox_for_instance(&mut self, instance_id: &str) -> StoreResult<usize> {
        self.cancel_pending_inbox_for_instance(instance_id)
    }
    fn record_skill_evidence(&self, evidence: SkillEvidence<'_>) -> StoreResult<String> {
        self.record_skill_evidence(evidence)
    }
    fn list_evidence(&self, instance_id: &str) -> StoreResult<Vec<EvidenceView>> {
        self.list_evidence(instance_id)
    }
    fn list_evidence_for_subject(
        &self,
        subject_type: &str,
        subject_id: &str,
    ) -> StoreResult<Vec<EvidenceView>> {
        self.list_evidence_for_subject(subject_type, subject_id)
    }
    fn list_evidence_links(&self, instance_id: &str) -> StoreResult<Vec<EvidenceLinkView>> {
        self.list_evidence_links(instance_id)
    }
    fn list_instances(&self) -> StoreResult<Vec<InstanceView>> {
        self.list_instances()
    }
    fn get_instance(&self, instance_id: &str) -> StoreResult<Option<InstanceView>> {
        self.get_instance(instance_id)
    }
    fn list_events(&self, instance_id: &str) -> StoreResult<Vec<EventView>> {
        self.list_events(instance_id)
    }
    fn list_facts(&self, instance_id: &str) -> StoreResult<Vec<FactView>> {
        self.list_facts(instance_id)
    }
    fn list_facts_including_consumed(&self, instance_id: &str) -> StoreResult<Vec<FactView>> {
        self.list_facts_including_consumed(instance_id)
    }
    fn list_effects(&self, instance_id: &str) -> StoreResult<Vec<EffectView>> {
        self.list_effects(instance_id)
    }
    fn list_runs(&self, instance_id: &str) -> StoreResult<Vec<RunView>> {
        self.list_runs(instance_id)
    }
    fn status(&self, instance_id: &str) -> StoreResult<Option<StatusView>> {
        self.status(instance_id)
    }
    fn satisfy_dependencies(&self, instance_id: &str) -> StoreResult<usize> {
        self.satisfy_dependencies(instance_id)
    }
    fn start_run(&mut self, run: RunStart<'_>) -> StoreResult<StoredEvent> {
        self.start_run(run)
    }
    fn block_effect_binding(
        &mut self,
        instance_id: &str,
        effect_id: &str,
        category: &str,
        detail: &str,
    ) -> StoreResult<StoredEvent> {
        self.block_effect_binding(instance_id, effect_id, category, detail)
    }
    fn transition_instance(
        &mut self,
        transition: InstanceTransition<'_>,
    ) -> StoreResult<StoredEvent> {
        self.transition_instance(transition)
    }
    fn due_time_effects(&self, instance_id: &str, now: &str) -> StoreResult<Vec<DueTimeEffect>> {
        self.due_time_effects(instance_id, now)
    }
    fn due_interval_occurrences(
        &self,
        after_scheduled: &str,
        interval_seconds: i64,
        now: &str,
    ) -> StoreResult<Vec<String>> {
        self.due_interval_occurrences(after_scheduled, interval_seconds, now)
    }
    fn resolve_clock(&self, now: &str) -> StoreResult<String> {
        self.resolve_clock(now)
    }
    fn last_clock_occurrence(
        &self,
        instance_id: &str,
        signal: &str,
    ) -> StoreResult<Option<String>> {
        self.last_clock_occurrence(instance_id, signal)
    }
    fn pending_time_effects(&self, instance_id: &str) -> StoreResult<Vec<DueTimeEffect>> {
        self.pending_time_effects(instance_id)
    }
    fn expire_effect(
        &mut self,
        instance_id: &str,
        effect_id: &str,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        self.expire_effect(instance_id, effect_id, idempotency_key)
    }
    fn retire_fact(&mut self, instance_id: &str, fact_id: &str) -> StoreResult<()> {
        self.retire_fact(instance_id, fact_id)
    }
    fn cancel_effect(&mut self, cancellation: EffectCancellation<'_>) -> StoreResult<StoredEvent> {
        self.cancel_effect(cancellation)
    }
    fn renew_lease(&mut self, renewal: LeaseRenewal<'_>) -> StoreResult<StoredEvent> {
        self.renew_lease(renewal)
    }
    fn expire_leases(&mut self, instance_id: &str, now: &str) -> StoreResult<Vec<ExpiredLease>> {
        self.expire_leases(instance_id, now)
    }
    fn retry_effect(&mut self, retry: RetryEffect<'_>) -> StoreResult<StoredEvent> {
        self.retry_effect(retry)
    }
    fn rebuild_projections(&mut self, instance_id: &str) -> StoreResult<()> {
        self.rebuild_projections(instance_id)
    }
    fn table_exists(&self, table: &str) -> StoreResult<bool> {
        self.table_exists(table)
    }
}

#[cfg(feature = "native")]
#[cfg(unix)]
fn harden_store_file_permissions(path: &Path) -> StoreResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    if permissions.mode() & 0o777 != 0o600 {
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(feature = "native")]
#[cfg(not(unix))]
fn harden_store_file_permissions(_path: &Path) -> StoreResult<()> {
    Ok(())
}

#[cfg(feature = "native")]
fn table_exists(connection: &Connection, table: &str) -> StoreResult<bool> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn validate_workspace_policy(policy: &str) -> StoreResult<()> {
    match policy {
        "shared"
        | "read_only"
        | "per_effect_worktree"
        | "per_issue_worktree"
        | "remote_sandbox" => Ok(()),
        _ => Err(StoreError::Conflict(format!(
            "unsupported workspace policy `{policy}`"
        ))),
    }
}

#[cfg(feature = "native")]
fn validate_workspace_status(status: &str) -> StoreResult<()> {
    match status {
        "prepared" | "active" | "released" | "failed" => Ok(()),
        _ => Err(StoreError::Conflict(format!(
            "unsupported workspace status `{status}`"
        ))),
    }
}

#[cfg(feature = "native")]
fn workspace_select_sql(predicate: &str) -> String {
    format!(
        r#"
        SELECT
            workspace_id,
            instance_id,
            effect_id,
            run_id,
            provider,
            policy,
            uri,
            status,
            metadata_json,
            created_at,
            updated_at
        FROM workspaces
        {predicate}
        "#
    )
}

#[cfg(feature = "native")]
fn workspace_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceView> {
    Ok(WorkspaceView {
        workspace_id: row.get(0)?,
        instance_id: row.get(1)?,
        effect_id: row.get(2)?,
        run_id: row.get(3)?,
        provider: row.get(4)?,
        policy: row.get(5)?,
        uri: row.get(6)?,
        status: row.get(7)?,
        metadata_json: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

#[cfg(feature = "native")]
fn append_event_on(connection: &Connection, event: NewEvent<'_>) -> StoreResult<StoredEvent> {
    connection
        .query_row(
            r#"
            INSERT INTO events (
                event_id,
                instance_id,
                sequence,
                event_type,
                payload_json,
                occurred_at,
                source,
                causation_id,
                correlation_id,
                idempotency_key
            )
            VALUES (
                'evt_' || lower(hex(randomblob(16))),
                ?1,
                (SELECT COALESCE(MAX(sequence), 0) + 1 FROM events WHERE instance_id = ?1),
                ?2,
                ?3,
                CURRENT_TIMESTAMP,
                ?4,
                ?5,
                ?6,
                ?7
            )
            RETURNING event_id, sequence
            "#,
            params![
                event.instance_id,
                event.event_type,
                event.payload_json,
                event.source,
                event.causation_id,
                event.correlation_id,
                event.idempotency_key,
            ],
            |row| {
                Ok(StoredEvent {
                    event_id: row.get(0)?,
                    sequence: row.get(1)?,
                })
            },
        )
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn insert_fact(
    connection: &Connection,
    instance_id: &str,
    rule: &str,
    event_id: &str,
    program_version_id: Option<&str>,
    revision_epoch: i64,
    fact: &NewFact<'_>,
) -> StoreResult<()> {
    if let Some(source_span_json) = fact.source_span_json {
        serde_json::from_str::<Value>(source_span_json)?;
    }
    connection.execute(
        r#"
        INSERT INTO facts (
            fact_id,
            instance_id,
            program_version_id,
            revision_epoch,
            name,
            key,
            value_json,
            source_event_id,
            source_rule,
            schema_id,
            provenance_class,
            correlation_id,
            source_span_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            fact.fact_id,
            instance_id,
            program_version_id,
            revision_epoch,
            fact.name,
            fact.key,
            fact.value_json,
            event_id,
            rule,
            fact.schema_id,
            fact.provenance_class,
            fact.correlation_id,
            fact.source_span_json,
        ],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn consume_facts(connection: &Connection, instance_id: &str, fact_ids: &[&str]) -> StoreResult<()> {
    for fact_id in fact_ids {
        let changed = connection.execute(
            r#"
            UPDATE facts
            SET consumed_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP
            WHERE instance_id = ?1
              AND fact_id = ?2
              AND consumed_at IS NULL
            "#,
            params![instance_id, fact_id],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict(format!(
                "fact `{fact_id}` is not active and cannot be consumed"
            )));
        }
    }
    Ok(())
}

#[cfg(feature = "native")]
fn insert_effect(
    connection: &Connection,
    instance_id: &str,
    rule: &str,
    event_id: &str,
    program_version_id: Option<&str>,
    revision_epoch: i64,
    effect: &NewEffect<'_>,
) -> StoreResult<()> {
    connection.execute(
        r#"
        INSERT INTO effects (
            effect_id,
            instance_id,
            kind,
            target,
            input_json,
            status,
            created_by_rule,
            created_by_event_id,
            program_version_id,
            revision_epoch,
            correlation_id,
            idempotency_key,
            required_capabilities,
            profile,
            timeout_seconds
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
        "#,
        params![
            effect.effect_id,
            instance_id,
            effect.kind,
            effect.target,
            effect.input_json,
            effect.status,
            rule,
            event_id,
            program_version_id,
            revision_epoch,
            effect.correlation_id,
            effect.idempotency_key,
            effect.required_capabilities_json,
            effect.profile,
            effect.timeout_seconds,
        ],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn insert_effect_dependency(
    connection: &Connection,
    instance_id: &str,
    rule: &str,
    dependency: &NewEffectDependency<'_>,
) -> StoreResult<()> {
    connection.execute(
        r#"
        INSERT INTO effect_dependencies (
            dependency_id,
            instance_id,
            upstream_effect_id,
            downstream_effect_id,
            predicate,
            created_by_rule
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            dependency.dependency_id,
            instance_id,
            dependency.upstream_effect_id,
            dependency.downstream_effect_id,
            dependency.predicate,
            rule,
        ],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn insert_evidence_on(
    connection: &Connection,
    evidence: EvidenceRecord<'_>,
) -> StoreResult<String> {
    serde_json::from_str::<Value>(evidence.metadata_json)?;
    let evidence_id = connection.query_row(
        r#"
        INSERT INTO evidence (
            evidence_id,
            instance_id,
            kind,
            subject_type,
            subject_id,
            causation_id,
            correlation_id,
            summary,
            metadata_json
        )
        VALUES (
            'evd_' || lower(hex(randomblob(16))),
            ?1,
            ?2,
            ?3,
            ?4,
            ?5,
            ?6,
            ?7,
            ?8
        )
        RETURNING evidence_id
        "#,
        params![
            evidence.instance_id,
            evidence.kind,
            evidence.subject_type,
            evidence.subject_id,
            evidence.causation_id,
            evidence.correlation_id,
            evidence.summary,
            evidence.metadata_json,
        ],
        |row| row.get::<_, String>(0),
    )?;
    insert_evidence_link_on(
        connection,
        EvidenceLink {
            evidence_id: &evidence_id,
            instance_id: evidence.instance_id,
            target_type: evidence.subject_type,
            target_id: evidence.subject_id,
            relation: "subject",
        },
    )?;
    if let Some(causation_id) = evidence.causation_id {
        insert_evidence_link_on(
            connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "causation",
                target_id: causation_id,
                relation: "caused_by",
            },
        )?;
    }
    if let Some(correlation_id) = evidence.correlation_id {
        insert_evidence_link_on(
            connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: evidence.instance_id,
                target_type: "correlation",
                target_id: correlation_id,
                relation: "correlates_with",
            },
        )?;
    }
    Ok(evidence_id)
}

#[cfg(feature = "native")]
fn insert_evidence_link_on(connection: &Connection, link: EvidenceLink<'_>) -> StoreResult<()> {
    connection.execute(
        r#"
        INSERT INTO evidence_links (
            link_id,
            evidence_id,
            instance_id,
            target_type,
            target_id,
            relation
        )
        VALUES (
            'evl_' || lower(hex(randomblob(16))),
            ?1,
            ?2,
            ?3,
            ?4,
            ?5
        )
        ON CONFLICT(evidence_id, target_type, target_id, relation) DO NOTHING
        "#,
        params![
            link.evidence_id,
            link.instance_id,
            link.target_type,
            link.target_id,
            link.relation,
        ],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn insert_diagnostic_on(
    connection: &Connection,
    diagnostic: DiagnosticRecord<'_>,
) -> StoreResult<String> {
    if let Some(source_span_json) = diagnostic.source_span_json {
        serde_json::from_str::<Value>(source_span_json)?;
    }
    parse_json_array(diagnostic.evidence_ids_json)?;
    parse_json_array(diagnostic.artifact_ids_json)?;

    if let Some(existing_id) = existing_diagnostic_id_on(connection, &diagnostic)? {
        return Ok(existing_id);
    }

    connection
        .query_row(
            r#"
            INSERT INTO diagnostics (
                diagnostic_id,
                instance_id,
                program_id,
                program_version_id,
                severity,
                code,
                message,
                source_span_json,
                subject_type,
                subject_id,
                event_id,
                effect_id,
                run_id,
                assertion_id,
                evidence_ids_json,
                artifact_ids_json,
                causation_id,
                correlation_id,
                idempotency_key
            )
            VALUES (
                'dia_' || lower(hex(randomblob(16))),
                ?1,
                ?2,
                ?3,
                ?4,
                ?5,
                ?6,
                ?7,
                ?8,
                ?9,
                ?10,
                ?11,
                ?12,
                ?13,
                ?14,
                ?15,
                ?16,
                ?17,
                ?18
            )
            RETURNING diagnostic_id
            "#,
            params![
                diagnostic.instance_id,
                diagnostic.program_id,
                diagnostic.program_version_id,
                diagnostic.severity.as_str(),
                diagnostic.code,
                diagnostic.message,
                diagnostic.source_span_json,
                diagnostic.subject_type,
                diagnostic.subject_id,
                diagnostic.event_id,
                diagnostic.effect_id,
                diagnostic.run_id,
                diagnostic.assertion_id,
                diagnostic.evidence_ids_json,
                diagnostic.artifact_ids_json,
                diagnostic.causation_id,
                diagnostic.correlation_id,
                diagnostic.idempotency_key,
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn optional_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

#[cfg(feature = "native")]
fn existing_diagnostic_id_on(
    connection: &Connection,
    diagnostic: &DiagnosticRecord<'_>,
) -> StoreResult<Option<String>> {
    let Some(idempotency_key) = diagnostic.idempotency_key else {
        return Ok(None);
    };
    if let Some(instance_id) = diagnostic.instance_id {
        return connection
            .query_row(
                r#"
                SELECT diagnostic_id
                FROM diagnostics
                WHERE instance_id = ?1 AND idempotency_key = ?2
                "#,
                params![instance_id, idempotency_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into);
    }
    if let Some(program_version_id) = diagnostic.program_version_id {
        return connection
            .query_row(
                r#"
                SELECT diagnostic_id
                FROM diagnostics
                WHERE instance_id IS NULL
                  AND program_version_id = ?1
                  AND idempotency_key = ?2
                "#,
                params![program_version_id, idempotency_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into);
    }
    if let Some(program_id) = diagnostic.program_id {
        return connection
            .query_row(
                r#"
                SELECT diagnostic_id
                FROM diagnostics
                WHERE instance_id IS NULL
                  AND program_id = ?1
                  AND idempotency_key = ?2
                "#,
                params![program_id, idempotency_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into);
    }
    Ok(None)
}

#[cfg(feature = "native")]
fn parse_json_array(json: &str) -> StoreResult<()> {
    let value = serde_json::from_str::<Value>(json)?;
    if value.is_array() {
        Ok(())
    } else {
        Err(StoreError::Conflict("expected JSON array".to_owned()))
    }
}

#[cfg(feature = "native")]
fn satisfy_dependencies_on(connection: &Connection, instance_id: &str) -> StoreResult<usize> {
    connection
        .execute(
            r#"
            UPDATE effects
            SET status = 'queued',
                updated_at = CURRENT_TIMESTAMP
            WHERE instance_id = ?1
              AND status = 'blocked_by_dependency'
              AND effect_id IN (
                  SELECT candidate.effect_id
                  FROM effects AS candidate
                  WHERE candidate.instance_id = ?1
                    AND NOT EXISTS (
                        SELECT 1
                        FROM effect_dependencies AS dependency
                        JOIN effects AS upstream
                          ON upstream.effect_id = dependency.upstream_effect_id
                         AND upstream.instance_id = dependency.instance_id
                        WHERE dependency.instance_id = candidate.instance_id
                          AND dependency.downstream_effect_id = candidate.effect_id
                          AND NOT (
                              (dependency.predicate = 'succeeds' AND upstream.status = 'completed')
                              OR (dependency.predicate = 'fails' AND upstream.status IN ('failed', 'timed_out'))
                              OR (dependency.predicate = 'timed_out' AND upstream.status = 'timed_out')
                              OR (dependency.predicate = 'cancelled' AND upstream.status = 'cancelled')
                              OR (dependency.predicate = 'completes' AND upstream.status IN ('completed', 'failed', 'timed_out', 'cancelled'))
                          )
                    )
              )
            "#,
            [instance_id],
        )
        .map_err(Into::into)
}

#[cfg(feature = "native")]
struct PolicyBlock {
    status: &'static str,
    reason: String,
}

#[cfg(feature = "native")]
struct PolicyEffect {
    kind: String,
    target: Option<String>,
    status: String,
    required_capabilities_json: String,
    profile: Option<String>,
    program_id: String,
    declared_profiles_json: String,
}

#[cfg(feature = "native")]
fn policy_block_on(
    connection: &Connection,
    instance_id: &str,
    effect_id: &str,
) -> StoreResult<Option<PolicyBlock>> {
    let Some(effect) = connection
        .query_row(
            r#"
            SELECT
                effects.kind,
                effects.target,
                effects.status,
                effects.required_capabilities,
                effects.profile,
                COALESCE(effect_versions.program_id, instances.program_id),
                COALESCE(effect_versions.declared_profiles, active_versions.declared_profiles)
            FROM effects
            JOIN instances ON instances.instance_id = effects.instance_id
            JOIN program_versions AS active_versions
              ON active_versions.version_id = instances.version_id
            LEFT JOIN program_versions AS effect_versions
              ON effect_versions.version_id = effects.program_version_id
            WHERE effects.instance_id = ?1
              AND effects.effect_id = ?2
            "#,
            params![instance_id, effect_id],
            |row| {
                Ok(PolicyEffect {
                    kind: row.get(0)?,
                    target: row.get(1)?,
                    status: row.get(2)?,
                    required_capabilities_json: row.get(3)?,
                    profile: row.get(4)?,
                    program_id: row.get(5)?,
                    declared_profiles_json: row.get(6)?,
                })
            },
        )
        .optional()?
    else {
        return Ok(None);
    };

    if !matches!(
        effect.status.as_str(),
        "queued" | "blocked_by_dependency" | "blocked_by_capacity"
    ) {
        return Ok(None);
    }

    if let Some(block) = agent_target_policy_block(&effect)? {
        return Ok(Some(block));
    }

    // Timers, builtin-tracker verbs, and coordination verbs
    // (spec/coordination.md) are resolved by the runtime itself on worker
    // passes: no provider, capability, or profile applies.
    if effect.kind == "timer.wait"
        || effect.kind.starts_with("tracker.")
        || effect.kind.starts_with("lease.")
        || effect.kind.starts_with("ledger.")
        || effect.kind.starts_with("counter.")
        || effect.kind == "signal.emit"
        || effect.kind.starts_with("file.")
    {
        return Ok(None);
    }

    if effect.kind == "exec.command" {
        let capabilities = explicit_required_capabilities(&effect)?;
        return policy_block_for_capabilities(connection, &effect, &capabilities);
    }

    if effect.kind == "capability.call" {
        let mut capabilities = explicit_required_capabilities(&effect)?;
        if capabilities.is_empty() {
            match effect.target.as_deref().filter(|target| !target.is_empty()) {
                Some(target) => capabilities.push(target.to_owned()),
                None => {
                    return Ok(Some(PolicyBlock {
                        status: "blocked_by_capability",
                        reason: "capability.call effect has no target capability requirement"
                            .to_owned(),
                    }))
                }
            }
        }
        return policy_block_for_capabilities(connection, &effect, &capabilities);
    }

    let capabilities = required_capabilities(&effect)?;
    if !effect_provider_exists(connection, &effect.kind)? {
        return Ok(Some(PolicyBlock {
            status: "blocked_by_capability",
            reason: format!("no effect provider is registered for `{}`", effect.kind),
        }));
    }

    policy_block_for_capabilities(connection, &effect, &capabilities)
}

#[cfg(feature = "native")]
fn policy_block_for_capabilities(
    connection: &Connection,
    effect: &PolicyEffect,
    capabilities: &[String],
) -> StoreResult<Option<PolicyBlock>> {
    for capability in capabilities {
        if !capability_schema_exists(connection, capability)? {
            return Ok(Some(PolicyBlock {
                status: "blocked_by_capability",
                reason: format!("capability `{capability}` is not registered"),
            }));
        }
        if !capability_bound(connection, &effect.program_id, capability)? {
            return Ok(Some(PolicyBlock {
                status: "blocked_by_capability",
                reason: format!(
                    "capability `{capability}` is not bound for program {}",
                    effect.program_id
                ),
            }));
        }
    }

    if let Some(profile) = &effect.profile {
        let Some((enforcement_mode, allowed_capabilities)) = profile_policy(connection, profile)?
        else {
            return Ok(Some(PolicyBlock {
                status: "blocked_by_profile",
                reason: format!("profile `{profile}` is not registered"),
            }));
        };
        if enforcement_mode != "audit" {
            for capability in capabilities {
                if !capability_allowed(&allowed_capabilities, capability) {
                    return Ok(Some(PolicyBlock {
                        status: "blocked_by_profile",
                        reason: format!(
                            "profile `{profile}` does not allow capability `{capability}`"
                        ),
                    }));
                }
            }
        }
    }

    Ok(None)
}

#[cfg(feature = "native")]
fn agent_target_policy_block(effect: &PolicyEffect) -> StoreResult<Option<PolicyBlock>> {
    if effect.kind != "agent.tell" {
        return Ok(None);
    }
    let Some(target) = effect.target.as_deref() else {
        return Ok(None);
    };
    if !declared_agents_present(&effect.declared_profiles_json)? {
        return Ok(None);
    }
    let Some(declared_profile) = declared_agent_profile(&effect.declared_profiles_json, target)?
    else {
        return Ok(Some(PolicyBlock {
            status: "blocked_by_profile",
            reason: format!("agent `{target}` is not declared by the program"),
        }));
    };
    match (effect.profile.as_deref(), declared_profile.as_deref()) {
        (Some(actual), Some(expected)) if actual != expected => Ok(Some(PolicyBlock {
            status: "blocked_by_profile",
            reason: format!(
                "agent `{target}` uses profile `{actual}`, expected declared profile `{expected}`"
            ),
        })),
        (None, Some(expected)) => Ok(Some(PolicyBlock {
            status: "blocked_by_profile",
            reason: format!("agent `{target}` requires declared profile `{expected}`"),
        })),
        _ => {
            let declared_capabilities =
                declared_agent_capabilities(&effect.declared_profiles_json, target)?;
            let required_capabilities = explicit_required_capabilities(effect)?;
            for capability in required_capabilities {
                if !declared_capabilities.contains(&capability) {
                    return Ok(Some(PolicyBlock {
                        status: "blocked_by_capability",
                        reason: format!(
                            "agent `{target}` does not declare required capability `{capability}`"
                        ),
                    }));
                }
            }
            Ok(None)
        }
    }
}

#[cfg(feature = "native")]
fn capacity_block_on(
    connection: &Connection,
    instance_id: &str,
    effect_id: &str,
) -> StoreResult<Option<String>> {
    let Some((kind, target, declared_profiles)) = connection
        .query_row(
            r#"
            SELECT effects.kind,
                   effects.target,
                   COALESCE(effect_versions.declared_profiles, active_versions.declared_profiles)
            FROM effects
            JOIN instances ON instances.instance_id = effects.instance_id
            JOIN program_versions AS active_versions
              ON active_versions.version_id = instances.version_id
            LEFT JOIN program_versions AS effect_versions
              ON effect_versions.version_id = effects.program_version_id
            WHERE effects.instance_id = ?1
              AND effects.effect_id = ?2
            "#,
            params![instance_id, effect_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    if kind != "agent.tell" {
        return Ok(None);
    }
    let Some(agent) = target else {
        return Ok(None);
    };
    let Some(capacity) = declared_agent_capacity(&declared_profiles, &agent)? else {
        return Ok(None);
    };
    let running = connection.query_row(
        r#"
        SELECT COUNT(*)
        FROM effects
        WHERE instance_id = ?1
          AND kind = 'agent.tell'
          AND target = ?2
          AND status = 'running'
        "#,
        params![instance_id, agent],
        |row| row.get::<_, i64>(0),
    )?;
    if running >= capacity {
        Ok(Some(format!(
            "agent `{agent}` capacity exhausted ({running}/{capacity} running)"
        )))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "native")]
fn declared_agent_profile(
    declared_profiles_json: &str,
    agent: &str,
) -> StoreResult<Option<Option<String>>> {
    let parsed = serde_json::from_str::<Value>(declared_profiles_json)?;
    Ok(agent_profile_in_value(&parsed, agent))
}

#[cfg(feature = "native")]
fn declared_agents_present(declared_profiles_json: &str) -> StoreResult<bool> {
    let parsed = serde_json::from_str::<Value>(declared_profiles_json)?;
    Ok(match &parsed {
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => {
            object
                .get("agents")
                .and_then(Value::as_array)
                .is_some_and(|agents| !agents.is_empty())
                || object.iter().any(|(key, value)| {
                    if matches!(key.as_str(), "harnesses" | "workflow" | "schemas") {
                        return false;
                    }
                    value.as_object().is_some_and(|entry| {
                        entry.contains_key("profile")
                            || entry.contains_key("capacity")
                            || entry.contains_key("capabilities")
                            || entry.contains_key("harness")
                            || entry.contains_key("provider")
                    })
                })
        }
        _ => false,
    })
}

#[cfg(feature = "native")]
fn agent_profile_in_value(value: &Value, agent: &str) -> Option<Option<String>> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| agent_profile_in_value(item, agent)),
        Value::Object(object) => {
            if let Some(entry) = object.get(agent) {
                return Some(
                    entry
                        .get("profile")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                );
            }
            if let Some(profile) = object
                .get("agents")
                .and_then(|agents| agent_profile_in_value(agents, agent))
            {
                return Some(profile);
            }
            let declared_agent = object
                .get("name")
                .or_else(|| object.get("agent"))
                .or_else(|| object.get("agent_name"))
                .or_else(|| object.get("target"))
                .and_then(Value::as_str);
            if declared_agent == Some(agent) {
                Some(
                    object
                        .get("profile")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                )
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(feature = "native")]
fn declared_agent_capacity(declared_profiles_json: &str, agent: &str) -> StoreResult<Option<i64>> {
    let parsed = serde_json::from_str::<Value>(declared_profiles_json)?;
    Ok(agent_capacity_in_value(&parsed, agent))
}

#[cfg(feature = "native")]
fn declared_agent_capabilities(
    declared_profiles_json: &str,
    agent: &str,
) -> StoreResult<BTreeSet<String>> {
    let parsed = serde_json::from_str::<Value>(declared_profiles_json)?;
    Ok(agent_capabilities_in_value(&parsed, agent).unwrap_or_default())
}

#[cfg(feature = "native")]
fn agent_capabilities_in_value(value: &Value, agent: &str) -> Option<BTreeSet<String>> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| agent_capabilities_in_value(item, agent)),
        Value::Object(object) => {
            if let Some(entry) = object.get(agent) {
                return Some(capabilities_value(entry.get("capabilities")?));
            }
            if let Some(capabilities) = object
                .get("agents")
                .and_then(|agents| agent_capabilities_in_value(agents, agent))
            {
                return Some(capabilities);
            }
            let declared_agent = object
                .get("name")
                .or_else(|| object.get("agent"))
                .or_else(|| object.get("agent_name"))
                .or_else(|| object.get("target"))
                .and_then(Value::as_str);
            if declared_agent == Some(agent) {
                object.get("capabilities").map(capabilities_value)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(feature = "native")]
fn capabilities_value(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(feature = "native")]
fn agent_capacity_in_value(value: &Value, agent: &str) -> Option<i64> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| agent_capacity_in_value(item, agent)),
        Value::Object(object) => {
            if let Some(capacity) = object.get(agent).and_then(capacity_value) {
                return Some(capacity);
            }
            if let Some(capacity) = object
                .get(agent)
                .and_then(|entry| entry.get("capacity"))
                .and_then(capacity_value)
            {
                return Some(capacity);
            }
            if let Some(capacity) = object
                .get("agents")
                .and_then(|agents| agent_capacity_in_value(agents, agent))
            {
                return Some(capacity);
            }
            let declared_agent = object
                .get("name")
                .or_else(|| object.get("agent"))
                .or_else(|| object.get("agent_name"))
                .or_else(|| object.get("target"))
                .and_then(Value::as_str);
            if declared_agent == Some(agent) {
                object.get("capacity").and_then(capacity_value)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(feature = "native")]
fn capacity_value(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_u64()
            .and_then(|capacity| i64::try_from(capacity).ok())
    })
}

#[cfg(feature = "native")]
fn required_capabilities(effect: &PolicyEffect) -> StoreResult<Vec<String>> {
    let mut capabilities = explicit_required_capabilities(effect)?;
    if capabilities.is_empty() {
        capabilities.push(effect.kind.clone());
    }
    capabilities.sort();
    capabilities.dedup();
    Ok(capabilities)
}

#[cfg(feature = "native")]
fn explicit_required_capabilities(effect: &PolicyEffect) -> StoreResult<Vec<String>> {
    let parsed = serde_json::from_str::<Value>(&effect.required_capabilities_json)?;
    let mut capabilities = parsed
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    capabilities.sort();
    capabilities.dedup();
    Ok(capabilities)
}

#[cfg(feature = "native")]
fn effect_provider_exists(connection: &Connection, effect_kind: &str) -> StoreResult<bool> {
    connection
        .query_row(
            "SELECT 1 FROM effect_providers WHERE effect_kind = ?1 LIMIT 1",
            [effect_kind],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn capability_schema_exists(connection: &Connection, capability: &str) -> StoreResult<bool> {
    connection
        .query_row(
            "SELECT 1 FROM capability_schemas WHERE capability = ?1 LIMIT 1",
            [capability],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn capability_bound(
    connection: &Connection,
    program_id: &str,
    capability: &str,
) -> StoreResult<bool> {
    connection
        .query_row(
            r#"
            SELECT 1
            FROM capability_bindings
            WHERE capability = ?1
              AND (program_id = ?2 OR program_id IS NULL)
            LIMIT 1
            "#,
            params![capability, program_id],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(Into::into)
}

/// The provider name bound to `capability`, mirroring `capability_bound`'s
/// program-scoped-wins-over-global scoping (a program-scoped row beats a global
/// `NULL` one). `Ok(None)` when no binding row exists or its `provider` column is
/// NULL. This reads the `capability_bindings.provider` column that `capability_bound`
/// only tested for existence — the registry-driven `CapabilityProvider` selection
/// seam (S5) that lands with the first real per-capability provider.
#[cfg(feature = "native")]
fn capability_bound_provider(
    connection: &Connection,
    program_id: &str,
    capability: &str,
) -> StoreResult<Option<String>> {
    connection
        .query_row(
            r#"
            SELECT provider
            FROM capability_bindings
            WHERE capability = ?1
              AND (program_id = ?2 OR program_id IS NULL)
            ORDER BY (program_id IS NULL) ASC
            LIMIT 1
            "#,
            params![capability, program_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn profile_policy(
    connection: &Connection,
    profile: &str,
) -> StoreResult<Option<(String, Vec<String>)>> {
    connection
        .query_row(
            "SELECT enforcement_mode, allowed_capabilities FROM profiles WHERE name = ?1",
            [profile],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .map(|(mode, allowed_json)| {
            let allowed = serde_json::from_str::<Value>(&allowed_json)?
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok((mode, allowed))
        })
        .transpose()
}

#[cfg(feature = "native")]
fn instance_status_on(connection: &Connection, instance_id: &str) -> StoreResult<Option<String>> {
    connection
        .query_row(
            "SELECT status FROM instances WHERE instance_id = ?1",
            [instance_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn active_revision_on(
    connection: &Connection,
    instance_id: &str,
) -> StoreResult<(Option<String>, i64)> {
    connection
        .query_row(
            "SELECT version_id, revision_epoch FROM instances WHERE instance_id = ?1",
            [instance_id],
            |row| Ok((Some(row.get::<_, String>(0)?), row.get::<_, i64>(1)?)),
        )
        .optional()
        .map(|row| row.unwrap_or((None, 0)))
        .map_err(Into::into)
}

#[cfg(feature = "native")]
struct RevisionInstanceContext {
    program_id: String,
    program_name: String,
    active_version_id: String,
    status: String,
}

#[cfg(feature = "native")]
fn revision_instance_context_on(
    connection: &Connection,
    instance_id: &str,
) -> StoreResult<RevisionInstanceContext> {
    connection
        .query_row(
            r#"
            SELECT instances.program_id, programs.name, instances.version_id, instances.status
            FROM instances
            JOIN programs ON programs.program_id = instances.program_id
            WHERE instances.instance_id = ?1
            "#,
            [instance_id],
            |row| {
                Ok(RevisionInstanceContext {
                    program_id: row.get(0)?,
                    program_name: row.get(1)?,
                    active_version_id: row.get(2)?,
                    status: row.get(3)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::Conflict("instance does not exist".to_owned()))
}

#[cfg(feature = "native")]
fn add_instance_revision_diagnostics(
    context: &RevisionInstanceContext,
    diagnostics: &mut Vec<RevisionCompatibilityDiagnostic>,
) {
    if matches!(
        context.status.as_str(),
        "completed" | "failed" | "cancelled"
    ) {
        diagnostics.push(revision_compatibility_diagnostic(
            "revision.terminal_instance",
            format!(
                "instance is {}; revisions require a non-terminal instance",
                context.status
            ),
            None,
        ));
    }
}

#[cfg(feature = "native")]
fn program_version_analysis_on(
    connection: &Connection,
    version_id: &str,
) -> StoreResult<(String, Value)> {
    let (program_id, analysis_summary_json) = connection
        .query_row(
            r#"
            SELECT program_id, analysis_summary
            FROM program_versions
            WHERE version_id = ?1
            "#,
            [version_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::Conflict("program version does not exist".to_owned()))?;
    let analysis_summary = serde_json::from_str::<Value>(&analysis_summary_json)?;
    Ok((program_id, analysis_summary))
}

#[cfg(feature = "native")]
fn compare_revision_summaries(
    active: &Value,
    candidate: &Value,
    diagnostics: &mut Vec<RevisionCompatibilityDiagnostic>,
) {
    let active_workflow = active.get("workflow").and_then(Value::as_str);
    let candidate_workflow = candidate.get("workflow").and_then(Value::as_str);
    match (active_workflow, candidate_workflow) {
        (Some(active_workflow), Some(candidate_workflow))
            if active_workflow != candidate_workflow =>
        {
            diagnostics.push(revision_compatibility_diagnostic(
                "revision.root_workflow_changed",
                format!(
                    "candidate root workflow `{candidate_workflow}` does not match active root `{active_workflow}`"
                ),
                Some(candidate_workflow),
            ));
        }
        (None, _) => diagnostics.push(revision_compatibility_diagnostic(
            "revision.active_analysis_missing",
            "active version does not include revision analysis metadata".to_owned(),
            None,
        )),
        (_, None) => diagnostics.push(revision_compatibility_diagnostic(
            "revision.candidate_analysis_missing",
            "candidate version does not include revision analysis metadata".to_owned(),
            None,
        )),
        _ => {}
    }

    compare_contracts("input", true, active, candidate, diagnostics);
    compare_contracts("output", false, active, candidate, diagnostics);
    compare_contracts("failure", false, active, candidate, diagnostics);
}

#[cfg(feature = "native")]
fn compare_contracts(
    kind: &str,
    reject_candidate_additions: bool,
    active: &Value,
    candidate: &Value,
    diagnostics: &mut Vec<RevisionCompatibilityDiagnostic>,
) {
    let active_contracts = contracts_by_name(active, kind);
    let candidate_contracts = contracts_by_name(candidate, kind);
    for (name, active_ty) in &active_contracts {
        match candidate_contracts.get(name) {
            Some(candidate_ty) if candidate_ty.ty == active_ty.ty => {}
            Some(candidate_ty) => diagnostics.push(revision_compatibility_diagnostic_with_span(
                "revision.contract_changed",
                format!(
                    "{kind} contract `{name}` changed from `{}` to `{}`",
                    active_ty.ty, candidate_ty.ty
                ),
                Some(name.as_str()),
                candidate_ty.source_span_json.clone(),
            )),
            None => diagnostics.push(revision_compatibility_diagnostic_with_span(
                "revision.contract_removed",
                format!("{kind} contract `{name}` is missing from the candidate version"),
                Some(name.as_str()),
                active_ty.source_span_json.clone(),
            )),
        }
    }
    if reject_candidate_additions {
        for (name, candidate_ty) in candidate_contracts {
            if !active_contracts.contains_key(&name) {
                diagnostics.push(revision_compatibility_diagnostic_with_span(
                    "revision.input_contract_added",
                    format!(
                        "candidate adds input contract `{name}` with type `{}` to an already-started instance",
                        candidate_ty.ty
                    ),
                    Some(name.as_str()),
                    candidate_ty.source_span_json,
                ));
            }
        }
    }
}

#[cfg(feature = "native")]
#[derive(Clone, Debug, Eq, PartialEq)]
struct ContractSummary {
    ty: String,
    source_span_json: Option<String>,
}

#[cfg(feature = "native")]
fn contracts_by_name(summary: &Value, kind: &str) -> BTreeMap<String, ContractSummary> {
    summary
        .get("workflow_contracts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|contract| contract.get("kind").and_then(Value::as_str) == Some(kind))
        .filter_map(|contract| {
            Some((
                contract.get("name")?.as_str()?.to_owned(),
                ContractSummary {
                    ty: contract.get("type")?.as_str()?.to_owned(),
                    source_span_json: summary_source_span_json(contract),
                },
            ))
        })
        .collect()
}

#[cfg(feature = "native")]
fn add_active_fact_schema_diagnostics(
    connection: &Connection,
    instance_id: &str,
    active_summary: &Value,
    candidate_summary: &Value,
    diagnostics: &mut Vec<RevisionCompatibilityDiagnostic>,
) -> StoreResult<()> {
    let active_schemas = schemas_by_name(active_summary);
    let candidate_schemas = schemas_by_name(candidate_summary);
    if active_schemas.is_empty() && candidate_schemas.is_empty() {
        return Ok(());
    }

    let mut statement = connection.prepare(
        r#"
        SELECT fact_id, name, schema_id, value_json
        FROM facts
        WHERE instance_id = ?1
          AND consumed_at IS NULL
        ORDER BY fact_id
        "#,
    )?;
    let rows = statement.query_map([instance_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    for row in rows {
        let (fact_id, name, schema_id, value_json) = row?;
        let schema_name = fact_schema_name(
            &name,
            schema_id.as_deref(),
            &active_schemas,
            &candidate_schemas,
        );
        let Some(schema_name) = schema_name else {
            continue;
        };
        let Some(candidate_schema) = candidate_schemas.get(schema_name) else {
            let source_span_json = active_schemas
                .get(schema_name)
                .and_then(summary_source_span_json);
            diagnostics.push(revision_compatibility_diagnostic_with_span(
                "revision.active_fact_schema_removed",
                format!("active fact `{fact_id}` uses schema `{schema_name}` missing from candidate version"),
                Some(schema_name),
                source_span_json,
            ));
            continue;
        };

        let value = serde_json::from_str::<Value>(&value_json)?;
        let mut errors = Vec::new();
        validate_fact_value_against_schema(
            &value,
            candidate_schema,
            &candidate_schemas,
            "$",
            &mut errors,
            0,
        );
        if !errors.is_empty() {
            diagnostics.push(revision_compatibility_diagnostic_with_span(
                "revision.active_fact_incompatible",
                format!(
                    "active fact `{fact_id}` no longer typechecks as `{schema_name}`: {}",
                    errors.join("; ")
                ),
                Some(schema_name),
                summary_source_span_json(candidate_schema),
            ));
        }
    }

    Ok(())
}

#[cfg(feature = "native")]
fn schemas_by_name(summary: &Value) -> BTreeMap<String, Value> {
    summary
        .get("schemas")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|schema| Some((schema.get("name")?.as_str()?.to_owned(), schema.clone())))
        .collect()
}

#[cfg(feature = "native")]
fn summary_source_span_json(summary: &Value) -> Option<String> {
    summary.get("source_span").map(Value::to_string)
}

#[cfg(feature = "native")]
fn fact_schema_name<'a>(
    fact_name: &'a str,
    schema_id: Option<&'a str>,
    active_schemas: &BTreeMap<String, Value>,
    candidate_schemas: &BTreeMap<String, Value>,
) -> Option<&'a str> {
    if let Some(schema_id) = schema_id {
        if active_schemas.contains_key(schema_id) || candidate_schemas.contains_key(schema_id) {
            return Some(schema_id);
        }
    }
    if active_schemas.contains_key(fact_name) || candidate_schemas.contains_key(fact_name) {
        return Some(fact_name);
    }
    None
}

#[cfg(feature = "native")]
fn validate_fact_value_against_schema(
    value: &Value,
    schema: &Value,
    schemas: &BTreeMap<String, Value>,
    path: &str,
    errors: &mut Vec<String>,
    depth: usize,
) {
    if depth > 32 {
        errors.push(format!("{path} exceeded schema recursion limit"));
        return;
    }
    match schema.get("kind").and_then(Value::as_str) {
        Some("class") => validate_value_against_fields(
            value,
            schema.get("fields").and_then(Value::as_array),
            schemas,
            path,
            errors,
            depth,
        ),
        Some("enum") => {
            let variants = schema
                .get("variants")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            match value.as_str() {
                Some(variant) if variants.contains(&variant) => {}
                Some(variant) => errors.push(format!(
                    "{path} has enum variant `{variant}` not declared by candidate"
                )),
                None => errors.push(format!("{path} must be a string enum variant")),
            }
        }
        Some(kind) => errors.push(format!("{path} uses unsupported schema kind `{kind}`")),
        None => errors.push(format!("{path} uses schema without a kind")),
    }
}

#[cfg(feature = "native")]
fn validate_value_against_fields(
    value: &Value,
    fields: Option<&Vec<Value>>,
    schemas: &BTreeMap<String, Value>,
    path: &str,
    errors: &mut Vec<String>,
    depth: usize,
) {
    let Some(object) = value.as_object() else {
        errors.push(format!("{path} must be an object"));
        return;
    };
    let fields = fields.map(Vec::as_slice).unwrap_or(&[]);
    let declared = fields
        .iter()
        .filter_map(|field| Some((field.get("name")?.as_str()?, field.get("type")?.as_str()?)))
        .collect::<BTreeMap<_, _>>();
    for key in object.keys() {
        if !declared.contains_key(key.as_str()) {
            errors.push(format!("{path}.{key} is not declared by candidate"));
        }
    }
    for (name, signature) in declared {
        let field_path = format!("{path}.{name}");
        match object.get(name) {
            Some(value) => {
                validate_value_against_type_signature(
                    value,
                    signature,
                    schemas,
                    &field_path,
                    errors,
                    depth + 1,
                );
            }
            None if is_optional_signature(signature) => {}
            None => errors.push(format!("{field_path} is required by candidate")),
        }
    }
}

#[cfg(feature = "native")]
fn validate_value_against_type_signature(
    value: &Value,
    signature: &str,
    schemas: &BTreeMap<String, Value>,
    path: &str,
    errors: &mut Vec<String>,
    depth: usize,
) {
    if depth > 32 {
        errors.push(format!("{path} exceeded schema recursion limit"));
        return;
    }
    match signature {
        "string" | "duration" | "time" | "image" | "audio" | "pdf" | "video" => {
            if !value.is_string() {
                errors.push(format!("{path} must be {signature}"));
            }
        }
        "int" => {
            if value.as_i64().is_none() {
                errors.push(format!("{path} must be int"));
            }
        }
        "float" => {
            if value.as_f64().is_none() {
                errors.push(format!("{path} must be float"));
            }
        }
        "bool" => {
            if !value.is_boolean() {
                errors.push(format!("{path} must be bool"));
            }
        }
        "null" => {
            if !value.is_null() {
                errors.push(format!("{path} must be null"));
            }
        }
        _ => {
            if let Some(expected) = signature_envelope(signature, "literal") {
                let expected = serde_json::from_str::<String>(expected)
                    .unwrap_or_else(|_| expected.to_owned());
                if value.as_str() != Some(expected.as_str()) {
                    errors.push(format!("{path} must be literal {expected:?}"));
                }
            } else if let Some(schema_name) = signature_envelope(signature, "ref") {
                match schemas.get(schema_name) {
                    Some(schema) => validate_fact_value_against_schema(
                        value,
                        schema,
                        schemas,
                        path,
                        errors,
                        depth + 1,
                    ),
                    None => errors.push(format!(
                        "{path} references schema `{schema_name}` missing from candidate"
                    )),
                }
            } else if let Some(inner) = signature_envelope(signature, "optional") {
                if !value.is_null() {
                    validate_value_against_type_signature(
                        value,
                        inner,
                        schemas,
                        path,
                        errors,
                        depth + 1,
                    );
                }
            } else if let Some(inner) = signature_envelope(signature, "array") {
                match value.as_array() {
                    Some(items) => {
                        for (index, item) in items.iter().enumerate() {
                            validate_value_against_type_signature(
                                item,
                                inner,
                                schemas,
                                &format!("{path}[{index}]"),
                                errors,
                                depth + 1,
                            );
                        }
                    }
                    None => errors.push(format!("{path} must be an array")),
                }
            } else if let Some(inner) = signature_envelope(signature, "map") {
                match value.as_object() {
                    Some(map) => {
                        for (key, item) in map {
                            validate_value_against_type_signature(
                                item,
                                inner,
                                schemas,
                                &format!("{path}.{key}"),
                                errors,
                                depth + 1,
                            );
                        }
                    }
                    None => errors.push(format!("{path} must be an object map")),
                }
            } else if let Some(inner) = signature_envelope(signature, "union") {
                let variants = split_top_level(inner, " | ");
                if !variants.iter().any(|variant| {
                    let mut candidate_errors = Vec::new();
                    validate_value_against_type_signature(
                        value,
                        variant,
                        schemas,
                        path,
                        &mut candidate_errors,
                        depth + 1,
                    );
                    candidate_errors.is_empty()
                }) {
                    errors.push(format!(
                        "{path} must match one of: {}",
                        variants.join(" | ")
                    ));
                }
            } else if let Some(inner) = signature_envelope(signature, "object") {
                validate_value_against_object_signature(value, inner, schemas, path, errors, depth);
            } else if let Some(inner) = signature_envelope(signature, "agentref") {
                let agents = split_top_level(inner, " | ");
                match value.as_str() {
                    Some(agent) if agents.iter().any(|candidate| candidate == agent) => {}
                    Some(_) => errors.push(format!(
                        "{path} must name one of these agents: {}",
                        agents.join(", ")
                    )),
                    None => errors.push(format!("{path} must be an agent name string")),
                }
            } else {
                errors.push(format!("{path} uses unsupported type `{signature}`"));
            }
        }
    }
}

#[cfg(feature = "native")]
fn validate_value_against_object_signature(
    value: &Value,
    inner: &str,
    schemas: &BTreeMap<String, Value>,
    path: &str,
    errors: &mut Vec<String>,
    depth: usize,
) {
    let Some(fields) = inner
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    else {
        errors.push(format!("{path} uses malformed object type"));
        return;
    };
    let fields = split_top_level(fields, ", ")
        .into_iter()
        .filter_map(|field| {
            let (name, signature) = field.split_once(' ')?;
            Some(json!({ "name": name, "type": signature }))
        })
        .collect::<Vec<_>>();
    validate_value_against_fields(value, Some(&fields), schemas, path, errors, depth + 1);
}

#[cfg(feature = "native")]
fn is_optional_signature(signature: &str) -> bool {
    signature_envelope(signature, "optional").is_some()
}

#[cfg(feature = "native")]
fn signature_envelope<'a>(signature: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}<");
    signature
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix('>'))
}

#[cfg(feature = "native")]
fn split_top_level(input: &str, separator: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut index = 0usize;
    while index < input.len() {
        let rest = &input[index..];
        if depth == 0 && rest.starts_with(separator) {
            parts.push(input[start..index].trim().to_owned());
            index += separator.len();
            start = index;
            continue;
        }
        if let Some(ch) = rest.chars().next() {
            match ch {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ => {}
            }
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    parts.push(input[start..].trim().to_owned());
    parts.retain(|part| !part.is_empty());
    parts
}

#[cfg(feature = "native")]
fn revision_compatibility_diagnostic(
    code: &str,
    message: String,
    subject: Option<&str>,
) -> RevisionCompatibilityDiagnostic {
    revision_compatibility_diagnostic_with_span(code, message, subject, None)
}

#[cfg(feature = "native")]
fn revision_compatibility_diagnostic_with_span(
    code: &str,
    message: String,
    subject: Option<&str>,
    source_span_json: Option<String>,
) -> RevisionCompatibilityDiagnostic {
    RevisionCompatibilityDiagnostic {
        code: code.to_owned(),
        message,
        subject: subject.map(str::to_owned),
        source_span_json,
    }
}

#[cfg(feature = "native")]
fn normalize_cancellation_policy(policy: &str) -> StoreResult<&'static str> {
    match policy {
        "keep" => Ok("keep"),
        "cancel_queued" | "cancel queued" | "queued" => Ok("cancel_queued"),
        "request_running" | "request running" | "running" => Ok("request_running"),
        _ => Err(StoreError::Conflict(format!(
            "unsupported revision cancellation policy `{policy}`"
        ))),
    }
}

#[cfg(feature = "native")]
fn random_id_on(connection: &Connection, prefix: &str) -> StoreResult<String> {
    connection
        .query_row(
            "SELECT ?1 || '_' || lower(hex(randomblob(16)))",
            [prefix],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn workflow_revision_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRevisionView> {
    Ok(WorkflowRevisionView {
        revision_id: row.get(0)?,
        instance_id: row.get(1)?,
        epoch: row.get(2)?,
        from_version_id: row.get(3)?,
        to_version_id: row.get(4)?,
        activated_by_event_id: row.get(5)?,
        activation_policy_json: row.get(6)?,
        cancellation_policy: row.get(7)?,
        status: row.get(8)?,
        idempotency_key: row.get(9)?,
        created_at: row.get(10)?,
        activated_at: row.get(11)?,
    })
}

#[cfg(feature = "native")]
fn revision_by_id_on(
    connection: &Connection,
    revision_id: &str,
) -> StoreResult<Option<WorkflowRevisionView>> {
    connection
        .query_row(
            r#"
            SELECT
                revision_id,
                instance_id,
                epoch,
                from_version_id,
                to_version_id,
                activated_by_event_id,
                activation_policy_json,
                cancellation_policy,
                status,
                idempotency_key,
                created_at,
                activated_at
            FROM instance_revisions
            WHERE revision_id = ?1
            "#,
            [revision_id],
            workflow_revision_from_row,
        )
        .optional()
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn revision_by_idempotency_on(
    connection: &Connection,
    instance_id: &str,
    idempotency_key: &str,
) -> StoreResult<Option<WorkflowRevisionView>> {
    connection
        .query_row(
            r#"
            SELECT
                revision_id,
                instance_id,
                epoch,
                from_version_id,
                to_version_id,
                activated_by_event_id,
                activation_policy_json,
                cancellation_policy,
                status,
                idempotency_key,
                created_at,
                activated_at
            FROM instance_revisions
            WHERE instance_id = ?1
              AND idempotency_key = ?2
            "#,
            params![instance_id, idempotency_key],
            workflow_revision_from_row,
        )
        .optional()
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn ensure_revision_idempotency_matches(
    existing: &WorkflowRevisionView,
    activation: &RevisionActivation<'_>,
    activation_policy: &Value,
    cancellation_policy: &str,
) -> StoreResult<()> {
    let existing_activation_policy: Value = serde_json::from_str(&existing.activation_policy_json)?;
    if existing.instance_id.as_str() == activation.instance_id
        && existing.from_version_id.as_str() == activation.from_version_id
        && existing.to_version_id.as_str() == activation.to_version_id
        && existing.cancellation_policy.as_str() == cancellation_policy
        && &existing_activation_policy == activation_policy
    {
        return Ok(());
    }
    Err(StoreError::Conflict(
        "revision idempotency key was reused with different activation input".to_owned(),
    ))
}

#[cfg(feature = "native")]
fn revision_policy_effects_on(
    connection: &Connection,
    instance_id: &str,
    running: bool,
) -> StoreResult<Vec<String>> {
    let predicate = if running {
        "status = 'running'"
    } else {
        "status IN ('queued', 'blocked', 'blocked_by_dependency', 'blocked_by_capacity', 'blocked_by_capability', 'blocked_by_profile')"
    };
    let mut statement = connection.prepare(&format!(
        r#"
        SELECT effect_id
        FROM effects
        WHERE instance_id = ?1
          AND {predicate}
        ORDER BY created_at, effect_id
        "#
    ))?;
    let rows = statement
        .query_map([instance_id], |row| row.get(0))?
        .collect::<result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(feature = "native")]
fn effect_has_open_cancellation_request_on(
    connection: &Connection,
    instance_id: &str,
    effect_id: &str,
) -> StoreResult<bool> {
    connection
        .query_row(
            r#"
            SELECT 1
            FROM effect_cancellation_requests
            WHERE instance_id = ?1
              AND effect_id = ?2
              AND status = 'requested'
            LIMIT 1
            "#,
            params![instance_id, effect_id],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn effect_cancellation_request_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<EffectCancellationRequestView> {
    Ok(EffectCancellationRequestView {
        request_id: row.get(0)?,
        instance_id: row.get(1)?,
        effect_id: row.get(2)?,
        revision_id: row.get(3)?,
        reason: row.get(4)?,
        requested_by: row.get(5)?,
        causation_event_id: row.get(6)?,
        status: row.get(7)?,
        idempotency_key: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        resolved_by_event_id: row.get(11)?,
    })
}

#[cfg(feature = "native")]
fn cancellation_request_by_idempotency_on(
    connection: &Connection,
    instance_id: &str,
    idempotency_key: &str,
) -> StoreResult<Option<EffectCancellationRequestView>> {
    connection
        .query_row(
            r#"
            SELECT
                request_id,
                instance_id,
                effect_id,
                revision_id,
                reason,
                requested_by,
                causation_event_id,
                status,
                idempotency_key,
                created_at,
                updated_at,
                resolved_by_event_id
            FROM effect_cancellation_requests
            WHERE instance_id = ?1
              AND idempotency_key = ?2
            "#,
            params![instance_id, idempotency_key],
            effect_cancellation_request_from_row,
        )
        .optional()
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn cancellation_request_by_id_on(
    connection: &Connection,
    request_id: &str,
) -> StoreResult<Option<EffectCancellationRequestView>> {
    connection
        .query_row(
            r#"
            SELECT
                request_id,
                instance_id,
                effect_id,
                revision_id,
                reason,
                requested_by,
                causation_event_id,
                status,
                idempotency_key,
                created_at,
                updated_at,
                resolved_by_event_id
            FROM effect_cancellation_requests
            WHERE request_id = ?1
            "#,
            [request_id],
            effect_cancellation_request_from_row,
        )
        .optional()
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn insert_effect_cancellation_request_on(
    connection: &Connection,
    request: EffectCancellationRequest<'_>,
) -> StoreResult<EffectCancellationRequestView> {
    if let Some(idempotency_key) = request.idempotency_key {
        if let Some(existing) = cancellation_request_by_idempotency_on(
            connection,
            request.instance_id,
            idempotency_key,
        )? {
            return Ok(existing);
        }
    }
    if effect_has_open_cancellation_request_on(connection, request.instance_id, request.effect_id)?
    {
        return Err(StoreError::Conflict(
            "effect already has an open cancellation request".to_owned(),
        ));
    }

    let request_id = random_id_on(connection, "ecr")?;
    let payload = json!({
        "request_id": &request_id,
        "effect_id": request.effect_id,
        "revision_id": request.revision_id,
        "reason": request.reason,
        "requested_by": request.requested_by,
    })
    .to_string();
    let event = append_event_on(
        connection,
        NewEvent {
            instance_id: request.instance_id,
            event_type: "effect.cancellation_requested",
            payload_json: &payload,
            source: "kernel",
            causation_id: request.causation_event_id.or(Some(request.effect_id)),
            correlation_id: request.revision_id,
            idempotency_key: request.idempotency_key,
        },
    )?;
    connection.execute(
        r#"
        INSERT INTO effect_cancellation_requests (
            request_id,
            instance_id,
            effect_id,
            revision_id,
            reason,
            requested_by,
            causation_event_id,
            status,
            idempotency_key
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'requested', ?8)
        "#,
        params![
            request_id,
            request.instance_id,
            request.effect_id,
            request.revision_id,
            request.reason,
            request.requested_by,
            request.causation_event_id,
            request.idempotency_key,
        ],
    )?;
    let active_run_ids =
        running_run_ids_for_effect_on(connection, request.instance_id, request.effect_id)?;
    let evidence_metadata = json!({
        "request_id": &request_id,
        "effect_id": request.effect_id,
        "revision_id": request.revision_id,
        "reason": request.reason,
        "requested_by": request.requested_by,
        "event_id": event.event_id,
        "active_run_ids": &active_run_ids,
    })
    .to_string();
    let evidence_id = insert_evidence_on(
        connection,
        EvidenceRecord {
            instance_id: request.instance_id,
            kind: "effect.cancellation.requested",
            subject_type: "effect_cancellation_request",
            subject_id: &request_id,
            causation_id: Some(&event.event_id),
            correlation_id: request.revision_id,
            summary: Some("effect cancellation requested"),
            metadata_json: &evidence_metadata,
        },
    )?;
    insert_evidence_link_on(
        connection,
        EvidenceLink {
            evidence_id: &evidence_id,
            instance_id: request.instance_id,
            target_type: "event",
            target_id: &event.event_id,
            relation: "requested",
        },
    )?;
    insert_evidence_link_on(
        connection,
        EvidenceLink {
            evidence_id: &evidence_id,
            instance_id: request.instance_id,
            target_type: "effect",
            target_id: request.effect_id,
            relation: "requested_cancellation",
        },
    )?;
    if let Some(revision_id) = request.revision_id {
        insert_evidence_link_on(
            connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: request.instance_id,
                target_type: "workflow_revision",
                target_id: revision_id,
                relation: "requested_by",
            },
        )?;
    }
    for run_id in &active_run_ids {
        insert_evidence_link_on(
            connection,
            EvidenceLink {
                evidence_id: &evidence_id,
                instance_id: request.instance_id,
                target_type: "run",
                target_id: run_id,
                relation: "active_run",
            },
        )?;
    }
    cancellation_request_by_id_on(connection, &request_id)?
        .ok_or_else(|| StoreError::Conflict("cancellation request was not recorded".to_owned()))
}

#[cfg(feature = "native")]
fn running_run_ids_for_effect_on(
    connection: &Connection,
    instance_id: &str,
    effect_id: &str,
) -> StoreResult<Vec<String>> {
    let mut statement = connection.prepare(
        r#"
        SELECT run_id
        FROM runs
        WHERE instance_id = ?1
          AND effect_id = ?2
          AND status = 'running'
        ORDER BY started_at, run_id
        "#,
    )?;
    let run_ids = statement
        .query_map(params![instance_id, effect_id], |row| row.get(0))?
        .collect::<result::Result<Vec<_>, _>>()
        .map_err(StoreError::from)?;
    Ok(run_ids)
}

#[cfg(feature = "native")]
fn mark_cancellation_requests_terminal_on(
    connection: &Connection,
    instance_id: &str,
    effect_id: &str,
    event_id: &str,
) -> StoreResult<()> {
    connection.execute(
        r#"
        UPDATE effect_cancellation_requests
        SET status = 'terminal',
            resolved_by_event_id = ?1,
            updated_at = CURRENT_TIMESTAMP
        WHERE instance_id = ?2
          AND effect_id = ?3
          AND status = 'requested'
        "#,
        params![event_id, instance_id, effect_id],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn capability_allowed(allowed: &[String], capability: &str) -> bool {
    allowed.iter().any(|item| item == "*" || item == capability)
}

#[cfg(feature = "native")]
fn required_string(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

#[cfg(feature = "native")]
fn required_manifest_string(value: &Value, fields: &[&str]) -> StoreResult<String> {
    fields
        .iter()
        .find_map(|field| {
            value
                .get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
        })
        .ok_or_else(|| {
            StoreError::Conflict(format!(
                "manifest entry must have one of these non-empty string fields: {}",
                fields
                    .iter()
                    .map(|field| format!("`{field}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

#[cfg(feature = "native")]
fn manifest_effect_kind(provider: &Value) -> String {
    provider
        .get("effect_kind")
        .or_else(|| provider.get("core_effect_kind"))
        .or_else(|| provider.get("capability"))
        .or_else(|| provider.get("effect_contract"))
        .or_else(|| provider.get("effect_contract_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("capability.call")
        .to_owned()
}

#[cfg(feature = "native")]
fn skill_view_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillView> {
    Ok(SkillView {
        skill_id: row.get(0)?,
        name: row.get(1)?,
        version: row.get(2)?,
        source: row.get(3)?,
        source_path: row.get(4)?,
        content_hash: row.get(5)?,
        description: row.get(6)?,
        required_capabilities_json: row.get(7)?,
    })
}

#[cfg(feature = "native")]
fn skill_to_json(skill: &SkillView) -> Value {
    json!({
        "skill_id": skill.skill_id,
        "name": skill.name,
        "version": skill.version,
        "source": skill.source,
        "source_path": skill.source_path,
        "content_hash": skill.content_hash,
        "description": skill.description,
        "required_capabilities": serde_json::from_str::<Value>(&skill.required_capabilities_json).unwrap_or(Value::Null),
    })
}

#[cfg(feature = "native")]
fn stable_hash_hex(value: &str) -> String {
    format!("{:016x}", stable_hash(value))
}

/// Execution fingerprint for a run, per spec/execution-contract.md: the
/// materialized input plus the resolved dependency outputs, recorded with the run
/// for replay verification and model-backed change detection. It is deliberately
/// **distinct** from the effect idempotency key (the key is fixed at creation; the
/// fingerprint reflects what the run actually executed against). Downstream effects
/// bake their upstream outputs into `input_json` at lowering, so the materialized
/// input already carries the resolved dependency outputs; the sorted upstream
/// effect ids record the dependency structure that fed the run.
#[cfg(feature = "native")]
fn execution_fingerprint_on(
    connection: &Connection,
    instance_id: &str,
    effect_id: &str,
    salt: Option<&str>,
) -> StoreResult<String> {
    let input_json: String = connection
        .query_row(
            "SELECT input_json FROM effects WHERE instance_id = ?1 AND effect_id = ?2",
            params![instance_id, effect_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_else(|| "{}".to_owned());
    let mut upstream: Vec<String> = connection
        .prepare(
            "SELECT upstream_effect_id FROM effect_dependencies \
             WHERE instance_id = ?1 AND downstream_effect_id = ?2",
        )?
        .query_map(params![instance_id, effect_id], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<_, _>>()?;
    upstream.sort();
    let base = format!("{input_json}|{}", upstream.join(","));
    // A non-empty salt (the runtime-resolved model for `schema.coerce` runs,
    // carried in run metadata under `__fingerprint_model`; DR-0014 amendment)
    // makes the execution fingerprint model-sensitive, so a coercion whose only
    // change is the model re-runs instead of deduping a stale result. An
    // absent/empty salt keeps the fingerprint byte-identical to the pre-salt
    // behaviour for every other effect kind.
    let fingerprint_input = match salt {
        Some(salt) if !salt.is_empty() => format!("{base}|model={salt}"),
        _ => base,
    };
    Ok(stable_hash_hex(&fingerprint_input))
}

/// The reserved run-metadata key carrying the runtime-resolved model for a
/// `schema.coerce` run, folded into the execution fingerprint (DR-0014). A run
/// whose `metadata_json` sets this key gets a model-sensitive fingerprint.
pub const FINGERPRINT_MODEL_METADATA_KEY: &str = "__fingerprint_model";

/// Extract the fingerprint model salt from a run's metadata JSON, if present.
fn fingerprint_salt_from_metadata(metadata_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(metadata_json)
        .ok()?
        .get(FINGERPRINT_MODEL_METADATA_KEY)
        .and_then(|value| value.as_str())
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

#[cfg(feature = "native")]
fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(feature = "native")]
fn count_where(
    connection: &Connection,
    table: &str,
    instance_id: &str,
    extra_predicate: Option<&str>,
) -> StoreResult<i64> {
    let mut sql = format!("SELECT COUNT(*) FROM {table} WHERE instance_id = ?1");
    if let Some(predicate) = extra_predicate {
        sql.push_str(" AND ");
        sql.push_str(predicate);
    }

    connection
        .query_row(&sql, [instance_id], |row| row.get(0))
        .map_err(Into::into)
}

#[cfg(feature = "native")]
fn workflow_invocation_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkflowInvocationView> {
    Ok(WorkflowInvocationView {
        invocation_id: row.get(0)?,
        parent_instance_id: row.get(1)?,
        parent_effect_id: row.get(2)?,
        parent_program_version_id: row.get(3)?,
        parent_revision_epoch: row.get(4)?,
        parent_active_program_version_id: row.get(5)?,
        parent_active_revision_epoch: row.get(6)?,
        child_instance_id: row.get(7)?,
        child_program_version_id: row.get(8)?,
        child_revision_epoch: row.get(9)?,
        child_active_program_version_id: row.get(10)?,
        child_active_revision_epoch: row.get(11)?,
        target_workflow: row.get(12)?,
        input_json: row.get(13)?,
        status: row.get(14)?,
        terminal_event_id: row.get(15)?,
        source_span_json: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
    })
}

#[cfg(feature = "native")]
fn rule_commit_payload(
    commit: RuleCommit<'_>,
    program_version_id: Option<&str>,
    revision_epoch: i64,
) -> StoreResult<String> {
    let facts = commit
        .facts
        .iter()
        .map(|fact| {
            if let Some(source_span_json) = fact.source_span_json {
                serde_json::from_str::<Value>(source_span_json)?;
            }
            Ok(json!({
                "fact_id": fact.fact_id,
                "name": fact.name,
                "key": fact.key,
                "value": serde_json::from_str::<Value>(fact.value_json)?,
                "program_version_id": program_version_id,
                "revision_epoch": revision_epoch,
                "schema_id": fact.schema_id,
                "provenance_class": fact.provenance_class,
                "correlation_id": fact.correlation_id,
                "source_span": fact.source_span_json
                    .map(serde_json::from_str::<Value>)
                    .transpose()?
                    .unwrap_or(Value::Null),
            }))
        })
        .collect::<StoreResult<Vec<_>>>()?;
    let consumed_facts = commit
        .consumed_fact_ids
        .iter()
        .map(|fact_id| json!({ "fact_id": fact_id }))
        .collect::<Vec<_>>();
    let effects = commit
        .effects
        .iter()
        .map(|effect| {
            if let Some(source_span_json) = effect.source_span_json {
                serde_json::from_str::<Value>(source_span_json)?;
            }
            Ok(json!({
                "effect_id": effect.effect_id,
                "kind": effect.kind,
                "target": effect.target,
                "input": serde_json::from_str::<Value>(effect.input_json)?,
                "status": effect.status,
                "program_version_id": program_version_id,
                "revision_epoch": revision_epoch,
                "idempotency_key": effect.idempotency_key,
                "required_capabilities": serde_json::from_str::<Value>(effect.required_capabilities_json)?,
                "profile": effect.profile,
                "correlation_id": effect.correlation_id,
                "source_span": effect
                    .source_span_json
                    .map(serde_json::from_str::<Value>)
                    .transpose()?
                    .unwrap_or(Value::Null),
            }))
        })
        .collect::<StoreResult<Vec<_>>>()?;
    let dependencies = commit
        .dependencies
        .iter()
        .map(|dependency| {
            json!({
                "dependency_id": dependency.dependency_id,
                "upstream_effect_id": dependency.upstream_effect_id,
                "downstream_effect_id": dependency.downstream_effect_id,
                "predicate": dependency.predicate,
            })
        })
        .collect::<Vec<_>>();

    let payload = json!({
        "rule": commit.rule,
        "program_version_id": program_version_id,
        "revision_epoch": revision_epoch,
        "facts": facts,
        "consumed_facts": consumed_facts,
        "effects": effects,
        "dependencies": dependencies,
        "terminal": match commit.terminal {
            Some(terminal) => Some(serde_json::from_str::<Value>(&workflow_terminal_payload(commit, terminal)?)?),
            None => None,
        },
    });
    serde_json::to_string(&payload).map_err(Into::into)
}

#[cfg(feature = "native")]
fn workflow_terminal_payload(
    commit: RuleCommit<'_>,
    terminal: WorkflowTerminal<'_>,
) -> StoreResult<String> {
    let payload = json!({
        "workflow_action": terminal.kind.action(),
        "workflow_status": terminal.kind.instance_status(),
        "terminal_name": terminal.name,
        "payload": serde_json::from_str::<Value>(terminal.payload_json)?,
        "rule": commit.rule,
    });
    serde_json::to_string(&payload).map_err(Into::into)
}

#[cfg(feature = "native")]
fn effect_completion_payload(
    completion: EffectCompletion<'_>,
    diagnostic: Option<&TerminalDiagnosticRecord>,
) -> String {
    json!({
        "effect_id": completion.effect_id,
        "run_id": completion.run_id,
        "provider": completion.provider,
        "worker_id": completion.worker_id,
        "status": completion.status,
        "exit_code": completion.exit_code,
        "summary": completion.summary,
        "metadata": serde_json::from_str::<Value>(completion.metadata_json).unwrap_or(Value::Null),
        "diagnostic": diagnostic.map(terminal_diagnostic_payload),
    })
    .to_string()
}

#[cfg(feature = "native")]
fn terminal_diagnostic_payload(diagnostic: &TerminalDiagnosticRecord) -> Value {
    json!({
        "program_id": diagnostic.program_id,
        "program_version_id": diagnostic.program_version_id,
        "severity": diagnostic.severity.as_str(),
        "code": diagnostic.code,
        "message": diagnostic.message,
        "source_span": diagnostic.source_span_json.as_deref().map(|span| {
            serde_json::from_str::<Value>(span).unwrap_or(Value::Null)
        }),
        "subject_type": diagnostic.subject_type,
        "subject_id": diagnostic.subject_id,
        "assertion_id": diagnostic.assertion_id,
        "evidence_ids": serde_json::from_str::<Value>(&diagnostic.evidence_ids_json)
            .unwrap_or_else(|_| json!([])),
        "artifact_ids": serde_json::from_str::<Value>(&diagnostic.artifact_ids_json)
            .unwrap_or_else(|_| json!([])),
        "causation_id": diagnostic.causation_id,
        "correlation_id": diagnostic.correlation_id,
        "idempotency_key": diagnostic.idempotency_key,
    })
}

#[cfg(feature = "native")]
fn run_start_payload(run: RunStart<'_>, metadata_json: &str) -> String {
    json!({
        "effect_id": run.effect_id,
        "run_id": run.run_id,
        "provider": run.provider,
        "worker_id": run.worker_id,
        "lease_id": run.lease_id,
        "lease_expires_at": run.lease_expires_at,
        "metadata": serde_json::from_str::<Value>(metadata_json).unwrap_or(Value::Null),
    })
    .to_string()
}

#[cfg(feature = "native")]
/// Merge the execution fingerprint into a run's metadata object so it is recorded
/// with the run (in the `run_started` event payload and the projected `runs` row)
/// and reconstructs on replay through the existing metadata flow.
fn inject_execution_fingerprint(metadata_json: &str, fingerprint: &str) -> String {
    let mut value: Value = serde_json::from_str(metadata_json).unwrap_or(Value::Null);
    if !value.is_object() {
        value = json!({});
    }
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "execution_fingerprint".to_owned(),
            Value::String(fingerprint.to_owned()),
        );
    }
    value.to_string()
}

#[cfg(feature = "native")]
fn replay_rule_commit(
    connection: &Connection,
    instance_id: &str,
    event_id: &str,
    payload_json: &str,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let rule = payload
        .get("rule")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let commit_program_version_id = payload.get("program_version_id").and_then(Value::as_str);
    let commit_revision_epoch = payload
        .get("revision_epoch")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    for fact in payload
        .get("facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let fact_id = fact.get("fact_id").and_then(Value::as_str).unwrap_or("");
        let name = fact.get("name").and_then(Value::as_str).unwrap_or("");
        let key = fact.get("key").and_then(Value::as_str).unwrap_or("");
        let value_json = fact
            .get("value")
            .map(Value::to_string)
            .unwrap_or_else(|| "{}".to_owned());
        let source_span_json = fact.get("source_span").map(Value::to_string);
        let program_version_id = fact
            .get("program_version_id")
            .and_then(Value::as_str)
            .or(commit_program_version_id);
        let revision_epoch = fact
            .get("revision_epoch")
            .and_then(Value::as_i64)
            .unwrap_or(commit_revision_epoch);
        let new_fact = NewFact {
            fact_id,
            name,
            key,
            value_json: &value_json,
            schema_id: fact.get("schema_id").and_then(Value::as_str),
            provenance_class: fact
                .get("provenance_class")
                .and_then(Value::as_str)
                .unwrap_or("replayed"),
            correlation_id: fact.get("correlation_id").and_then(Value::as_str),
            source_span_json: source_span_json.as_deref(),
        };
        insert_fact(
            connection,
            instance_id,
            rule,
            event_id,
            program_version_id,
            revision_epoch,
            &new_fact,
        )?;
    }

    let consumed_fact_ids = payload
        .get("consumed_facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|fact| {
            fact.get("fact_id")
                .and_then(Value::as_str)
                .or_else(|| fact.as_str())
        })
        .collect::<Vec<_>>();
    consume_facts(connection, instance_id, &consumed_fact_ids)?;

    for effect in payload
        .get("effects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let effect_id = effect
            .get("effect_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let kind = effect.get("kind").and_then(Value::as_str).unwrap_or("");
        let input_json = effect
            .get("input")
            .map(Value::to_string)
            .unwrap_or_else(|| "{}".to_owned());
        let required_capabilities_json = effect
            .get("required_capabilities")
            .map(Value::to_string)
            .unwrap_or_else(|| "[]".to_owned());
        let source_span_json = effect.get("source_span").map(Value::to_string);
        let program_version_id = effect
            .get("program_version_id")
            .and_then(Value::as_str)
            .or(commit_program_version_id);
        let revision_epoch = effect
            .get("revision_epoch")
            .and_then(Value::as_i64)
            .unwrap_or(commit_revision_epoch);
        let new_effect = NewEffect {
            timeout_seconds: None,
            effect_id,
            kind,
            target: effect.get("target").and_then(Value::as_str),
            input_json: &input_json,
            status: effect
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("queued"),
            idempotency_key: effect
                .get("idempotency_key")
                .and_then(Value::as_str)
                .unwrap_or(""),
            required_capabilities_json: &required_capabilities_json,
            profile: effect.get("profile").and_then(Value::as_str),
            correlation_id: effect.get("correlation_id").and_then(Value::as_str),
            source_span_json: source_span_json.as_deref(),
        };
        insert_effect(
            connection,
            instance_id,
            rule,
            event_id,
            program_version_id,
            revision_epoch,
            &new_effect,
        )?;
    }

    for dependency in payload
        .get("dependencies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let new_dependency = NewEffectDependency {
            dependency_id: dependency
                .get("dependency_id")
                .and_then(Value::as_str)
                .unwrap_or(""),
            upstream_effect_id: dependency
                .get("upstream_effect_id")
                .and_then(Value::as_str)
                .unwrap_or(""),
            downstream_effect_id: dependency
                .get("downstream_effect_id")
                .and_then(Value::as_str)
                .unwrap_or(""),
            predicate: dependency
                .get("predicate")
                .and_then(Value::as_str)
                .unwrap_or("succeeds"),
        };
        insert_effect_dependency(connection, instance_id, rule, &new_dependency)?;
    }

    Ok(())
}

#[cfg(feature = "native")]
fn replay_fact_derived(
    connection: &Connection,
    instance_id: &str,
    event_id: &str,
    source: &str,
    payload_json: &str,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let fact_id = payload.get("fact_id").and_then(Value::as_str).unwrap_or("");
    let name = payload.get("name").and_then(Value::as_str).unwrap_or("");
    let key = payload.get("key").and_then(Value::as_str).unwrap_or("");
    if fact_id.is_empty() || name.is_empty() || key.is_empty() {
        return Ok(());
    }

    let value_json = payload
        .get("value")
        .cloned()
        .unwrap_or(Value::Null)
        .to_string();
    let fact = NewFact {
        fact_id,
        name,
        key,
        value_json: &value_json,
        schema_id: payload.get("schema_id").and_then(Value::as_str),
        provenance_class: payload
            .get("provenance_class")
            .and_then(Value::as_str)
            .unwrap_or("derived"),
        correlation_id: payload.get("correlation_id").and_then(Value::as_str),
        source_span_json: None,
    };
    let (program_version_id, revision_epoch) = active_revision_on(connection, instance_id)?;
    insert_fact(
        connection,
        instance_id,
        source,
        event_id,
        program_version_id.as_deref(),
        revision_epoch,
        &fact,
    )
}

#[cfg(feature = "native")]
fn replay_workflow_terminal(
    connection: &Connection,
    instance_id: &str,
    event_id: &str,
    event_type: &str,
    payload_json: &str,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let status = payload
        .get("workflow_status")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if event_type == "workflow.failed" {
                "failed"
            } else {
                "completed"
            }
        });
    let terminal_name = payload
        .get("terminal_name")
        .and_then(Value::as_str)
        .unwrap_or(event_type);
    connection.execute(
        r#"
        UPDATE instances
        SET status = ?1,
            last_event_id = ?2,
            last_error = CASE WHEN ?1 = 'failed' THEN ?3 ELSE last_error END,
            updated_at = CURRENT_TIMESTAMP,
            completed_at = CURRENT_TIMESTAMP
        WHERE instance_id = ?4
        "#,
        params![status, event_id, terminal_name, instance_id],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn replay_instance_transition(
    connection: &Connection,
    instance_id: &str,
    event_id: &str,
    payload_json: &str,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let status = payload.get("status").and_then(Value::as_str).unwrap_or("");
    if status.is_empty() {
        return Ok(());
    }
    connection.execute(
        r#"
        UPDATE instances
        SET status = ?1,
            last_event_id = ?2,
            last_error = ?3,
            updated_at = CURRENT_TIMESTAMP,
            completed_at = CASE
                WHEN ?1 IN ('completed', 'cancelled') THEN CURRENT_TIMESTAMP
                ELSE completed_at
            END
        WHERE instance_id = ?4
        "#,
        params![
            status,
            event_id,
            payload.get("reason").and_then(Value::as_str),
            instance_id,
        ],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn replay_revision_activation(
    connection: &Connection,
    instance_id: &str,
    event_id: &str,
    payload_json: &str,
    idempotency_key: Option<&str>,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let revision_id = payload
        .get("revision_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let from_version_id = payload
        .get("from_version_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let to_version_id = payload
        .get("to_version_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let epoch = payload
        .get("to_epoch")
        .and_then(Value::as_i64)
        .or_else(|| payload.get("revision_epoch").and_then(Value::as_i64))
        .unwrap_or(0);
    if revision_id.is_empty() || from_version_id.is_empty() || to_version_id.is_empty() {
        return Ok(());
    }
    let activation_policy_json = payload
        .get("activation_policy")
        .map(Value::to_string)
        .unwrap_or_else(|| "{}".to_owned());
    let cancellation_policy = payload
        .get("cancellation_policy")
        .and_then(Value::as_str)
        .unwrap_or("keep");
    connection.execute(
        r#"
        INSERT INTO instance_revisions (
            revision_id,
            instance_id,
            epoch,
            from_version_id,
            to_version_id,
            activated_by_event_id,
            activation_policy_json,
            cancellation_policy,
            status,
            idempotency_key
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9)
        ON CONFLICT(revision_id) DO NOTHING
        "#,
        params![
            revision_id,
            instance_id,
            epoch,
            from_version_id,
            to_version_id,
            event_id,
            activation_policy_json,
            cancellation_policy,
            idempotency_key,
        ],
    )?;
    connection.execute(
        r#"
        UPDATE instances
        SET version_id = ?1,
            revision_epoch = ?2,
            last_event_id = ?3,
            updated_at = CURRENT_TIMESTAMP
        WHERE instance_id = ?4
        "#,
        params![to_version_id, epoch, event_id, instance_id],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn replay_run_started(
    connection: &Connection,
    instance_id: &str,
    payload_json: &str,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let effect_id = payload
        .get("effect_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let run_id = payload.get("run_id").and_then(Value::as_str).unwrap_or("");
    let lease_id = payload
        .get("lease_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if effect_id.is_empty() || run_id.is_empty() || lease_id.is_empty() {
        return Ok(());
    }
    let provider = payload
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("replay");
    let worker_id = payload
        .get("worker_id")
        .and_then(Value::as_str)
        .unwrap_or("replay");
    let lease_expires_at = payload
        .get("lease_expires_at")
        .and_then(Value::as_str)
        .unwrap_or("");
    let metadata_json = payload
        .get("metadata")
        .map(Value::to_string)
        .unwrap_or_else(|| "{}".to_owned());

    connection.execute(
        r#"
        UPDATE effects
        SET status = 'running',
            policy_block_reason = NULL,
            updated_at = CURRENT_TIMESTAMP
        WHERE instance_id = ?1
          AND effect_id = ?2
          AND status NOT IN ('completed', 'failed', 'timed_out', 'cancelled')
        "#,
        params![instance_id, effect_id],
    )?;
    connection.execute(
        r#"
        INSERT INTO runs (
            run_id,
            effect_id,
            instance_id,
            provider,
            worker_id,
            status,
            metadata_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)
        ON CONFLICT(run_id) DO UPDATE SET
            effect_id = excluded.effect_id,
            instance_id = excluded.instance_id,
            provider = excluded.provider,
            worker_id = excluded.worker_id,
            status = 'running',
            completed_at = NULL,
            exit_code = NULL,
            summary = NULL,
            metadata_json = excluded.metadata_json
        "#,
        params![
            run_id,
            effect_id,
            instance_id,
            provider,
            worker_id,
            metadata_json,
        ],
    )?;
    connection.execute(
        r#"
        INSERT INTO leases (
            lease_id,
            run_id,
            effect_id,
            instance_id,
            worker_id,
            status,
            expires_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6)
        ON CONFLICT(lease_id) DO UPDATE SET
            run_id = excluded.run_id,
            effect_id = excluded.effect_id,
            instance_id = excluded.instance_id,
            worker_id = excluded.worker_id,
            status = 'active',
            expires_at = excluded.expires_at,
            released_at = NULL
        "#,
        params![
            lease_id,
            run_id,
            effect_id,
            instance_id,
            worker_id,
            lease_expires_at,
        ],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn replay_effect_terminal(
    connection: &Connection,
    instance_id: &str,
    event_id: &str,
    payload_json: &str,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let effect_id = payload
        .get("effect_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let run_id = payload.get("run_id").and_then(Value::as_str).unwrap_or("");
    if effect_id.is_empty() {
        return Ok(());
    }
    let status = payload
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let provider = payload
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("replay");
    let worker_id = payload
        .get("worker_id")
        .and_then(Value::as_str)
        .unwrap_or("replay");
    let metadata_json = payload
        .get("metadata")
        .map(Value::to_string)
        .unwrap_or_else(|| "{}".to_owned());
    if !run_id.is_empty() {
        connection.execute(
            r#"
            INSERT INTO runs (
                run_id,
                effect_id,
                instance_id,
                provider,
                worker_id,
                status,
                completed_at,
                exit_code,
                summary,
                metadata_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP, ?7, ?8, ?9)
            ON CONFLICT(run_id) DO UPDATE SET
                effect_id = excluded.effect_id,
                instance_id = excluded.instance_id,
                provider = excluded.provider,
                worker_id = excluded.worker_id,
                status = excluded.status,
                completed_at = CURRENT_TIMESTAMP,
                exit_code = excluded.exit_code,
                summary = excluded.summary,
                metadata_json = excluded.metadata_json
            "#,
            params![
                run_id,
                effect_id,
                instance_id,
                provider,
                worker_id,
                status,
                payload.get("exit_code").and_then(Value::as_i64),
                payload.get("summary").and_then(Value::as_str),
                metadata_json,
            ],
        )?;
        connection.execute(
            r#"
            UPDATE leases
            SET status = 'released',
                released_at = CURRENT_TIMESTAMP
            WHERE run_id = ?1
              AND effect_id = ?2
              AND instance_id = ?3
              AND status = 'active'
            "#,
            params![run_id, effect_id, instance_id],
        )?;
    }
    connection.execute(
        r#"
        UPDATE effects
        SET status = ?1,
            updated_at = CURRENT_TIMESTAMP
        WHERE effect_id = ?2
          AND instance_id = ?3
        "#,
        params![status, effect_id, instance_id],
    )?;
    mark_cancellation_requests_terminal_on(connection, instance_id, effect_id, event_id)?;
    satisfy_dependencies_on(connection, instance_id)?;
    Ok(())
}

#[cfg(feature = "native")]
fn replay_effect_cancelled(
    connection: &Connection,
    instance_id: &str,
    event_id: &str,
    payload_json: &str,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let effect_id = payload
        .get("effect_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if effect_id.is_empty() {
        return Ok(());
    }
    connection.execute(
        r#"
        UPDATE effects
        SET status = 'cancelled',
            updated_at = CURRENT_TIMESTAMP
        WHERE instance_id = ?1
          AND effect_id = ?2
          AND status NOT IN ('completed', 'failed', 'timed_out', 'cancelled')
        "#,
        params![instance_id, effect_id],
    )?;
    mark_cancellation_requests_terminal_on(connection, instance_id, effect_id, event_id)?;
    satisfy_dependencies_on(connection, instance_id)?;
    Ok(())
}

#[cfg(feature = "native")]
fn replay_lease_expired(
    connection: &Connection,
    instance_id: &str,
    payload_json: &str,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let lease_id = payload
        .get("lease_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let run_id = payload.get("run_id").and_then(Value::as_str).unwrap_or("");
    let effect_id = payload
        .get("effect_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if lease_id.is_empty() || run_id.is_empty() || effect_id.is_empty() {
        return Ok(());
    }
    connection.execute(
        r#"
        UPDATE leases
        SET status = 'expired',
            released_at = CURRENT_TIMESTAMP
        WHERE lease_id = ?1
        "#,
        [lease_id],
    )?;
    connection.execute(
        r#"
        UPDATE runs
        SET status = 'lease_expired',
            completed_at = CURRENT_TIMESTAMP
        WHERE run_id = ?1
          AND status = 'running'
        "#,
        [run_id],
    )?;
    connection.execute(
        r#"
        UPDATE effects
        SET status = 'queued',
            updated_at = CURRENT_TIMESTAMP
        WHERE instance_id = ?1
          AND effect_id = ?2
          AND status = 'running'
        "#,
        params![instance_id, effect_id],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn replay_cancellation_request(
    connection: &Connection,
    instance_id: &str,
    event_id: &str,
    payload_json: &str,
    idempotency_key: Option<&str>,
    causation_event_id: Option<&str>,
) -> StoreResult<()> {
    let payload: Value = serde_json::from_str(payload_json)?;
    let request_id = payload
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let effect_id = payload
        .get("effect_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if request_id.is_empty() || effect_id.is_empty() {
        return Ok(());
    }
    let causation_event_id = match causation_event_id {
        Some(candidate)
            if connection
                .query_row(
                    "SELECT 1 FROM events WHERE instance_id = ?1 AND event_id = ?2",
                    params![instance_id, candidate],
                    |_| Ok(()),
                )
                .optional()?
                .is_some() =>
        {
            Some(candidate)
        }
        _ => Some(event_id),
    };
    connection.execute(
        r#"
        INSERT INTO effect_cancellation_requests (
            request_id,
            instance_id,
            effect_id,
            revision_id,
            reason,
            requested_by,
            causation_event_id,
            status,
            idempotency_key
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'requested', ?8)
        ON CONFLICT(request_id) DO NOTHING
        "#,
        params![
            request_id,
            instance_id,
            effect_id,
            payload.get("revision_id").and_then(Value::as_str),
            payload.get("reason").and_then(Value::as_str),
            payload
                .get("requested_by")
                .and_then(Value::as_str)
                .unwrap_or("replay"),
            causation_event_id,
            idempotency_key,
        ],
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn apply_migrations(connection: &mut Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )?;

    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for migration in MIGRATIONS {
        let applied = transaction
            .query_row(
                "SELECT 1 FROM schema_migrations WHERE version = ?1",
                [migration.version],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if applied {
            continue;
        }

        transaction.execute_batch(migration.sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![migration.version, migration.name],
        )?;
    }
    transaction.commit()?;
    ensure_fact_schema(connection)?;
    ensure_diagnostics_schema(connection)?;
    ensure_workflow_invocation_schema(connection)?;
    ensure_revision_schema(connection)?;
    ensure_instance_authority_schema(connection)?;
    ensure_workspace_schema(connection)?;
    ensure_effect_time_columns(connection)?;
    ensure_skill_body_column(connection)?;
    ensure_lookup_indexes(connection)?;
    Ok(())
}

/// Idempotent patch (context-assembly Phase 2, Decision 3): give `skills` a `body`
/// column carrying the content-addressed SKILL.md bytes. Applied on every open, so
/// a store from before the column gains it; a no-op once present.
#[cfg(feature = "native")]
fn ensure_skill_body_column(connection: &Connection) -> StoreResult<()> {
    let skills_table_exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'skills'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !skills_table_exists {
        return Ok(());
    }
    if !column_exists(connection, "skills", "body")? {
        connection.execute(
            "ALTER TABLE skills ADD COLUMN body TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    Ok(())
}

#[cfg(feature = "native")]
fn ensure_effect_time_columns(connection: &Connection) -> StoreResult<()> {
    let effects_table_exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'effects'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !effects_table_exists {
        return Ok(());
    }
    if !column_exists(connection, "effects", "timeout_seconds")? {
        connection.execute("ALTER TABLE effects ADD COLUMN timeout_seconds INTEGER", [])?;
    }
    Ok(())
}

#[cfg(feature = "native")]
fn ensure_lookup_indexes(connection: &Connection) -> StoreResult<()> {
    {
        let (table, statement) = (
            "facts",
            "CREATE INDEX IF NOT EXISTS idx_facts_instance_name ON facts(instance_id, name)",
        );
        let table_exists = connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if table_exists {
            connection.execute(statement, [])?;
        }
    }
    Ok(())
}

#[cfg(feature = "native")]
fn ensure_instance_authority_schema(connection: &Connection) -> StoreResult<()> {
    if !table_exists(connection, "instances")? {
        return Ok(());
    }
    if !column_exists(connection, "instances", "workflow_principal")? {
        connection.execute(
            "ALTER TABLE instances ADD COLUMN workflow_principal TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !column_exists(connection, "instances", "effective_authority")? {
        connection.execute(
            "ALTER TABLE instances ADD COLUMN effective_authority TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    Ok(())
}

#[cfg(feature = "native")]
fn ensure_workspace_schema(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS workspaces (
            workspace_id TEXT PRIMARY KEY,
            instance_id TEXT REFERENCES instances(instance_id),
            effect_id TEXT REFERENCES effects(effect_id),
            run_id TEXT REFERENCES runs(run_id),
            provider TEXT,
            policy TEXT NOT NULL,
            uri TEXT NOT NULL,
            status TEXT NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(instance_id, effect_id, run_id, policy)
        );
        "#,
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn ensure_revision_schema(connection: &Connection) -> StoreResult<()> {
    if table_exists(connection, "instances")?
        && !column_exists(connection, "instances", "revision_epoch")?
    {
        connection.execute(
            "ALTER TABLE instances ADD COLUMN revision_epoch INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if table_exists(connection, "effects")?
        && !column_exists(connection, "effects", "program_version_id")?
    {
        connection.execute("ALTER TABLE effects ADD COLUMN program_version_id TEXT", [])?;
    }
    if table_exists(connection, "effects")?
        && !column_exists(connection, "effects", "revision_epoch")?
    {
        connection.execute(
            "ALTER TABLE effects ADD COLUMN revision_epoch INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS instance_revisions (
            revision_id TEXT PRIMARY KEY,
            instance_id TEXT NOT NULL REFERENCES instances(instance_id),
            epoch INTEGER NOT NULL,
            from_version_id TEXT NOT NULL REFERENCES program_versions(version_id),
            to_version_id TEXT NOT NULL REFERENCES program_versions(version_id),
            activated_by_event_id TEXT NOT NULL REFERENCES events(event_id),
            activation_policy_json TEXT NOT NULL DEFAULT '{}',
            cancellation_policy TEXT NOT NULL,
            status TEXT NOT NULL,
            idempotency_key TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            activated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(instance_id, epoch)
        );

        CREATE UNIQUE INDEX IF NOT EXISTS instance_revisions_instance_idempotency_key_idx
            ON instance_revisions(instance_id, idempotency_key)
            WHERE idempotency_key IS NOT NULL;

        CREATE TABLE IF NOT EXISTS effect_cancellation_requests (
            request_id TEXT PRIMARY KEY,
            instance_id TEXT NOT NULL REFERENCES instances(instance_id),
            effect_id TEXT NOT NULL REFERENCES effects(effect_id),
            revision_id TEXT REFERENCES instance_revisions(revision_id),
            reason TEXT,
            requested_by TEXT NOT NULL,
            causation_event_id TEXT REFERENCES events(event_id),
            status TEXT NOT NULL,
            idempotency_key TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            resolved_by_event_id TEXT,
            UNIQUE(instance_id, effect_id, revision_id)
        );

        CREATE UNIQUE INDEX IF NOT EXISTS effect_cancellation_requests_instance_idempotency_key_idx
            ON effect_cancellation_requests(instance_id, idempotency_key)
            WHERE idempotency_key IS NOT NULL;
        "#,
    )?;
    if table_exists(connection, "effects")? && table_exists(connection, "instances")? {
        connection.execute(
            r#"
            UPDATE effects
            SET program_version_id = (
                SELECT version_id
                FROM instances
                WHERE instances.instance_id = effects.instance_id
            )
            WHERE program_version_id IS NULL
            "#,
            [],
        )?;
    }
    Ok(())
}

#[cfg(feature = "native")]
fn ensure_workflow_invocation_schema(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS workflow_invocations (
            invocation_id TEXT PRIMARY KEY,
            parent_instance_id TEXT NOT NULL,
            parent_effect_id TEXT NOT NULL,
            parent_program_version_id TEXT,
            parent_revision_epoch INTEGER NOT NULL DEFAULT 0,
            child_instance_id TEXT NOT NULL,
            child_program_version_id TEXT,
            child_revision_epoch INTEGER,
            target_workflow TEXT NOT NULL,
            input_json TEXT NOT NULL DEFAULT '{}',
            status TEXT NOT NULL DEFAULT 'running',
            terminal_event_id TEXT,
            source_span_json TEXT,
            idempotency_key TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )?;
    for (column, definition) in [
        ("parent_program_version_id", "TEXT"),
        ("parent_revision_epoch", "INTEGER NOT NULL DEFAULT 0"),
        ("child_program_version_id", "TEXT"),
        ("child_revision_epoch", "INTEGER"),
        ("status", "TEXT NOT NULL DEFAULT 'running'"),
        ("terminal_event_id", "TEXT"),
        ("source_span_json", "TEXT"),
        ("updated_at", "TEXT"),
    ] {
        if !column_exists(connection, "workflow_invocations", column)? {
            connection.execute(
                &format!("ALTER TABLE workflow_invocations ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
    }
    Ok(())
}

#[cfg(feature = "native")]
fn ensure_fact_schema(connection: &Connection) -> StoreResult<()> {
    let facts_table_exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'facts'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !facts_table_exists {
        return Ok(());
    }
    if !column_exists(connection, "facts", "consumed_at")? {
        connection.execute("ALTER TABLE facts ADD COLUMN consumed_at TEXT", [])?;
    }
    for (column, definition) in [
        ("program_version_id", "TEXT"),
        ("revision_epoch", "INTEGER NOT NULL DEFAULT 0"),
        ("source_span_json", "TEXT"),
    ] {
        if !column_exists(connection, "facts", column)? {
            connection.execute(
                &format!("ALTER TABLE facts ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
    }
    Ok(())
}

#[cfg(feature = "native")]
fn ensure_diagnostics_schema(connection: &Connection) -> StoreResult<()> {
    for (column, definition) in [
        ("program_id", "TEXT"),
        ("program_version_id", "TEXT"),
        ("subject_type", "TEXT"),
        ("subject_id", "TEXT"),
        ("event_id", "TEXT"),
        ("effect_id", "TEXT"),
        ("run_id", "TEXT"),
        ("assertion_id", "TEXT"),
        ("evidence_ids_json", "TEXT NOT NULL DEFAULT '[]'"),
        ("artifact_ids_json", "TEXT NOT NULL DEFAULT '[]'"),
        ("causation_id", "TEXT"),
        ("correlation_id", "TEXT"),
        ("idempotency_key", "TEXT"),
    ] {
        if !column_exists(connection, "diagnostics", column)? {
            connection.execute(
                &format!("ALTER TABLE diagnostics ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
    }

    connection.execute_batch(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS diagnostics_instance_idempotency_key_idx
            ON diagnostics(instance_id, idempotency_key)
            WHERE instance_id IS NOT NULL AND idempotency_key IS NOT NULL;

        CREATE UNIQUE INDEX IF NOT EXISTS diagnostics_program_idempotency_key_idx
            ON diagnostics(program_id, idempotency_key)
            WHERE instance_id IS NULL
              AND program_id IS NOT NULL
              AND program_version_id IS NULL
              AND idempotency_key IS NOT NULL;

        CREATE UNIQUE INDEX IF NOT EXISTS diagnostics_version_idempotency_key_idx
            ON diagnostics(program_version_id, idempotency_key)
            WHERE instance_id IS NULL AND program_version_id IS NOT NULL AND idempotency_key IS NOT NULL;
        "#,
    )?;
    Ok(())
}

#[cfg(feature = "native")]
fn column_exists(connection: &Connection, table: &str, column: &str) -> StoreResult<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<result::Result<Vec<_>, _>>()?;
    Ok(columns.iter().any(|name| name == column))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_scaffold_links_to_core() {
        assert_eq!(store_stage(), "stage-0-skeleton");
    }

    /// Drive the store through the `RuntimeStore` trait as a `&dyn` object:
    /// proves the trait is object-safe (a boxed durable-object backend is legal)
    /// and that its methods delegate to the inherent methods — the contract a
    /// DO-backed store satisfies. Spot-checks representative reads against known
    /// post-migration state.
    #[test]
    fn runtime_store_trait_seam_is_object_safe_and_faithful() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let runtime: &dyn RuntimeStore = &store;
        assert_eq!(runtime.schema_version().expect("version"), 1);
        assert!(runtime.table_exists("events").expect("table exists"));
        assert!(!runtime
            .fact_exists("no_such_instance", "no_such_fact")
            .expect("fact query"));
    }

    #[test]
    fn migrations_create_runtime_tables() {
        let store = SqliteStore::open_in_memory().expect("store opens");

        assert_eq!(store.schema_version().expect("version loads"), 1);
        for table in [
            "programs",
            "program_versions",
            "instances",
            "instance_revisions",
            "events",
            "facts",
            "effects",
            "effect_cancellation_requests",
            "effect_dependencies",
            "runs",
            "leases",
            "artifacts",
            "evidence",
            "evidence_links",
            "diagnostics",
            "package_registrations",
            "capability_schemas",
            "effect_providers",
            "profiles",
            "skills",
            "skill_attachments",
            "capability_bindings",
            "inbox_items",
            "compute_result_cache",
        ] {
            assert!(store.table_exists(table).expect("table lookup"), "{table}");
        }
    }

    /// Delta-kernel result cache (compute plane P8-1): a recorded content key
    /// is served back on lookup, and insertion is first-writer-wins — the
    /// existing entry stays canonical, mirroring the Maude model's
    /// at-most-one-run-per-key invariant (compute-result-cache.maude I1).
    #[test]
    fn compute_result_cache_roundtrip_and_first_writer_wins() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        assert_eq!(
            store
                .lookup_compute_result("key-1")
                .expect("lookup on empty cache"),
            None
        );

        let inserted = store
            .record_compute_result(ComputeResultRegistration {
                content_key: "key-1",
                effect_kind: "exec.command",
                result_json: r#"{"exit_code":0,"stdout":"first"}"#,
                source_instance_id: "instance-a",
                source_effect_id: "effect-1",
            })
            .expect("first record");
        assert!(inserted, "first writer inserts");

        let replay = store
            .record_compute_result(ComputeResultRegistration {
                content_key: "key-1",
                effect_kind: "exec.command",
                result_json: r#"{"exit_code":0,"stdout":"second"}"#,
                source_instance_id: "instance-b",
                source_effect_id: "effect-2",
            })
            .expect("second record");
        assert!(!replay, "second writer is ignored");

        let hit = store
            .lookup_compute_result("key-1")
            .expect("lookup")
            .expect("entry exists");
        assert_eq!(hit.result_json, r#"{"exit_code":0,"stdout":"first"}"#);
        assert_eq!(hit.source_instance_id, "instance-a");
        assert_eq!(hit.source_effect_id, "effect-1");
        assert_eq!(hit.effect_kind, "exec.command");
        assert_eq!(
            store.lookup_compute_result("key-2").expect("other key"),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn opening_file_store_hardens_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "whipplescript-store-permissions-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after epoch")
                .as_nanos()
        ));
        fs::write(&path, "").expect("precreated store file writes");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("precreated permissions set");

        let store = SqliteStore::open(&path).expect("file store opens");
        let mode = fs::metadata(&path)
            .expect("store metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        drop(store);
        fs::remove_file(path).expect("store file removes");
    }

    #[test]
    fn append_events_assigns_per_instance_sequences() {
        let store = SqliteStore::open_in_memory().expect("store opens");

        let first = store
            .append_event(new_event("instance-a", "external.started", None))
            .expect("first event appends");
        let second = store
            .append_event(new_event("instance-a", "rule.fired", None))
            .expect("second event appends");
        let other = store
            .append_event(new_event("instance-b", "external.started", None))
            .expect("other instance event appends");

        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, 2);
        assert_eq!(other.sequence, 1);
        assert_ne!(first.event_id, second.event_id);
    }

    #[test]
    fn duplicate_event_idempotency_key_is_rejected_per_instance() {
        let store = SqliteStore::open_in_memory().expect("store opens");

        store
            .append_event(new_event("instance-a", "external.started", Some("start")))
            .expect("first event appends");
        let duplicate =
            store.append_event(new_event("instance-a", "external.started", Some("start")));
        store
            .append_event(new_event("instance-b", "external.started", Some("start")))
            .expect("same key on another instance is allowed");

        assert!(duplicate.is_err());
    }

    #[test]
    fn derives_fact_atomically_from_event() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let event = store
            .derive_fact(DerivedFact {
                instance_id: "instance-a",
                fact: test_fact("fact-started", "pattern:started", "started"),
                source: "external",
                causation_id: None,
                idempotency_key: Some("derive-started"),
            })
            .expect("fact derives");

        assert_eq!(event.sequence, 1);
        assert!(store
            .fact_exists("instance-a", "pattern:started")
            .expect("fact query"));
        assert_eq!(row_count(&store, "facts"), 1);
    }

    #[test]
    fn admit_fact_batch_admits_atomically_and_is_per_row_idempotent() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let rows = [
            FactBatchRow {
                fact_id: "imp:row:0",
                key: "0",
                value_json: r#"{"title":"a"}"#,
            },
            FactBatchRow {
                fact_id: "imp:row:1",
                key: "1",
                value_json: r#"{"title":"b"}"#,
            },
        ];
        let batch = FactBatch {
            instance_id: "instance-a",
            source: "kernel",
            causation_id: None,
            correlation_id: Some("effect-1"),
            schema_name: "IssueRow",
            schema_id: Some("IssueRow"),
            rows: &rows,
        };

        let first = store.admit_fact_batch(batch).expect("batch admits");
        assert_eq!(first.admitted, 2);
        assert_eq!(first.skipped, 0);
        assert_eq!(row_count(&store, "facts"), 2);

        // Re-admitting the same batch admits nothing (each derived key is already
        // present) -- the idempotency the Maude importRow model proves.
        let second = store.admit_fact_batch(batch).expect("re-admit");
        assert_eq!(second.admitted, 0);
        assert_eq!(second.skipped, 2);
        assert_eq!(row_count(&store, "facts"), 2);

        // A batch overlapping the admitted set admits only the fresh rows.
        let overlap_rows = [
            rows[1],
            FactBatchRow {
                fact_id: "imp:row:2",
                key: "2",
                value_json: r#"{"title":"c"}"#,
            },
        ];
        let overlap = FactBatch {
            rows: &overlap_rows,
            ..batch
        };
        let third = store.admit_fact_batch(overlap).expect("overlap admits");
        assert_eq!(third.admitted, 1);
        assert_eq!(third.skipped, 1);
        assert_eq!(row_count(&store, "facts"), 3);

        let facts = store.list_facts("instance-a").expect("facts");
        assert_eq!(
            facts.iter().filter(|fact| fact.name == "IssueRow").count(),
            3
        );
    }

    #[test]
    fn creates_program_versions_and_instances() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");

        let version = store
            .create_program_version(test_program_version("Ralph", "source-1", "ir-1"))
            .expect("program version creates");
        let same_version = store
            .create_program_version(test_program_version("Ralph", "source-1", "ir-1"))
            .expect("matching program version reuses existing row");
        let next_version = store
            .create_program_version(test_program_version("Ralph", "source-2", "ir-2"))
            .expect("second program version creates");
        let instance = store
            .create_instance_with_authority(
                NewInstance {
                    program_id: &version.program_id,
                    version_id: &version.version_id,
                    input_json: r#"{"issue":"one"}"#,
                },
                NewInstanceAuthority {
                    workflow_principal: "workflow:local/Ralph",
                    effective_authority_json: r#"["workflow:local/Ralph"]"#,
                },
            )
            .expect("instance creates");

        assert_eq!(version.version_id, same_version.version_id);
        assert_eq!(version.program_id, next_version.program_id);
        assert_ne!(version.version_id, next_version.version_id);
        assert!(instance.instance_id.starts_with("ins_"));
        assert_eq!(instance.status, "running");
        assert_eq!(row_count(&store, "programs"), 1);
        assert_eq!(row_count(&store, "program_versions"), 2);
        assert_eq!(row_count(&store, "instances"), 1);
    }

    #[test]
    fn rule_commit_persists_event_fact_effect_and_dependency_atomically() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let facts = [test_fact("fact-ready", "issue", "issue-1")];
        let effects = [
            test_effect("claim", "loft.claim", "rule=start;effect=claim"),
            test_effect("tell", "agent.tell", "rule=start;effect=tell"),
        ];
        let dependencies = [NewEffectDependency {
            dependency_id: "dep-claim-tell",
            upstream_effect_id: "claim",
            downstream_effect_id: "tell",
            predicate: "succeeds",
        }];

        let event = store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &facts,
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &dependencies,
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        assert_eq!(event.sequence, 1);
        assert_eq!(row_count(&store, "events"), 1);
        assert_eq!(row_count(&store, "facts"), 1);
        assert_eq!(row_count(&store, "effects"), 2);
        assert_eq!(row_count(&store, "effect_dependencies"), 1);
    }

    #[test]
    fn rule_commit_with_workflow_terminal_updates_instance_atomically() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("Terminal", "source", "ir"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        let event = store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "finish",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[],
                dependencies: &[],
                terminal: Some(WorkflowTerminal {
                    kind: WorkflowTerminalKind::Completed,
                    name: "result",
                    payload_json: r#"{"status":"ok"}"#,
                    idempotency_key: Some("workflow-complete-result"),
                }),
                idempotency_key: Some("commit-finish"),
            })
            .expect("terminal commit succeeds");

        assert_eq!(event.sequence, 1);
        assert_eq!(instance_status(&store, &instance.instance_id), "completed");
        let event_type = store
            .connection
            .query_row(
                "SELECT event_type FROM events WHERE instance_id = ?1 AND sequence = 2",
                [&instance.instance_id],
                |row| row.get::<_, String>(0),
            )
            .expect("terminal event type");
        assert_eq!(event_type, "workflow.completed");

        let duplicate = store.commit_rule(RuleCommit {
            instance_id: &instance.instance_id,
            rule: "again",
            trigger_event_id: None,
            facts: &[],
            consumed_fact_ids: &[],
            effects: &[],
            dependencies: &[],
            terminal: None,
            idempotency_key: Some("commit-again"),
        });
        assert!(matches!(duplicate, Err(StoreError::Conflict(_))));
    }

    #[test]
    fn failed_rule_commit_rolls_back_partial_writes() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [
            test_effect("same-effect", "loft.claim", "rule=bad;effect=one"),
            test_effect("same-effect", "agent.tell", "rule=bad;effect=two"),
        ];
        let result = store.commit_rule(RuleCommit {
            instance_id: "instance-a",
            rule: "bad",
            trigger_event_id: None,
            facts: &[],
            consumed_fact_ids: &[],
            effects: &effects,
            dependencies: &[],
            terminal: None,
            idempotency_key: Some("bad-commit"),
        });

        assert!(result.is_err());
        assert_eq!(row_count(&store, "events"), 0);
        assert_eq!(row_count(&store, "effects"), 0);
    }

    #[test]
    fn pending_effect_kinds_lists_nonterminal_and_excludes_settled() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[
                    test_effect("t", "agent.tell", "rule=start;effect=t"),
                    test_effect("s", "signal.emit", "rule=start;effect=s"),
                ],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        // Both queued effects are reported.
        let mut kinds = store.pending_effect_kinds().expect("pending kinds");
        kinds.sort();
        assert_eq!(
            kinds,
            vec!["agent.tell".to_owned(), "signal.emit".to_owned()]
        );

        // Settle one; a terminal effect drops out of the pending set.
        store
            .connection
            .execute(
                "UPDATE effects SET status = 'completed' WHERE effect_id = 't'",
                [],
            )
            .expect("settle effect");
        assert_eq!(
            store.pending_effect_kinds().expect("pending kinds"),
            vec!["signal.emit".to_owned()]
        );
    }

    #[test]
    fn block_effect_binding_is_idempotent_and_recoverable() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "rule=start;effect=tell")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        let effect = |store: &SqliteStore| {
            store
                .list_effects("instance-a")
                .expect("effects list")
                .into_iter()
                .find(|e| e.effect_id == "tell")
                .expect("tell effect")
        };
        let blocked_event_count = |store: &SqliteStore| -> i64 {
            store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE instance_id = 'instance-a' \
                     AND event_type = 'effect.blocked'",
                    [],
                    |row| row.get(0),
                )
                .expect("count")
        };
        assert_eq!(effect(&store).status, "queued");

        // First binding block: queued -> blocked, categorized, one event.
        let first = store
            .block_effect_binding(
                "instance-a",
                "tell",
                "credentials",
                "missing credentials_ref",
            )
            .expect("first block");
        let e = effect(&store);
        assert_eq!(e.status, "blocked");
        assert_eq!(e.policy_block_category.as_deref(), Some("credentials"));
        assert_eq!(
            e.policy_block_reason.as_deref(),
            Some("missing credentials_ref")
        );
        assert_eq!(blocked_event_count(&store), 1);

        // Idempotent re-block (same category): no new event, returns the same one.
        let second = store
            .block_effect_binding(
                "instance-a",
                "tell",
                "credentials",
                "missing credentials_ref",
            )
            .expect("re-block");
        assert_eq!(first.event_id, second.event_id, "re-block reuses the event");
        assert_eq!(blocked_event_count(&store), 1, "re-block adds no event");

        // Recoverable: a binding-blocked effect stays claimable so a later worker
        // pass retries once the binding is available.
        assert!(
            store
                .claimable_effects("instance-a")
                .expect("claimable")
                .iter()
                .any(|c| c.effect_id == "tell"),
            "blocked effect is re-claimable"
        );
    }

    #[test]
    fn replay_reconstructs_facts_effects_and_dependencies_from_events() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let facts = [test_fact("fact-ready", "issue", "issue-1")];
        let effects = [
            test_effect("claim", "loft.claim", "rule=start;effect=claim"),
            test_effect("tell", "agent.tell", "rule=start;effect=tell"),
        ];
        let dependencies = [NewEffectDependency {
            dependency_id: "dep-claim-tell",
            upstream_effect_id: "claim",
            downstream_effect_id: "tell",
            predicate: "succeeds",
        }];

        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &facts,
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &dependencies,
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");
        store
            .derive_fact(DerivedFact {
                instance_id: "instance-a",
                fact: NewFact {
                    fact_id: "fact-derived",
                    name: "derived",
                    key: "derived-1",
                    value_json: r#"{"ok":true}"#,
                    schema_id: None,
                    provenance_class: "derived",
                    correlation_id: Some("claim"),
                    source_span_json: None,
                },
                source: "kernel",
                causation_id: None,
                idempotency_key: Some("derive-fact"),
            })
            .expect("derived fact succeeds");
        store
            .connection
            .execute("DELETE FROM effect_dependencies", [])
            .expect("dependencies clear");
        store
            .connection
            .execute("DELETE FROM effects", [])
            .expect("effects clear");
        store
            .connection
            .execute("DELETE FROM facts", [])
            .expect("facts clear");

        store
            .rebuild_projections("instance-a")
            .expect("projections rebuild");

        assert_eq!(row_count(&store, "events"), 2);
        assert_eq!(row_count(&store, "facts"), 2);
        assert_eq!(row_count(&store, "effects"), 2);
        assert_eq!(row_count(&store, "effect_dependencies"), 1);
        let rebuilt_facts = store.list_facts("instance-a").expect("facts list");
        assert!(rebuilt_facts.iter().any(|fact| {
            fact.fact_id == "fact-derived"
                && fact.name == "derived"
                && fact.value_json.contains(r#""ok":true"#)
        }));
    }

    #[test]
    fn rule_commit_consumes_facts_and_replay_preserves_consumption() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let facts = [test_fact("fact-task", "Task", "Task:queued")];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "seed",
                trigger_event_id: None,
                facts: &facts,
                consumed_fact_ids: &[],
                effects: &[],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-seed"),
            })
            .expect("seed commit succeeds");

        assert_eq!(
            store
                .list_facts("instance-a")
                .expect("active facts before consume")
                .len(),
            1
        );

        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "finish",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &["fact-task"],
                effects: &[],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-finish"),
            })
            .expect("consume commit succeeds");

        assert_eq!(
            store
                .list_facts("instance-a")
                .expect("active facts after consume")
                .len(),
            0
        );
        assert_eq!(row_count(&store, "facts"), 1);

        store
            .connection
            .execute("DELETE FROM facts", [])
            .expect("facts clear");
        store
            .rebuild_projections("instance-a")
            .expect("projections rebuild");

        assert_eq!(
            store
                .list_facts("instance-a")
                .expect("active replayed facts")
                .len(),
            0
        );
        assert_eq!(row_count(&store, "facts"), 1);
    }

    #[test]
    fn duplicate_terminal_completion_rolls_back_event() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [test_effect("tell", "agent.tell", "rule=start;effect=tell")];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        store
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-1",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-1",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        store
            .complete_effect(test_completion("run-1"))
            .expect("completion succeeds");
        let duplicate = store.complete_effect(test_completion("run-1"));

        assert!(duplicate.is_err());
        assert_eq!(row_count(&store, "events"), 3);
        assert_eq!(row_count(&store, "runs"), 1);
    }

    #[test]
    fn start_run_records_execution_fingerprint_distinct_from_effect_identity() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [
            test_effect("tell", "agent.tell", "rule=start;effect=tell"),
            NewEffect {
                input_json: r#"{"prompt":"different"}"#,
                ..test_effect("other", "agent.tell", "rule=start;effect=other")
            },
        ];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        store
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-1",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-1",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");

        // The run carries an execution fingerprint in its metadata (recorded with
        // the run for replay verification), distinct from the effect identity.
        let metadata: String = store
            .connection
            .query_row(
                "SELECT metadata_json FROM runs WHERE run_id = ?1",
                params!["run-1"],
                |row| row.get(0),
            )
            .expect("run row");
        let recorded: Value = serde_json::from_str(&metadata).expect("metadata json");
        let fingerprint = recorded
            .get("execution_fingerprint")
            .and_then(Value::as_str)
            .expect("execution_fingerprint recorded with the run");
        assert!(!fingerprint.is_empty());
        assert_ne!(
            fingerprint, "tell",
            "the fingerprint is not the effect id/idempotency key"
        );
        // It is the deterministic materialized-input fingerprint.
        assert_eq!(
            fingerprint,
            execution_fingerprint_on(&store.connection, "instance-a", "tell", None)
                .expect("recompute")
        );
        // A different materialized input is a different fingerprint.
        assert_ne!(
            execution_fingerprint_on(&store.connection, "instance-a", "tell", None)
                .expect("fp tell"),
            execution_fingerprint_on(&store.connection, "instance-a", "other", None)
                .expect("fp other"),
        );

        // DR-0014: the model salt makes the execution fingerprint model-sensitive
        // so a coercion whose only change is the model re-runs. An empty/absent
        // salt is byte-identical to the pre-salt fingerprint (every other effect).
        let base =
            execution_fingerprint_on(&store.connection, "instance-a", "tell", None).expect("base");
        assert_eq!(
            base,
            execution_fingerprint_on(&store.connection, "instance-a", "tell", Some(""))
                .expect("empty salt"),
            "an empty salt leaves the fingerprint unchanged",
        );
        assert_ne!(
            base,
            execution_fingerprint_on(&store.connection, "instance-a", "tell", Some("gpt-5"))
                .expect("salted"),
            "a model salt changes the fingerprint",
        );
        assert_ne!(
            execution_fingerprint_on(&store.connection, "instance-a", "tell", Some("gpt-5"))
                .expect("model a"),
            execution_fingerprint_on(&store.connection, "instance-a", "tell", Some("claude"))
                .expect("model b"),
            "different models produce different fingerprints",
        );
        // The salt is carried under the reserved run-metadata key.
        assert_eq!(
            fingerprint_salt_from_metadata(r#"{"__fingerprint_model":"gpt-5"}"#).as_deref(),
            Some("gpt-5"),
        );
        assert_eq!(fingerprint_salt_from_metadata("{}"), None);
        assert_eq!(
            fingerprint_salt_from_metadata(r#"{"__fingerprint_model":""}"#),
            None,
            "an empty model is treated as no salt",
        );
    }

    #[test]
    fn due_interval_occurrences_enumerates_each_due_tick_after_the_cursor() {
        let store = SqliteStore::open_in_memory().expect("store opens");

        // From the source start, every hour, three ticks are due by 12:00.
        let due = store
            .due_interval_occurrences("2026-06-18T09:00:00Z", 3600, "2026-06-18T12:00:00Z")
            .expect("due occurrences");
        assert_eq!(
            due,
            vec![
                "2026-06-18T10:00:00Z",
                "2026-06-18T11:00:00Z",
                "2026-06-18T12:00:00Z",
            ]
        );

        // From a cursor (last admitted tick), only later ticks are due.
        let after_cursor = store
            .due_interval_occurrences("2026-06-18T10:00:00Z", 3600, "2026-06-18T12:00:00Z")
            .expect("due after cursor");
        assert_eq!(
            after_cursor,
            vec!["2026-06-18T11:00:00Z", "2026-06-18T12:00:00Z"]
        );

        // Before the first tick is reached, nothing is due.
        let none_yet = store
            .due_interval_occurrences("2026-06-18T09:00:00Z", 3600, "2026-06-18T09:30:00Z")
            .expect("none yet");
        assert!(none_yet.is_empty());

        // A non-positive interval admits nothing (guard).
        assert!(store
            .due_interval_occurrences("2026-06-18T09:00:00Z", 0, "2026-06-18T12:00:00Z")
            .expect("zero interval")
            .is_empty());
    }

    #[test]
    fn contradictory_terminal_completion_rolls_back_event_even_with_distinct_key() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [test_effect("tell", "agent.tell", "rule=start;effect=tell")];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        store
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-1",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-1",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        store
            .complete_effect(EffectCompletion {
                idempotency_key: Some("provider-session-a:turn-a:terminal-hash-ok"),
                ..test_completion("run-1")
            })
            .expect("completion succeeds");
        let contradictory = store.complete_effect(EffectCompletion {
            status: "failed",
            exit_code: Some(1),
            summary: Some("provider reported a later failure"),
            metadata_json: r#"{"terminal_payload_hash":"different"}"#,
            idempotency_key: Some("provider-session-a:turn-a:terminal-hash-different"),
            ..test_completion("run-1")
        });

        assert!(contradictory.is_err());
        assert_eq!(row_count(&store, "events"), 3);
        let runs = store.list_runs("instance-a").expect("runs list");
        assert_eq!(runs[0].status, "completed");
        let terminal_events = store
            .list_events("instance-a")
            .expect("events list")
            .into_iter()
            .filter(|event| event.event_type == "effect.terminal")
            .collect::<Vec<_>>();
        assert_eq!(terminal_events.len(), 1);
        assert!(terminal_events[0]
            .payload_json
            .contains("\"summary\":\"done\""));
        assert!(!terminal_events[0]
            .payload_json
            .contains("provider reported a later failure"));
    }

    #[test]
    fn terminal_completion_requires_running_run() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [test_effect("tell", "agent.tell", "rule=start;effect=tell")];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        let completion = store.complete_effect(test_completion("run-1"));

        assert!(completion.is_err());
        assert_eq!(row_count(&store, "events"), 1);
        assert_eq!(row_count(&store, "runs"), 0);
        assert_eq!(effect_status(&store, "tell"), "queued");
    }

    #[test]
    fn scheduler_claims_only_dependency_satisfied_effects() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [
            test_effect("claim", "loft.claim", "rule=start;effect=claim"),
            NewEffect {
                timeout_seconds: None,
                effect_id: "tell",
                kind: "agent.tell",
                target: Some("worker"),
                input_json: r#"{"prompt":"go"}"#,
                status: "blocked_by_dependency",
                idempotency_key: "rule=start;effect=tell",
                required_capabilities_json: "[]",
                profile: Some("repo-writer"),
                correlation_id: None,
                source_span_json: None,
            },
        ];
        let dependencies = [NewEffectDependency {
            dependency_id: "dep-claim-tell",
            upstream_effect_id: "claim",
            downstream_effect_id: "tell",
            predicate: "succeeds",
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &dependencies,
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        let claimable = store
            .claimable_effects("instance-a")
            .expect("claimable effects load");
        assert_eq!(claimable.len(), 1);
        assert_eq!(claimable[0].effect_id, "claim");

        store
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "claim",
                run_id: "run-claim",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-claim",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("claim run starts");
        store
            .complete_effect(EffectCompletion {
                instance_id: "instance-a",
                effect_id: "claim",
                run_id: "run-claim",
                provider: "test",
                worker_id: "worker-1",
                status: "completed",
                exit_code: Some(0),
                summary: Some("claimed"),
                metadata_json: "{}",
                idempotency_key: Some("complete-claim"),
            })
            .expect("claim completes");

        assert_eq!(effect_status(&store, "tell"), "queued");
        let claimable = store
            .claimable_effects("instance-a")
            .expect("claimable effects reload");
        assert_eq!(claimable.len(), 1);
        assert_eq!(claimable[0].effect_id, "tell");
        assert_eq!(lease_status(&store, "lease-claim"), "released");
    }

    #[test]
    fn timed_out_dependency_releases_only_on_timeout() {
        // A `timed_out` dependency predicate releases the downstream effect only
        // when the upstream reaches the `timed_out` terminal status, not on a
        // successful completion (terminal-union after-branch precision).
        for (instance, upstream_status, expect_claimable) in [
            ("ti-completed", "completed", false),
            ("ti-timedout", "timed_out", true),
        ] {
            let mut store = SqliteStore::open_in_memory().expect("store opens");
            let effects = [
                test_effect("up", "agent.tell", "rule=start;effect=up"),
                test_effect("down", "agent.tell", "rule=start;effect=down"),
            ];
            let dependencies = [NewEffectDependency {
                dependency_id: "dep-up-down",
                upstream_effect_id: "up",
                downstream_effect_id: "down",
                predicate: "timed_out",
            }];
            store
                .commit_rule(RuleCommit {
                    instance_id: instance,
                    rule: "start",
                    trigger_event_id: None,
                    facts: &[],
                    consumed_fact_ids: &[],
                    effects: &effects,
                    dependencies: &dependencies,
                    terminal: None,
                    idempotency_key: Some("commit-start"),
                })
                .expect("rule commit succeeds");
            store
                .start_run(RunStart {
                    instance_id: instance,
                    effect_id: "up",
                    run_id: "run-up",
                    provider: "test",
                    worker_id: "worker-1",
                    lease_id: "lease-up",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    metadata_json: "{}",
                })
                .expect("upstream run starts");
            store
                .complete_effect(EffectCompletion {
                    instance_id: instance,
                    effect_id: "up",
                    run_id: "run-up",
                    provider: "test",
                    worker_id: "worker-1",
                    status: upstream_status,
                    exit_code: None,
                    summary: None,
                    metadata_json: "{}",
                    idempotency_key: Some("complete-up"),
                })
                .expect("upstream reaches terminal");
            let claimable = store
                .claimable_effects(instance)
                .expect("claimable effects load");
            let down_claimable = claimable.iter().any(|e| e.effect_id == "down");
            assert_eq!(
                down_claimable, expect_claimable,
                "instance {instance}: upstream `{upstream_status}` should{} release the `timed_out` downstream",
                if expect_claimable { "" } else { " not" }
            );
        }
    }

    #[test]
    fn start_run_rejects_blocked_dependency_without_partial_event() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [
            test_effect("claim", "loft.claim", "rule=start;effect=claim"),
            NewEffect {
                timeout_seconds: None,
                effect_id: "tell",
                kind: "agent.tell",
                target: Some("worker"),
                input_json: r#"{"prompt":"go"}"#,
                status: "blocked_by_dependency",
                idempotency_key: "rule=start;effect=tell",
                required_capabilities_json: "[]",
                profile: Some("repo-writer"),
                correlation_id: None,
                source_span_json: None,
            },
        ];
        let dependencies = [NewEffectDependency {
            dependency_id: "dep-claim-tell",
            upstream_effect_id: "claim",
            downstream_effect_id: "tell",
            predicate: "succeeds",
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &dependencies,
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        let result = store.start_run(RunStart {
            instance_id: "instance-a",
            effect_id: "tell",
            run_id: "run-tell",
            provider: "test",
            worker_id: "worker-1",
            lease_id: "lease-tell",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        });

        assert!(result.is_err());
        assert_eq!(row_count(&store, "events"), 1);
        assert_eq!(row_count(&store, "runs"), 0);
        assert_eq!(effect_status(&store, "tell"), "blocked_by_dependency");
    }

    #[test]
    fn transitions_instance_statuses_with_events() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("Ralph", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        store
            .transition_instance(InstanceTransition {
                instance_id: &instance.instance_id,
                status: "paused",
                reason: Some("maintenance"),
                idempotency_key: Some("pause"),
            })
            .expect("instance pauses");
        store
            .transition_instance(InstanceTransition {
                instance_id: &instance.instance_id,
                status: "running",
                reason: None,
                idempotency_key: Some("resume"),
            })
            .expect("instance resumes");
        store
            .transition_instance(InstanceTransition {
                instance_id: &instance.instance_id,
                status: "cancelled",
                reason: Some("operator"),
                idempotency_key: Some("cancel-instance"),
            })
            .expect("instance cancels");

        assert_eq!(instance_status(&store, &instance.instance_id), "cancelled");
        assert_eq!(row_count(&store, "events"), 3);
    }

    #[test]
    fn start_run_rejects_non_running_instances() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("Paused", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "rule=start;effect=tell")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        store
            .transition_instance(InstanceTransition {
                instance_id: &instance.instance_id,
                status: "paused",
                reason: Some("operator"),
                idempotency_key: Some("pause"),
            })
            .expect("instance pauses");

        let result = store.start_run(RunStart {
            instance_id: &instance.instance_id,
            effect_id: "tell",
            run_id: "run-tell",
            provider: "test",
            worker_id: "worker-1",
            lease_id: "lease-tell",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        });

        assert!(matches!(result, Err(StoreError::Conflict(message)) if message.contains("paused")));
        assert!(store
            .claimable_effects(&instance.instance_id)
            .expect("claimable effects")
            .is_empty());
    }

    #[test]
    fn terminal_instance_statuses_are_absorbing() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("TerminalGuard", "source-1", "ir-1"))
            .expect("program version creates");
        let completed = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        store
            .commit_rule(RuleCommit {
                instance_id: &completed.instance_id,
                rule: "finish",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[],
                dependencies: &[],
                terminal: Some(WorkflowTerminal {
                    kind: WorkflowTerminalKind::Completed,
                    name: "result",
                    payload_json: "{}",
                    idempotency_key: Some("workflow-complete-terminal-guard"),
                }),
                idempotency_key: Some("commit-finish-terminal-guard"),
            })
            .expect("terminal commit succeeds");

        let cancel_completed = store.transition_instance(InstanceTransition {
            instance_id: &completed.instance_id,
            status: "cancelled",
            reason: Some("late cancel"),
            idempotency_key: Some("cancel-completed"),
        });

        assert!(matches!(
            cancel_completed,
            Err(StoreError::Conflict(message)) if message.contains("completed to cancelled")
        ));
        assert_eq!(instance_status(&store, &completed.instance_id), "completed");

        let cancelled = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .transition_instance(InstanceTransition {
                instance_id: &cancelled.instance_id,
                status: "cancelled",
                reason: Some("operator"),
                idempotency_key: Some("cancel-instance-terminal-guard"),
            })
            .expect("instance cancels");

        let resume_cancelled = store.transition_instance(InstanceTransition {
            instance_id: &cancelled.instance_id,
            status: "running",
            reason: None,
            idempotency_key: Some("resume-cancelled"),
        });

        assert!(matches!(
            resume_cancelled,
            Err(StoreError::Conflict(message)) if message.contains("cancelled to running")
        ));
        assert_eq!(instance_status(&store, &cancelled.instance_id), "cancelled");
    }

    #[test]
    fn activate_revision_updates_active_version_and_preserves_effect_attribution() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version1 = store
            .create_program_version(test_program_version("Revision", "source-1", "ir-1"))
            .expect("first program version creates");
        let version2 = store
            .create_program_version(test_program_version("Revision", "source-2", "ir-2"))
            .expect("second program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version1.program_id,
                version_id: &version1.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "old-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("old-effect", "agent.tell", "old-effect-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-old"),
            })
            .expect("old rule commits");
        assert_eq!(
            effect_revision(&store, "old-effect"),
            (Some(version1.version_id.clone()), 0, false)
        );

        let revision = store
            .activate_revision(RevisionActivation {
                instance_id: &instance.instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-keep"),
            })
            .expect("revision activates");
        assert_eq!(revision.epoch, 1);
        assert_eq!(revision.from_version_id, version1.version_id.as_str());
        assert_eq!(revision.to_version_id, version2.version_id.as_str());

        let active = store
            .get_instance(&instance.instance_id)
            .expect("instance loads")
            .expect("instance exists");
        assert_eq!(active.version_id, version2.version_id);
        assert_eq!(active.revision_epoch, 1);

        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "new-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("new-effect", "agent.tell", "new-effect-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-new"),
            })
            .expect("new rule commits");

        assert_eq!(
            effect_revision(&store, "old-effect"),
            (Some(version1.version_id.clone()), 0, false)
        );
        assert_eq!(
            effect_revision(&store, "new-effect"),
            (Some(version2.version_id.clone()), 1, false)
        );
        let duplicate = store
            .activate_revision(RevisionActivation {
                instance_id: &instance.instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-keep"),
            })
            .expect("idempotent revision returns existing row");
        assert_eq!(duplicate.revision_id, revision.revision_id);
        assert_eq!(row_count(&store, "instance_revisions"), 1);

        let reused_key = store.activate_revision(RevisionActivation {
            instance_id: &instance.instance_id,
            from_version_id: &version1.version_id,
            to_version_id: &version2.version_id,
            activation_policy_json: r#"{"changed":true}"#,
            cancellation_policy: "keep",
            idempotency_key: Some("revise-keep"),
        });
        assert!(matches!(
            reused_key,
            Err(StoreError::Conflict(message))
                if message.contains("idempotency key was reused")
        ));
        assert_eq!(row_count(&store, "instance_revisions"), 1);
    }

    #[test]
    fn guarded_rule_commit_rejects_stale_revision_epoch() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version1 = store
            .create_program_version(test_program_version("RevisionGuard", "source-1", "ir-1"))
            .expect("first program version creates");
        let version2 = store
            .create_program_version(test_program_version("RevisionGuard", "source-2", "ir-2"))
            .expect("second program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version1.program_id,
                version_id: &version1.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        store
            .activate_revision(RevisionActivation {
                instance_id: &instance.instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-before-stale-commit"),
            })
            .expect("revision activates");

        let stale = store.commit_rule_with_revision_guard(
            RuleCommit {
                instance_id: &instance.instance_id,
                rule: "old-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect(
                    "stale-effect",
                    "agent.tell",
                    "stale-effect-key",
                )],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-stale"),
            },
            RuleCommitRevisionGuard {
                program_version_id: &version1.version_id,
                revision_epoch: 0,
            },
        );

        assert!(matches!(
            stale,
            Err(StoreError::Conflict(message))
                if message.contains("active revision changed before rule commit")
        ));
        assert_eq!(row_count(&store, "effects"), 0);
    }

    #[test]
    fn activate_revision_rechecks_compatibility_in_transaction() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version1 = store
            .create_program_version(test_program_version(
                "RevisionCompatTxn",
                "source-1",
                "ir-1",
            ))
            .expect("first program version creates");
        let incompatible_summary =
            r#"{"workflow":"OtherWorkflow","workflow_contracts":[],"schemas":[]}"#;
        let version2 = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: incompatible_summary,
                ..test_program_version("RevisionCompatTxn", "source-2", "ir-2")
            })
            .expect("second program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version1.program_id,
                version_id: &version1.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        let result = store.activate_revision(RevisionActivation {
            instance_id: &instance.instance_id,
            from_version_id: &version1.version_id,
            to_version_id: &version2.version_id,
            activation_policy_json: "{}",
            cancellation_policy: "keep",
            idempotency_key: Some("revise-incompatible-direct"),
        });

        assert!(matches!(
            result,
            Err(StoreError::Conflict(message))
                if message.contains("revision.root_workflow_changed")
        ));
        let active = store
            .get_instance(&instance.instance_id)
            .expect("instance loads")
            .expect("instance exists");
        assert_eq!(active.version_id, version1.version_id);
        assert_eq!(row_count(&store, "instance_revisions"), 0);
    }

    #[test]
    fn old_effect_policy_checks_use_effect_version_after_revision() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version1 = store
            .create_program_version(NewProgramVersion {
                declared_profiles_json: r#"{"agents":[{"name":"worker","profile":"repo-writer","capacity":1,"capabilities":["agent.tell"]}]}"#,
                ..test_program_version("RevisionAgents", "source-1", "ir-1")
            })
            .expect("first program version creates");
        let version2 = store
            .create_program_version(NewProgramVersion {
                declared_profiles_json: r#"{"agents":[{"name":"other","profile":"repo-writer","capacity":1,"capabilities":["agent.tell"]}]}"#,
                ..test_program_version("RevisionAgents", "source-2", "ir-2")
            })
            .expect("second program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version1.program_id,
                version_id: &version1.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "old-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("old-effect", "agent.tell", "old-effect-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-old"),
            })
            .expect("old rule commits");
        store
            .activate_revision(RevisionActivation {
                instance_id: &instance.instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-agent-removal"),
            })
            .expect("revision activates");

        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "old-effect",
                run_id: "run-old",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-old",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("old effect uses old version declarations");
        store
            .complete_effect(EffectCompletion {
                instance_id: &instance.instance_id,
                effect_id: "old-effect",
                run_id: "run-old",
                provider: "test",
                worker_id: "worker-1",
                status: "completed",
                exit_code: Some(0),
                summary: Some("old done"),
                metadata_json: "{}",
                idempotency_key: Some("complete-old"),
            })
            .expect("old run completes");

        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "new-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("new-effect", "agent.tell", "new-effect-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-new"),
            })
            .expect("new rule commits");
        let blocked = store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "new-effect",
                run_id: "run-new",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-new",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect_err("new effect uses candidate declarations");

        assert!(
            matches!(blocked, StoreError::PolicyBlocked { reason, .. } if reason.contains("not declared"))
        );
        assert_eq!(effect_status(&store, "new-effect"), "blocked_by_profile");
    }

    #[test]
    fn revision_request_running_cancels_queued_and_requests_running_effects() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version1 = store
            .create_program_version(test_program_version("RevisionPolicy", "source-1", "ir-1"))
            .expect("first program version creates");
        let version2 = store
            .create_program_version(test_program_version("RevisionPolicy", "source-2", "ir-2"))
            .expect("second program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version1.program_id,
                version_id: &version1.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        let effects = [
            test_effect("queued-effect", "agent.tell", "queued-effect-key"),
            test_effect("running-effect", "agent.tell", "running-effect-key"),
        ];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "old-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-policy-old"),
            })
            .expect("old rule commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "running-effect",
                run_id: "run-running",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-running",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");

        let impact = store
            .revision_cancellation_impact(&instance.instance_id, "running")
            .expect("revision impact reports");
        assert_eq!(impact.active_version_id, version1.version_id.as_str());
        assert_eq!(impact.active_revision_epoch, 0);
        assert_eq!(impact.cancellation_policy, "request_running");
        assert_eq!(impact.terminal_cancel_effects, vec!["queued-effect"]);
        assert_eq!(impact.request_cancel_effects, vec!["running-effect"]);

        let revision = store
            .activate_revision(RevisionActivation {
                instance_id: &instance.instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "request_running",
                idempotency_key: Some("revise-request-running"),
            })
            .expect("revision activates");

        assert_eq!(revision.epoch, 1);
        assert_eq!(effect_status(&store, "queued-effect"), "cancelled");
        assert_eq!(effect_status(&store, "running-effect"), "running");
        let requests = store
            .list_effect_cancellation_requests(&instance.instance_id)
            .expect("requests list");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            effect_revision(&store, "running-effect"),
            (Some(version1.version_id.clone()), 0, true)
        );
        let runs = store
            .list_runs(&instance.instance_id)
            .expect("runs include request state");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].cancel_requested);
        let evidence = store
            .list_evidence(&instance.instance_id)
            .expect("evidence lists");
        let revision_evidence = evidence
            .iter()
            .find(|evidence| evidence.kind == "workflow.revision.activated")
            .expect("revision evidence exists");
        let cancellation_evidence = evidence
            .iter()
            .find(|evidence| evidence.kind == "effect.cancellation.requested")
            .expect("cancellation request evidence exists");
        assert_eq!(revision_evidence.subject_id, revision.revision_id);
        assert_eq!(cancellation_evidence.subject_id, requests[0].request_id);
        let metadata =
            serde_json::from_str::<Value>(&revision_evidence.metadata_json).expect("metadata json");
        assert_eq!(
            metadata.get("from_version_id").and_then(Value::as_str),
            Some(version1.version_id.as_str())
        );
        assert_eq!(
            metadata.get("to_version_id").and_then(Value::as_str),
            Some(version2.version_id.as_str())
        );
        assert_eq!(
            metadata
                .get("terminal_cancel_effects")
                .and_then(Value::as_array)
                .and_then(|effects| effects.first())
                .and_then(Value::as_str),
            Some("queued-effect")
        );
        assert_eq!(
            metadata
                .get("cancellation_request_ids")
                .and_then(Value::as_array)
                .and_then(|requests| requests.first())
                .and_then(Value::as_str),
            Some(requests[0].request_id.as_str())
        );
        let cancellation_metadata =
            serde_json::from_str::<Value>(&cancellation_evidence.metadata_json)
                .expect("cancellation metadata json");
        assert_eq!(
            cancellation_metadata
                .get("active_run_ids")
                .and_then(Value::as_array)
                .and_then(|runs| runs.first())
                .and_then(Value::as_str),
            Some("run-running")
        );
        let links = store
            .list_evidence_links(&instance.instance_id)
            .expect("evidence links list");
        assert!(links.iter().any(|link| {
            link.evidence_id == revision_evidence.evidence_id
                && link.target_type == "program_version"
                && link.target_id == version1.version_id.as_str()
                && link.relation == "from_version"
        }));
        assert!(links.iter().any(|link| {
            link.evidence_id == revision_evidence.evidence_id
                && link.target_type == "program_version"
                && link.target_id == version2.version_id.as_str()
                && link.relation == "to_version"
        }));
        assert!(links.iter().any(|link| {
            link.evidence_id == revision_evidence.evidence_id
                && link.target_type == "effect"
                && link.target_id == "queued-effect"
                && link.relation == "terminal_cancelled"
        }));
        assert!(links.iter().any(|link| {
            link.evidence_id == revision_evidence.evidence_id
                && link.target_type == "effect"
                && link.target_id == "running-effect"
                && link.relation == "cancellation_requested"
        }));
        assert!(links.iter().any(|link| {
            link.evidence_id == revision_evidence.evidence_id
                && link.target_type == "effect_cancellation_request"
                && link.target_id == requests[0].request_id.as_str()
                && link.relation == "created"
        }));
        assert!(links.iter().any(|link| {
            link.evidence_id == cancellation_evidence.evidence_id
                && link.target_type == "effect"
                && link.target_id == "running-effect"
                && link.relation == "requested_cancellation"
        }));
        assert!(links.iter().any(|link| {
            link.evidence_id == cancellation_evidence.evidence_id
                && link.target_type == "run"
                && link.target_id == "run-running"
                && link.relation == "active_run"
        }));
        assert!(links.iter().any(|link| {
            link.evidence_id == cancellation_evidence.evidence_id
                && link.target_type == "workflow_revision"
                && link.target_id == revision.revision_id.as_str()
                && link.relation == "requested_by"
        }));

        store
            .expire_leases(&instance.instance_id, "2030-01-02T00:00:00Z")
            .expect("lease expires");
        assert_eq!(effect_status(&store, "running-effect"), "queued");
        assert!(
            store
                .claimable_effects(&instance.instance_id)
                .expect("claimable effects")
                .is_empty(),
            "cancel-requested effects must not become claimable after lease expiry"
        );
    }

    #[test]
    fn later_revision_policy_includes_effects_kept_across_prior_revisions() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version1 = store
            .create_program_version(test_program_version(
                "RevisionPolicyAll",
                "source-1",
                "ir-1",
            ))
            .expect("first program version creates");
        let version2 = store
            .create_program_version(test_program_version(
                "RevisionPolicyAll",
                "source-2",
                "ir-2",
            ))
            .expect("second program version creates");
        let version3 = store
            .create_program_version(test_program_version(
                "RevisionPolicyAll",
                "source-3",
                "ir-3",
            ))
            .expect("third program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version1.program_id,
                version_id: &version1.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        let effects = [
            test_effect("kept-queued", "agent.tell", "kept-queued-key"),
            test_effect("kept-running", "agent.tell", "kept-running-key"),
        ];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "old-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-kept-effects"),
            })
            .expect("old effects commit");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "kept-running",
                run_id: "run-kept-running",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-kept-running",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        store
            .activate_revision(RevisionActivation {
                instance_id: &instance.instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-keep-old-effects"),
            })
            .expect("first revision keeps existing work");

        let impact = store
            .revision_cancellation_impact(&instance.instance_id, "running")
            .expect("later revision impact reports");
        assert_eq!(impact.active_version_id, version2.version_id.as_str());
        assert_eq!(impact.terminal_cancel_effects, vec!["kept-queued"]);
        assert_eq!(impact.request_cancel_effects, vec!["kept-running"]);

        store
            .activate_revision(RevisionActivation {
                instance_id: &instance.instance_id,
                from_version_id: &version2.version_id,
                to_version_id: &version3.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "running",
                idempotency_key: Some("revise-request-kept-effects"),
            })
            .expect("second revision applies policy to all existing old work");

        assert_eq!(effect_status(&store, "kept-queued"), "cancelled");
        assert_eq!(effect_status(&store, "kept-running"), "running");
        assert_eq!(
            effect_revision(&store, "kept-running"),
            (Some(version1.version_id.clone()), 0, true)
        );
    }

    #[test]
    fn cancellation_requests_are_idempotent_and_resolve_on_terminal_completion() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("RequestCancel", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "tell-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-request-cancel"),
            })
            .expect("rule commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");

        let first = store
            .request_effect_cancellation(EffectCancellationRequest {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                revision_id: None,
                reason: Some("operator"),
                requested_by: "test",
                causation_event_id: None,
                idempotency_key: Some("request-cancel-tell"),
            })
            .expect("request records");
        let second = store
            .request_effect_cancellation(EffectCancellationRequest {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                revision_id: None,
                reason: Some("operator"),
                requested_by: "test",
                causation_event_id: None,
                idempotency_key: Some("request-cancel-tell"),
            })
            .expect("idempotent request returns existing row");

        assert_eq!(first.request_id, second.request_id);
        assert_eq!(row_count(&store, "effect_cancellation_requests"), 1);
        assert_eq!(
            effect_revision(&store, "tell"),
            (Some(version.version_id), 0, true)
        );

        store
            .complete_effect(EffectCompletion {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                status: "completed",
                exit_code: Some(0),
                summary: Some("done"),
                metadata_json: "{}",
                idempotency_key: Some("complete-requested-tell"),
            })
            .expect("completion succeeds");

        let requests = store
            .list_effect_cancellation_requests(&instance.instance_id)
            .expect("requests load");
        assert_eq!(requests[0].status, "terminal");
        assert_eq!(effect_status(&store, "tell"), "completed");
    }

    #[test]
    fn cancellation_request_after_terminal_completion_is_rejected() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version(
                "RequestCancelAfterTerminal",
                "source-1",
                "ir-1",
            ))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "tell-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-request-cancel-after-terminal"),
            })
            .expect("rule commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        store
            .complete_effect(EffectCompletion {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                status: "completed",
                exit_code: Some(0),
                summary: Some("done"),
                metadata_json: "{}",
                idempotency_key: Some("complete-before-cancel-request"),
            })
            .expect("completion succeeds");

        let request = store.request_effect_cancellation(EffectCancellationRequest {
            instance_id: &instance.instance_id,
            effect_id: "tell",
            revision_id: None,
            reason: Some("too late"),
            requested_by: "test",
            causation_event_id: None,
            idempotency_key: Some("request-cancel-after-terminal"),
        });

        assert!(matches!(request, Err(StoreError::Conflict(_))));
        assert!(store
            .list_effect_cancellation_requests(&instance.instance_id)
            .expect("requests load")
            .is_empty());
        assert_eq!(effect_status(&store, "tell"), "completed");
    }

    #[test]
    fn cancellation_request_resolves_on_timeout_and_rejects_late_completion() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version(
                "RequestCancelThenTimeout",
                "source-1",
                "ir-1",
            ))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "tell-timeout-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-request-cancel-timeout"),
            })
            .expect("rule commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-tell-timeout",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell-timeout",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        store
            .request_effect_cancellation(EffectCancellationRequest {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                revision_id: None,
                reason: Some("operator"),
                requested_by: "operator",
                causation_event_id: None,
                idempotency_key: Some("request-cancel-before-timeout"),
            })
            .expect("cancellation requests");

        let timeout = store
            .complete_effect(EffectCompletion {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-tell-timeout",
                provider: "test",
                worker_id: "worker-1",
                status: "timed_out",
                exit_code: Some(124),
                summary: Some("provider timed out after cancellation request"),
                metadata_json: r#"{"cancellation":"timeout_after_request"}"#,
                idempotency_key: Some("timeout-after-cancel-request"),
            })
            .expect("timeout terminal succeeds");

        let requests = store
            .list_effect_cancellation_requests(&instance.instance_id)
            .expect("requests load");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].status, "terminal");
        assert_eq!(
            requests[0].resolved_by_event_id.as_deref(),
            Some(timeout.event_id.as_str())
        );
        assert_eq!(effect_status(&store, "tell"), "timed_out");
        assert_eq!(lease_status(&store, "lease-tell-timeout"), "released");

        let late_completion = store.complete_effect(EffectCompletion {
            instance_id: &instance.instance_id,
            effect_id: "tell",
            run_id: "run-tell-timeout",
            provider: "test",
            worker_id: "worker-1",
            status: "completed",
            exit_code: Some(0),
            summary: Some("late success"),
            metadata_json: "{}",
            idempotency_key: Some("late-success-after-timeout"),
        });
        assert!(matches!(
            late_completion,
            Err(StoreError::Conflict(message))
                if message == "run already has a terminal completion"
        ));
    }

    #[test]
    fn workflow_invocations_preserve_parent_and_child_revision_attribution() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let parent_version = store
            .create_program_version(test_program_version("InvokeRevision", "source-1", "ir-1"))
            .expect("parent program version creates");
        let child_version = store
            .create_program_version(test_program_version("InvokeRevision", "source-2", "ir-2"))
            .expect("child program version creates");
        let revised_parent_version = store
            .create_program_version(test_program_version("InvokeRevision", "source-3", "ir-3"))
            .expect("revised parent program version creates");
        let revised_child_version = store
            .create_program_version(test_program_version("InvokeRevision", "source-4", "ir-4"))
            .expect("revised child program version creates");
        let parent = store
            .create_instance(NewInstance {
                program_id: &parent_version.program_id,
                version_id: &parent_version.version_id,
                input_json: "{}",
            })
            .expect("parent instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &parent.instance_id,
                rule: "invoke-child",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect(
                    "invoke-child",
                    "workflow.invoke",
                    "invoke-child-key",
                )],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-invoke-child"),
            })
            .expect("parent invoke rule commits");
        let child = store
            .create_instance(NewInstance {
                program_id: &child_version.program_id,
                version_id: &child_version.version_id,
                input_json: r#"{"task":"child"}"#,
            })
            .expect("child instance creates");

        store
            .record_workflow_invocation(NewWorkflowInvocation {
                invocation_id: "inv-parent-child",
                parent_instance_id: &parent.instance_id,
                parent_effect_id: "invoke-child",
                child_instance_id: &child.instance_id,
                target_workflow: "Child",
                input_json: r#"{"task":"child"}"#,
                source_span_json: Some(r#"{"start":1,"end":6}"#),
                idempotency_key: "invoke-parent-child",
            })
            .expect("invocation records");

        let invocation = store
            .get_workflow_invocation(&parent.instance_id, "invoke-child")
            .expect("invocation loads")
            .expect("invocation exists");
        assert_eq!(
            invocation.parent_program_version_id.as_deref(),
            Some(parent_version.version_id.as_str())
        );
        assert_eq!(invocation.parent_revision_epoch, 0);
        assert_eq!(
            invocation.parent_active_program_version_id.as_deref(),
            Some(parent_version.version_id.as_str())
        );
        assert_eq!(invocation.parent_active_revision_epoch, Some(0));
        assert_eq!(
            invocation.child_program_version_id.as_deref(),
            Some(child_version.version_id.as_str())
        );
        assert_eq!(invocation.child_revision_epoch, Some(0));
        assert_eq!(
            invocation.child_active_program_version_id.as_deref(),
            Some(child_version.version_id.as_str())
        );
        assert_eq!(invocation.child_active_revision_epoch, Some(0));

        store
            .activate_revision(RevisionActivation {
                instance_id: &parent.instance_id,
                from_version_id: &parent_version.version_id,
                to_version_id: &revised_parent_version.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-parent-after-invoke"),
            })
            .expect("parent revision activates");
        let after_revision = store
            .get_workflow_invocation(&parent.instance_id, "invoke-child")
            .expect("invocation reloads")
            .expect("invocation still exists");
        assert_eq!(
            after_revision.parent_program_version_id.as_deref(),
            Some(parent_version.version_id.as_str())
        );
        assert_eq!(after_revision.parent_revision_epoch, 0);
        assert_eq!(
            after_revision.parent_active_program_version_id.as_deref(),
            Some(revised_parent_version.version_id.as_str())
        );
        assert_eq!(after_revision.parent_active_revision_epoch, Some(1));
        assert_eq!(
            after_revision.child_program_version_id.as_deref(),
            Some(child_version.version_id.as_str())
        );
        assert_eq!(after_revision.child_revision_epoch, Some(0));
        assert_eq!(
            after_revision.child_active_program_version_id.as_deref(),
            Some(child_version.version_id.as_str())
        );
        assert_eq!(after_revision.child_active_revision_epoch, Some(0));

        store
            .activate_revision(RevisionActivation {
                instance_id: &child.instance_id,
                from_version_id: &child_version.version_id,
                to_version_id: &revised_child_version.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-child-after-invoke"),
            })
            .expect("child revision activates");
        let after_child_revision = store
            .get_workflow_invocation(&parent.instance_id, "invoke-child")
            .expect("invocation reloads after child revision")
            .expect("invocation still exists after child revision");
        assert_eq!(
            after_child_revision.parent_program_version_id.as_deref(),
            Some(parent_version.version_id.as_str())
        );
        assert_eq!(after_child_revision.parent_revision_epoch, 0);
        assert_eq!(
            after_child_revision
                .parent_active_program_version_id
                .as_deref(),
            Some(revised_parent_version.version_id.as_str())
        );
        assert_eq!(after_child_revision.parent_active_revision_epoch, Some(1));
        assert_eq!(
            after_child_revision.child_program_version_id.as_deref(),
            Some(child_version.version_id.as_str())
        );
        assert_eq!(after_child_revision.child_revision_epoch, Some(0));
        assert_eq!(
            after_child_revision
                .child_active_program_version_id
                .as_deref(),
            Some(revised_child_version.version_id.as_str())
        );
        assert_eq!(after_child_revision.child_active_revision_epoch, Some(1));
    }

    #[test]
    fn revision_compatibility_accepts_same_root_and_additive_terminals() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let active_summary = json!({
            "workflow": "Compat",
            "workflow_contracts": [
                {"kind": "input", "name": "request", "type": "ref<Request>"},
                {"kind": "output", "name": "done", "type": "ref<Result>"},
                {"kind": "failure", "name": "failed", "type": "ref<Failure>"}
            ]
        })
        .to_string();
        let candidate_summary = json!({
            "workflow": "Compat",
            "workflow_contracts": [
                {"kind": "input", "name": "request", "type": "ref<Request>"},
                {"kind": "output", "name": "done", "type": "ref<Result>"},
                {"kind": "output", "name": "skipped", "type": "ref<Result>"},
                {"kind": "failure", "name": "failed", "type": "ref<Failure>"}
            ]
        })
        .to_string();
        let active_version = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: &active_summary,
                ..test_program_version("Compat", "source-1", "ir-1")
            })
            .expect("active version creates");
        let candidate_version = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: &candidate_summary,
                ..test_program_version("Compat", "source-2", "ir-2")
            })
            .expect("candidate version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &active_version.program_id,
                version_id: &active_version.version_id,
                input_json: r#"{"request":{"id":"1"}}"#,
            })
            .expect("instance creates");

        let report = store
            .analyze_revision_compatibility(&instance.instance_id, &candidate_version.version_id)
            .expect("compatibility report");
        assert!(report.compatible);
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn revision_compatibility_reports_root_and_contract_breaks() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let active_summary = json!({
            "workflow": "Compat",
            "workflow_contracts": [
                {"kind": "input", "name": "request", "type": "ref<Request>", "source_span": {"start": 1, "end": 10, "construct": "workflow_contract"}},
                {"kind": "output", "name": "done", "type": "ref<Result>", "source_span": {"start": 11, "end": 20, "construct": "workflow_contract"}},
                {"kind": "failure", "name": "failed", "type": "ref<Failure>", "source_span": {"start": 21, "end": 30, "construct": "workflow_contract"}}
            ]
        })
        .to_string();
        let candidate_summary = json!({
            "workflow": "Other",
            "workflow_contracts": [
                {"kind": "input", "name": "request", "type": "ref<ChangedRequest>", "source_span": {"start": 31, "end": 40, "construct": "workflow_contract"}},
                {"kind": "input", "name": "extra", "type": "string", "source_span": {"start": 41, "end": 50, "construct": "workflow_contract"}},
                {"kind": "failure", "name": "failed", "type": "ref<Failure>", "source_span": {"start": 51, "end": 60, "construct": "workflow_contract"}}
            ]
        })
        .to_string();
        let active_version = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: &active_summary,
                ..test_program_version("CompatBreak", "source-1", "ir-1")
            })
            .expect("active version creates");
        let candidate_version = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: &candidate_summary,
                ..test_program_version("CompatBreak", "source-2", "ir-2")
            })
            .expect("candidate version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &active_version.program_id,
                version_id: &active_version.version_id,
                input_json: r#"{"request":{"id":"1"}}"#,
            })
            .expect("instance creates");

        let report = store
            .analyze_revision_compatibility(&instance.instance_id, &candidate_version.version_id)
            .expect("compatibility report");
        assert!(!report.compatible);
        let codes = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("revision.root_workflow_changed"));
        assert!(codes.contains("revision.contract_changed"));
        assert!(codes.contains("revision.input_contract_added"));
        assert!(codes.contains("revision.contract_removed"));
        let changed = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "revision.contract_changed")
            .expect("contract change diagnostic");
        assert_eq!(
            changed
                .source_span_json
                .as_deref()
                .and_then(|span| serde_json::from_str::<Value>(span).ok())
                .and_then(|span| span.get("start").and_then(Value::as_u64)),
            Some(31)
        );
        let removed = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "revision.contract_removed")
            .expect("contract removed diagnostic");
        assert_eq!(
            removed
                .source_span_json
                .as_deref()
                .and_then(|span| serde_json::from_str::<Value>(span).ok())
                .and_then(|span| span.get("start").and_then(Value::as_u64)),
            Some(11)
        );
    }

    #[test]
    fn revision_compatibility_accepts_optional_schema_additions_for_active_facts() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let active_summary = json!({
            "workflow": "FactCompat",
            "schemas": [
                {
                    "kind": "class",
                    "name": "WorkItem",
                    "fields": [
                        {"name": "title", "type": "string"}
                    ]
                }
            ]
        })
        .to_string();
        let candidate_summary = json!({
            "workflow": "FactCompat",
            "schemas": [
                {
                    "kind": "class",
                    "name": "WorkItem",
                    "fields": [
                        {"name": "title", "type": "string"},
                        {"name": "notes", "type": "optional<string>"}
                    ]
                }
            ]
        })
        .to_string();
        let active_version = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: &active_summary,
                ..test_program_version("FactCompat", "source-1", "ir-1")
            })
            .expect("active version creates");
        let candidate_version = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: &candidate_summary,
                ..test_program_version("FactCompat", "source-2", "ir-2")
            })
            .expect("candidate version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &active_version.program_id,
                version_id: &active_version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "seed",
                trigger_event_id: None,
                facts: &[test_fact("fact-work", "WorkItem", "work-1")],
                consumed_fact_ids: &[],
                effects: &[],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-work"),
            })
            .expect("fact commits");

        let report = store
            .analyze_revision_compatibility(&instance.instance_id, &candidate_version.version_id)
            .expect("compatibility report");

        assert!(report.compatible, "{:#?}", report.diagnostics);
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn revision_compatibility_reports_active_fact_schema_breaks() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let active_summary = json!({
            "workflow": "FactBreak",
            "schemas": [
                {
                    "kind": "class",
                    "name": "WorkItem",
                    "fields": [
                        {"name": "title", "type": "string"}
                    ]
                },
                {
                    "kind": "enum",
                    "name": "State",
                    "variants": ["open", "closed"]
                }
            ]
        })
        .to_string();
        let changed_summary = json!({
            "workflow": "FactBreak",
            "schemas": [
                {
                    "kind": "class",
                    "name": "WorkItem",
                    "fields": [
                        {"name": "title", "type": "int"},
                        {"name": "status", "type": "string"}
                    ]
                },
                {
                    "kind": "enum",
                    "name": "State",
                    "variants": ["closed"]
                }
            ]
        })
        .to_string();
        let removed_summary = json!({
            "workflow": "FactBreak",
            "schemas": []
        })
        .to_string();
        let active_version = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: &active_summary,
                ..test_program_version("FactBreak", "source-1", "ir-1")
            })
            .expect("active version creates");
        let changed_version = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: &changed_summary,
                ..test_program_version("FactBreak", "source-2", "ir-2")
            })
            .expect("changed candidate version creates");
        let removed_version = store
            .create_program_version(NewProgramVersion {
                analysis_summary_json: &removed_summary,
                ..test_program_version("FactBreak", "source-3", "ir-3")
            })
            .expect("removed candidate version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &active_version.program_id,
                version_id: &active_version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "seed",
                trigger_event_id: None,
                facts: &[
                    test_fact("fact-work", "WorkItem", "work-1"),
                    NewFact {
                        fact_id: "fact-state",
                        name: "State",
                        key: "state-1",
                        value_json: r#""open""#,
                        schema_id: Some("State"),
                        provenance_class: "derived",
                        correlation_id: None,
                        source_span_json: None,
                    },
                ],
                consumed_fact_ids: &[],
                effects: &[],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-work"),
            })
            .expect("fact commits");

        let changed_report = store
            .analyze_revision_compatibility(&instance.instance_id, &changed_version.version_id)
            .expect("changed compatibility report");
        assert!(!changed_report.compatible);
        let changed = changed_report
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == "revision.active_fact_incompatible"
                    && diagnostic.subject.as_deref() == Some("WorkItem")
            })
            .expect("active fact incompatibility diagnostic");
        assert_eq!(changed.subject.as_deref(), Some("WorkItem"));
        assert!(changed.message.contains("$.title must be int"));
        assert!(changed.message.contains("$.status is required"));
        assert!(changed_report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "revision.active_fact_incompatible"
                && diagnostic.subject.as_deref() == Some("State")
                && diagnostic
                    .message
                    .contains("enum variant `open` not declared")
        }));

        let removed_report = store
            .analyze_revision_compatibility(&instance.instance_id, &removed_version.version_id)
            .expect("removed compatibility report");
        assert!(!removed_report.compatible);
        assert!(removed_report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "revision.active_fact_schema_removed"
                && diagnostic.subject.as_deref() == Some("WorkItem")
        }));
    }

    #[test]
    fn terminal_instances_cannot_activate_revisions() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version1 = store
            .create_program_version(test_program_version("TerminalRevision", "source-1", "ir-1"))
            .expect("first program version creates");
        let version2 = store
            .create_program_version(test_program_version("TerminalRevision", "source-2", "ir-2"))
            .expect("second program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version1.program_id,
                version_id: &version1.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .transition_instance(InstanceTransition {
                instance_id: &instance.instance_id,
                status: "cancelled",
                reason: Some("operator"),
                idempotency_key: Some("cancel-before-revision"),
            })
            .expect("instance cancels");

        let result = store.activate_revision(RevisionActivation {
            instance_id: &instance.instance_id,
            from_version_id: &version1.version_id,
            to_version_id: &version2.version_id,
            activation_policy_json: "{}",
            cancellation_policy: "keep",
            idempotency_key: Some("revise-terminal"),
        });

        assert!(matches!(
            result,
            Err(StoreError::Conflict(message)) if message.contains("cancelled")
        ));
        assert_eq!(row_count(&store, "instance_revisions"), 0);
    }

    #[test]
    fn replay_reconstructs_active_revision_cancelled_effects_and_requests() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version1 = store
            .create_program_version(test_program_version(
                "RevisionReplayFull",
                "source-1",
                "ir-1",
            ))
            .expect("first program version creates");
        let version2 = store
            .create_program_version(test_program_version(
                "RevisionReplayFull",
                "source-2",
                "ir-2",
            ))
            .expect("second program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version1.program_id,
                version_id: &version1.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let effects = [
            test_effect("queued-effect", "agent.tell", "replay-queued-key"),
            test_effect("running-effect", "agent.tell", "replay-running-key"),
        ];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "old-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-replay-full-old"),
            })
            .expect("old rule commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "running-effect",
                run_id: "run-replay-running",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-replay-running",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        store
            .activate_revision(RevisionActivation {
                instance_id: &instance.instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "request_running",
                idempotency_key: Some("revise-replay-full"),
            })
            .expect("revision activates");

        store
            .connection
            .execute("DELETE FROM effect_cancellation_requests", [])
            .expect("requests clear");
        store
            .connection
            .execute("DELETE FROM instance_revisions", [])
            .expect("revisions clear");
        store
            .connection
            .execute("DELETE FROM leases", [])
            .expect("leases clear");
        store
            .connection
            .execute("DELETE FROM runs", [])
            .expect("runs clear");
        store
            .connection
            .execute("DELETE FROM effects", [])
            .expect("effects clear");
        store
            .connection
            .execute(
                "UPDATE instances SET version_id = ?1, revision_epoch = 0 WHERE instance_id = ?2",
                params![&version1.version_id, &instance.instance_id],
            )
            .expect("active revision cache corrupts");

        store
            .rebuild_projections(&instance.instance_id)
            .expect("projections rebuild");

        let active = store
            .get_instance(&instance.instance_id)
            .expect("instance loads")
            .expect("instance exists");
        assert_eq!(active.version_id, version2.version_id);
        assert_eq!(active.revision_epoch, 1);
        assert_eq!(
            store
                .list_instance_revisions(&instance.instance_id)
                .expect("revisions list")
                .len(),
            1
        );
        assert_eq!(effect_status(&store, "queued-effect"), "cancelled");
        assert_eq!(
            effect_revision(&store, "running-effect"),
            (Some(version1.version_id), 0, true)
        );
        assert_eq!(
            store
                .list_effect_cancellation_requests(&instance.instance_id)
                .expect("requests list")
                .len(),
            1
        );
        let runs = store
            .list_runs(&instance.instance_id)
            .expect("runs rebuild from run-start event");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "running");
        assert!(runs[0].cancel_requested);
        assert_eq!(lease_status(&store, "lease-replay-running"), "active");
    }

    #[test]
    fn replay_reconstructs_terminal_runs_leases_and_resolved_cancel_requests() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version(
                "RevisionReplayTerminal",
                "source-1",
                "ir-1",
            ))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "terminal-replay-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-terminal-replay"),
            })
            .expect("effect commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-terminal-replay",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-terminal-replay",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        store
            .request_effect_cancellation(EffectCancellationRequest {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                revision_id: None,
                reason: Some("operator"),
                requested_by: "operator",
                causation_event_id: None,
                idempotency_key: Some("request-terminal-replay"),
            })
            .expect("cancellation requests");
        store
            .complete_effect(EffectCompletion {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-terminal-replay",
                provider: "test",
                worker_id: "worker-1",
                status: "completed",
                exit_code: Some(0),
                summary: Some("done"),
                metadata_json: r#"{"ok":true}"#,
                idempotency_key: Some("complete-terminal-replay"),
            })
            .expect("effect completes");

        store
            .rebuild_projections(&instance.instance_id)
            .expect("live projections rebuild");

        assert_eq!(effect_status(&store, "tell"), "completed");
        let runs = store
            .list_runs(&instance.instance_id)
            .expect("runs list after replay");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "completed");
        assert!(!runs[0].cancel_requested);
        assert_eq!(lease_status(&store, "lease-terminal-replay"), "released");
        let requests = store
            .list_effect_cancellation_requests(&instance.instance_id)
            .expect("requests replay");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].status, "terminal");
        assert!(requests[0].resolved_by_event_id.is_some());
    }

    #[test]
    fn replay_reconstructs_expired_leases_without_reclaiming_cancel_requested_effects() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version(
                "RevisionReplayLeaseExpired",
                "source-1",
                "ir-1",
            ))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "lease-replay-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-lease-replay"),
            })
            .expect("effect commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-lease-replay",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-lease-replay",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        store
            .request_effect_cancellation(EffectCancellationRequest {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                revision_id: None,
                reason: Some("operator"),
                requested_by: "operator",
                causation_event_id: None,
                idempotency_key: Some("request-lease-replay"),
            })
            .expect("cancellation requests");
        store
            .expire_leases(&instance.instance_id, "2030-01-02T00:00:00Z")
            .expect("lease expires");

        store
            .rebuild_projections(&instance.instance_id)
            .expect("live projections rebuild");

        assert_eq!(effect_status(&store, "tell"), "queued");
        assert_eq!(lease_status(&store, "lease-lease-replay"), "expired");
        let runs = store
            .list_runs(&instance.instance_id)
            .expect("runs list after replay");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "lease_expired");
        assert!(runs[0].cancel_requested);
        assert!(
            store
                .claimable_effects(&instance.instance_id)
                .expect("claimable effects")
                .is_empty(),
            "cancel-requested effects must not become claimable after replayed lease expiry"
        );
    }

    #[test]
    fn replay_preserves_rule_commit_revision_attribution() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version1 = store
            .create_program_version(test_program_version("RevisionReplay", "source-1", "ir-1"))
            .expect("first program version creates");
        let version2 = store
            .create_program_version(test_program_version("RevisionReplay", "source-2", "ir-2"))
            .expect("second program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version1.program_id,
                version_id: &version1.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "old-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("old-effect", "agent.tell", "old-effect-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-replay-old"),
            })
            .expect("old rule commits");
        store
            .activate_revision(RevisionActivation {
                instance_id: &instance.instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-replay"),
            })
            .expect("revision activates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "new-rule",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("new-effect", "agent.tell", "new-effect-key")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-replay-new"),
            })
            .expect("new rule commits");

        store
            .connection
            .execute("DELETE FROM effects", [])
            .expect("effects clear");
        store
            .rebuild_projections(&instance.instance_id)
            .expect("projections rebuild");

        assert_eq!(
            effect_revision(&store, "old-effect"),
            (Some(version1.version_id), 0, false)
        );
        assert_eq!(
            effect_revision(&store, "new-effect"),
            (Some(version2.version_id), 1, false)
        );
    }

    #[test]
    fn cancel_effect_marks_non_terminal_effect_and_releases_completes_dependencies() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [
            test_effect("cleanup", "agent.tell", "rule=start;effect=cleanup"),
            NewEffect {
                timeout_seconds: None,
                effect_id: "notify",
                kind: "agent.tell",
                target: Some("worker"),
                input_json: r#"{"prompt":"notify"}"#,
                status: "blocked_by_dependency",
                idempotency_key: "rule=start;effect=notify",
                required_capabilities_json: "[]",
                profile: Some("repo-writer"),
                correlation_id: None,
                source_span_json: None,
            },
        ];
        let dependencies = [NewEffectDependency {
            dependency_id: "dep-cleanup-notify",
            upstream_effect_id: "cleanup",
            downstream_effect_id: "notify",
            predicate: "completes",
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &dependencies,
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");

        store
            .cancel_effect(EffectCancellation {
                instance_id: "instance-a",
                effect_id: "cleanup",
                reason: Some("operator"),
                idempotency_key: Some("cancel-cleanup"),
            })
            .expect("effect cancels");

        assert_eq!(effect_status(&store, "cleanup"), "cancelled");
        assert_eq!(effect_status(&store, "notify"), "queued");
    }

    #[test]
    fn renews_and_expires_leases_for_recovery() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [test_effect("tell", "agent.tell", "rule=start;effect=tell")];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");
        store
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");

        store
            .renew_lease(LeaseRenewal {
                instance_id: "instance-a",
                lease_id: "lease-tell",
                run_id: "run-tell",
                new_expires_at: "2030-01-02T00:00:00Z",
                idempotency_key: Some("renew-lease"),
            })
            .expect("lease renews");
        let not_expired = store
            .expire_leases("instance-a", "2030-01-01T12:00:00Z")
            .expect("expiry scans");
        assert!(not_expired.is_empty());

        let expired = store
            .expire_leases("instance-a", "2030-01-03T00:00:00Z")
            .expect("lease expires");
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].run_id, "run-tell");
        assert_eq!(effect_status(&store, "tell"), "queued");
        assert_eq!(lease_status(&store, "lease-tell"), "expired");
    }

    #[test]
    fn retries_failed_effects_through_backoff_gate() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let effects = [test_effect("tell", "agent.tell", "rule=start;effect=tell")];
        store
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commit succeeds");
        store
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        store
            .complete_effect(EffectCompletion {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                status: "failed",
                exit_code: Some(1),
                summary: Some("failed"),
                metadata_json: "{}",
                idempotency_key: Some("fail-tell"),
            })
            .expect("effect fails");
        assert_eq!(effect_status(&store, "tell"), "failed");

        store
            .retry_effect(RetryEffect {
                instance_id: "instance-a",
                effect_id: "tell",
                retry_after: Some("2030-01-01T00:00:00Z"),
                idempotency_key: Some("retry-tell"),
            })
            .expect("effect retries");
        assert_eq!(effect_status(&store, "tell"), "queued");

        store
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell-2",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell-2",
                lease_expires_at: "2030-01-02T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("retry run starts");
        store
            .complete_effect(EffectCompletion {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell-2",
                provider: "test",
                worker_id: "worker-1",
                status: "completed",
                exit_code: Some(0),
                summary: Some("retry completed"),
                metadata_json: "{}",
                idempotency_key: Some("complete-tell-2"),
            })
            .expect("retry run completes");
        assert_eq!(effect_status(&store, "tell"), "completed");
    }

    #[test]
    fn inspection_views_report_instance_state() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let declared_profiles_json = r#"{"harnesses":[{"name":"coder","kind":"codex"}],"agents":[{"name":"worker","profile":"repo-writer","capacity":1,"harness":"coder","capabilities":["agent.tell"]}]}"#;
        let mut program_version = test_program_version("Ralph", "source-1", "ir-1");
        program_version.declared_profiles_json = declared_profiles_json;
        let version = store
            .create_program_version(program_version)
            .expect("program version creates");
        let instance = store
            .create_instance_with_authority(
                NewInstance {
                    program_id: &version.program_id,
                    version_id: &version.version_id,
                    input_json: r#"{"issue":"one"}"#,
                },
                NewInstanceAuthority {
                    workflow_principal: "workflow:local/Ralph",
                    effective_authority_json: r#"["workflow:local/Ralph"]"#,
                },
            )
            .expect("instance creates");
        let instance_id = instance.instance_id;

        store
            .derive_fact(DerivedFact {
                instance_id: &instance_id,
                fact: test_fact("fact-started", "pattern:started", "started"),
                source: "external",
                causation_id: None,
                idempotency_key: Some("derive-started"),
            })
            .expect("fact derives");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "rule=start;effect=tell")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        let run_metadata_json = r#"{"native_provider":{"provider_id":"coder","provider_kind":"codex","surface":"codex.app_server"}}"#;
        store
            .start_run(RunStart {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: run_metadata_json,
            })
            .expect("run starts");

        let instances = store.list_instances().expect("instances list");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].instance_id, instance_id);
        assert_eq!(instances[0].status, "running");
        assert_eq!(instances[0].workflow_principal, "workflow:local/Ralph");
        assert_eq!(
            instances[0].effective_authority_json,
            r#"["workflow:local/Ralph"]"#
        );

        let facts = store.list_facts(&instance_id).expect("facts list");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].name, "pattern:started");

        let effects = store.list_effects(&instance_id).expect("effects list");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].effect_id, "tell");
        assert_eq!(effects[0].status, "running");
        assert_eq!(effects[0].declared_profiles_json, declared_profiles_json);

        let runs = store.list_runs(&instance_id).expect("runs list");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run-tell");
        assert_eq!(runs[0].status, "running");
        // The caller-provided run metadata is preserved, and the run additionally
        // records an execution fingerprint (see start_run / execution_fingerprint_on).
        let run_metadata: Value =
            serde_json::from_str(&runs[0].metadata_json).expect("run metadata json");
        let expected_metadata: Value =
            serde_json::from_str(run_metadata_json).expect("expected metadata json");
        assert_eq!(
            run_metadata.get("native_provider"),
            expected_metadata.get("native_provider")
        );
        assert!(run_metadata
            .get("execution_fingerprint")
            .and_then(Value::as_str)
            .is_some_and(|fingerprint| !fingerprint.is_empty()));
        assert!(!runs[0].cancel_requested);

        let events = store.list_events(&instance_id).expect("events list");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].sequence, 1);

        let status = store
            .status(&instance_id)
            .expect("status loads")
            .expect("instance exists");
        assert_eq!(status.fact_count, 1);
        assert_eq!(status.queued_effect_count, 0);
        assert_eq!(status.blocked_effect_count, 0);
        assert_eq!(status.active_run_count, 1);
        assert_eq!(status.failure_count, 0);
        assert_eq!(status.recent_events.len(), 3);
    }

    #[test]
    fn creates_and_answers_human_inbox_items() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("HumanReview", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        store
            .create_inbox_item(NewInboxItem {
                inbox_item_id: "inbox-review",
                instance_id: &instance.instance_id,
                effect_id: None,
                status: "pending",
                prompt: "Approve this change?",
                choices_json: r#"["approve","reject"]"#,
                freeform_allowed: false,
                severity: "normal",
                related_effects_json: "[]",
                related_artifacts_json: "[]",
            })
            .expect("inbox item creates");
        let pending = store
            .list_inbox_items(Some("pending"))
            .expect("pending inbox lists");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].inbox_item_id, "inbox-review");
        assert!(!pending[0].freeform_allowed);

        let event = store
            .answer_inbox_item(HumanAnswer {
                inbox_item_id: "inbox-review",
                answer_json: r#"{"kind":"choice","choice":"approve"}"#,
                answered_by: "jack",
                idempotency_key: Some("answer-inbox-review"),
            })
            .expect("inbox item answers");
        assert_eq!(event.sequence, 1);
        let item = store
            .get_inbox_item("inbox-review")
            .expect("item loads")
            .expect("item exists");
        assert_eq!(item.status, "answered");
        assert_eq!(item.answered_by.as_deref(), Some("jack"));
        let facts = store.list_facts(&instance.instance_id).expect("facts list");
        assert!(facts.iter().any(
            |fact| fact.name == "human.answer.received" && fact.value_json.contains("approve")
        ));
    }

    #[test]
    fn records_lists_and_deduplicates_diagnostics_with_links() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("Diagnostics", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");

        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "rule=start;effect=tell")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        let run_event = store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        let artifact_id = store
            .record_artifact(ArtifactRecord {
                run_id: "run-tell",
                kind: "transcript",
                path: "artifacts/run-tell.txt",
                content_hash: Some("sha256:abc"),
                mime_type: Some("text/plain"),
            })
            .expect("artifact records");
        let evidence_id = store
            .record_evidence(EvidenceRecord {
                instance_id: &instance.instance_id,
                kind: "provider_failure",
                subject_type: "run",
                subject_id: "run-tell",
                causation_id: Some("tell"),
                correlation_id: Some("corr-tell"),
                summary: Some("provider failed"),
                metadata_json: r#"{"stderr":"boom"}"#,
            })
            .expect("evidence records");
        let evidence_ids_json = format!(r#"["{evidence_id}"]"#);
        let artifact_ids_json = format!(r#"["{artifact_id}"]"#);

        let diagnostic_id = store
            .record_diagnostic(DiagnosticRecord {
                instance_id: Some(&instance.instance_id),
                program_id: Some(&version.program_id),
                program_version_id: Some(&version.version_id),
                severity: Severity::Error,
                code: Some("provider.transport"),
                message: "provider transport failed",
                source_span_json: Some(
                    r#"{"path":"workflow.whip","start":{"line":3,"column":5},"end":{"line":3,"column":18},"construct":"effect"}"#,
                ),
                subject_type: Some("effect"),
                subject_id: Some("tell"),
                event_id: Some(&run_event.event_id),
                effect_id: Some("tell"),
                run_id: Some("run-tell"),
                assertion_id: None,
                evidence_ids_json: &evidence_ids_json,
                artifact_ids_json: &artifact_ids_json,
                causation_id: Some("tell"),
                correlation_id: Some("corr-tell"),
                idempotency_key: Some("diagnostic:run-tell"),
            })
            .expect("diagnostic records");
        let duplicate_id = store
            .record_diagnostic(DiagnosticRecord {
                message: "different retry message",
                ..DiagnosticRecord {
                    instance_id: Some(&instance.instance_id),
                    program_id: Some(&version.program_id),
                    program_version_id: Some(&version.version_id),
                    severity: Severity::Error,
                    code: Some("provider.transport"),
                    message: "provider transport failed",
                    source_span_json: None,
                    subject_type: Some("effect"),
                    subject_id: Some("tell"),
                    event_id: Some(&run_event.event_id),
                    effect_id: Some("tell"),
                    run_id: Some("run-tell"),
                    assertion_id: None,
                    evidence_ids_json: &evidence_ids_json,
                    artifact_ids_json: &artifact_ids_json,
                    causation_id: Some("tell"),
                    correlation_id: Some("corr-tell"),
                    idempotency_key: Some("diagnostic:run-tell"),
                }
            })
            .expect("duplicate diagnostic returns existing row");

        assert_eq!(diagnostic_id, duplicate_id);
        let diagnostics = store
            .list_diagnostics(Some(&instance.instance_id))
            .expect("diagnostics list");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].diagnostic_id, diagnostic_id);
        assert_eq!(diagnostics[0].severity, "error");
        assert_eq!(diagnostics[0].code.as_deref(), Some("provider.transport"));
        assert_eq!(diagnostics[0].subject_type.as_deref(), Some("effect"));
        assert_eq!(diagnostics[0].subject_id.as_deref(), Some("tell"));
        assert_eq!(
            diagnostics[0].event_id.as_deref(),
            Some(run_event.event_id.as_str())
        );
        assert_eq!(diagnostics[0].effect_id.as_deref(), Some("tell"));
        assert_eq!(diagnostics[0].run_id.as_deref(), Some("run-tell"));
        assert_eq!(diagnostics[0].evidence_ids_json, evidence_ids_json);
        assert_eq!(diagnostics[0].artifact_ids_json, artifact_ids_json);
        assert_eq!(diagnostics[0].correlation_id.as_deref(), Some("corr-tell"));
        assert!(diagnostics[0]
            .source_span_json
            .as_deref()
            .expect("source span")
            .contains("workflow.whip"));
    }

    #[test]
    fn reconstructs_terminal_diagnostics_from_event_payloads() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("DiagnosticReplay", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let source_span_json =
            r#"{"path":"workflow.whip","start":42,"end":73,"construct":"effect"}"#;
        let effects = [NewEffect {
            timeout_seconds: None,
            source_span_json: Some(source_span_json),
            ..test_effect("tell", "agent.tell", "rule=start;effect=tell")
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "fixture",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        let event = store
            .complete_effect_with_terminal_diagnostic(
                EffectCompletion {
                    instance_id: &instance.instance_id,
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider: "fixture",
                    worker_id: "worker-1",
                    status: "failed",
                    exit_code: Some(42),
                    summary: Some("fixture failed"),
                    metadata_json: r#"{"stderr":"boom"}"#,
                    idempotency_key: Some("terminal-run-tell"),
                },
                Some(TerminalDiagnosticRecord {
                    program_id: Some(version.program_id.clone()),
                    program_version_id: Some(version.version_id.clone()),
                    severity: Severity::Error,
                    code: Some("fixture.failed".to_owned()),
                    message: "fixture failed: boom".to_owned(),
                    source_span_json: Some(source_span_json.to_owned()),
                    subject_type: Some("effect".to_owned()),
                    subject_id: Some("tell".to_owned()),
                    assertion_id: None,
                    evidence_ids_json: "[]".to_owned(),
                    artifact_ids_json: "[]".to_owned(),
                    causation_id: Some("run-tell".to_owned()),
                    correlation_id: Some("tell".to_owned()),
                    idempotency_key: Some("diagnostic-run-tell".to_owned()),
                }),
            )
            .expect("terminal diagnostic completes");

        let replayed = store
            .list_diagnostics_from_events(&instance.instance_id)
            .expect("event diagnostics replay");
        assert_eq!(replayed.len(), 1);
        assert_eq!(
            replayed[0].event_id.as_deref(),
            Some(event.event_id.as_str())
        );
        assert_eq!(replayed[0].effect_id.as_deref(), Some("tell"));
        assert_eq!(replayed[0].run_id.as_deref(), Some("run-tell"));
        assert_eq!(replayed[0].code.as_deref(), Some("fixture.failed"));
        assert_eq!(
            serde_json::from_str::<Value>(
                replayed[0]
                    .source_span_json
                    .as_deref()
                    .expect("source span")
            )
            .expect("replayed source span json"),
            serde_json::from_str::<Value>(source_span_json).expect("expected source span json")
        );
    }

    #[test]
    fn opening_v1_store_adds_diagnostic_columns() {
        let path = std::env::temp_dir().join(format!(
            "whipplescript-store-v1-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        {
            let connection = Connection::open(&path).expect("v1 db opens");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE schema_migrations (
                        version INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                    );
                    INSERT INTO schema_migrations (version, name)
                    VALUES (1, 'runtime-store-schema');
                    CREATE TABLE instances (
                        instance_id TEXT PRIMARY KEY,
                        version_id TEXT
                    );
                    CREATE TABLE effects (
                        effect_id TEXT PRIMARY KEY,
                        instance_id TEXT
                    );
                    CREATE TABLE runs (
                        run_id TEXT PRIMARY KEY
                    );
                    CREATE TABLE diagnostics (
                        diagnostic_id TEXT PRIMARY KEY,
                        instance_id TEXT,
                        severity TEXT NOT NULL,
                        code TEXT,
                        message TEXT NOT NULL,
                        source_span_json TEXT,
                        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                    );
                    "#,
                )
                .expect("v1 schema creates");
        }

        let store = SqliteStore::open(&path).expect("store opens v1 schema");
        let diagnostic_id = store
            .record_diagnostic(DiagnosticRecord {
                instance_id: None,
                program_id: Some("program-v1"),
                program_version_id: Some("version-v1"),
                severity: Severity::Warning,
                code: Some("compile.unused"),
                message: "unused binding",
                source_span_json: None,
                subject_type: Some("program_version"),
                subject_id: Some("version-v1"),
                event_id: None,
                effect_id: None,
                run_id: None,
                assertion_id: None,
                evidence_ids_json: "[]",
                artifact_ids_json: "[]",
                causation_id: None,
                correlation_id: Some("compile"),
                idempotency_key: Some("diag:compile:unused"),
            })
            .expect("diagnostic records on upgraded schema");

        let diagnostics = store.list_diagnostics(None).expect("diagnostics list");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].diagnostic_id, diagnostic_id);
        assert_eq!(diagnostics[0].program_id.as_deref(), Some("program-v1"));
        assert_eq!(
            diagnostics[0].program_version_id.as_deref(),
            Some("version-v1")
        );

        fs::remove_file(path).expect("v1 db removes");
    }

    #[test]
    fn records_and_updates_workspace_records_for_provider_runs() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("WorkspaceRecords", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[test_effect("tell", "agent.tell", "workspace-record-effect")],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-workspace-record"),
            })
            .expect("rule commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell",
                run_id: "run-workspace",
                provider: "codex-main",
                worker_id: "worker-1",
                lease_id: "lease-workspace",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");

        let workspace_id = store
            .record_workspace(WorkspaceRecord {
                instance_id: Some(&instance.instance_id),
                effect_id: Some("tell"),
                run_id: Some("run-workspace"),
                provider: Some("codex-main"),
                policy: "per_effect_worktree",
                uri: "file:///tmp/whipplescript/worktrees/run-workspace",
                status: "prepared",
                metadata_json: r#"{"source":"test"}"#,
            })
            .expect("workspace records");
        let same_workspace_id = store
            .record_workspace(WorkspaceRecord {
                instance_id: Some(&instance.instance_id),
                effect_id: Some("tell"),
                run_id: Some("run-workspace"),
                provider: Some("codex-main"),
                policy: "per_effect_worktree",
                uri: "file:///tmp/whipplescript/worktrees/run-workspace",
                status: "active",
                metadata_json: r#"{"source":"test","updated":true}"#,
            })
            .expect("workspace upserts");

        assert_eq!(workspace_id, same_workspace_id);
        let workspace = store
            .get_workspace(&workspace_id)
            .expect("workspace loads")
            .expect("workspace exists");
        assert_eq!(workspace.policy, "per_effect_worktree");
        assert_eq!(workspace.status, "active");
        assert_eq!(workspace.run_id.as_deref(), Some("run-workspace"));
        assert!(workspace.metadata_json.contains("\"updated\":true"));
        let workspaces = store
            .list_workspaces_for_instance(&instance.instance_id)
            .expect("workspaces list");
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].workspace_id, workspace_id);
    }

    #[test]
    fn workspace_records_reject_unknown_policy_status_and_invalid_metadata() {
        let store = SqliteStore::open_in_memory().expect("store opens");

        let bad_policy = store.record_workspace(WorkspaceRecord {
            instance_id: None,
            effect_id: None,
            run_id: None,
            provider: Some("codex-main"),
            policy: "host_everything",
            uri: "file:///tmp/workspace",
            status: "prepared",
            metadata_json: "{}",
        });
        assert!(
            matches!(bad_policy, Err(StoreError::Conflict(message)) if message.contains("unsupported workspace policy"))
        );

        let bad_status = store.record_workspace(WorkspaceRecord {
            instance_id: None,
            effect_id: None,
            run_id: None,
            provider: Some("codex-main"),
            policy: "shared",
            uri: "file:///tmp/workspace",
            status: "teleported",
            metadata_json: "{}",
        });
        assert!(
            matches!(bad_status, Err(StoreError::Conflict(message)) if message.contains("unsupported workspace status"))
        );

        let bad_metadata = store.record_workspace(WorkspaceRecord {
            instance_id: None,
            effect_id: None,
            run_id: None,
            provider: Some("codex-main"),
            policy: "shared",
            uri: "file:///tmp/workspace",
            status: "prepared",
            metadata_json: "{not json",
        });
        assert!(matches!(bad_metadata, Err(StoreError::Json(_))));
    }

    #[test]
    fn opening_v1_store_adds_workspace_table() {
        let path = std::env::temp_dir().join(format!(
            "whipplescript-store-workspace-v1-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        {
            let connection = Connection::open(&path).expect("v1 db opens");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE schema_migrations (
                        version INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                    );
                    INSERT INTO schema_migrations (version, name)
                    VALUES (1, 'runtime-store-schema');
                    CREATE TABLE instances (
                        instance_id TEXT PRIMARY KEY,
                        version_id TEXT
                    );
                    CREATE TABLE effects (
                        effect_id TEXT PRIMARY KEY,
                        instance_id TEXT
                    );
                    CREATE TABLE runs (
                        run_id TEXT PRIMARY KEY
                    );
                    CREATE TABLE diagnostics (
                        diagnostic_id TEXT PRIMARY KEY,
                        instance_id TEXT,
                        severity TEXT NOT NULL,
                        code TEXT,
                        message TEXT NOT NULL,
                        source_span_json TEXT,
                        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                    );
                    "#,
                )
                .expect("v1 schema creates");
        }

        let store = SqliteStore::open(&path).expect("store opens v1 schema");
        assert!(store.table_exists("workspaces").expect("table check"));
        let workspace_id = store
            .record_workspace(WorkspaceRecord {
                instance_id: None,
                effect_id: None,
                run_id: None,
                provider: Some("fixture"),
                policy: "shared",
                uri: "file:///tmp/shared",
                status: "prepared",
                metadata_json: "{}",
            })
            .expect("workspace records on upgraded schema");
        assert!(store
            .get_workspace(&workspace_id)
            .expect("workspace loads")
            .is_some());

        fs::remove_file(path).expect("v1 db removes");
    }

    #[test]
    fn missing_capability_blocks_effect_before_provider_run() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("Ralph", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            required_capabilities_json: r#"["plugin.memory"]"#,
            ..test_effect("memory", "agent.tell", "rule=start;effect=memory")
        }];

        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        let blocked = store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "memory",
                run_id: "run-memory",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-memory",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect_err("policy blocks run");

        assert!(matches!(blocked, StoreError::PolicyBlocked { .. }));
        assert_eq!(effect_status(&store, "memory"), "blocked_by_capability");
        assert_eq!(row_count(&store, "runs"), 0);
        let effects = store
            .list_effects(&instance.instance_id)
            .expect("effects list");
        assert!(effects[0]
            .policy_block_reason
            .as_deref()
            .expect("policy reason")
            .contains("plugin.memory"));
        let status = store
            .status(&instance.instance_id)
            .expect("status loads")
            .expect("instance exists");
        assert_eq!(status.blocked_effect_count, 1);
        assert_eq!(
            status
                .recent_events
                .last()
                .map(|event| event.event_type.as_str()),
            Some("effect.blocked")
        );
    }

    #[test]
    fn profile_mismatch_blocks_effect_before_provider_run() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("Ralph", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "write",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"write"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=write",
            required_capabilities_json: r#"["agent.tell","repo.write"]"#,
            profile: Some("repo-reader"),
            correlation_id: None,
            source_span_json: None,
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let blocked = store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "write",
                run_id: "run-write",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-write",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect_err("profile blocks run");

        assert!(matches!(blocked, StoreError::PolicyBlocked { .. }));
        assert_eq!(effect_status(&store, "write"), "blocked_by_profile");
        let effect = store
            .list_effects(&instance.instance_id)
            .expect("effects list")
            .pop()
            .expect("effect exists");
        assert_eq!(effect.profile.as_deref(), Some("repo-reader"));
        assert!(effect
            .policy_block_reason
            .as_deref()
            .expect("policy reason")
            .contains("repo.write"));
    }

    #[test]
    fn policy_blocked_effects_are_not_claimable() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("Ralph", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            required_capabilities_json: r#"["plugin.memory"]"#,
            ..test_effect("memory", "agent.tell", "rule=start;effect=memory")
        }];

        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let claimable = store
            .claimable_effects(&instance.instance_id)
            .expect("claimable effects load");

        assert!(claimable.is_empty());
        assert_eq!(effect_status(&store, "memory"), "queued");
    }

    #[test]
    fn declared_agents_present_supports_map_shaped_metadata() {
        assert!(declared_agents_present(
            r#"{"worker":{"profile":"repo-writer","capacity":1,"capabilities":["agent.tell"]}}"#
        )
        .expect("metadata parses"));
        assert!(
            !declared_agents_present(r#"{"harnesses":[{"name":"coder","kind":"codex"}]}"#)
                .expect("metadata parses")
        );
    }

    #[test]
    fn agent_capacity_limits_claimability_and_run_start() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(NewProgramVersion {
                declared_profiles_json: r#"[{"name":"worker","profile":"repo-writer","capacity":1,"capabilities":["agent.tell"]}]"#,
                ..test_program_version("Capacity", "source-1", "ir-1")
            })
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let effects = [
            test_effect("tell-one", "agent.tell", "rule=start;effect=tell-one"),
            test_effect("tell-two", "agent.tell", "rule=start;effect=tell-two"),
        ];

        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        let claimable = store
            .claimable_effects(&instance.instance_id)
            .expect("claimable effects load");
        assert_eq!(
            claimable
                .iter()
                .map(|effect| effect.effect_id.as_str())
                .collect::<Vec<_>>(),
            vec!["tell-one", "tell-two"]
        );

        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell-one",
                run_id: "run-one",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-one",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("first run starts");
        let claimable = store
            .claimable_effects(&instance.instance_id)
            .expect("claimable effects reload");
        assert!(claimable.is_empty());

        let blocked = store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell-two",
                run_id: "run-two",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-two",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect_err("capacity blocks second run");
        assert!(
            matches!(blocked, StoreError::CapacityBlocked { reason, .. } if reason.contains("capacity exhausted"))
        );
        assert_eq!(effect_status(&store, "tell-two"), "blocked_by_capacity");
        let status = store
            .status(&instance.instance_id)
            .expect("status loads")
            .expect("instance exists");
        assert_eq!(status.blocked_effect_count, 1);
        assert_eq!(
            status
                .recent_events
                .last()
                .map(|event| event.event_type.as_str()),
            Some("effect.blocked")
        );

        store
            .complete_effect(EffectCompletion {
                instance_id: &instance.instance_id,
                effect_id: "tell-one",
                run_id: "run-one",
                provider: "test",
                worker_id: "worker-1",
                status: "completed",
                exit_code: Some(0),
                summary: Some("done"),
                metadata_json: "{}",
                idempotency_key: Some("complete-one"),
            })
            .expect("first run completes");
        let claimable = store
            .claimable_effects(&instance.instance_id)
            .expect("claimable effects after completion");
        assert_eq!(claimable.len(), 1);
        assert_eq!(claimable[0].effect_id, "tell-two");
    }

    #[test]
    fn undeclared_agent_targets_are_durably_blocked() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(NewProgramVersion {
                declared_profiles_json: r#"{"agents":[{"name":"worker","profile":"repo-writer","capacity":1}]}"#,
                ..test_program_version("AgentRefs", "source-1", "ir-1")
            })
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            target: Some("rogue"),
            ..test_effect("tell-rogue", "agent.tell", "rule=start;effect=tell-rogue")
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let blocked = store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell-rogue",
                run_id: "run-rogue",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-rogue",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect_err("undeclared agent blocks run");

        assert!(
            matches!(blocked, StoreError::PolicyBlocked { reason, .. } if reason.contains("not declared"))
        );
        assert_eq!(effect_status(&store, "tell-rogue"), "blocked_by_profile");
        let effect = store
            .list_effects(&instance.instance_id)
            .expect("effects load")
            .pop()
            .expect("effect exists");
        assert!(effect
            .policy_block_reason
            .as_deref()
            .expect("block reason")
            .contains("not declared"));
    }

    #[test]
    fn agent_missing_declared_capability_is_durably_blocked() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(NewProgramVersion {
                declared_profiles_json: r#"{"agents":[{"name":"worker","profile":"repo-writer","capacity":1,"capabilities":["agent.tell"]}]}"#,
                ..test_program_version("AgentCapabilities", "source-1", "ir-1")
            })
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            required_capabilities_json: r#"["repo.write"]"#,
            ..test_effect("tell-write", "agent.tell", "rule=start;effect=tell-write")
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let blocked = store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "tell-write",
                run_id: "run-write",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-write",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect_err("agent capability blocks run");

        assert!(
            matches!(blocked, StoreError::PolicyBlocked { reason, .. } if reason.contains("does not declare required capability `repo.write`"))
        );
        assert_eq!(effect_status(&store, "tell-write"), "blocked_by_capability");
        let effect = store
            .list_effects(&instance.instance_id)
            .expect("effects load")
            .pop()
            .expect("effect exists");
        assert!(effect
            .policy_block_reason
            .as_deref()
            .expect("block reason")
            .contains("repo.write"));
    }

    #[test]
    fn legacy_manifest_registered_effect_contract_can_run_without_kernel_changes() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("MemoryWorkflow", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let package_id = store
            .register_package_manifest(include_str!(
                "../../../examples/legacy-plugin-manifests/memory.json"
            ))
            .expect("legacy manifest loads");
        assert_eq!(package_id, "legacy-memory-package");

        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "query",
            kind: "memory.query",
            target: None,
            input_json: r#"{"query":"context"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=query",
            required_capabilities_json: "[]",
            profile: Some("memory-user"),
            correlation_id: None,
            source_span_json: None,
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        store
            .start_run(RunStart {
                instance_id: &instance.instance_id,
                effect_id: "query",
                run_id: "run-query",
                provider: "memory-legacy-provider",
                worker_id: "worker-1",
                lease_id: "lease-query",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("legacy manifest effect starts");

        assert_eq!(effect_status(&store, "query"), "running");
        assert_eq!(row_count(&store, "runs"), 1);
    }

    #[test]
    fn capability_call_uses_registered_target_capability_policy() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("MemoryWorkflow", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .register_package_manifest(include_str!(
                "../../../examples/legacy-plugin-manifests/memory.json"
            ))
            .expect("legacy manifest loads");

        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "query",
            kind: "capability.call",
            target: Some("memory.query"),
            input_json: r#"{"target":"memory.query"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=query",
            required_capabilities_json: r#"["memory.query"]"#,
            profile: Some("memory-user"),
            correlation_id: None,
            source_span_json: None,
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let claimable = store
            .claimable_effects(&instance.instance_id)
            .expect("claimable effects load");
        assert_eq!(claimable.len(), 1);
        assert_eq!(claimable[0].effect_id, "query");
    }

    #[test]
    fn package_manifest_shape_registers_runtime_capability_policy() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("MemoryWorkflow", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        let package_id = store
            .register_package_manifest(include_str!("../../../examples/packages/memory.json"))
            .expect("package manifest loads");
        assert_eq!(package_id, "package-memory");

        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "query",
            kind: "capability.call",
            target: Some("memory.query"),
            input_json: r#"{"target":"memory.query"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=query",
            required_capabilities_json: "[]",
            profile: Some("memory-user"),
            correlation_id: None,
            source_span_json: None,
        }];
        store
            .commit_rule(RuleCommit {
                instance_id: &instance.instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let claimable = store
            .claimable_effects(&instance.instance_id)
            .expect("claimable effects load");
        assert_eq!(claimable.len(), 1);
        assert_eq!(claimable[0].effect_id, "query");
    }

    #[test]
    fn loads_legacy_manifests_from_directory() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let loaded = store
            .load_package_manifests_from_dir(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../examples/legacy-plugin-manifests"),
            )
            .expect("legacy manifests load");

        assert_eq!(
            loaded,
            vec![
                "legacy-external-notification-package".to_owned(),
                "legacy-memory-package".to_owned(),
            ]
        );
    }

    #[test]
    fn registers_attaches_and_records_skill_evidence() {
        let mut store = SqliteStore::open_in_memory().expect("store opens");
        let version = store
            .create_program_version(test_program_version("Ralph", "source-1", "ir-1"))
            .expect("program version creates");
        let instance = store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json: "{}",
            })
            .expect("instance creates");
        store
            .register_skill(SkillRegistration {
                skill_id: "skill-loft-user",
                name: "loft-user",
                version: "1.0.0",
                source: "# Loft User\nUse Loft carefully.\n",
                source_path: "skills/loft-user/SKILL.md",
                body: "# Loft User\nUse Loft carefully.\n",
                description: "Loft workflow instructions",
                required_capabilities_json: r#"["loft.claim"]"#,
                metadata_json: r#"{"package":"core"}"#,
            })
            .expect("skill registers");
        store
            .attach_skill(SkillAttachment {
                attachment_id: "attach-program-loft",
                scope_type: "program",
                scope_id: &version.program_id,
                skill_name: "loft-user",
            })
            .expect("program skill attaches");
        store
            .attach_skill(SkillAttachment {
                attachment_id: "attach-agent-loft",
                scope_type: "agent",
                scope_id: "Ralph/worker",
                skill_name: "loft-user",
            })
            .expect("agent skill attaches");
        store
            .attach_skill(SkillAttachment {
                attachment_id: "attach-run-loft",
                scope_type: "run",
                scope_id: "run-tell",
                skill_name: "loft-user",
            })
            .expect("run skill attaches");

        let skills = store.list_skills().expect("skills list");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "loft-user");
        assert_eq!(skills[0].version, "1.0.0");
        assert_eq!(skills[0].source_path, "skills/loft-user/SKILL.md");
        assert!(skills[0].content_hash.len() >= 16);

        let program_attachments = store
            .list_skill_attachments("program", &version.program_id)
            .expect("program attachments load");
        assert_eq!(program_attachments.len(), 1);
        assert_eq!(program_attachments[0].skill.name, "loft-user");
        let agent_attachments = store
            .list_skill_attachments("agent", "Ralph/worker")
            .expect("agent attachments load");
        assert_eq!(agent_attachments.len(), 1);
        let run_attachments = store
            .list_skill_attachments("run", "run-tell")
            .expect("run attachments load");
        assert_eq!(run_attachments.len(), 1);

        let evidence_id = store
            .record_skill_evidence(SkillEvidence {
                instance_id: &instance.instance_id,
                run_id: "run-tell",
                effect_id: "tell",
                skill_names: &["loft-user"],
                idempotency_key: Some("skills-run-tell"),
            })
            .expect("skill evidence records");
        assert!(evidence_id.starts_with("evd_"));
        let evidence = store
            .list_evidence(&instance.instance_id)
            .expect("evidence lists");
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].kind, "skills.injected");
        assert!(evidence[0]
            .metadata_json
            .contains("skills/loft-user/SKILL.md"));
        assert!(evidence[0].metadata_json.contains("content_hash"));
        assert!(evidence[0]
            .summary
            .as_deref()
            .expect("summary")
            .contains("loft-user@1.0.0"));
    }

    #[test]
    fn provider_validation_evidence_records_refs_and_reopens() {
        let path = std::env::temp_dir().join(format!(
            "whipplescript-provider-validation-evidence-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time after epoch")
                .as_nanos()
        ));
        {
            let mut store = SqliteStore::open(&path).expect("store opens");
            let version = store
                .create_program_version(test_program_version("ProviderEvidence", "source", "ir"))
                .expect("program version creates");
            let instance = store
                .create_instance(NewInstance {
                    program_id: &version.program_id,
                    version_id: &version.version_id,
                    input_json: "{}",
                })
                .expect("instance creates");
            let evidence_id = store
                .record_provider_validation_evidence(ProviderValidationEvidence {
                    instance_id: &instance.instance_id,
                    provider_id: "codex-main",
                    provider_kind: "codex",
                    surface: "codex_app_server",
                    status: "pass",
                    config_json: r#"{"provider_id":"codex-main","provider_kind":"codex","surface":"codex_app_server","credentials_ref":"secret:codex","extra_keys":["api_key"]}"#,
                    capability_json: r#"{"provider_kind":"codex","surface":"codex_app_server","cancellation_depths":["native_stop"]}"#,
                    validation_results_json: r#"[{"provider":"codex-main","surface":"codex_app_server","status":"pass","phase":"provider.surface.valid","code":"surface_supported","message":"ok","recoverable":false,"missing_config_refs":[]}]"#,
                    source_path: Some("providers/native.json"),
                    correlation_id: Some("provider-validation:codex-main"),
                })
                .expect("provider validation evidence records");
            assert!(evidence_id.starts_with("evd_"));
            let evidence = store
                .list_evidence(&instance.instance_id)
                .expect("evidence lists");
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].kind, "provider.validation");
            assert_eq!(evidence[0].subject_type, "provider_config");
            assert_eq!(evidence[0].subject_id, "codex-main");
            let metadata =
                serde_json::from_str::<Value>(&evidence[0].metadata_json).expect("metadata json");
            assert_eq!(
                metadata.get("provider_kind").and_then(Value::as_str),
                Some("codex")
            );
            assert_eq!(
                metadata
                    .pointer("/config/extra_keys/0")
                    .and_then(Value::as_str),
                Some("api_key")
            );
            assert_eq!(
                metadata
                    .pointer("/validation_results/0/code")
                    .and_then(Value::as_str),
                Some("surface_supported")
            );
            let links = store
                .list_evidence_links(&instance.instance_id)
                .expect("evidence links list");
            assert!(links.iter().any(|link| {
                link.evidence_id == evidence_id
                    && link.target_type == "provider_config"
                    && link.target_id == "codex-main"
                    && link.relation == "subject"
            }));
            assert!(links.iter().any(|link| {
                link.evidence_id == evidence_id
                    && link.target_type == "provider"
                    && link.target_id == "codex-main"
                    && link.relation == "validates"
            }));
            assert!(links.iter().any(|link| {
                link.evidence_id == evidence_id
                    && link.target_type == "provider_capability"
                    && link.target_id == "codex:codex_app_server"
                    && link.relation == "uses"
            }));
        }
        {
            let store = SqliteStore::open(&path).expect("store reopens");
            let evidence = store
                .list_evidence_for_subject("provider_config", "codex-main")
                .expect("subject evidence lists");
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].kind, "provider.validation");
            assert_eq!(
                evidence[0].correlation_id.as_deref(),
                Some("provider-validation:codex-main")
            );
        }
        fs::remove_file(path).expect("provider evidence db removes");
    }

    #[test]
    fn codex_app_server_evidence_records_refs_and_reopens() {
        let path = std::env::temp_dir().join(format!(
            "whipplescript-codex-app-server-evidence-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time after epoch")
                .as_nanos()
        ));
        {
            let mut store = SqliteStore::open(&path).expect("store opens");
            let version = store
                .create_program_version(test_program_version("CodexEvidence", "source", "ir"))
                .expect("program version creates");
            let instance = store
                .create_instance(NewInstance {
                    program_id: &version.program_id,
                    version_id: &version.version_id,
                    input_json: "{}",
                })
                .expect("instance creates");
            let evidence_id = store
                .record_codex_app_server_evidence(CodexAppServerEvidence {
                    instance_id: &instance.instance_id,
                    provider_id: "codex-main",
                    thread_id: "thread-1",
                    turn_id: "turn-1",
                    metadata_json: r#"{"approvalRequests":[{"method":"item/commandExecution/requestApproval","commandBytes":14}],"toolRequests":[],"diffNotifications":[{"method":"turn/diff/updated","diffBytes":42}],"itemNotifications":[]}"#,
                    correlation_id: Some("codex-app-server:turn-1"),
                })
                .expect("codex evidence records");
            assert!(evidence_id.starts_with("evd_"));
            let evidence = store
                .list_evidence(&instance.instance_id)
                .expect("evidence lists");
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].kind, "codex.app_server.evidence");
            assert_eq!(evidence[0].subject_type, "provider_turn");
            assert_eq!(evidence[0].subject_id, "turn-1");
            let metadata =
                serde_json::from_str::<Value>(&evidence[0].metadata_json).expect("metadata json");
            assert_eq!(
                metadata.get("provider_id").and_then(Value::as_str),
                Some("codex-main")
            );
            assert_eq!(
                metadata
                    .pointer("/evidence/diffNotifications/0/diffBytes")
                    .and_then(Value::as_i64),
                Some(42)
            );
            let links = store
                .list_evidence_links(&instance.instance_id)
                .expect("evidence links list");
            assert!(links.iter().any(|link| {
                link.evidence_id == evidence_id
                    && link.target_type == "provider"
                    && link.target_id == "codex-main"
                    && link.relation == "observes"
            }));
            assert!(links.iter().any(|link| {
                link.evidence_id == evidence_id
                    && link.target_type == "provider_thread"
                    && link.target_id == "thread-1"
                    && link.relation == "observes"
            }));
        }
        {
            let store = SqliteStore::open(&path).expect("store reopens");
            let evidence = store
                .list_evidence_for_subject("provider_turn", "turn-1")
                .expect("subject evidence lists");
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].kind, "codex.app_server.evidence");
            assert_eq!(
                evidence[0].correlation_id.as_deref(),
                Some("codex-app-server:turn-1")
            );
        }
        fs::remove_file(path).expect("codex evidence db removes");
    }

    #[test]
    fn claude_agent_sdk_evidence_records_refs_and_reopens() {
        let path = std::env::temp_dir().join(format!(
            "whipplescript-claude-agent-sdk-evidence-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time after epoch")
                .as_nanos()
        ));
        {
            let mut store = SqliteStore::open(&path).expect("store opens");
            let version = store
                .create_program_version(test_program_version("ClaudeEvidence", "source", "ir"))
                .expect("program version creates");
            let instance = store
                .create_instance(NewInstance {
                    program_id: &version.program_id,
                    version_id: &version.version_id,
                    input_json: "{}",
                })
                .expect("instance creates");
            let evidence_id = store
                .record_claude_agent_sdk_evidence(ClaudeAgentSdkEvidence {
                    instance_id: &instance.instance_id,
                    provider_id: "claude-main",
                    session_id: "session-1",
                    run_id: "run-1",
                    metadata_json: r#"{"session_id":"session-1","event_counts":{"claude.stream.message":2,"claude.turn.completed":1},"terminal_type":"claude.turn.completed","terminal_payload":{"subtype":"success","result_shape":{"type":"string","chars":12},"usage_shape":{"type":"object","keys":2}}}"#,
                    correlation_id: Some("claude-agent-sdk:run-1"),
                })
                .expect("claude evidence records");
            assert!(evidence_id.starts_with("evd_"));
            let evidence = store
                .list_evidence(&instance.instance_id)
                .expect("evidence lists");
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].kind, "claude.agent_sdk.evidence");
            assert_eq!(evidence[0].subject_type, "provider_session");
            assert_eq!(evidence[0].subject_id, "session-1");
            assert_eq!(evidence[0].causation_id.as_deref(), Some("run-1"));
            let metadata =
                serde_json::from_str::<Value>(&evidence[0].metadata_json).expect("metadata json");
            assert_eq!(
                metadata.get("provider_id").and_then(Value::as_str),
                Some("claude-main")
            );
            assert_eq!(
                metadata
                    .pointer("/evidence/event_counts/claude.turn.completed")
                    .and_then(Value::as_i64),
                Some(1)
            );
            let links = store
                .list_evidence_links(&instance.instance_id)
                .expect("evidence links list");
            assert!(links.iter().any(|link| {
                link.evidence_id == evidence_id
                    && link.target_type == "provider"
                    && link.target_id == "claude-main"
                    && link.relation == "observes"
            }));
            assert!(links.iter().any(|link| {
                link.evidence_id == evidence_id
                    && link.target_type == "provider_run"
                    && link.target_id == "run-1"
                    && link.relation == "observes"
            }));
        }
        {
            let store = SqliteStore::open(&path).expect("store reopens");
            let evidence = store
                .list_evidence_for_subject("provider_session", "session-1")
                .expect("subject evidence lists");
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].kind, "claude.agent_sdk.evidence");
            assert_eq!(
                evidence[0].correlation_id.as_deref(),
                Some("claude-agent-sdk:run-1")
            );
        }
        fs::remove_file(path).expect("claude evidence db removes");
    }

    #[test]
    fn pi_rpc_evidence_records_refs_and_reopens() {
        let path = std::env::temp_dir().join(format!(
            "whipplescript-pi-rpc-evidence-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time after epoch")
                .as_nanos()
        ));
        {
            let mut store = SqliteStore::open(&path).expect("store opens");
            let version = store
                .create_program_version(test_program_version("PiEvidence", "source", "ir"))
                .expect("program version creates");
            let instance = store
                .create_instance(NewInstance {
                    program_id: &version.program_id,
                    version_id: &version.version_id,
                    input_json: "{}",
                })
                .expect("instance creates");
            let evidence_id = store
                .record_pi_rpc_evidence(PiRpcEvidence {
                    instance_id: &instance.instance_id,
                    provider_id: "pi-main",
                    session_id: "session-1",
                    run_id: "run-1",
                    metadata_json: r#"{"session_id":"session-1","model_provider":"openai-codex","model_id":"gpt-5.5","event_counts":{"message":2,"tool_call":1},"terminal_type":"completed","terminal_payload":{"result_shape":{"type":"string","chars":12}}}"#,
                    correlation_id: Some("pi-rpc:run-1"),
                })
                .expect("pi evidence records");
            assert!(evidence_id.starts_with("evd_"));
            let evidence = store
                .list_evidence(&instance.instance_id)
                .expect("evidence lists");
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].kind, "pi.rpc.evidence");
            assert_eq!(evidence[0].subject_type, "provider_session");
            assert_eq!(evidence[0].subject_id, "session-1");
            assert_eq!(evidence[0].causation_id.as_deref(), Some("run-1"));
            let metadata =
                serde_json::from_str::<Value>(&evidence[0].metadata_json).expect("metadata json");
            assert_eq!(
                metadata.get("provider_id").and_then(Value::as_str),
                Some("pi-main")
            );
            assert_eq!(
                metadata
                    .pointer("/evidence/event_counts/tool_call")
                    .and_then(Value::as_i64),
                Some(1)
            );
            assert_eq!(
                metadata
                    .pointer("/evidence/model_provider")
                    .and_then(Value::as_str),
                Some("openai-codex")
            );
            let links = store
                .list_evidence_links(&instance.instance_id)
                .expect("evidence links list");
            assert!(links.iter().any(|link| {
                link.evidence_id == evidence_id
                    && link.target_type == "provider"
                    && link.target_id == "pi-main"
                    && link.relation == "observes"
            }));
            assert!(links.iter().any(|link| {
                link.evidence_id == evidence_id
                    && link.target_type == "provider_run"
                    && link.target_id == "run-1"
                    && link.relation == "observes"
            }));
        }
        {
            let store = SqliteStore::open(&path).expect("store reopens");
            let evidence = store
                .list_evidence_for_subject("provider_session", "session-1")
                .expect("subject evidence lists");
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].kind, "pi.rpc.evidence");
            assert_eq!(evidence[0].correlation_id.as_deref(), Some("pi-rpc:run-1"));
        }
        fs::remove_file(path).expect("pi evidence db removes");
    }

    fn new_event<'a>(
        instance_id: &'a str,
        event_type: &'a str,
        idempotency_key: Option<&'a str>,
    ) -> NewEvent<'a> {
        NewEvent {
            instance_id,
            event_type,
            payload_json: "{}",
            source: "test",
            causation_id: None,
            correlation_id: None,
            idempotency_key,
        }
    }

    fn test_fact<'a>(fact_id: &'a str, name: &'a str, key: &'a str) -> NewFact<'a> {
        NewFact {
            fact_id,
            name,
            key,
            value_json: r#"{"title":"Implement store"}"#,
            schema_id: Some("WorkItem"),
            provenance_class: "derived",
            correlation_id: None,
            source_span_json: None,
        }
    }

    fn test_program_version<'a>(
        program_name: &'a str,
        source_hash: &'a str,
        ir_hash: &'a str,
    ) -> NewProgramVersion<'a> {
        let analysis_summary_json = Box::leak(
            json!({
                "workflow": program_name,
                "workflow_contracts": [],
                "schemas": [],
            })
            .to_string()
            .into_boxed_str(),
        );
        NewProgramVersion {
            program_name,
            source_hash,
            ir_hash,
            compiler_version: "test",
            declared_capabilities_json: "[]",
            declared_profiles_json: "[]",
            declared_skills_json: "[]",
            declared_schemas_json: "[]",
            analysis_summary_json,
            generated_artifacts_json: "[]",
            artifact_root: None,
        }
    }

    fn test_effect<'a>(
        effect_id: &'a str,
        kind: &'a str,
        idempotency_key: &'a str,
    ) -> NewEffect<'a> {
        NewEffect {
            timeout_seconds: None,
            effect_id,
            kind,
            target: Some("worker"),
            input_json: r#"{"prompt":"go"}"#,
            status: "queued",
            idempotency_key,
            required_capabilities_json: "[]",
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }
    }

    fn test_completion(run_id: &str) -> EffectCompletion<'_> {
        EffectCompletion {
            instance_id: "instance-a",
            effect_id: "tell",
            run_id,
            provider: "test",
            worker_id: "worker-1",
            status: "completed",
            exit_code: Some(0),
            summary: Some("done"),
            metadata_json: "{}",
            idempotency_key: None,
        }
    }

    fn row_count(store: &SqliteStore, table: &str) -> i64 {
        store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("row count")
    }

    fn effect_revision(store: &SqliteStore, effect_id: &str) -> (Option<String>, i64, bool) {
        store
            .connection
            .query_row(
                r#"
                SELECT
                    program_version_id,
                    revision_epoch,
                    EXISTS (
                        SELECT 1
                        FROM effect_cancellation_requests AS request
                        WHERE request.effect_id = effects.effect_id
                          AND request.instance_id = effects.instance_id
                          AND request.status = 'requested'
                    )
                FROM effects
                WHERE effect_id = ?1
                "#,
                [effect_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("effect revision")
    }

    fn effect_status(store: &SqliteStore, effect_id: &str) -> String {
        store
            .connection
            .query_row(
                "SELECT status FROM effects WHERE effect_id = ?1",
                [effect_id],
                |row| row.get(0),
            )
            .expect("effect status")
    }

    fn lease_status(store: &SqliteStore, lease_id: &str) -> String {
        store
            .connection
            .query_row(
                "SELECT status FROM leases WHERE lease_id = ?1",
                [lease_id],
                |row| row.get(0),
            )
            .expect("lease status")
    }

    fn instance_status(store: &SqliteStore, instance_id: &str) -> String {
        store
            .connection
            .query_row(
                "SELECT status FROM instances WHERE instance_id = ?1",
                [instance_id],
                |row| row.get(0),
            )
            .expect("instance status")
    }
}
