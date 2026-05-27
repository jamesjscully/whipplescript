//! Durable workflow interpreter skeleton.
//!
//! This crate owns runtime state, durable event queues, append-only logs,
//! effect dispatch contracts, and status projections. It executes validated
//! workflow IR from `armature-workflow`.

use armature_workflow::{expr::Expr, ir::AgentTarget, ir::State, WorkflowIr};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use thiserror::Error;

pub mod queue {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct WorkflowEvent {
        pub event_id: String,
        pub workflow_id: String,
        pub event_type: String,
        pub payload: serde_json::Value,
        pub source: Option<EventSource>,
        pub occurred_at: Option<String>,
        pub enqueued_at: Option<String>,
        pub correlation_id: Option<String>,
        pub causation_id: Option<String>,
        pub dedupe_key: Option<String>,
        pub status: EventStatus,
        pub attempt_count: u32,
        pub last_error: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct EventSource {
        pub kind: String,
        pub name: Option<String>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum EventStatus {
        Queued,
        Processing,
        Processed,
        Ignored,
        Failed,
        DeadLettered,
    }

    #[derive(Debug, Default)]
    pub struct InMemoryEventQueue {
        events: Vec<WorkflowEvent>,
    }

    impl InMemoryEventQueue {
        pub fn push(&mut self, event: WorkflowEvent) {
            self.events.push(event);
        }

        pub fn pop_front(&mut self) -> Option<WorkflowEvent> {
            if self.events.is_empty() {
                None
            } else {
                Some(self.events.remove(0))
            }
        }
    }
}

pub mod log {
    use serde::{Deserialize, Serialize};

    #[allow(clippy::large_enum_variant)]
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum WorkflowLogRecord {
        Transition {
            transition_id: String,
            workflow_id: String,
            from_state: String,
            to_state: String,
            #[serde(default)]
            event_type: Option<String>,
            event_id: Option<String>,
        },
        Effect {
            effect_id: String,
            workflow_id: String,
            transition_id: String,
            #[serde(default)]
            idempotency_key: Option<String>,
            effect: String,
            category: crate::effects::EffectCategory,
            target: Option<String>,
            args: serde_json::Value,
            #[serde(default)]
            required_capabilities: Vec<String>,
            status: EffectStatus,
            outcome: Option<crate::effects::EffectOutcome>,
        },
        Diagnostic {
            message: String,
        },
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum EffectStatus {
        Intended,
        Dispatched,
        Succeeded,
        Failed,
    }

    #[derive(Debug, Default)]
    pub struct AppendOnlyLog {
        records: Vec<WorkflowLogRecord>,
    }

    impl AppendOnlyLog {
        pub fn append(&mut self, record: WorkflowLogRecord) {
            self.records.push(record);
        }

        pub fn records(&self) -> &[WorkflowLogRecord] {
            &self.records
        }
    }
}

pub mod effects {
    use serde::{Deserialize, Serialize};
    use thiserror::Error;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct EffectRequest {
        pub effect_id: String,
        pub workflow_id: String,
        pub transition_id: String,
        pub idempotency_key: String,
        pub effect: String,
        pub category: EffectCategory,
        pub target: Option<String>,
        pub args: serde_json::Value,
        pub required_capabilities: Vec<String>,
        pub timeout_ms: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct EffectOutcome {
        pub effect_id: String,
        pub status: EffectOutcomeStatus,
        pub accepted: bool,
        pub invocation_id: Option<String>,
        #[serde(default)]
        pub required_capabilities: Vec<String>,
        pub output: Option<serde_json::Value>,
        pub error: Option<String>,
        pub completed_at: Option<String>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum EffectCategory {
        Context,
        SyncValue,
        AsyncInvocation,
        Message,
        HumanObligation,
        Event,
        Timer,
        Terminal,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum EffectOutcomeStatus {
        Accepted,
        Rejected,
        Succeeded,
        Failed,
    }

    #[derive(Debug, Error)]
    pub enum EffectError {
        #[error("effect `{0}` is not supported by this adapter")]
        Unsupported(String),
        #[error("{0}")]
        RuntimeRejected(String),
        #[error("{message}")]
        CapabilityDenied {
            message: String,
            required_capabilities: Vec<String>,
        },
    }

    impl EffectError {
        pub fn required_capabilities(&self) -> &[String] {
            match self {
                Self::CapabilityDenied {
                    required_capabilities,
                    ..
                } => required_capabilities,
                Self::Unsupported(_) | Self::RuntimeRejected(_) => &[],
            }
        }
    }

    pub trait EffectDispatcher {
        fn dispatch(&mut self, request: EffectRequest) -> Result<EffectOutcome, EffectError>;
    }

    #[derive(Debug, Default)]
    pub struct NoopEffectDispatcher;

    impl EffectDispatcher for NoopEffectDispatcher {
        fn dispatch(&mut self, request: EffectRequest) -> Result<EffectOutcome, EffectError> {
            Ok(EffectOutcome {
                effect_id: request.effect_id,
                status: EffectOutcomeStatus::Succeeded,
                accepted: true,
                invocation_id: Some(request.idempotency_key),
                required_capabilities: request.required_capabilities,
                output: None,
                error: None,
                completed_at: None,
            })
        }
    }
}

use effects::{EffectCategory, EffectRequest};

pub mod coerce {
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};
    use thiserror::Error;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct CoerceRequest {
        pub workflow_id: String,
        pub function_name: String,
        pub args: BTreeMap<String, serde_json::Value>,
        pub idempotency_key: Option<String>,
        pub event_id: Option<String>,
        pub step_path: Option<String>,
        pub backend: CoerceBackend,
        pub timeout_ms: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct CoerceOutcome {
        pub function_name: String,
        pub status: CoerceStatus,
        pub value: Option<serde_json::Value>,
        pub backend: CoerceBackend,
        pub http_status: Option<u16>,
        pub raw_response: Option<serde_json::Value>,
        pub error: Option<String>,
        pub duration_ms: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct CoerceCallRecord {
        pub coerce_call_id: String,
        pub workflow_id: String,
        pub workflow_version: String,
        pub transition_id: Option<String>,
        pub event_id: Option<String>,
        pub step_path: String,
        pub function_name: String,
        pub idempotency_key: String,
        pub backend: CoerceBackend,
        pub args: BTreeMap<String, serde_json::Value>,
        pub status: CoerceStatus,
        pub http_status: Option<u16>,
        pub raw_response: Option<serde_json::Value>,
        pub parsed_output: Option<serde_json::Value>,
        pub error: Option<String>,
        pub duration_ms: Option<u64>,
        pub created_at: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum CoerceBackend {
        None,
        Fake,
        BamlHttp {
            url: String,
            baml_src_hash: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            runtime_mode: Option<String>,
        },
        BamlGeneratedStdio {
            baml_src_hash: Option<String>,
            runner_hash: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            runtime_mode: Option<String>,
        },
        BamlBrokered {
            request_id: String,
            baml_src_hash: Option<String>,
        },
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CoerceStatus {
        Succeeded,
        Failed,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CoerceErrorCategory {
        MissingExecutor,
        MissingFakeOutput,
        BamlServerUnavailable,
        BamlRunnerUnavailable,
        BamlRunnerProtocolError,
        BamlBrokerUnavailable,
        BamlHttpError,
        BamlTimeout,
        BamlParseFailure,
        BamlSchemaValidationFailure,
        BamlPolicyDenied,
        InternalError,
    }

    #[derive(Debug, Error)]
    #[error("{category:?}: {message}")]
    pub struct CoerceError {
        pub category: CoerceErrorCategory,
        pub message: String,
        pub http_status: Option<u16>,
    }

    impl CoerceError {
        pub fn new(category: CoerceErrorCategory, message: impl Into<String>) -> Self {
            Self {
                category,
                message: message.into(),
                http_status: None,
            }
        }

        pub fn with_http_status(mut self, http_status: Option<u16>) -> Self {
            self.http_status = http_status;
            self
        }
    }

    pub trait CoerceExecutor: std::fmt::Debug {
        fn coerce(&self, request: CoerceRequest) -> Result<CoerceOutcome, CoerceError>;
    }

    #[derive(Debug, Default)]
    pub struct NoopCoerceExecutor;

    impl CoerceExecutor for NoopCoerceExecutor {
        fn coerce(&self, request: CoerceRequest) -> Result<CoerceOutcome, CoerceError> {
            Err(CoerceError::new(
                CoerceErrorCategory::MissingExecutor,
                format!(
                    "no coerce executor configured for `{}`",
                    request.function_name
                ),
            ))
        }
    }

    #[derive(Debug, Default)]
    pub struct FakeCoerceExecutor {
        outputs: BTreeMap<String, serde_json::Value>,
    }

    impl FakeCoerceExecutor {
        pub fn new(outputs: BTreeMap<String, serde_json::Value>) -> Self {
            Self { outputs }
        }
    }

    impl CoerceExecutor for FakeCoerceExecutor {
        fn coerce(&self, request: CoerceRequest) -> Result<CoerceOutcome, CoerceError> {
            let value = self
                .outputs
                .get(&request.function_name)
                .cloned()
                .ok_or_else(|| {
                    CoerceError::new(
                        CoerceErrorCategory::MissingFakeOutput,
                        format!("no fake output configured for `{}`", request.function_name),
                    )
                })?;

            Ok(CoerceOutcome {
                function_name: request.function_name,
                status: CoerceStatus::Succeeded,
                value: Some(value),
                backend: CoerceBackend::Fake,
                http_status: None,
                raw_response: None,
                error: None,
                duration_ms: None,
            })
        }
    }

    #[derive(Debug, Clone)]
    pub struct BamlHttpCoerceExecutor {
        base_url: String,
        baml_src_hash: Option<String>,
        runtime_mode: Option<String>,
        timeout_ms: Option<u64>,
        api_key: Option<String>,
        store_raw_response: bool,
    }

    impl BamlHttpCoerceExecutor {
        pub fn new(base_url: impl Into<String>) -> Self {
            Self {
                base_url: base_url.into(),
                baml_src_hash: None,
                runtime_mode: None,
                timeout_ms: None,
                api_key: None,
                store_raw_response: true,
            }
        }

        pub fn with_baml_src_hash(mut self, baml_src_hash: Option<String>) -> Self {
            self.baml_src_hash = baml_src_hash;
            self
        }

        pub fn with_runtime_mode(mut self, runtime_mode: Option<String>) -> Self {
            self.runtime_mode = runtime_mode;
            self
        }

        pub fn with_timeout_ms(mut self, timeout_ms: Option<u64>) -> Self {
            self.timeout_ms = timeout_ms;
            self
        }

        pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
            self.api_key = api_key;
            self
        }

        pub fn with_store_raw_response(mut self, store_raw_response: bool) -> Self {
            self.store_raw_response = store_raw_response;
            self
        }

        fn endpoint(&self, function_name: &str) -> String {
            format!(
                "{}/call/{}",
                self.base_url.trim_end_matches('/'),
                function_name
            )
        }

        fn backend(&self) -> CoerceBackend {
            CoerceBackend::BamlHttp {
                url: self.base_url.clone(),
                baml_src_hash: self.baml_src_hash.clone(),
                runtime_mode: self.runtime_mode.clone(),
            }
        }
    }

    impl CoerceExecutor for BamlHttpCoerceExecutor {
        fn coerce(&self, request: CoerceRequest) -> Result<CoerceOutcome, CoerceError> {
            let timeout_ms = request.timeout_ms.or(self.timeout_ms).unwrap_or(30_000);
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(timeout_ms))
                .build()
                .map_err(|error| {
                    CoerceError::new(
                        CoerceErrorCategory::InternalError,
                        format!("failed to build BAML HTTP client: {error}"),
                    )
                })?;
            let endpoint = self.endpoint(&request.function_name);
            let started = Instant::now();
            let mut builder = client.post(&endpoint).json(&request.args);
            if let Some(api_key) = &self.api_key {
                builder = builder.header("x-baml-api-key", api_key);
            }

            let response = builder.send().map_err(|error| {
                let category = if error.is_timeout() {
                    CoerceErrorCategory::BamlTimeout
                } else if error.is_connect() {
                    CoerceErrorCategory::BamlServerUnavailable
                } else {
                    CoerceErrorCategory::BamlHttpError
                };
                CoerceError::new(
                    category,
                    format!("failed to call BAML HTTP endpoint `{endpoint}`: {error}"),
                )
            })?;
            let status = response.status();
            let http_status = Some(status.as_u16());
            if !status.is_success() {
                let body = response.text().unwrap_or_default();
                return Err(CoerceError::new(
                    CoerceErrorCategory::BamlHttpError,
                    format!(
                        "BAML HTTP endpoint `{endpoint}` returned status {}: {}",
                        status,
                        body.trim()
                    ),
                )
                .with_http_status(http_status));
            }

            let value = response.json::<serde_json::Value>().map_err(|error| {
                CoerceError::new(
                    CoerceErrorCategory::BamlParseFailure,
                    format!("BAML HTTP response from `{endpoint}` was not valid JSON: {error}"),
                )
                .with_http_status(http_status)
            })?;
            let duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

            Ok(CoerceOutcome {
                function_name: request.function_name,
                status: CoerceStatus::Succeeded,
                value: Some(value.clone()),
                backend: self.backend(),
                http_status,
                raw_response: if self.store_raw_response {
                    Some(value)
                } else {
                    Some(serde_json::json!({
                        "redacted": true,
                        "reason": "policy"
                    }))
                },
                error: None,
                duration_ms: Some(duration_ms),
            })
        }
    }

    pub struct DurableCoerceExecutor {
        store: crate::storage::WorkflowStore,
        inner: Box<dyn CoerceExecutor>,
        workflow_version: String,
    }

    impl std::fmt::Debug for DurableCoerceExecutor {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("DurableCoerceExecutor")
                .field("workflow_version", &self.workflow_version)
                .field("inner", &self.inner)
                .finish_non_exhaustive()
        }
    }

    impl DurableCoerceExecutor {
        pub fn new(
            store: crate::storage::WorkflowStore,
            inner: Box<dyn CoerceExecutor>,
            workflow_version: impl Into<String>,
        ) -> Self {
            Self {
                store,
                inner,
                workflow_version: workflow_version.into(),
            }
        }
    }

    impl CoerceExecutor for DurableCoerceExecutor {
        fn coerce(&self, request: CoerceRequest) -> Result<CoerceOutcome, CoerceError> {
            let idempotency_key = request
                .idempotency_key
                .clone()
                .unwrap_or_else(|| derived_idempotency_key(&request, &self.workflow_version));
            if let Some(record) = self
                .store
                .find_successful_coerce_call(&request.workflow_id, &idempotency_key)
                .map_err(storage_error)?
            {
                return Ok(CoerceOutcome {
                    function_name: record.function_name,
                    status: record.status,
                    value: record.parsed_output,
                    backend: record.backend,
                    http_status: record.http_status,
                    raw_response: record.raw_response,
                    error: record.error,
                    duration_ms: record.duration_ms,
                });
            }

            match self.inner.coerce(request.clone()) {
                Ok(outcome) => {
                    self.store
                        .append_coerce_call_attempt(&record_from_outcome(
                            &request,
                            &idempotency_key,
                            &self.workflow_version,
                            &outcome,
                        ))
                        .map_err(storage_error)?;
                    Ok(outcome)
                }
                Err(error) => {
                    let record = failed_record_from_error(
                        &request,
                        &idempotency_key,
                        &self.workflow_version,
                        &error,
                    );
                    self.store
                        .append_coerce_call_attempt(&record)
                        .map_err(storage_error)?;
                    Err(error)
                }
            }
        }
    }

    fn record_from_outcome(
        request: &CoerceRequest,
        idempotency_key: &str,
        workflow_version: &str,
        outcome: &CoerceOutcome,
    ) -> CoerceCallRecord {
        CoerceCallRecord {
            coerce_call_id: format!("coerce_{}", ulid::Ulid::new()),
            workflow_id: request.workflow_id.clone(),
            workflow_version: workflow_version.to_string(),
            transition_id: None,
            event_id: request.event_id.clone(),
            step_path: request
                .step_path
                .clone()
                .unwrap_or_else(|| "expression".to_string()),
            function_name: request.function_name.clone(),
            idempotency_key: idempotency_key.to_string(),
            backend: outcome.backend.clone(),
            args: request.args.clone(),
            status: outcome.status,
            http_status: outcome.http_status,
            raw_response: outcome.raw_response.clone(),
            parsed_output: outcome.value.clone(),
            error: outcome.error.clone(),
            duration_ms: outcome.duration_ms,
            created_at: crate::current_unix_millis().to_string(),
        }
    }

    fn failed_record_from_error(
        request: &CoerceRequest,
        idempotency_key: &str,
        workflow_version: &str,
        error: &CoerceError,
    ) -> CoerceCallRecord {
        CoerceCallRecord {
            coerce_call_id: format!("coerce_{}", ulid::Ulid::new()),
            workflow_id: request.workflow_id.clone(),
            workflow_version: workflow_version.to_string(),
            transition_id: None,
            event_id: request.event_id.clone(),
            step_path: request
                .step_path
                .clone()
                .unwrap_or_else(|| "expression".to_string()),
            function_name: request.function_name.clone(),
            idempotency_key: idempotency_key.to_string(),
            backend: request.backend.clone(),
            args: request.args.clone(),
            status: CoerceStatus::Failed,
            http_status: error.http_status,
            raw_response: None,
            parsed_output: None,
            error: Some(error.to_string()),
            duration_ms: None,
            created_at: crate::current_unix_millis().to_string(),
        }
    }

    fn derived_idempotency_key(request: &CoerceRequest, workflow_version: &str) -> String {
        format!(
            "{}/{}/{}/{}/{}/{}",
            request.workflow_id,
            workflow_version,
            request.event_id.as_deref().unwrap_or("<no-event>"),
            request.step_path.as_deref().unwrap_or("<no-step>"),
            request.function_name,
            serde_json::to_string(&request.args).unwrap_or_default()
        )
    }

    fn storage_error(error: crate::storage::StorageError) -> CoerceError {
        CoerceError::new(
            CoerceErrorCategory::InternalError,
            format!("coerce storage error: {error}"),
        )
    }
}

pub mod state {
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct WorkflowState {
        pub workflow_name: String,
        pub current_state: String,
        #[serde(default)]
        pub context: BTreeMap<String, serde_json::Value>,
    }
}

pub mod status {
    use crate::queue::EventSource;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct WorkflowStatus {
        pub workflow_id: String,
        pub workflow_name: String,
        pub current_state: String,
        pub blocked_reason: Option<String>,
        #[serde(default)]
        pub data: BTreeMap<String, serde_json::Value>,
        #[serde(default)]
        pub data_summary: BTreeMap<String, serde_json::Value>,
        pub pending_events: usize,
        #[serde(default)]
        pub queued_events: Vec<QueuedEventSummary>,
        pub active_invocations: Vec<ActiveInvocation>,
        pub recent_transition: Option<String>,
        #[serde(default)]
        pub recent_effects: Vec<EffectSummary>,
        #[serde(default)]
        pub latest_coerce_calls: Vec<CoerceCallSummary>,
        #[serde(default)]
        pub latest_coerce_failures: Vec<CoerceCallSummary>,
        #[serde(default)]
        pub current_coerce_failure: Option<CoerceCallSummary>,
        #[serde(default)]
        pub baml_runtime: Option<BamlRuntimeSummary>,
        #[serde(default)]
        pub current_effect_failures: Vec<String>,
        #[serde(default)]
        pub policy_blockers: Vec<String>,
        #[serde(default)]
        pub current_blockers: Vec<String>,
        pub recent_failures: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ActiveInvocation {
        pub agent: String,
        pub count: u32,
        pub max: Option<u32>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct EffectSummary {
        pub effect_id: String,
        pub idempotency_key: Option<String>,
        pub effect: String,
        pub status: crate::log::EffectStatus,
        pub target: Option<String>,
        pub args: serde_json::Value,
        pub required_capabilities: Vec<String>,
        pub error: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct CoerceCallSummary {
        pub coerce_call_id: String,
        pub function_name: String,
        pub status: crate::coerce::CoerceStatus,
        pub backend: crate::coerce::CoerceBackend,
        pub http_status: Option<u16>,
        pub parsed_output: Option<serde_json::Value>,
        pub error: Option<String>,
        pub duration_ms: Option<u64>,
        pub created_at: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct BamlRuntimeSummary {
        pub mode: String,
        pub status: String,
        pub url: String,
        pub baml_src_hash: Option<String>,
        pub last_error: Option<String>,
        pub last_call_at: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct QueuedEventSummary {
        pub event_id: String,
        pub event_type: String,
        pub source: Option<EventSource>,
        pub attempt_count: u32,
    }
}

pub mod storage {
    use crate::coerce::{CoerceBackend, CoerceCallRecord, CoerceStatus};
    use crate::log::WorkflowLogRecord;
    use crate::queue::{EventStatus, WorkflowEvent};
    use crate::state::WorkflowState;
    use rusqlite::{params, Connection, OptionalExtension};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::rc::Rc;
    use thiserror::Error;

    const STORAGE_SCHEMA_VERSION: u32 = 4;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum AgentInvocationStatus {
        Queued,
        Claimed,
        Running,
        Succeeded,
        Failed,
        Cancelled,
        TimedOut,
        CompletionRejected,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct AgentInvocationRecord {
        pub workflow_id: String,
        pub invocation_id: String,
        pub agent: String,
        pub effect_id: String,
        pub transition_id: String,
        pub event_id: Option<String>,
        pub idempotency_key: String,
        pub input: serde_json::Value,
        pub requested_profile: Option<String>,
        pub resolved_profile: Option<String>,
        pub profile_enforcement: Option<String>,
        pub status: AgentInvocationStatus,
        pub claimed_by: Option<String>,
        pub claim_expires_at: Option<String>,
        pub provider: Option<String>,
        pub provider_run_id: Option<String>,
        pub run_dir: Option<String>,
        pub stdout_path: Option<String>,
        pub stderr_path: Option<String>,
        pub exit_code: Option<i32>,
        pub error: Option<String>,
        pub created_at: String,
        pub updated_at: String,
    }

    pub struct AgentInvocationStartedUpdate<'a> {
        pub invocation_id: &'a str,
        pub provider: Option<&'a str>,
        pub provider_run_id: Option<&'a str>,
        pub resolved_profile: Option<&'a str>,
        pub profile_enforcement: Option<&'a str>,
        pub run_dir: Option<&'a str>,
        pub stdout_path: Option<&'a str>,
        pub stderr_path: Option<&'a str>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct AgentMessageRecord {
        pub workflow_id: String,
        pub message_id: String,
        pub agent: String,
        pub invocation_id: Option<String>,
        pub effect_id: String,
        pub transition_id: String,
        pub event_id: Option<String>,
        pub idempotency_key: String,
        pub message: serde_json::Value,
        pub status: String,
        pub created_at: String,
        pub updated_at: String,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct AgentCompletionRecord {
        pub workflow_id: String,
        pub completion_id: String,
        pub invocation_id: String,
        pub agent: String,
        pub status: String,
        pub summary: Option<String>,
        pub exit_code: Option<i32>,
        pub event_id: Option<String>,
        pub payload: serde_json::Value,
        pub created_at: String,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct HarnessEventRecord {
        pub workflow_id: String,
        pub event_id: String,
        pub invocation_id: Option<String>,
        pub kind: String,
        pub payload: serde_json::Value,
        pub created_at: String,
    }

    #[derive(Debug, Error)]
    pub enum StorageError {
        #[error("sqlite error: {0}")]
        Sqlite(#[from] rusqlite::Error),
        #[error("json error: {0}")]
        Json(#[from] serde_json::Error),
        #[error("workflow event not found: workflow_id={workflow_id}, event_id={event_id}")]
        EventNotFound {
            workflow_id: String,
            event_id: String,
        },
        #[error(
            "workflow event cannot be retried from status {status}: workflow_id={workflow_id}, event_id={event_id}"
        )]
        EventRetryNotAllowed {
            workflow_id: String,
            event_id: String,
            status: String,
        },
        #[error(
            "unsupported workflow store schema version {found}; supported version is {supported}"
        )]
        UnsupportedSchemaVersion { found: u32, supported: u32 },
        #[error("invalid workflow store schema version `{0}`")]
        InvalidSchemaVersion(String),
    }

    pub struct WorkflowStore {
        connection: Rc<Connection>,
    }

    impl Clone for WorkflowStore {
        fn clone(&self) -> Self {
            Self {
                connection: Rc::clone(&self.connection),
            }
        }
    }

    impl WorkflowStore {
        pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StorageError> {
            let connection = Connection::open(path)?;
            let store = Self {
                connection: Rc::new(connection),
            };
            store.migrate()?;
            Ok(store)
        }

        pub fn open_in_memory() -> Result<Self, StorageError> {
            let connection = Connection::open_in_memory()?;
            let store = Self {
                connection: Rc::new(connection),
            };
            store.migrate()?;
            Ok(store)
        }

        pub fn connection(&self) -> &Connection {
            self.connection.as_ref()
        }

        pub fn migrate(&self) -> Result<(), StorageError> {
            if let Some(version) = self.storage_schema_version()? {
                if version > STORAGE_SCHEMA_VERSION {
                    return Err(StorageError::UnsupportedSchemaVersion {
                        found: version,
                        supported: STORAGE_SCHEMA_VERSION,
                    });
                }
            }

            self.connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS armature_meta (
                  key TEXT PRIMARY KEY NOT NULL,
                  value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS workflow_state (
                  workflow_id TEXT PRIMARY KEY NOT NULL,
                  workflow_name TEXT NOT NULL,
                  current_state TEXT NOT NULL,
                  context_json TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS workflow_events (
                  seq INTEGER PRIMARY KEY AUTOINCREMENT,
                  workflow_id TEXT NOT NULL,
                  event_id TEXT NOT NULL,
                  status TEXT NOT NULL,
                  event_json TEXT NOT NULL,
                  UNIQUE(workflow_id, event_id)
                );

                CREATE INDEX IF NOT EXISTS workflow_events_pending
                  ON workflow_events(workflow_id, status, seq);

                CREATE TABLE IF NOT EXISTS workflow_log (
                  seq INTEGER PRIMARY KEY AUTOINCREMENT,
                  workflow_id TEXT NOT NULL,
                  record_json TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS coerce_calls (
                  seq INTEGER PRIMARY KEY AUTOINCREMENT,
                  coerce_call_id TEXT NOT NULL UNIQUE,
                  workflow_id TEXT NOT NULL,
                  workflow_version TEXT NOT NULL,
                  transition_id TEXT,
                  event_id TEXT,
                  step_path TEXT NOT NULL,
                  function_name TEXT NOT NULL,
                  idempotency_key TEXT NOT NULL,
                  backend_json TEXT NOT NULL,
                  args_json TEXT NOT NULL,
                  status TEXT NOT NULL,
                  http_status INTEGER,
                  raw_response_json TEXT,
                  parsed_output_json TEXT,
                  error TEXT,
                  duration_ms INTEGER,
                  created_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS coerce_calls_latest
                  ON coerce_calls(workflow_id, function_name, seq DESC);

                CREATE UNIQUE INDEX IF NOT EXISTS coerce_calls_success_idempotency
                  ON coerce_calls(workflow_id, idempotency_key)
                  WHERE status = 'succeeded';

                CREATE TABLE IF NOT EXISTS agent_invocations (
                  seq INTEGER PRIMARY KEY AUTOINCREMENT,
                  workflow_id TEXT NOT NULL,
                  invocation_id TEXT NOT NULL UNIQUE,
                  agent TEXT NOT NULL,
                  effect_id TEXT NOT NULL,
                  transition_id TEXT NOT NULL,
                  event_id TEXT,
                  idempotency_key TEXT NOT NULL,
                  input_json TEXT NOT NULL,
                  requested_profile TEXT,
                  resolved_profile TEXT,
                  profile_enforcement TEXT,
                  status TEXT NOT NULL,
                  claimed_by TEXT,
                  claim_expires_at TEXT,
                  provider TEXT,
                  provider_run_id TEXT,
                  run_dir TEXT,
                  stdout_path TEXT,
                  stderr_path TEXT,
                  exit_code INTEGER,
                  error TEXT,
                  created_at TEXT NOT NULL,
                  updated_at TEXT NOT NULL,
                  UNIQUE(workflow_id, idempotency_key)
                );

                CREATE INDEX IF NOT EXISTS agent_invocations_status
                  ON agent_invocations(workflow_id, status, seq);

                CREATE INDEX IF NOT EXISTS agent_invocations_agent_status
                  ON agent_invocations(workflow_id, agent, status);

                CREATE INDEX IF NOT EXISTS agent_invocations_claim_expiry
                  ON agent_invocations(claim_expires_at);

                CREATE INDEX IF NOT EXISTS agent_invocations_provider_run
                  ON agent_invocations(provider, provider_run_id);

                CREATE TABLE IF NOT EXISTS agent_messages (
                  seq INTEGER PRIMARY KEY AUTOINCREMENT,
                  workflow_id TEXT NOT NULL,
                  message_id TEXT NOT NULL UNIQUE,
                  agent TEXT NOT NULL,
                  invocation_id TEXT,
                  effect_id TEXT NOT NULL,
                  transition_id TEXT NOT NULL,
                  event_id TEXT,
                  idempotency_key TEXT NOT NULL,
                  message_json TEXT NOT NULL,
                  status TEXT NOT NULL,
                  created_at TEXT NOT NULL,
                  updated_at TEXT NOT NULL,
                  UNIQUE(workflow_id, idempotency_key)
                );

                CREATE TABLE IF NOT EXISTS agent_completions (
                  seq INTEGER PRIMARY KEY AUTOINCREMENT,
                  workflow_id TEXT NOT NULL,
                  completion_id TEXT NOT NULL UNIQUE,
                  invocation_id TEXT NOT NULL,
                  agent TEXT NOT NULL,
                  status TEXT NOT NULL,
                  summary TEXT,
                  exit_code INTEGER,
                  event_id TEXT,
                  payload_json TEXT NOT NULL,
                  created_at TEXT NOT NULL,
                  UNIQUE(workflow_id, invocation_id)
                );

                CREATE TABLE IF NOT EXISTS harness_events (
                  seq INTEGER PRIMARY KEY AUTOINCREMENT,
                  workflow_id TEXT NOT NULL,
                  event_id TEXT NOT NULL UNIQUE,
                  invocation_id TEXT,
                  kind TEXT NOT NULL,
                  payload_json TEXT NOT NULL,
                  created_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS harness_events_recent
                  ON harness_events(workflow_id, seq DESC);

                CREATE INDEX IF NOT EXISTS harness_events_kind_recent
                  ON harness_events(workflow_id, kind, seq DESC);

                CREATE INDEX IF NOT EXISTS harness_events_invocation
                  ON harness_events(invocation_id, seq);
                "#,
            )?;
            self.migrate_workflow_event_identity()?;
            self.migrate_agent_invocation_profiles()?;
            self.set_storage_schema_version(STORAGE_SCHEMA_VERSION)?;
            Ok(())
        }

        fn storage_schema_version(&self) -> Result<Option<u32>, StorageError> {
            let meta_exists: i64 = self.connection.query_row(
                r#"
                SELECT COUNT(*)
                FROM sqlite_master
                WHERE type = 'table' AND name = 'armature_meta'
                "#,
                [],
                |row| row.get(0),
            )?;
            if meta_exists == 0 {
                return Ok(None);
            }

            let Some(version) = self
                .connection
                .query_row(
                    "SELECT value FROM armature_meta WHERE key = 'schema_version'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            else {
                return Ok(None);
            };

            version
                .parse::<u32>()
                .map(Some)
                .map_err(|_| StorageError::InvalidSchemaVersion(version))
        }

        fn set_storage_schema_version(&self, version: u32) -> Result<(), StorageError> {
            self.connection.execute(
                r#"
                INSERT INTO armature_meta (key, value)
                VALUES ('schema_version', ?1)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value
                "#,
                params![version.to_string()],
            )?;
            Ok(())
        }

        fn migrate_workflow_event_identity(&self) -> Result<(), StorageError> {
            if self.workflow_events_have_scoped_identity()? {
                return Ok(());
            }

            self.connection.execute_batch(
                r#"
                DROP INDEX IF EXISTS workflow_events_pending;

                ALTER TABLE workflow_events RENAME TO workflow_events_legacy_identity;

                CREATE TABLE workflow_events (
                  seq INTEGER PRIMARY KEY AUTOINCREMENT,
                  workflow_id TEXT NOT NULL,
                  event_id TEXT NOT NULL,
                  status TEXT NOT NULL,
                  event_json TEXT NOT NULL,
                  UNIQUE(workflow_id, event_id)
                );

                INSERT INTO workflow_events (
                  seq,
                  workflow_id,
                  event_id,
                  status,
                  event_json
                )
                SELECT
                  seq,
                  workflow_id,
                  event_id,
                  status,
                  event_json
                FROM workflow_events_legacy_identity;

                DROP TABLE workflow_events_legacy_identity;

                CREATE INDEX IF NOT EXISTS workflow_events_pending
                  ON workflow_events(workflow_id, status, seq);
                "#,
            )?;
            Ok(())
        }

        fn workflow_events_have_scoped_identity(&self) -> Result<bool, StorageError> {
            let mut indexes = self
                .connection
                .prepare("PRAGMA index_list('workflow_events')")?;
            let index_names = indexes.query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, bool>(2)?))
            })?;

            for index_name in index_names {
                let (index_name, is_unique) = index_name?;
                if !is_unique {
                    continue;
                }

                let escaped_index_name = index_name.replace('\'', "''");
                let mut index_info = self
                    .connection
                    .prepare(&format!("PRAGMA index_info('{escaped_index_name}')"))?;
                let columns = index_info.query_map([], |row| row.get::<_, String>(2))?;
                let columns = columns.collect::<Result<Vec<_>, _>>()?;
                if columns == ["workflow_id", "event_id"] {
                    return Ok(true);
                }
            }

            Ok(false)
        }

        fn migrate_agent_invocation_profiles(&self) -> Result<(), StorageError> {
            for column in [
                "requested_profile",
                "resolved_profile",
                "profile_enforcement",
            ] {
                if self.table_has_column("agent_invocations", column)? {
                    continue;
                }
                self.connection.execute(
                    &format!("ALTER TABLE agent_invocations ADD COLUMN {column} TEXT"),
                    [],
                )?;
            }
            Ok(())
        }

        fn table_has_column(&self, table: &str, column: &str) -> Result<bool, StorageError> {
            let escaped_table = table.replace('\'', "''");
            let mut columns = self
                .connection
                .prepare(&format!("PRAGMA table_info('{escaped_table}')"))?;
            let names = columns.query_map([], |row| row.get::<_, String>(1))?;
            for name in names {
                if name? == column {
                    return Ok(true);
                }
            }
            Ok(false)
        }

        pub fn save_state(&self, state: &WorkflowState) -> Result<(), StorageError> {
            self.connection.execute(
                r#"
                INSERT INTO workflow_state (
                  workflow_id,
                  workflow_name,
                  current_state,
                  context_json
                )
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(workflow_id) DO UPDATE SET
                  workflow_name = excluded.workflow_name,
                  current_state = excluded.current_state,
                  context_json = excluded.context_json
                "#,
                params![
                    state.workflow_name,
                    state.workflow_name,
                    state.current_state,
                    serde_json::to_string(&state.context)?,
                ],
            )?;
            Ok(())
        }

        pub fn load_state(&self, workflow_id: &str) -> Result<Option<WorkflowState>, StorageError> {
            self.connection
                .query_row(
                    r#"
                    SELECT workflow_name, current_state, context_json
                    FROM workflow_state
                    WHERE workflow_id = ?1
                    "#,
                    params![workflow_id],
                    |row| {
                        let context_json: String = row.get(2)?;
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            context_json,
                        ))
                    },
                )
                .optional()?
                .map(|(workflow_name, current_state, context_json)| {
                    Ok(WorkflowState {
                        workflow_name,
                        current_state,
                        context: serde_json::from_str(&context_json)?,
                    })
                })
                .transpose()
        }

        pub fn enqueue_event(&self, event: &WorkflowEvent) -> Result<(), StorageError> {
            self.connection.execute(
                r#"
                INSERT INTO workflow_events (
                  workflow_id,
                  event_id,
                  status,
                  event_json
                )
                VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    event.workflow_id,
                    event.event_id,
                    event_status_name(event.status),
                    serde_json::to_string(event)?,
                ],
            )?;
            Ok(())
        }

        pub fn dequeue_next_event(
            &self,
            workflow_id: &str,
        ) -> Result<Option<WorkflowEvent>, StorageError> {
            let Some((seq, event_json)) = self
                .connection
                .query_row(
                    r#"
                    SELECT seq, event_json
                    FROM workflow_events
                    WHERE workflow_id = ?1 AND status = 'queued'
                    ORDER BY seq
                    LIMIT 1
                    "#,
                    params![workflow_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?
            else {
                return Ok(None);
            };

            let mut event: WorkflowEvent = serde_json::from_str(&event_json)?;
            event.status = EventStatus::Processing;
            event.attempt_count = event.attempt_count.saturating_add(1);

            let rows_updated = self.connection.execute(
                r#"
                UPDATE workflow_events
                SET status = ?1, event_json = ?2
                WHERE seq = ?3
                "#,
                params![
                    event_status_name(event.status),
                    serde_json::to_string(&event)?,
                    seq,
                ],
            )?;
            if rows_updated == 0 {
                return Err(StorageError::EventNotFound {
                    workflow_id: event.workflow_id,
                    event_id: event.event_id,
                });
            }

            Ok(Some(event))
        }

        pub fn recover_processing_events(&self, workflow_id: &str) -> Result<usize, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT event_json
                FROM workflow_events
                WHERE workflow_id = ?1 AND status = 'processing'
                ORDER BY seq
                "#,
            )?;
            let rows = statement.query_map(params![workflow_id], |row| row.get::<_, String>(0))?;
            let mut events = Vec::new();
            for row in rows {
                let mut event: WorkflowEvent = serde_json::from_str(&row?)?;
                event.status = EventStatus::Queued;
                events.push(event);
            }
            drop(statement);

            let transaction = self.connection.unchecked_transaction()?;
            for event in &events {
                update_event_status_in(&transaction, event)?;
            }
            transaction.commit()?;
            Ok(events.len())
        }

        pub fn pending_event_count(&self, workflow_id: &str) -> Result<usize, StorageError> {
            let count: i64 = self.connection.query_row(
                r#"
                SELECT COUNT(*)
                FROM workflow_events
                WHERE workflow_id = ?1 AND status = 'queued'
                "#,
                params![workflow_id],
                |row| row.get(0),
            )?;
            Ok(count as usize)
        }

        pub fn queued_event_summaries(
            &self,
            workflow_id: &str,
            limit: usize,
        ) -> Result<Vec<crate::status::QueuedEventSummary>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT event_json
                FROM workflow_events
                WHERE workflow_id = ?1 AND status = 'queued'
                ORDER BY seq
                LIMIT ?2
                "#,
            )?;

            let rows = statement.query_map(params![workflow_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })?;
            let mut summaries = Vec::new();
            for row in rows {
                let event: WorkflowEvent = serde_json::from_str(&row?)?;
                summaries.push(crate::status::QueuedEventSummary {
                    event_id: event.event_id,
                    event_type: event.event_type,
                    source: event.source,
                    attempt_count: event.attempt_count,
                });
            }
            Ok(summaries)
        }

        pub fn events(
            &self,
            workflow_id: &str,
            limit: usize,
        ) -> Result<Vec<WorkflowEvent>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT event_json
                FROM workflow_events
                WHERE workflow_id = ?1
                ORDER BY seq DESC
                LIMIT ?2
                "#,
            )?;

            let rows = statement.query_map(params![workflow_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })?;
            let mut events = Vec::new();
            for row in rows {
                events.push(serde_json::from_str(&row?)?);
            }
            Ok(events)
        }

        pub fn events_by_status(
            &self,
            workflow_id: &str,
            status: EventStatus,
            limit: usize,
        ) -> Result<Vec<WorkflowEvent>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT event_json
                FROM workflow_events
                WHERE workflow_id = ?1 AND status = ?2
                ORDER BY seq DESC
                LIMIT ?3
                "#,
            )?;

            let rows = statement.query_map(
                params![workflow_id, event_status_name(status), limit as i64],
                |row| row.get::<_, String>(0),
            )?;
            let mut events = Vec::new();
            for row in rows {
                events.push(serde_json::from_str(&row?)?);
            }
            Ok(events)
        }

        pub fn event_by_id(
            &self,
            workflow_id: &str,
            event_id: &str,
        ) -> Result<Option<WorkflowEvent>, StorageError> {
            let event_json = self
                .connection
                .query_row(
                    r#"
                    SELECT event_json
                    FROM workflow_events
                    WHERE workflow_id = ?1 AND event_id = ?2
                    "#,
                    params![workflow_id, event_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            event_json
                .map(|event_json| serde_json::from_str(&event_json).map_err(StorageError::from))
                .transpose()
        }

        pub fn retry_event(
            &self,
            workflow_id: &str,
            event_id: &str,
        ) -> Result<WorkflowEvent, StorageError> {
            let mut event = self.event_by_id(workflow_id, event_id)?.ok_or_else(|| {
                StorageError::EventNotFound {
                    workflow_id: workflow_id.to_string(),
                    event_id: event_id.to_string(),
                }
            })?;
            match event.status {
                EventStatus::Failed | EventStatus::DeadLettered => {}
                status => {
                    return Err(StorageError::EventRetryNotAllowed {
                        workflow_id: workflow_id.to_string(),
                        event_id: event_id.to_string(),
                        status: event_status_name(status).to_string(),
                    });
                }
            }
            event.status = EventStatus::Queued;
            event.last_error = None;
            update_event_status_in(&self.connection, &event)?;
            Ok(event)
        }

        pub fn update_event_status(
            &self,
            event: &WorkflowEvent,
            status: EventStatus,
        ) -> Result<(), StorageError> {
            let mut event = event.clone();
            event.status = status;

            let rows_updated = self.connection.execute(
                r#"
                UPDATE workflow_events
                SET status = ?1, event_json = ?2
                WHERE workflow_id = ?3 AND event_id = ?4
                "#,
                params![
                    event_status_name(status),
                    serde_json::to_string(&event)?,
                    event.workflow_id,
                    event.event_id,
                ],
            )?;
            if rows_updated == 0 {
                return Err(StorageError::EventNotFound {
                    workflow_id: event.workflow_id,
                    event_id: event.event_id,
                });
            }
            Ok(())
        }

        pub fn commit_event_processing(
            &self,
            event: &WorkflowEvent,
            status: EventStatus,
            state: &WorkflowState,
            records: &[WorkflowLogRecord],
            agent_invocations: &[AgentInvocationRecord],
            agent_messages: &[AgentMessageRecord],
        ) -> Result<(), StorageError> {
            let transaction = self.connection.unchecked_transaction()?;
            let mut event = event.clone();
            event.status = status;
            update_event_status_in(&transaction, &event)?;
            save_state_in(&transaction, state)?;
            append_log_records_in(&transaction, &state.workflow_name, records)?;
            for invocation in agent_invocations {
                insert_agent_invocation_in(&transaction, invocation)?;
            }
            for message in agent_messages {
                insert_agent_message_in(&transaction, message)?;
            }
            transaction.commit()?;
            Ok(())
        }

        pub fn commit_event_failure(
            &self,
            event: &WorkflowEvent,
            records: &[WorkflowLogRecord],
        ) -> Result<(), StorageError> {
            let transaction = self.connection.unchecked_transaction()?;
            let mut event = event.clone();
            event.status = EventStatus::Failed;
            update_event_status_in(&transaction, &event)?;
            append_log_records_in(&transaction, &event.workflow_id, records)?;
            transaction.commit()?;
            Ok(())
        }

        pub fn append_log(
            &self,
            workflow_id: &str,
            record: &WorkflowLogRecord,
        ) -> Result<(), StorageError> {
            self.connection.execute(
                r#"
                INSERT INTO workflow_log (workflow_id, record_json)
                VALUES (?1, ?2)
                "#,
                params![workflow_id, serde_json::to_string(record)?],
            )?;
            Ok(())
        }

        pub fn insert_agent_invocation(
            &self,
            record: &AgentInvocationRecord,
        ) -> Result<(), StorageError> {
            insert_agent_invocation_in(&self.connection, record)
        }

        pub fn insert_agent_message(
            &self,
            record: &AgentMessageRecord,
        ) -> Result<(), StorageError> {
            insert_agent_message_in(&self.connection, record)
        }

        pub fn queued_agent_invocations(
            &self,
            workflow_id: &str,
            limit: usize,
        ) -> Result<Vec<AgentInvocationRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT
                  workflow_id, invocation_id, agent, effect_id, transition_id,
                  event_id, idempotency_key, input_json, requested_profile,
                  resolved_profile, profile_enforcement, status, claimed_by,
                  claim_expires_at, provider, provider_run_id, run_dir,
                  stdout_path, stderr_path, exit_code, error, created_at, updated_at
                FROM agent_invocations
                WHERE workflow_id = ?1 AND status = 'queued'
                ORDER BY seq
                LIMIT ?2
                "#,
            )?;
            let rows = statement.query_map(
                params![workflow_id, limit as i64],
                agent_invocation_record_from_row,
            )?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row?);
            }
            Ok(records)
        }

        pub fn recent_agent_invocations(
            &self,
            workflow_id: &str,
            limit: usize,
        ) -> Result<Vec<AgentInvocationRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT
                  workflow_id, invocation_id, agent, effect_id, transition_id,
                  event_id, idempotency_key, input_json, requested_profile,
                  resolved_profile, profile_enforcement, status, claimed_by,
                  claim_expires_at, provider, provider_run_id, run_dir,
                  stdout_path, stderr_path, exit_code, error, created_at, updated_at
                FROM agent_invocations
                WHERE workflow_id = ?1
                ORDER BY seq DESC
                LIMIT ?2
                "#,
            )?;
            let rows = statement.query_map(
                params![workflow_id, limit as i64],
                agent_invocation_record_from_row,
            )?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row?);
            }
            Ok(records)
        }

        pub fn recent_harness_events(
            &self,
            workflow_id: &str,
            limit: usize,
        ) -> Result<Vec<HarnessEventRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT workflow_id, event_id, invocation_id, kind, payload_json, created_at
                FROM harness_events
                WHERE workflow_id = ?1
                ORDER BY seq DESC
                LIMIT ?2
                "#,
            )?;
            let rows = statement.query_map(params![workflow_id, limit as i64], |row| {
                let payload_json: String = row.get(4)?;
                Ok(HarnessEventRecord {
                    workflow_id: row.get(0)?,
                    event_id: row.get(1)?,
                    invocation_id: row.get(2)?,
                    kind: row.get(3)?,
                    payload: serde_json::from_str(&payload_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            payload_json.len(),
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    created_at: row.get(5)?,
                })
            })?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row?);
            }
            Ok(records)
        }

        pub fn recent_agent_completions(
            &self,
            workflow_id: &str,
            limit: usize,
        ) -> Result<Vec<AgentCompletionRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT
                  workflow_id, completion_id, invocation_id, agent, status,
                  summary, exit_code, event_id, payload_json, created_at
                FROM agent_completions
                WHERE workflow_id = ?1
                ORDER BY seq DESC
                LIMIT ?2
                "#,
            )?;
            let rows = statement.query_map(params![workflow_id, limit as i64], |row| {
                let payload_json: String = row.get(8)?;
                Ok(AgentCompletionRecord {
                    workflow_id: row.get(0)?,
                    completion_id: row.get(1)?,
                    invocation_id: row.get(2)?,
                    agent: row.get(3)?,
                    status: row.get(4)?,
                    summary: row.get(5)?,
                    exit_code: row.get(6)?,
                    event_id: row.get(7)?,
                    payload: serde_json::from_str(&payload_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            payload_json.len(),
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    created_at: row.get(9)?,
                })
            })?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row?);
            }
            Ok(records)
        }

        pub fn recover_expired_agent_leases(
            &self,
            workflow_id: &str,
            now: &str,
        ) -> Result<Vec<AgentInvocationRecord>, StorageError> {
            let transaction = self.connection.unchecked_transaction()?;
            let mut statement = transaction.prepare(
                r#"
                SELECT
                  workflow_id, invocation_id, agent, effect_id, transition_id,
                  event_id, idempotency_key, input_json, requested_profile,
                  resolved_profile, profile_enforcement, status, claimed_by,
                  claim_expires_at, provider, provider_run_id, run_dir,
                  stdout_path, stderr_path, exit_code, error, created_at, updated_at
                FROM agent_invocations
                WHERE workflow_id = ?1
                  AND status IN ('claimed', 'running')
                  AND claim_expires_at IS NOT NULL
                  AND claim_expires_at <= ?2
                ORDER BY seq
                "#,
            )?;
            let rows =
                statement.query_map(params![workflow_id, now], agent_invocation_record_from_row)?;
            let mut recovered = Vec::new();
            for row in rows {
                recovered.push(row?);
            }
            drop(statement);

            if !recovered.is_empty() {
                transaction.execute(
                    r#"
                    UPDATE agent_invocations
                    SET status = 'queued',
                        claimed_by = NULL,
                        claim_expires_at = NULL,
                        error = NULL,
                        updated_at = ?3
                    WHERE workflow_id = ?1
                      AND status IN ('claimed', 'running')
                      AND claim_expires_at IS NOT NULL
                      AND claim_expires_at <= ?2
                    "#,
                    params![workflow_id, now, crate::current_unix_millis().to_string()],
                )?;
            }
            transaction.commit()?;
            Ok(recovered)
        }

        pub fn claim_agent_invocation(
            &self,
            invocation_id: &str,
            worker_id: &str,
            lease_until: &str,
        ) -> Result<Option<AgentInvocationRecord>, StorageError> {
            let transaction = self.connection.unchecked_transaction()?;
            let updated_at = crate::current_unix_millis().to_string();
            let rows = transaction.execute(
                r#"
                UPDATE agent_invocations
                SET status = 'claimed',
                    claimed_by = ?2,
                    claim_expires_at = ?3,
                    updated_at = ?4
                WHERE invocation_id = ?1
                  AND status = 'queued'
                "#,
                params![invocation_id, worker_id, lease_until, updated_at],
            )?;
            if rows == 0 {
                transaction.commit()?;
                return Ok(None);
            }
            let record = select_agent_invocation_by_id_in(&transaction, invocation_id)?;
            transaction.commit()?;
            Ok(record)
        }

        pub fn mark_agent_invocation_started(
            &self,
            update: AgentInvocationStartedUpdate<'_>,
        ) -> Result<(), StorageError> {
            self.connection.execute(
                r#"
                UPDATE agent_invocations
                SET status = 'running',
                    provider = ?2,
                    provider_run_id = ?3,
                    resolved_profile = ?4,
                    profile_enforcement = ?5,
                    run_dir = ?6,
                    stdout_path = ?7,
                    stderr_path = ?8,
                    updated_at = ?9
                WHERE invocation_id = ?1
                "#,
                params![
                    update.invocation_id,
                    update.provider,
                    update.provider_run_id,
                    update.resolved_profile,
                    update.profile_enforcement,
                    update.run_dir,
                    update.stdout_path,
                    update.stderr_path,
                    crate::current_unix_millis().to_string(),
                ],
            )?;
            Ok(())
        }

        pub fn mark_agent_invocation_exited(
            &self,
            invocation_id: &str,
            status: AgentInvocationStatus,
            exit_code: Option<i32>,
            error: Option<&str>,
        ) -> Result<(), StorageError> {
            self.connection.execute(
                r#"
                UPDATE agent_invocations
                SET status = ?2,
                    exit_code = ?3,
                    error = ?4,
                    updated_at = ?5
                WHERE invocation_id = ?1
                "#,
                params![
                    invocation_id,
                    agent_invocation_status_name(&status),
                    exit_code.map(i64::from),
                    error,
                    crate::current_unix_millis().to_string(),
                ],
            )?;
            Ok(())
        }

        pub fn record_agent_completion(
            &self,
            completion: &AgentCompletionRecord,
            event: &WorkflowEvent,
        ) -> Result<bool, StorageError> {
            let transaction = self.connection.unchecked_transaction()?;
            let existing: Option<String> = transaction
                .query_row(
                    r#"
                    SELECT completion_id
                    FROM agent_completions
                    WHERE workflow_id = ?1 AND invocation_id = ?2
                    "#,
                    params![&completion.workflow_id, &completion.invocation_id],
                    |row| row.get(0),
                )
                .optional()?;
            if existing.is_some() {
                transaction.commit()?;
                return Ok(false);
            }
            let inserted = insert_agent_completion_in(&transaction, completion)?;
            if inserted {
                insert_workflow_event_in(&transaction, event)?;
            }
            transaction.commit()?;
            Ok(inserted)
        }

        pub fn append_harness_event(
            &self,
            record: &HarnessEventRecord,
        ) -> Result<(), StorageError> {
            self.connection.execute(
                r#"
                INSERT INTO harness_events (
                  workflow_id, event_id, invocation_id, kind, payload_json, created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    &record.workflow_id,
                    &record.event_id,
                    &record.invocation_id,
                    &record.kind,
                    serde_json::to_string(&record.payload)?,
                    &record.created_at,
                ],
            )?;
            Ok(())
        }

        pub fn active_agent_invocation_counts(
            &self,
            workflow_id: &str,
        ) -> Result<BTreeMap<String, u32>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT agent, COUNT(*)
                FROM agent_invocations
                WHERE workflow_id = ?1
                  AND status IN ('queued', 'claimed', 'running')
                GROUP BY agent
                "#,
            )?;
            let rows = statement.query_map(params![workflow_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u32))
            })?;
            let mut counts = BTreeMap::new();
            for row in rows {
                let (agent, count) = row?;
                counts.insert(agent, count);
            }
            Ok(counts)
        }

        pub fn append_coerce_call_attempt(
            &self,
            record: &CoerceCallRecord,
        ) -> Result<(), StorageError> {
            self.connection.execute(
                r#"
                INSERT INTO coerce_calls (
                  coerce_call_id,
                  workflow_id,
                  workflow_version,
                  transition_id,
                  event_id,
                  step_path,
                  function_name,
                  idempotency_key,
                  backend_json,
                  args_json,
                  status,
                  http_status,
                  raw_response_json,
                  parsed_output_json,
                  error,
                  duration_ms,
                  created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                "#,
                params![
                    &record.coerce_call_id,
                    &record.workflow_id,
                    &record.workflow_version,
                    &record.transition_id,
                    &record.event_id,
                    &record.step_path,
                    &record.function_name,
                    &record.idempotency_key,
                    serde_json::to_string(&record.backend)?,
                    serde_json::to_string(&record.args)?,
                    coerce_status_name(record.status),
                    record.http_status.map(i64::from),
                    optional_json_to_string(record.raw_response.as_ref())?,
                    optional_json_to_string(record.parsed_output.as_ref())?,
                    &record.error,
                    record.duration_ms.map(|value| value as i64),
                    &record.created_at,
                ],
            )?;
            Ok(())
        }

        pub fn find_successful_coerce_call(
            &self,
            workflow_id: &str,
            idempotency_key: &str,
        ) -> Result<Option<CoerceCallRecord>, StorageError> {
            self.connection
                .query_row(
                    r#"
                    SELECT
                      coerce_call_id,
                      workflow_id,
                      workflow_version,
                      transition_id,
                      event_id,
                      step_path,
                      function_name,
                      idempotency_key,
                      backend_json,
                      args_json,
                      status,
                      http_status,
                      raw_response_json,
                      parsed_output_json,
                      error,
                      duration_ms,
                      created_at
                    FROM coerce_calls
                    WHERE workflow_id = ?1
                      AND idempotency_key = ?2
                      AND status = 'succeeded'
                    ORDER BY seq DESC
                    LIMIT 1
                    "#,
                    params![workflow_id, idempotency_key],
                    coerce_call_record_from_row,
                )
                .optional()
                .map_err(Into::into)
        }

        pub fn latest_coerce_calls(
            &self,
            workflow_id: &str,
            limit: usize,
        ) -> Result<Vec<CoerceCallRecord>, StorageError> {
            self.coerce_calls_by_filter(workflow_id, None, limit)
        }

        pub fn latest_coerce_failures(
            &self,
            workflow_id: &str,
            limit: usize,
        ) -> Result<Vec<CoerceCallRecord>, StorageError> {
            self.coerce_calls_by_filter(workflow_id, Some(CoerceStatus::Failed), limit)
        }

        fn coerce_calls_by_filter(
            &self,
            workflow_id: &str,
            status: Option<CoerceStatus>,
            limit: usize,
        ) -> Result<Vec<CoerceCallRecord>, StorageError> {
            let sql = match status {
                Some(_) => {
                    r#"
                    SELECT
                      coerce_call_id,
                      workflow_id,
                      workflow_version,
                      transition_id,
                      event_id,
                      step_path,
                      function_name,
                      idempotency_key,
                      backend_json,
                      args_json,
                      status,
                      http_status,
                      raw_response_json,
                      parsed_output_json,
                      error,
                      duration_ms,
                      created_at
                    FROM coerce_calls
                    WHERE workflow_id = ?1 AND status = ?2
                    ORDER BY seq DESC
                    LIMIT ?3
                    "#
                }
                None => {
                    r#"
                    SELECT
                      coerce_call_id,
                      workflow_id,
                      workflow_version,
                      transition_id,
                      event_id,
                      step_path,
                      function_name,
                      idempotency_key,
                      backend_json,
                      args_json,
                      status,
                      http_status,
                      raw_response_json,
                      parsed_output_json,
                      error,
                      duration_ms,
                      created_at
                    FROM coerce_calls
                    WHERE workflow_id = ?1
                    ORDER BY seq DESC
                    LIMIT ?2
                    "#
                }
            };
            let mut statement = self.connection.prepare(sql)?;
            let mut records = Vec::new();
            match status {
                Some(status) => {
                    let rows = statement.query_map(
                        params![workflow_id, coerce_status_name(status), limit as i64],
                        coerce_call_record_from_row,
                    )?;
                    for row in rows {
                        records.push(row?);
                    }
                }
                None => {
                    let rows = statement.query_map(
                        params![workflow_id, limit as i64],
                        coerce_call_record_from_row,
                    )?;
                    for row in rows {
                        records.push(row?);
                    }
                }
            }
            Ok(records)
        }

        pub fn log_records(
            &self,
            workflow_id: &str,
        ) -> Result<Vec<WorkflowLogRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT record_json
                FROM workflow_log
                WHERE workflow_id = ?1
                ORDER BY seq
                "#,
            )?;

            let rows = statement.query_map(params![workflow_id], |row| row.get::<_, String>(0))?;
            let mut records = Vec::new();
            for row in rows {
                records.push(serde_json::from_str(&row?)?);
            }
            Ok(records)
        }

        pub fn recent_log_records(
            &self,
            workflow_id: &str,
            limit: usize,
        ) -> Result<Vec<WorkflowLogRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                r#"
                SELECT record_json
                FROM workflow_log
                WHERE workflow_id = ?1
                ORDER BY seq DESC
                LIMIT ?2
                "#,
            )?;

            let rows = statement.query_map(params![workflow_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })?;
            let mut records = Vec::new();
            for row in rows {
                records.push(serde_json::from_str(&row?)?);
            }
            Ok(records)
        }
    }

    fn coerce_status_name(status: CoerceStatus) -> &'static str {
        match status {
            CoerceStatus::Succeeded => "succeeded",
            CoerceStatus::Failed => "failed",
        }
    }

    fn coerce_status_from_name(status: &str) -> CoerceStatus {
        match status {
            "succeeded" => CoerceStatus::Succeeded,
            _ => CoerceStatus::Failed,
        }
    }

    fn optional_json_to_string(
        value: Option<&serde_json::Value>,
    ) -> Result<Option<String>, StorageError> {
        value
            .map(serde_json::to_string)
            .transpose()
            .map_err(Into::into)
    }

    fn optional_json_from_string(
        value: Option<String>,
    ) -> Result<Option<serde_json::Value>, rusqlite::Error> {
        value
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        value.len(),
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .transpose()
    }

    fn coerce_call_record_from_row(
        row: &rusqlite::Row<'_>,
    ) -> Result<CoerceCallRecord, rusqlite::Error> {
        let backend_json: String = row.get(8)?;
        let args_json: String = row.get(9)?;
        let status: String = row.get(10)?;
        let raw_response_json: Option<String> = row.get(12)?;
        let parsed_output_json: Option<String> = row.get(13)?;
        let http_status: Option<i64> = row.get(11)?;
        let duration_ms: Option<i64> = row.get(15)?;
        Ok(CoerceCallRecord {
            coerce_call_id: row.get(0)?,
            workflow_id: row.get(1)?,
            workflow_version: row.get(2)?,
            transition_id: row.get(3)?,
            event_id: row.get(4)?,
            step_path: row.get(5)?,
            function_name: row.get(6)?,
            idempotency_key: row.get(7)?,
            backend: serde_json::from_str::<CoerceBackend>(&backend_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    backend_json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
            args: serde_json::from_str::<std::collections::BTreeMap<String, serde_json::Value>>(
                &args_json,
            )
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    args_json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
            status: coerce_status_from_name(&status),
            http_status: http_status.map(|value| value as u16),
            raw_response: optional_json_from_string(raw_response_json)?,
            parsed_output: optional_json_from_string(parsed_output_json)?,
            error: row.get(14)?,
            duration_ms: duration_ms.map(|value| value as u64),
            created_at: row.get(16)?,
        })
    }

    fn event_status_name(status: EventStatus) -> &'static str {
        match status {
            EventStatus::Queued => "queued",
            EventStatus::Processing => "processing",
            EventStatus::Processed => "processed",
            EventStatus::Ignored => "ignored",
            EventStatus::Failed => "failed",
            EventStatus::DeadLettered => "dead_lettered",
        }
    }

    fn agent_invocation_status_name(status: &AgentInvocationStatus) -> &'static str {
        match status {
            AgentInvocationStatus::Queued => "queued",
            AgentInvocationStatus::Claimed => "claimed",
            AgentInvocationStatus::Running => "running",
            AgentInvocationStatus::Succeeded => "succeeded",
            AgentInvocationStatus::Failed => "failed",
            AgentInvocationStatus::Cancelled => "cancelled",
            AgentInvocationStatus::TimedOut => "timed_out",
            AgentInvocationStatus::CompletionRejected => "completion_rejected",
        }
    }

    fn agent_invocation_status_from_name(status: &str) -> AgentInvocationStatus {
        match status {
            "queued" => AgentInvocationStatus::Queued,
            "claimed" => AgentInvocationStatus::Claimed,
            "running" => AgentInvocationStatus::Running,
            "succeeded" => AgentInvocationStatus::Succeeded,
            "failed" => AgentInvocationStatus::Failed,
            "cancelled" => AgentInvocationStatus::Cancelled,
            "timed_out" => AgentInvocationStatus::TimedOut,
            "completion_rejected" => AgentInvocationStatus::CompletionRejected,
            _ => AgentInvocationStatus::Failed,
        }
    }

    fn insert_workflow_event_in(
        connection: &rusqlite::Connection,
        event: &WorkflowEvent,
    ) -> Result<(), StorageError> {
        connection.execute(
            r#"
            INSERT INTO workflow_events (
              workflow_id,
              event_id,
              status,
              event_json
            )
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                event.workflow_id,
                event.event_id,
                event_status_name(event.status),
                serde_json::to_string(event)?,
            ],
        )?;
        Ok(())
    }

    fn insert_agent_invocation_in(
        connection: &rusqlite::Connection,
        record: &AgentInvocationRecord,
    ) -> Result<(), StorageError> {
        connection.execute(
            r#"
            INSERT INTO agent_invocations (
              workflow_id, invocation_id, agent, effect_id, transition_id,
              event_id, idempotency_key, input_json, requested_profile,
                  resolved_profile, profile_enforcement, status, claimed_by,
              claim_expires_at, provider, provider_run_id, run_dir,
              stdout_path, stderr_path, exit_code, error, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)
            ON CONFLICT(workflow_id, idempotency_key) DO NOTHING
            "#,
            params![
                &record.workflow_id,
                &record.invocation_id,
                &record.agent,
                &record.effect_id,
                &record.transition_id,
                &record.event_id,
                &record.idempotency_key,
                serde_json::to_string(&record.input)?,
                &record.requested_profile,
                &record.resolved_profile,
                &record.profile_enforcement,
                agent_invocation_status_name(&record.status),
                &record.claimed_by,
                &record.claim_expires_at,
                &record.provider,
                &record.provider_run_id,
                &record.run_dir,
                &record.stdout_path,
                &record.stderr_path,
                record.exit_code.map(i64::from),
                &record.error,
                &record.created_at,
                &record.updated_at,
            ],
        )?;
        Ok(())
    }

    fn insert_agent_message_in(
        connection: &rusqlite::Connection,
        record: &AgentMessageRecord,
    ) -> Result<(), StorageError> {
        connection.execute(
            r#"
            INSERT INTO agent_messages (
              workflow_id, message_id, agent, invocation_id, effect_id,
              transition_id, event_id, idempotency_key, message_json, status,
              created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(workflow_id, idempotency_key) DO NOTHING
            "#,
            params![
                &record.workflow_id,
                &record.message_id,
                &record.agent,
                &record.invocation_id,
                &record.effect_id,
                &record.transition_id,
                &record.event_id,
                &record.idempotency_key,
                serde_json::to_string(&record.message)?,
                &record.status,
                &record.created_at,
                &record.updated_at,
            ],
        )?;
        Ok(())
    }

    fn insert_agent_completion_in(
        connection: &rusqlite::Connection,
        record: &AgentCompletionRecord,
    ) -> Result<bool, StorageError> {
        let rows = connection.execute(
            r#"
            INSERT INTO agent_completions (
              workflow_id, completion_id, invocation_id, agent, status, summary,
              exit_code, event_id, payload_json, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(workflow_id, invocation_id) DO NOTHING
            "#,
            params![
                &record.workflow_id,
                &record.completion_id,
                &record.invocation_id,
                &record.agent,
                &record.status,
                &record.summary,
                record.exit_code.map(i64::from),
                &record.event_id,
                serde_json::to_string(&record.payload)?,
                &record.created_at,
            ],
        )?;
        Ok(rows > 0)
    }

    fn select_agent_invocation_by_id_in(
        connection: &rusqlite::Connection,
        invocation_id: &str,
    ) -> Result<Option<AgentInvocationRecord>, StorageError> {
        connection
            .query_row(
                r#"
                SELECT
                  workflow_id, invocation_id, agent, effect_id, transition_id,
                  event_id, idempotency_key, input_json, requested_profile,
                  resolved_profile, profile_enforcement, status, claimed_by,
                  claim_expires_at, provider, provider_run_id, run_dir,
                  stdout_path, stderr_path, exit_code, error, created_at, updated_at
                FROM agent_invocations
                WHERE invocation_id = ?1
                "#,
                params![invocation_id],
                agent_invocation_record_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn agent_invocation_record_from_row(
        row: &rusqlite::Row<'_>,
    ) -> Result<AgentInvocationRecord, rusqlite::Error> {
        let input_json: String = row.get(7)?;
        let status: String = row.get(11)?;
        let exit_code: Option<i64> = row.get(19)?;
        Ok(AgentInvocationRecord {
            workflow_id: row.get(0)?,
            invocation_id: row.get(1)?,
            agent: row.get(2)?,
            effect_id: row.get(3)?,
            transition_id: row.get(4)?,
            event_id: row.get(5)?,
            idempotency_key: row.get(6)?,
            input: serde_json::from_str(&input_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    input_json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
            requested_profile: row.get(8)?,
            resolved_profile: row.get(9)?,
            profile_enforcement: row.get(10)?,
            status: agent_invocation_status_from_name(&status),
            claimed_by: row.get(12)?,
            claim_expires_at: row.get(13)?,
            provider: row.get(14)?,
            provider_run_id: row.get(15)?,
            run_dir: row.get(16)?,
            stdout_path: row.get(17)?,
            stderr_path: row.get(18)?,
            exit_code: exit_code.map(|value| value as i32),
            error: row.get(20)?,
            created_at: row.get(21)?,
            updated_at: row.get(22)?,
        })
    }

    fn save_state_in(
        connection: &rusqlite::Connection,
        state: &WorkflowState,
    ) -> Result<(), StorageError> {
        connection.execute(
            r#"
            INSERT INTO workflow_state (
              workflow_id,
              workflow_name,
              current_state,
              context_json
            )
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(workflow_id) DO UPDATE SET
              workflow_name = excluded.workflow_name,
              current_state = excluded.current_state,
              context_json = excluded.context_json
            "#,
            params![
                state.workflow_name,
                state.workflow_name,
                state.current_state,
                serde_json::to_string(&state.context)?,
            ],
        )?;
        Ok(())
    }

    fn update_event_status_in(
        connection: &rusqlite::Connection,
        event: &WorkflowEvent,
    ) -> Result<(), StorageError> {
        let rows_updated = connection.execute(
            r#"
            UPDATE workflow_events
            SET status = ?1, event_json = ?2
            WHERE workflow_id = ?3 AND event_id = ?4
            "#,
            params![
                event_status_name(event.status),
                serde_json::to_string(event)?,
                event.workflow_id,
                event.event_id,
            ],
        )?;
        if rows_updated == 0 {
            return Err(StorageError::EventNotFound {
                workflow_id: event.workflow_id.clone(),
                event_id: event.event_id.clone(),
            });
        }
        Ok(())
    }

    fn append_log_records_in(
        connection: &rusqlite::Connection,
        workflow_id: &str,
        records: &[WorkflowLogRecord],
    ) -> Result<(), StorageError> {
        for record in records {
            connection.execute(
                r#"
                INSERT INTO workflow_log (workflow_id, record_json)
                VALUES (?1, ?2)
                "#,
                params![workflow_id, serde_json::to_string(record)?],
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("storage error: {0}")]
    Storage(#[from] storage::StorageError),
    #[error("interpreter error: {0}")]
    Interpreter(#[from] InterpreterError),
    #[error("raised event effect is malformed")]
    MalformedRaisedEvent,
}

pub struct WorkflowRuntime {
    store: storage::WorkflowStore,
    interpreter: Interpreter,
    dispatcher: Rc<RefCell<Box<dyn effects::EffectDispatcher>>>,
}

impl WorkflowRuntime {
    pub fn new(ir: WorkflowIr, store: storage::WorkflowStore) -> Result<Self, RuntimeError> {
        Self::with_dispatcher(ir, store, Box::new(effects::NoopEffectDispatcher))
    }

    pub fn with_dispatcher(
        ir: WorkflowIr,
        store: storage::WorkflowStore,
        dispatcher: Box<dyn effects::EffectDispatcher>,
    ) -> Result<Self, RuntimeError> {
        Self::with_dispatcher_and_coerce_executor(
            ir,
            store,
            dispatcher,
            Box::new(coerce::NoopCoerceExecutor),
        )
    }

    pub fn with_dispatcher_and_coerce_executor(
        ir: WorkflowIr,
        store: storage::WorkflowStore,
        dispatcher: Box<dyn effects::EffectDispatcher>,
        coerce_executor: Box<dyn coerce::CoerceExecutor>,
    ) -> Result<Self, RuntimeError> {
        let workflow_id = ir.workflow.name.clone();
        store.recover_processing_events(&workflow_id)?;
        let coerce_executor = Box::new(coerce::DurableCoerceExecutor::new(
            store.clone(),
            coerce_executor,
            workflow_id.clone(),
        ));
        let dispatcher = Rc::new(RefCell::new(dispatcher));
        let interpreter = if let Some(state) = store.load_state(&workflow_id)? {
            Interpreter::from_state(ir, state).with_coerce_executor(coerce_executor)
        } else {
            Interpreter::new(ir).with_coerce_executor(coerce_executor)
        }
        .with_value_dispatcher(dispatcher.clone());
        store.save_state(&interpreter.state)?;
        Ok(Self {
            store,
            interpreter,
            dispatcher,
        })
    }

    pub fn with_fake_outputs(
        mut self,
        coerce_outputs: BTreeMap<String, serde_json::Value>,
        call_outputs: BTreeMap<String, serde_json::Value>,
    ) -> Self {
        let workflow_id = self.interpreter.state.workflow_name.clone();
        self.interpreter = self
            .interpreter
            .with_fake_call_outputs(call_outputs)
            .with_coerce_executor(Box::new(coerce::DurableCoerceExecutor::new(
                self.store.clone(),
                Box::new(coerce::FakeCoerceExecutor::new(coerce_outputs)),
                workflow_id,
            )));
        self
    }

    pub fn with_fake_call_outputs(
        mut self,
        call_outputs: BTreeMap<String, serde_json::Value>,
    ) -> Self {
        self.interpreter = self.interpreter.with_fake_call_outputs(call_outputs);
        self
    }

    pub fn enqueue_event(&self, event: &queue::WorkflowEvent) -> Result<(), RuntimeError> {
        self.store.enqueue_event(event)?;
        Ok(())
    }

    pub fn process_next_event(&mut self) -> Result<Option<EventProcessingOutcome>, RuntimeError> {
        let workflow_id = self.interpreter.state.workflow_name.clone();
        let Some(event) = self.store.dequeue_next_event(&workflow_id)? else {
            return Ok(None);
        };

        match self.interpreter.process_event(&event) {
            Ok(outcome) => {
                let event_status = match outcome.status {
                    EventProcessingStatus::Processed => queue::EventStatus::Processed,
                    EventProcessingStatus::Ignored => queue::EventStatus::Ignored,
                };

                match outcome.status {
                    EventProcessingStatus::Processed => {
                        let mut commit_records = vec![log::WorkflowLogRecord::Transition {
                            transition_id: outcome.transition_id.clone(),
                            workflow_id: workflow_id.clone(),
                            from_state: outcome.from_state.clone(),
                            to_state: outcome.to_state.clone(),
                            event_type: Some(event.event_type.clone()),
                            event_id: Some(event.event_id.clone()),
                        }];
                        commit_records.extend(outcome.effects.iter().map(|effect| {
                            log::WorkflowLogRecord::Effect {
                                effect_id: effect.effect_id.clone(),
                                transition_id: outcome.transition_id.clone(),
                                workflow_id: workflow_id.clone(),
                                idempotency_key: Some(effect.idempotency_key.clone()),
                                effect: effect.effect.clone(),
                                category: effect.category,
                                target: effect.target.clone(),
                                args: effect.args.clone(),
                                required_capabilities: effect.required_capabilities.clone(),
                                status: log::EffectStatus::Intended,
                                outcome: None,
                            }
                        }));
                        let mut start_capacity_errors = BTreeMap::new();
                        for effect in &outcome.effects {
                            if let Some(error) = self.start_capacity_error(&workflow_id, effect)? {
                                start_capacity_errors.insert(effect.effect_id.clone(), error);
                            }
                        }
                        let native_agent_invocations = outcome
                            .effects
                            .iter()
                            .filter(|effect| !start_capacity_errors.contains_key(&effect.effect_id))
                            .filter_map(|effect| {
                                native_agent_invocation_from_effect(
                                    self.interpreter.workflow(),
                                    effect,
                                    &event,
                                )
                            })
                            .collect::<Vec<_>>();
                        let native_agent_messages = outcome
                            .effects
                            .iter()
                            .filter_map(|effect| {
                                native_agent_message_from_effect(
                                    self.interpreter.workflow(),
                                    effect,
                                    &event,
                                )
                            })
                            .collect::<Vec<_>>();
                        self.store.commit_event_processing(
                            &event,
                            event_status,
                            &self.interpreter.state,
                            &commit_records,
                            &native_agent_invocations,
                            &native_agent_messages,
                        )?;

                        for effect in &outcome.effects {
                            let mut dispatch_result =
                                if is_native_agent_effect(self.interpreter.workflow(), effect) {
                                    if let Some(error) =
                                        start_capacity_errors.get(&effect.effect_id)
                                    {
                                        Err(effects::EffectError::RuntimeRejected(error.clone()))
                                    } else {
                                        Ok(native_agent_effect_outcome(effect))
                                    }
                                } else {
                                    match start_capacity_errors.get(&effect.effect_id) {
                                        Some(error) => Err(effects::EffectError::RuntimeRejected(
                                            error.clone(),
                                        )),
                                        None => {
                                            self.dispatcher.borrow_mut().dispatch(effect.clone())
                                        }
                                    }
                                };
                            if dispatch_result.is_ok() && effect.effect == "raise" {
                                match raised_event_from_effect(effect, &event) {
                                    Ok(raised_event) => {
                                        if let Err(error) = self.store.enqueue_event(&raised_event)
                                        {
                                            dispatch_result =
                                                Err(effects::EffectError::Unsupported(format!(
                                                    "failed to enqueue raised event: {error}"
                                                )));
                                        }
                                    }
                                    Err(error) => {
                                        dispatch_result = Err(effects::EffectError::Unsupported(
                                            error.to_string(),
                                        ));
                                    }
                                }
                            }
                            let (status, effect_outcome) = match dispatch_result {
                                Ok(outcome) => (effect_log_status(outcome.status), Some(outcome)),
                                Err(error) => {
                                    let required_capabilities =
                                        if error.required_capabilities().is_empty() {
                                            effect.required_capabilities.clone()
                                        } else {
                                            error.required_capabilities().to_vec()
                                        };
                                    let failed_outcome = effects::EffectOutcome {
                                        effect_id: effect.effect_id.clone(),
                                        status: effects::EffectOutcomeStatus::Failed,
                                        accepted: false,
                                        invocation_id: None,
                                        required_capabilities,
                                        output: None,
                                        error: Some(error.to_string()),
                                        completed_at: None,
                                    };
                                    (log::EffectStatus::Failed, Some(failed_outcome))
                                }
                            };
                            self.store.append_log(
                                &workflow_id,
                                &log::WorkflowLogRecord::Effect {
                                    effect_id: effect.effect_id.clone(),
                                    workflow_id: workflow_id.clone(),
                                    transition_id: outcome.transition_id.clone(),
                                    idempotency_key: Some(effect.idempotency_key.clone()),
                                    effect: effect.effect.clone(),
                                    category: effect.category,
                                    target: effect.target.clone(),
                                    args: effect.args.clone(),
                                    required_capabilities: effect.required_capabilities.clone(),
                                    status,
                                    outcome: effect_outcome,
                                },
                            )?;
                        }
                    }
                    EventProcessingStatus::Ignored => {
                        let reason = outcome
                            .reason
                            .clone()
                            .unwrap_or_else(|| "event ignored".to_string());
                        let records = vec![log::WorkflowLogRecord::Diagnostic {
                            message: reason.clone(),
                        }];
                        let mut ignored_event = event.clone();
                        ignored_event.last_error = Some(reason);
                        self.store.commit_event_processing(
                            &ignored_event,
                            event_status,
                            &self.interpreter.state,
                            &records,
                            &[],
                            &[],
                        )?;
                    }
                }

                Ok(Some(outcome))
            }
            Err(error) => {
                let mut failed_event = event;
                failed_event.last_error = Some(error.to_string());
                self.store.commit_event_failure(
                    &failed_event,
                    &[log::WorkflowLogRecord::Diagnostic {
                        message: error.to_string(),
                    }],
                )?;
                Err(error.into())
            }
        }
    }

    pub fn status(&self, pending_events: usize) -> status::WorkflowStatus {
        self.interpreter.status(pending_events)
    }

    pub fn projected_status(&self) -> Result<status::WorkflowStatus, RuntimeError> {
        let workflow_id = self.interpreter.state.workflow_name.clone();
        Ok(project_status(
            self.interpreter.workflow(),
            &self.store,
            &workflow_id,
        )?)
    }

    fn start_capacity_error(
        &self,
        workflow_id: &str,
        effect: &EffectRequest,
    ) -> Result<Option<String>, RuntimeError> {
        if effect.effect != "start" {
            return Ok(None);
        }

        let Some(agent) = effect.target.as_deref() else {
            return Ok(None);
        };
        let Some(max_active) = self
            .interpreter
            .workflow()
            .agents
            .get(agent)
            .and_then(|agent| agent.max_active)
        else {
            return Ok(None);
        };

        let records = self.store.log_records(workflow_id)?;
        let events = self.store.events(workflow_id, 10_000)?;
        let native_active = self.store.active_agent_invocation_counts(workflow_id)?;
        let active = if native_active.is_empty() {
            active_invocation_counts(&records, &events)
                .get(agent)
                .copied()
                .unwrap_or_default()
        } else {
            native_active.get(agent).copied().unwrap_or_default()
        };

        Ok((active >= max_active).then(|| {
            format!("agent `{agent}` is at maxActive {max_active}; active invocations: {active}")
        }))
    }

    pub fn store(&self) -> &storage::WorkflowStore {
        &self.store
    }

    pub fn interpreter(&self) -> &Interpreter {
        &self.interpreter
    }
}

fn is_native_agent_effect(ir: &WorkflowIr, effect: &EffectRequest) -> bool {
    matches!(effect.effect.as_str(), "start" | "send")
        && effect
            .target
            .as_deref()
            .and_then(|target| ir.agents.get(target))
            .is_some_and(|agent| !matches!(agent.target, AgentTarget::Adapter { .. }))
}

fn native_agent_effect_outcome(effect: &EffectRequest) -> effects::EffectOutcome {
    effects::EffectOutcome {
        effect_id: effect.effect_id.clone(),
        status: effects::EffectOutcomeStatus::Accepted,
        accepted: true,
        invocation_id: (effect.effect == "start").then(|| effect.idempotency_key.clone()),
        required_capabilities: effect.required_capabilities.clone(),
        output: None,
        error: None,
        completed_at: Some(current_unix_millis().to_string()),
    }
}

fn native_agent_invocation_from_effect(
    ir: &WorkflowIr,
    effect: &EffectRequest,
    event: &queue::WorkflowEvent,
) -> Option<storage::AgentInvocationRecord> {
    if effect.effect != "start" || !is_native_agent_effect(ir, effect) {
        return None;
    }
    let agent = effect.target.clone()?;
    let requested_profile = ir
        .agents
        .get(&agent)
        .and_then(|agent| agent.profile.clone());
    let input = effect
        .args
        .get("input")
        .cloned()
        .unwrap_or_else(|| effect.args.clone());
    let now = current_unix_millis().to_string();
    Some(storage::AgentInvocationRecord {
        workflow_id: effect.workflow_id.clone(),
        invocation_id: effect.idempotency_key.clone(),
        agent,
        effect_id: effect.effect_id.clone(),
        transition_id: effect.transition_id.clone(),
        event_id: Some(event.event_id.clone()),
        idempotency_key: effect.idempotency_key.clone(),
        input,
        requested_profile,
        resolved_profile: None,
        profile_enforcement: None,
        status: storage::AgentInvocationStatus::Queued,
        claimed_by: None,
        claim_expires_at: None,
        provider: None,
        provider_run_id: None,
        run_dir: None,
        stdout_path: None,
        stderr_path: None,
        exit_code: None,
        error: None,
        created_at: now.clone(),
        updated_at: now,
    })
}

fn native_agent_message_from_effect(
    ir: &WorkflowIr,
    effect: &EffectRequest,
    event: &queue::WorkflowEvent,
) -> Option<storage::AgentMessageRecord> {
    if effect.effect != "send" || !is_native_agent_effect(ir, effect) {
        return None;
    }
    let agent = effect.target.clone()?;
    let message = effect
        .args
        .get("message")
        .cloned()
        .unwrap_or_else(|| effect.args.clone());
    let now = current_unix_millis().to_string();
    Some(storage::AgentMessageRecord {
        workflow_id: effect.workflow_id.clone(),
        message_id: effect.idempotency_key.clone(),
        agent,
        invocation_id: None,
        effect_id: effect.effect_id.clone(),
        transition_id: effect.transition_id.clone(),
        event_id: Some(event.event_id.clone()),
        idempotency_key: effect.idempotency_key.clone(),
        message,
        status: "queued".to_string(),
        created_at: now.clone(),
        updated_at: now,
    })
}

pub fn project_status(
    ir: &WorkflowIr,
    store: &storage::WorkflowStore,
    workflow_id: &str,
) -> Result<status::WorkflowStatus, storage::StorageError> {
    let pending_events = store.pending_event_count(workflow_id)?;
    let queued_events = store.queued_event_summaries(workflow_id, 20)?;
    let events = store.events(workflow_id, 10_000)?;
    let state = store.load_state(workflow_id)?;
    let current_state = state
        .as_ref()
        .map(|state| state.current_state.clone())
        .unwrap_or_else(|| initial_state_name(ir));
    let data = state
        .as_ref()
        .map(|state| state.context.clone())
        .unwrap_or_else(|| initial_context_from_ir(ir));
    let records = store.log_records(workflow_id)?;
    let recent_transition = records.iter().rev().find_map(|record| match record {
        log::WorkflowLogRecord::Transition {
            from_state,
            to_state,
            event_type,
            event_id,
            ..
        } => {
            let label = event_type
                .clone()
                .or_else(|| event_id.clone())
                .unwrap_or_else(|| "<internal>".to_string());
            Some(format!("{} --{}--> {}", from_state, label, to_state))
        }
        _ => None,
    });
    let mut seen_effects = BTreeSet::new();
    let recent_effects = records
        .iter()
        .rev()
        .filter_map(|record| match record {
            log::WorkflowLogRecord::Effect {
                effect_id,
                idempotency_key,
                effect,
                status,
                target,
                args,
                required_capabilities,
                outcome,
                ..
            } => {
                if seen_effects.insert(effect_id.clone()) {
                    Some(status::EffectSummary {
                        effect_id: effect_id.clone(),
                        idempotency_key: idempotency_key.clone(),
                        effect: effect.clone(),
                        status: *status,
                        target: target.clone(),
                        args: args.clone(),
                        required_capabilities: outcome
                            .as_ref()
                            .map(|outcome| outcome.required_capabilities.clone())
                            .filter(|capabilities| !capabilities.is_empty())
                            .unwrap_or_else(|| required_capabilities.clone()),
                        error: outcome.as_ref().and_then(|outcome| outcome.error.clone()),
                    })
                } else {
                    None
                }
            }
            _ => None,
        })
        .take(10)
        .collect::<Vec<_>>();
    let recent_failures = records
        .iter()
        .rev()
        .filter_map(|record| match record {
            log::WorkflowLogRecord::Effect {
                effect,
                status: log::EffectStatus::Failed,
                outcome,
                ..
            } => Some(format!(
                "{} failed{}",
                effect,
                outcome
                    .as_ref()
                    .and_then(|outcome| outcome.error.as_deref())
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            )),
            log::WorkflowLogRecord::Diagnostic { message } => Some(message.clone()),
            _ => None,
        })
        .take(10)
        .collect::<Vec<_>>();
    let current_effect_failures = recent_effects
        .iter()
        .filter(|effect| effect.status == log::EffectStatus::Failed)
        .map(|effect| {
            effect
                .error
                .as_deref()
                .map(|error| format!("{} failed: {error}", effect.effect))
                .unwrap_or_else(|| format!("{} failed", effect.effect))
        })
        .collect::<Vec<_>>();
    let policy_blockers = records
        .iter()
        .rev()
        .filter_map(|record| match record {
            log::WorkflowLogRecord::Effect {
                effect,
                status: log::EffectStatus::Failed,
                outcome,
                ..
            } => {
                let error = outcome
                    .as_ref()
                    .and_then(|outcome| outcome.error.as_deref())?;
                let required_capabilities = outcome
                    .as_ref()
                    .map(|outcome| outcome.required_capabilities.as_slice())
                    .unwrap_or(&[]);
                if required_capabilities.is_empty()
                    || !(error.contains("capability") || error.contains("policy"))
                {
                    return None;
                }
                Some(format!(
                    "{} requires {}: {}",
                    effect,
                    required_capabilities.join(","),
                    error
                ))
            }
            _ => None,
        })
        .take(10)
        .collect::<Vec<_>>();
    let native_active_invocations = store.active_agent_invocation_counts(workflow_id)?;
    let active_invocations =
        project_active_invocations(ir, &records, &events, native_active_invocations);
    let latest_coerce_records = store.latest_coerce_calls(workflow_id, 10)?;
    let latest_coerce_calls = latest_coerce_records
        .iter()
        .cloned()
        .map(coerce_call_summary)
        .collect::<Vec<_>>();
    let baml_runtime = baml_runtime_summary(&latest_coerce_records);
    let latest_coerce_failures = store
        .latest_coerce_failures(workflow_id, 10)?
        .into_iter()
        .map(coerce_call_summary)
        .collect::<Vec<_>>();
    let unresolved_event_ids = events
        .iter()
        .filter(|event| {
            matches!(
                event.status,
                queue::EventStatus::Failed | queue::EventStatus::DeadLettered
            )
        })
        .map(|event| event.event_id.as_str())
        .collect::<BTreeSet<_>>();
    let current_coerce_failure = latest_coerce_records
        .first()
        .filter(|call| call.status == coerce::CoerceStatus::Failed)
        .filter(|call| {
            call.event_id
                .as_deref()
                .is_none_or(|event_id| unresolved_event_ids.contains(event_id))
        })
        .cloned()
        .map(coerce_call_summary);
    let mut current_blockers = events
        .iter()
        .filter(|event| {
            matches!(
                event.status,
                queue::EventStatus::Failed | queue::EventStatus::DeadLettered
            )
        })
        .filter_map(|event| {
            event
                .last_error
                .as_deref()
                .map(|error| format!("{} {}: {error}", event.event_id, event.event_type))
        })
        .rev()
        .take(10)
        .collect::<Vec<_>>();
    current_blockers.extend(policy_blockers.iter().cloned());

    Ok(status::WorkflowStatus {
        workflow_id: workflow_id.to_string(),
        workflow_name: workflow_id.to_string(),
        current_state,
        blocked_reason: None,
        data_summary: summarize_status_data(&data),
        data,
        pending_events,
        queued_events,
        active_invocations,
        recent_transition,
        recent_effects,
        latest_coerce_calls,
        latest_coerce_failures,
        current_coerce_failure,
        baml_runtime,
        current_effect_failures,
        policy_blockers,
        current_blockers,
        recent_failures,
    })
}

pub fn summarize_status_data(
    data: &BTreeMap<String, serde_json::Value>,
) -> BTreeMap<String, serde_json::Value> {
    data.iter()
        .map(|(key, value)| (key.clone(), summarize_status_value(value)))
        .collect()
}

fn summarize_status_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => serde_json::json!(values.len()),
        serde_json::Value::Object(object) => serde_json::json!({
            "fields": object.len()
        }),
        value => value.clone(),
    }
}

fn coerce_call_summary(record: coerce::CoerceCallRecord) -> status::CoerceCallSummary {
    status::CoerceCallSummary {
        coerce_call_id: record.coerce_call_id,
        function_name: record.function_name,
        status: record.status,
        backend: record.backend,
        http_status: record.http_status,
        parsed_output: record.parsed_output,
        error: record.error,
        duration_ms: record.duration_ms,
        created_at: record.created_at,
    }
}

fn baml_runtime_summary(
    records: &[coerce::CoerceCallRecord],
) -> Option<status::BamlRuntimeSummary> {
    records.iter().find_map(|record| {
        let (mode, url, baml_src_hash) = match &record.backend {
            coerce::CoerceBackend::BamlHttp {
                url,
                baml_src_hash,
                runtime_mode,
            } => (
                runtime_mode
                    .clone()
                    .unwrap_or_else(|| "external_http".to_string()),
                url.clone(),
                baml_src_hash.clone(),
            ),
            coerce::CoerceBackend::BamlGeneratedStdio {
                baml_src_hash,
                runtime_mode,
                ..
            } => (
                runtime_mode
                    .clone()
                    .unwrap_or_else(|| "generated_stdio".to_string()),
                "stdio".to_string(),
                baml_src_hash.clone(),
            ),
            coerce::CoerceBackend::BamlBrokered {
                request_id,
                baml_src_hash,
            } => (
                "brokered".to_string(),
                format!("brokered:{request_id}"),
                baml_src_hash.clone(),
            ),
            _ => return None,
        };
        let status = match record.status {
            coerce::CoerceStatus::Succeeded => "observed",
            coerce::CoerceStatus::Failed => "failed",
        }
        .to_string();
        Some(status::BamlRuntimeSummary {
            mode,
            status,
            url,
            baml_src_hash,
            last_error: record.error.clone(),
            last_call_at: record.created_at.clone(),
        })
    })
}

fn project_active_invocations(
    ir: &WorkflowIr,
    records: &[log::WorkflowLogRecord],
    events: &[queue::WorkflowEvent],
    native_active_by_agent: BTreeMap<String, u32>,
) -> Vec<status::ActiveInvocation> {
    let active_by_agent = if native_active_by_agent.is_empty() {
        active_invocation_counts(records, events)
    } else {
        subtract_finished_events(native_active_by_agent, events)
    };

    active_by_agent
        .into_iter()
        .filter_map(|(agent, active)| {
            let max = ir.agents.get(&agent).and_then(|agent| agent.max_active);
            (active > 0).then_some(status::ActiveInvocation {
                agent,
                count: active,
                max,
            })
        })
        .collect()
}

fn subtract_finished_events(
    started_by_agent: BTreeMap<String, u32>,
    events: &[queue::WorkflowEvent],
) -> BTreeMap<String, u32> {
    let mut completed_by_agent: BTreeMap<String, u32> = BTreeMap::new();
    for event in events {
        if event.event_type != "finished" || event.status != queue::EventStatus::Processed {
            continue;
        }
        let Some(name) = event.payload.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        if let Some(agent) = matching_finished_agent(name, started_by_agent.keys()) {
            *completed_by_agent.entry(agent.to_string()).or_default() += 1;
        }
    }

    started_by_agent
        .into_iter()
        .map(|(agent, started)| {
            let completed = completed_by_agent.get(&agent).copied().unwrap_or_default();
            (agent, started.saturating_sub(completed))
        })
        .collect()
}

fn active_invocation_counts(
    records: &[log::WorkflowLogRecord],
    events: &[queue::WorkflowEvent],
) -> BTreeMap<String, u32> {
    let mut started_by_agent: BTreeMap<String, u32> = BTreeMap::new();
    let mut completed_by_agent: BTreeMap<String, u32> = BTreeMap::new();
    let mut latest_start_outcome_by_effect: BTreeMap<
        String,
        (String, log::EffectStatus, Option<effects::EffectOutcome>),
    > = BTreeMap::new();

    for record in records {
        let log::WorkflowLogRecord::Effect {
            effect_id,
            effect,
            status,
            target: Some(agent),
            outcome,
            ..
        } = record
        else {
            continue;
        };
        if effect != "start" {
            continue;
        }
        latest_start_outcome_by_effect
            .insert(effect_id.clone(), (agent.clone(), *status, outcome.clone()));
    }

    for (_effect_id, (agent, status, outcome)) in latest_start_outcome_by_effect {
        if matches!(
            status,
            log::EffectStatus::Dispatched | log::EffectStatus::Succeeded
        ) && outcome.as_ref().is_some_and(|outcome| outcome.accepted)
        {
            *started_by_agent.entry(agent).or_default() += 1;
        }
    }

    for event in events {
        if event.event_type != "finished" || event.status != queue::EventStatus::Processed {
            continue;
        }
        let Some(name) = event.payload.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        if let Some(agent) = matching_finished_agent(name, started_by_agent.keys()) {
            *completed_by_agent.entry(agent.to_string()).or_default() += 1;
        }
    }

    started_by_agent
        .into_iter()
        .map(|(agent, started)| {
            let completed = completed_by_agent.get(&agent).copied().unwrap_or_default();
            (agent, started.saturating_sub(completed))
        })
        .collect()
}

fn effect_log_status(status: effects::EffectOutcomeStatus) -> log::EffectStatus {
    match status {
        effects::EffectOutcomeStatus::Accepted => log::EffectStatus::Dispatched,
        effects::EffectOutcomeStatus::Succeeded => log::EffectStatus::Succeeded,
        effects::EffectOutcomeStatus::Rejected | effects::EffectOutcomeStatus::Failed => {
            log::EffectStatus::Failed
        }
    }
}

fn matching_finished_agent<'a>(
    finished_name: &str,
    agents: impl Iterator<Item = &'a String>,
) -> Option<&'a str> {
    agents
        .filter_map(|agent| {
            (finished_name == agent || finished_name.starts_with(&format!("{agent}-")))
                .then_some(agent.as_str())
        })
        .max_by_key(|agent| agent.len())
}

fn raised_event_from_effect(
    effect: &EffectRequest,
    source_event: &queue::WorkflowEvent,
) -> Result<queue::WorkflowEvent, RuntimeError> {
    let args = effect
        .args
        .as_object()
        .ok_or(RuntimeError::MalformedRaisedEvent)?;
    let event_type = args
        .get("event")
        .and_then(|value| value.as_str())
        .ok_or(RuntimeError::MalformedRaisedEvent)?;
    let payload = args
        .get("payload")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    Ok(queue::WorkflowEvent {
        event_id: format!("evt_raise_{}", ulid::Ulid::new()),
        workflow_id: effect.workflow_id.clone(),
        event_type: event_type.to_string(),
        payload,
        source: Some(queue::EventSource {
            kind: "runtime".to_string(),
            name: Some("raise".to_string()),
        }),
        occurred_at: None,
        enqueued_at: None,
        correlation_id: source_event
            .correlation_id
            .clone()
            .or_else(|| Some(source_event.event_id.clone())),
        causation_id: Some(source_event.event_id.clone()),
        dedupe_key: Some(effect.idempotency_key.clone()),
        status: queue::EventStatus::Queued,
        attempt_count: 0,
        last_error: None,
    })
}

pub fn initial_state_name(ir: &WorkflowIr) -> String {
    initial_leaf_state(ir).unwrap_or_else(|| ir.statechart.initial.clone())
}

pub struct Interpreter {
    ir: WorkflowIr,
    state: state::WorkflowState,
    last_transition: Option<String>,
    recent_failures: Vec<String>,
    coerce_executor: Box<dyn coerce::CoerceExecutor>,
    value_dispatcher: Option<Rc<RefCell<Box<dyn effects::EffectDispatcher>>>>,
    fake_call_outputs: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventProcessingStatus {
    Processed,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventProcessingOutcome {
    pub status: EventProcessingStatus,
    pub transition_id: String,
    pub from_state: String,
    pub to_state: String,
    #[serde(default)]
    pub effects: Vec<EffectRequest>,
    pub reason: Option<String>,
}

#[derive(Debug, Default)]
struct StepExecutionOutcome {
    effects: Vec<EffectRequest>,
    transition: Option<String>,
}

#[derive(Debug, Error)]
pub enum InterpreterError {
    #[error("current state `{0}` is not declared")]
    UnknownCurrentState(String),
    #[error("event `{event_type}` payload does not match declared schema: {reason}")]
    InvalidEventPayload { event_type: String, reason: String },
    #[error("assign target `{0}` is not a workflow data path")]
    InvalidAssignTarget(String),
    #[error("assign target `{0}` is not declared in workflow data")]
    UndeclaredDataField(String),
    #[error("unsupported expression in runtime slice")]
    UnsupportedExpression,
    #[error("coerce `{function_name}` output does not match declared schema: {reason}")]
    InvalidCoerceOutput {
        function_name: String,
        reason: String,
    },
    #[error("coerce `{function_name}` parameter `{param_name}` does not match declared schema: {reason}")]
    InvalidCoerceInput {
        function_name: String,
        param_name: String,
        reason: String,
    },
    #[error("coerce `{function_name}` expected {expected} arguments but received {actual}")]
    InvalidCoerceArity {
        function_name: String,
        expected: usize,
        actual: usize,
    },
    #[error("coerce `{function_name}` failed: {message}")]
    CoerceExecutionFailed {
        function_name: String,
        category: coerce::CoerceErrorCategory,
        message: String,
    },
    #[error("capability value call `{name}` failed: {message}")]
    CapabilityValueCallFailed { name: String, message: String },
    #[error("guard expression did not evaluate to a boolean")]
    GuardNotBoolean,
    #[error("invariant `{name}` expression did not evaluate to a boolean")]
    InvariantNotBoolean { name: String },
    #[error("invariant `{name}` was violated")]
    InvariantViolated { name: String },
    #[error("event path `{0}` is not present in payload")]
    MissingEventPath(String),
    #[error("data path `{0}` is not present in workflow data")]
    MissingDataPath(String),
    #[error("entry transition limit exceeded")]
    EntryTransitionLimitExceeded,
    #[error("always transition limit exceeded")]
    AlwaysTransitionLimitExceeded,
}

impl Interpreter {
    pub fn new(ir: WorkflowIr) -> Self {
        let current_state = initial_state_name(&ir);
        let state = state::WorkflowState {
            workflow_name: ir.workflow.name.clone(),
            current_state,
            context: initial_context_from_ir(&ir),
        };

        Self {
            ir,
            state,
            last_transition: None,
            recent_failures: Vec::new(),
            coerce_executor: Box::new(coerce::NoopCoerceExecutor),
            value_dispatcher: None,
            fake_call_outputs: BTreeMap::new(),
        }
    }

    pub fn from_state(ir: WorkflowIr, state: state::WorkflowState) -> Self {
        Self {
            ir,
            state,
            last_transition: None,
            recent_failures: Vec::new(),
            coerce_executor: Box::new(coerce::NoopCoerceExecutor),
            value_dispatcher: None,
            fake_call_outputs: BTreeMap::new(),
        }
    }

    pub fn with_coerce_executor(mut self, executor: Box<dyn coerce::CoerceExecutor>) -> Self {
        self.coerce_executor = executor;
        self
    }

    pub fn with_value_dispatcher(
        mut self,
        dispatcher: Rc<RefCell<Box<dyn effects::EffectDispatcher>>>,
    ) -> Self {
        self.value_dispatcher = Some(dispatcher);
        self
    }

    pub fn with_fake_coerce_outputs(
        mut self,
        outputs: BTreeMap<String, serde_json::Value>,
    ) -> Self {
        self.coerce_executor = Box::new(coerce::FakeCoerceExecutor::new(outputs));
        self
    }

    pub fn with_fake_call_outputs(mut self, outputs: BTreeMap<String, serde_json::Value>) -> Self {
        self.fake_call_outputs = outputs;
        self
    }

    pub fn status(&self, pending_events: usize) -> status::WorkflowStatus {
        status::WorkflowStatus {
            workflow_id: self.state.workflow_name.clone(),
            workflow_name: self.state.workflow_name.clone(),
            current_state: self.state.current_state.clone(),
            blocked_reason: None,
            data_summary: summarize_status_data(&self.state.context),
            data: self.state.context.clone(),
            pending_events,
            queued_events: Vec::new(),
            active_invocations: Vec::new(),
            recent_transition: self.last_transition.clone(),
            recent_effects: Vec::new(),
            latest_coerce_calls: Vec::new(),
            latest_coerce_failures: Vec::new(),
            current_coerce_failure: None,
            baml_runtime: None,
            current_effect_failures: Vec::new(),
            policy_blockers: Vec::new(),
            current_blockers: Vec::new(),
            recent_failures: self.recent_failures.clone(),
        }
    }

    pub fn workflow(&self) -> &WorkflowIr {
        &self.ir
    }

    pub fn context(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.state.context
    }

    pub fn process_event(
        &mut self,
        event: &queue::WorkflowEvent,
    ) -> Result<EventProcessingOutcome, InterpreterError> {
        let state = self.state.clone();
        let last_transition = self.last_transition.clone();
        let recent_failures = self.recent_failures.clone();
        match self.process_event_inner(event) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.state = state;
                self.last_transition = last_transition;
                self.recent_failures = recent_failures;
                Err(error)
            }
        }
    }

    fn process_event_inner(
        &mut self,
        event: &queue::WorkflowEvent,
    ) -> Result<EventProcessingOutcome, InterpreterError> {
        if let Some(schema) = self
            .ir
            .events
            .get(&event.event_type)
            .map(|event| &event.payload)
        {
            if let Some(reason) = schema.explain_json_mismatch(&event.payload, &self.ir.types) {
                return Err(InterpreterError::InvalidEventPayload {
                    event_type: event.event_type.clone(),
                    reason,
                });
            }
        }

        let from_state = self.state.current_state.clone();
        let transition_id = ulid::Ulid::new().to_string();
        let state_chain = state_chain(&self.ir, &from_state)
            .ok_or_else(|| InterpreterError::UnknownCurrentState(from_state.clone()))?;
        let guard_locals = BTreeMap::new();

        let mut handler = None;
        for state in state_chain.iter().rev() {
            for candidate in &state.on {
                if candidate.event != event.event_type {
                    continue;
                }

                let guard_matches = candidate
                    .guard
                    .as_ref()
                    .map(|guard| {
                        self.eval_guard(
                            guard,
                            event,
                            candidate.binding.as_deref(),
                            &guard_locals,
                            Some("handler.guard"),
                        )
                    })
                    .transpose()?
                    .unwrap_or(true);

                if guard_matches {
                    handler = Some(candidate.clone());
                    break;
                }
            }

            if handler.is_some() {
                break;
            }
        }

        let Some(handler) = handler else {
            return Ok(EventProcessingOutcome {
                status: EventProcessingStatus::Ignored,
                transition_id,
                from_state: from_state.clone(),
                to_state: from_state,
                effects: Vec::new(),
                reason: Some(format!(
                    "state `{}` has no matching handler for event `{}`",
                    self.state.current_state, event.event_type
                )),
            });
        };

        let mut effects = Vec::new();
        let mut locals = BTreeMap::new();
        let outcome = self.execute_steps(
            &handler.steps,
            event,
            handler.binding.as_deref(),
            &mut locals,
            &transition_id,
            "handler",
        )?;
        effects.extend(outcome.effects);
        let transition = outcome.transition.or(handler.transition);

        if let Some(target) = transition {
            self.transition_to(
                target,
                event,
                handler.binding.as_deref(),
                &mut locals,
                &transition_id,
                &mut effects,
            )?;
        }
        self.process_always_transitions(event, &mut locals, &transition_id, &mut effects)?;
        self.check_expression_invariants(event)?;

        let to_state = self.state.current_state.clone();
        self.last_transition = Some(format!(
            "{} --{}--> {}",
            from_state, event.event_type, to_state
        ));

        Ok(EventProcessingOutcome {
            status: EventProcessingStatus::Processed,
            transition_id,
            from_state,
            to_state,
            effects,
            reason: None,
        })
    }

    fn check_expression_invariants(
        &self,
        event: &queue::WorkflowEvent,
    ) -> Result<(), InterpreterError> {
        let locals = BTreeMap::new();
        for invariant in &self.ir.invariants {
            let armature_workflow::ir::Invariant::Expression { name, expr, .. } = invariant else {
                continue;
            };
            let value = self.eval_expr(expr, event, None, &locals, Some("invariant"))?;
            match value.as_bool() {
                Some(true) => {}
                Some(false) => {
                    return Err(InterpreterError::InvariantViolated { name: name.clone() });
                }
                None => {
                    return Err(InterpreterError::InvariantNotBoolean { name: name.clone() });
                }
            }
        }
        Ok(())
    }

    fn execute_steps(
        &mut self,
        steps: &[armature_workflow::ir::Step],
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &mut BTreeMap<String, serde_json::Value>,
        transition_id: &str,
        path_prefix: &str,
    ) -> Result<StepExecutionOutcome, InterpreterError> {
        let mut outcome = StepExecutionOutcome::default();
        for (index, step) in steps.iter().enumerate() {
            let step_outcome = self.apply_step(
                step,
                event,
                event_binding,
                locals,
                transition_id,
                &format!("{path_prefix}.{index}"),
            )?;
            outcome.effects.extend(step_outcome.effects);
            if step_outcome.transition.is_some() {
                outcome.transition = step_outcome.transition;
                break;
            }
        }
        Ok(outcome)
    }

    fn transition_to(
        &mut self,
        mut target: String,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &mut BTreeMap<String, serde_json::Value>,
        transition_id: &str,
        effects: &mut Vec<EffectRequest>,
    ) -> Result<(), InterpreterError> {
        const MAX_ENTRY_TRANSITIONS: usize = 16;

        for depth in 0..MAX_ENTRY_TRANSITIONS {
            let active_state = target_leaf_state(&self.ir, &target).unwrap_or(target.clone());
            self.state.current_state = active_state;

            let entry_steps = entry_steps_for_target(&self.ir, &target).unwrap_or_default();
            let mut entry_transition = None;
            for (state_index, steps) in entry_steps.iter().enumerate() {
                let outcome = self.execute_steps(
                    steps,
                    event,
                    event_binding,
                    locals,
                    transition_id,
                    &format!("entry.{depth}.{state_index}"),
                )?;
                effects.extend(outcome.effects);
                if outcome.transition.is_some() {
                    entry_transition = outcome.transition;
                    break;
                }
            }

            let Some(next_target) = entry_transition else {
                return Ok(());
            };
            target = next_target;
        }

        Err(InterpreterError::EntryTransitionLimitExceeded)
    }

    fn process_always_transitions(
        &mut self,
        event: &queue::WorkflowEvent,
        locals: &mut BTreeMap<String, serde_json::Value>,
        transition_id: &str,
        effects: &mut Vec<EffectRequest>,
    ) -> Result<(), InterpreterError> {
        const MAX_ALWAYS_TRANSITIONS: usize = 16;

        for depth in 0..MAX_ALWAYS_TRANSITIONS {
            let current_state = self.state.current_state.clone();
            let state_chain = state_chain(&self.ir, &current_state)
                .ok_or_else(|| InterpreterError::UnknownCurrentState(current_state.clone()))?;
            let mut selected = None;
            for state in state_chain.iter().rev() {
                for transition in &state.always {
                    let guard_matches = transition
                        .guard
                        .as_ref()
                        .map(|guard| {
                            self.eval_guard(guard, event, None, locals, Some("always.guard"))
                        })
                        .transpose()?
                        .unwrap_or(true);

                    if guard_matches {
                        selected = Some(transition.clone());
                        break;
                    }
                }

                if selected.is_some() {
                    break;
                }
            }

            let Some(always) = selected else {
                return Ok(());
            };

            let outcome = self.execute_steps(
                &always.steps,
                event,
                None,
                locals,
                transition_id,
                &format!("always.{depth}"),
            )?;
            effects.extend(outcome.effects);
            let target = outcome.transition.unwrap_or(always.transition);
            self.transition_to(target, event, None, locals, transition_id, effects)?;
        }

        Err(InterpreterError::AlwaysTransitionLimitExceeded)
    }

    fn apply_step(
        &mut self,
        step: &armature_workflow::ir::Step,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &mut BTreeMap<String, serde_json::Value>,
        transition_id: &str,
        step_path: &str,
    ) -> Result<StepExecutionOutcome, InterpreterError> {
        match step.effect.as_str() {
            "assign" => {
                self.apply_assign(step, event, event_binding, locals, step_path)?;
                Ok(StepExecutionOutcome::default())
            }
            "let" => {
                self.apply_let(step, event, event_binding, locals, step_path)?;
                Ok(StepExecutionOutcome::default())
            }
            "send" | "start" | "askHuman" | "capability_call" => Ok(StepExecutionOutcome {
                effects: vec![self.build_effect_request(
                    step,
                    event,
                    event_binding,
                    locals,
                    transition_id,
                    step_path,
                )?],
                transition: None,
            }),
            "raise" => Ok(StepExecutionOutcome {
                effects: vec![self.build_effect_request(
                    step,
                    event,
                    event_binding,
                    locals,
                    transition_id,
                    step_path,
                )?],
                transition: None,
            }),
            "case" => self.apply_case(step, event, event_binding, locals, transition_id, step_path),
            "goto" => Ok(StepExecutionOutcome {
                effects: Vec::new(),
                transition: step
                    .args
                    .get("target")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            }),
            _ => Ok(StepExecutionOutcome::default()),
        }
    }

    fn apply_case(
        &mut self,
        step: &armature_workflow::ir::Step,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &mut BTreeMap<String, serde_json::Value>,
        transition_id: &str,
        step_path: &str,
    ) -> Result<StepExecutionOutcome, InterpreterError> {
        let expr: Expr = serde_json::from_value(
            step.args
                .get("expr")
                .cloned()
                .ok_or(InterpreterError::UnsupportedExpression)?,
        )
        .map_err(|_| InterpreterError::UnsupportedExpression)?;
        let value = self.eval_expr(&expr, event, event_binding, locals, Some(step_path))?;

        for (arm_index, arm) in step.case_arms.iter().enumerate() {
            if !case_pattern_matches(&arm.pattern, &value) {
                continue;
            }

            let mut outcome = StepExecutionOutcome {
                effects: Vec::new(),
                transition: arm.transition.clone(),
            };
            for (nested_index, nested_step) in arm.steps.iter().enumerate() {
                let nested_outcome = self.apply_step(
                    nested_step,
                    event,
                    event_binding,
                    locals,
                    transition_id,
                    &format!("{step_path}.{arm_index}.{nested_index}"),
                )?;
                outcome.effects.extend(nested_outcome.effects);
                if nested_outcome.transition.is_some() {
                    outcome.transition = nested_outcome.transition;
                    break;
                }
            }
            return Ok(outcome);
        }

        Ok(StepExecutionOutcome::default())
    }

    fn build_effect_request(
        &self,
        step: &armature_workflow::ir::Step,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        transition_id: &str,
        step_path: &str,
    ) -> Result<EffectRequest, InterpreterError> {
        let (category, target, effect_name, required_capabilities) = match step.effect.as_str() {
            "send" => (
                EffectCategory::Message,
                step.args
                    .get("agent")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                "send".to_string(),
                Vec::new(),
            ),
            "start" => (
                EffectCategory::AsyncInvocation,
                step.args
                    .get("agent")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                "start".to_string(),
                Vec::new(),
            ),
            "askHuman" => (
                EffectCategory::HumanObligation,
                None,
                "askHuman".to_string(),
                Vec::new(),
            ),
            "raise" => (EffectCategory::Event, None, "raise".to_string(), Vec::new()),
            "capability_call" => {
                let capability = step
                    .args
                    .get("capability")
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
                let operation = step
                    .args
                    .get("operation")
                    .and_then(|value| value.as_str())
                    .unwrap_or("call");
                (
                    EffectCategory::SyncValue,
                    capability.clone(),
                    capability
                        .map(|capability| format!("{capability}.{operation}"))
                        .unwrap_or_else(|| format!("capability.{operation}")),
                    Vec::new(),
                )
            }
            effect => (
                EffectCategory::SyncValue,
                None,
                effect.to_string(),
                Vec::new(),
            ),
        };

        let effect_id = format!("{transition_id}:{step_path}");
        Ok(EffectRequest {
            effect_id: effect_id.clone(),
            workflow_id: self.state.workflow_name.clone(),
            transition_id: transition_id.to_string(),
            idempotency_key: format!(
                "{}:{}:{}",
                self.state.workflow_name, event.event_id, effect_id
            ),
            effect: effect_name,
            category,
            target,
            args: self.eval_effect_args(step, event, event_binding, locals, step_path)?,
            required_capabilities,
            timeout_ms: None,
        })
    }

    fn eval_effect_args(
        &self,
        step: &armature_workflow::ir::Step,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: &str,
    ) -> Result<serde_json::Value, InterpreterError> {
        let mut args = serde_json::Map::new();

        for (key, value) in &step.args {
            if matches!(key.as_str(), "agent" | "capability" | "operation" | "event") {
                args.insert(key.clone(), value.clone());
                continue;
            }

            match key.as_str() {
                "message" | "reason" | "input" | "payload" => {
                    let expr: Expr = serde_json::from_value(value.clone())
                        .map_err(|_| InterpreterError::UnsupportedExpression)?;
                    args.insert(
                        key.clone(),
                        self.eval_expr(&expr, event, event_binding, locals, Some(step_path))?,
                    );
                }
                "call_args" => {
                    let exprs: Vec<Expr> = serde_json::from_value(value.clone())
                        .map_err(|_| InterpreterError::UnsupportedExpression)?;
                    let mut evaluated = Vec::new();
                    for expr in exprs {
                        evaluated.push(self.eval_expr(
                            &expr,
                            event,
                            event_binding,
                            locals,
                            Some(step_path),
                        )?);
                    }
                    args.insert(key.clone(), serde_json::Value::Array(evaluated));
                }
                _ => {
                    args.insert(key.clone(), value.clone());
                }
            }
        }

        Ok(serde_json::Value::Object(args))
    }

    fn apply_assign(
        &mut self,
        step: &armature_workflow::ir::Step,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: &str,
    ) -> Result<(), InterpreterError> {
        let target = step
            .args
            .get("target")
            .and_then(|value| value.as_str())
            .ok_or_else(|| InterpreterError::InvalidAssignTarget("<missing>".to_string()))?;

        let data_path = target
            .strip_prefix("data.")
            .ok_or_else(|| InterpreterError::InvalidAssignTarget(target.to_string()))?;
        let field_name = data_path
            .split('.')
            .next()
            .ok_or_else(|| InterpreterError::InvalidAssignTarget(target.to_string()))?;

        if !self.ir.context_schema.contains_key(field_name) {
            return Err(InterpreterError::UndeclaredDataField(
                field_name.to_string(),
            ));
        }

        let expr: Expr = serde_json::from_value(
            step.args
                .get("value")
                .cloned()
                .ok_or(InterpreterError::UnsupportedExpression)?,
        )
        .map_err(|_| InterpreterError::UnsupportedExpression)?;
        let value = self.eval_expr(&expr, event, event_binding, locals, Some(step_path))?;

        let Some((field_name, nested_path)) = data_path.split_once('.') else {
            self.state.context.insert(data_path.to_string(), value);
            return Ok(());
        };

        let mut root = self
            .state
            .context
            .get(field_name)
            .cloned()
            .or_else(|| {
                self.ir
                    .context_schema
                    .get(field_name)
                    .and_then(default_value_for_schema)
            })
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        set_json_path(&mut root, nested_path, value)?;
        self.state.context.insert(field_name.to_string(), root);

        Ok(())
    }

    fn apply_let(
        &mut self,
        step: &armature_workflow::ir::Step,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &mut BTreeMap<String, serde_json::Value>,
        step_path: &str,
    ) -> Result<(), InterpreterError> {
        let Some(name) = step.assign.as_ref() else {
            return Err(InterpreterError::UnsupportedExpression);
        };
        let expr: Expr = serde_json::from_value(
            step.args
                .get("value")
                .cloned()
                .ok_or(InterpreterError::UnsupportedExpression)?,
        )
        .map_err(|_| InterpreterError::UnsupportedExpression)?;
        let value = self.eval_expr(&expr, event, event_binding, locals, Some(step_path))?;
        locals.insert(name.clone(), value);
        Ok(())
    }

    fn eval_expr(
        &self,
        expr: &Expr,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<serde_json::Value, InterpreterError> {
        match expr {
            Expr::Literal { value } => match value.as_str() {
                Some(text) if text.contains("{{") => {
                    self.eval_interpolated_string(text, event, event_binding, locals)
                }
                _ => Ok(value.clone()),
            },
            Expr::Path { path } => self.eval_path(path, event, event_binding, locals),
            Expr::Object { fields } => {
                let mut object = serde_json::Map::new();
                for (key, value) in fields {
                    object.insert(
                        key.clone(),
                        self.eval_expr(value, event, event_binding, locals, step_path)?,
                    );
                }
                Ok(serde_json::Value::Object(object))
            }
            Expr::List { items } => {
                let mut values = Vec::new();
                for item in items {
                    values.push(self.eval_expr(item, event, event_binding, locals, step_path)?);
                }
                Ok(serde_json::Value::Array(values))
            }
            Expr::Call { name, args } => {
                self.eval_call(name, args, event, event_binding, locals, step_path)
            }
            Expr::Eq { left, right } => Ok(serde_json::Value::Bool(
                self.eval_expr(left, event, event_binding, locals, step_path)?
                    == self.eval_expr(right, event, event_binding, locals, step_path)?,
            )),
            Expr::Neq { left, right } => Ok(serde_json::Value::Bool(
                self.eval_expr(left, event, event_binding, locals, step_path)?
                    != self.eval_expr(right, event, event_binding, locals, step_path)?,
            )),
            Expr::Lt { left, right } => self.eval_ordered_comparison(
                self.eval_expr(left, event, event_binding, locals, step_path)?,
                self.eval_expr(right, event, event_binding, locals, step_path)?,
                |ordering| ordering.is_lt(),
            ),
            Expr::Lte { left, right } => self.eval_ordered_comparison(
                self.eval_expr(left, event, event_binding, locals, step_path)?,
                self.eval_expr(right, event, event_binding, locals, step_path)?,
                |ordering| ordering.is_le(),
            ),
            Expr::Gt { left, right } => self.eval_ordered_comparison(
                self.eval_expr(left, event, event_binding, locals, step_path)?,
                self.eval_expr(right, event, event_binding, locals, step_path)?,
                |ordering| ordering.is_gt(),
            ),
            Expr::Gte { left, right } => self.eval_ordered_comparison(
                self.eval_expr(left, event, event_binding, locals, step_path)?,
                self.eval_expr(right, event, event_binding, locals, step_path)?,
                |ordering| ordering.is_ge(),
            ),
            Expr::In { left, right } => {
                let needle = self.eval_expr(left, event, event_binding, locals, step_path)?;
                let haystack = self.eval_expr(right, event, event_binding, locals, step_path)?;
                let contains = if let Some(items) = haystack.as_array() {
                    items.iter().any(|item| item == &needle)
                } else if let (Some(entries), Some(key)) = (haystack.as_object(), needle.as_str()) {
                    entries.contains_key(key)
                } else {
                    false
                };
                Ok(serde_json::Value::Bool(contains))
            }
            Expr::And { exprs } => {
                for expr in exprs {
                    if !self.eval_bool(expr, event, event_binding, locals, step_path)? {
                        return Ok(serde_json::Value::Bool(false));
                    }
                }
                Ok(serde_json::Value::Bool(true))
            }
            Expr::Or { exprs } => {
                for expr in exprs {
                    if self.eval_bool(expr, event, event_binding, locals, step_path)? {
                        return Ok(serde_json::Value::Bool(true));
                    }
                }
                Ok(serde_json::Value::Bool(false))
            }
            Expr::Not { expr } => Ok(serde_json::Value::Bool(!self.eval_bool(
                expr,
                event,
                event_binding,
                locals,
                step_path,
            )?)),
        }
    }

    fn eval_call(
        &self,
        name: &str,
        args: &[Expr],
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<serde_json::Value, InterpreterError> {
        match name {
            "now" if args.is_empty() => Ok(serde_json::Value::Number(serde_json::Number::from(
                current_unix_millis(),
            ))),
            "elapsedSince" if args.len() == 1 => {
                let value = self.eval_expr(&args[0], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Number(serde_json::Number::from(
                    elapsed_millis_since(&value)?,
                )))
            }
            "time.elapsedSince" if args.len() == 1 => {
                let value = self.eval_expr(&args[0], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Number(serde_json::Number::from(
                    elapsed_millis_since(&value)?,
                )))
            }
            "list.length" if args.len() == 1 => {
                let values =
                    self.eval_list_arg(&args[0], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Number(serde_json::Number::from(
                    values.len(),
                )))
            }
            "list.isEmpty" if args.len() == 1 => {
                let values =
                    self.eval_list_arg(&args[0], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Bool(values.is_empty()))
            }
            "list.contains" if args.len() == 2 => {
                let values =
                    self.eval_list_arg(&args[0], event, event_binding, locals, step_path)?;
                let needle = self.eval_expr(&args[1], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Bool(
                    values.iter().any(|value| value == &needle),
                ))
            }
            "list.append" if args.len() == 2 => {
                let mut values =
                    self.eval_list_arg(&args[0], event, event_binding, locals, step_path)?;
                values.push(self.eval_expr(&args[1], event, event_binding, locals, step_path)?);
                Ok(serde_json::Value::Array(values))
            }
            "list.remove" if args.len() == 2 => {
                let mut values =
                    self.eval_list_arg(&args[0], event, event_binding, locals, step_path)?;
                let needle = self.eval_expr(&args[1], event, event_binding, locals, step_path)?;
                values.retain(|value| value != &needle);
                Ok(serde_json::Value::Array(values))
            }
            "list.first" if args.len() == 1 => {
                let values =
                    self.eval_list_arg(&args[0], event, event_binding, locals, step_path)?;
                Ok(values.first().cloned().unwrap_or(serde_json::Value::Null))
            }
            "map.get" if args.len() == 2 => {
                let map = self.eval_map_arg(&args[0], event, event_binding, locals, step_path)?;
                let key =
                    self.eval_map_key_arg(&args[1], event, event_binding, locals, step_path)?;
                Ok(map.get(&key).cloned().unwrap_or(serde_json::Value::Null))
            }
            "map.set" if args.len() == 3 => {
                let mut map =
                    self.eval_map_arg(&args[0], event, event_binding, locals, step_path)?;
                let key =
                    self.eval_map_key_arg(&args[1], event, event_binding, locals, step_path)?;
                let value = self.eval_expr(&args[2], event, event_binding, locals, step_path)?;
                map.insert(key, value);
                Ok(serde_json::Value::Object(map))
            }
            "map.remove" if args.len() == 2 => {
                let mut map =
                    self.eval_map_arg(&args[0], event, event_binding, locals, step_path)?;
                let key =
                    self.eval_map_key_arg(&args[1], event, event_binding, locals, step_path)?;
                map.remove(&key);
                Ok(serde_json::Value::Object(map))
            }
            "map.containsKey" if args.len() == 2 => {
                let map = self.eval_map_arg(&args[0], event, event_binding, locals, step_path)?;
                let key =
                    self.eval_map_key_arg(&args[1], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Bool(map.contains_key(&key)))
            }
            "text.trim" if args.len() == 1 => {
                let value =
                    self.eval_text_arg(&args[0], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::String(value.trim().to_string()))
            }
            "text.contains" if args.len() == 2 => {
                let value =
                    self.eval_text_arg(&args[0], event, event_binding, locals, step_path)?;
                let needle =
                    self.eval_text_arg(&args[1], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Bool(value.contains(&needle)))
            }
            "text.startsWith" if args.len() == 2 => {
                let value =
                    self.eval_text_arg(&args[0], event, event_binding, locals, step_path)?;
                let prefix =
                    self.eval_text_arg(&args[1], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Bool(value.starts_with(&prefix)))
            }
            "text.endsWith" if args.len() == 2 => {
                let value =
                    self.eval_text_arg(&args[0], event, event_binding, locals, step_path)?;
                let suffix =
                    self.eval_text_arg(&args[1], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Bool(value.ends_with(&suffix)))
            }
            "text.matchesGlob" if args.len() == 2 => {
                let value =
                    self.eval_text_arg(&args[0], event, event_binding, locals, step_path)?;
                let pattern =
                    self.eval_text_arg(&args[1], event, event_binding, locals, step_path)?;
                Ok(serde_json::Value::Bool(simple_glob_matches(
                    &pattern, &value,
                )))
            }
            _ if (name.ends_with(".append") || name.ends_with(".remove")) && args.len() == 1 => {
                let operation = if name.ends_with(".append") {
                    ".append"
                } else {
                    ".remove"
                };
                let receiver = name
                    .strip_suffix(operation)
                    .ok_or(InterpreterError::UnsupportedExpression)?;
                let mut values = self
                    .eval_path(receiver, event, event_binding, locals)?
                    .as_array()
                    .cloned()
                    .ok_or(InterpreterError::UnsupportedExpression)?;
                let value = self.eval_expr(&args[0], event, event_binding, locals, step_path)?;
                if operation == ".append" {
                    values.push(value);
                } else {
                    values.retain(|item| item != &value);
                }
                Ok(serde_json::Value::Array(values))
            }
            _ if name.starts_with("coerce ") => {
                let function_name = name
                    .strip_prefix("coerce ")
                    .ok_or(InterpreterError::UnsupportedExpression)?;
                self.eval_coerce_call(function_name, args, event, event_binding, locals, step_path)
            }
            _ if self.ir.coerce_functions.contains_key(name) => {
                self.eval_coerce_call(name, args, event, event_binding, locals, step_path)
            }
            _ if self.fake_call_outputs.contains_key(name) => {
                for arg in args {
                    self.eval_expr(arg, event, event_binding, locals, step_path)?;
                }
                self.fake_call_outputs
                    .get(name)
                    .cloned()
                    .ok_or(InterpreterError::UnsupportedExpression)
            }
            _ if name
                .split_once('.')
                .is_some_and(|(capability, _)| self.ir.capabilities.contains_key(capability)) =>
            {
                self.eval_capability_value_call(name, args, event, event_binding, locals, step_path)
            }
            _ => Err(InterpreterError::UnsupportedExpression),
        }
    }

    fn eval_capability_value_call(
        &self,
        name: &str,
        args: &[Expr],
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<serde_json::Value, InterpreterError> {
        let Some(dispatcher) = &self.value_dispatcher else {
            return Err(InterpreterError::UnsupportedExpression);
        };
        let Some((capability, operation)) = name.split_once('.') else {
            return Err(InterpreterError::UnsupportedExpression);
        };

        let mut call_args = Vec::with_capacity(args.len());
        for arg in args {
            call_args.push(self.eval_expr(arg, event, event_binding, locals, step_path)?);
        }
        let step_path = step_path.unwrap_or("expression");
        let effect_id = format!("{}:{}:value:{name}", event.event_id, step_path);
        let request = effects::EffectRequest {
            effect_id: effect_id.clone(),
            workflow_id: self.state.workflow_name.clone(),
            transition_id: format!("{}:value", event.event_id),
            idempotency_key: format!(
                "{}:{}:{}",
                self.state.workflow_name, event.event_id, effect_id
            ),
            effect: name.to_string(),
            category: effects::EffectCategory::SyncValue,
            target: Some(capability.to_string()),
            args: serde_json::json!({
                "capability": capability,
                "operation": operation,
                "call_args": call_args,
            }),
            required_capabilities: Vec::new(),
            timeout_ms: None,
        };

        let outcome = dispatcher.borrow_mut().dispatch(request).map_err(|error| {
            InterpreterError::CapabilityValueCallFailed {
                name: name.to_string(),
                message: error.to_string(),
            }
        })?;
        match outcome.status {
            effects::EffectOutcomeStatus::Succeeded => {
                outcome
                    .output
                    .ok_or_else(|| InterpreterError::CapabilityValueCallFailed {
                        name: name.to_string(),
                        message: "adapter returned no output".to_string(),
                    })
            }
            effects::EffectOutcomeStatus::Accepted
            | effects::EffectOutcomeStatus::Rejected
            | effects::EffectOutcomeStatus::Failed => {
                Err(InterpreterError::CapabilityValueCallFailed {
                    name: name.to_string(),
                    message: outcome
                        .error
                        .unwrap_or_else(|| format!("adapter returned status {:?}", outcome.status)),
                })
            }
        }
    }

    fn eval_interpolated_string(
        &self,
        template: &str,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, InterpreterError> {
        let trimmed = template.trim();
        if let Some(inner) = trimmed
            .strip_prefix("{{")
            .and_then(|value| value.strip_suffix("}}"))
        {
            return self.eval_template_path(inner.trim(), event, event_binding, locals);
        }

        let mut output = String::new();
        let mut rest = template;
        while let Some(start) = rest.find("{{") {
            output.push_str(&rest[..start]);
            let after_start = &rest[start + 2..];
            let Some(end) = after_start.find("}}") else {
                return Err(InterpreterError::UnsupportedExpression);
            };
            let expr = after_start[..end].trim();
            let value = self.eval_template_path(expr, event, event_binding, locals)?;
            output.push_str(&stringify_template_value(&value));
            rest = &after_start[end + 2..];
        }
        output.push_str(rest);

        Ok(serde_json::Value::String(output))
    }

    fn eval_template_path(
        &self,
        expr: &str,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, InterpreterError> {
        if expr.is_empty()
            || !expr.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
            })
        {
            return Err(InterpreterError::UnsupportedExpression);
        }

        self.eval_path(expr, event, event_binding, locals)
    }

    fn eval_list_arg(
        &self,
        expr: &Expr,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, InterpreterError> {
        self.eval_expr(expr, event, event_binding, locals, step_path)?
            .as_array()
            .cloned()
            .ok_or(InterpreterError::UnsupportedExpression)
    }

    fn eval_map_arg(
        &self,
        expr: &Expr,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<serde_json::Map<String, serde_json::Value>, InterpreterError> {
        self.eval_expr(expr, event, event_binding, locals, step_path)?
            .as_object()
            .cloned()
            .ok_or(InterpreterError::UnsupportedExpression)
    }

    fn eval_map_key_arg(
        &self,
        expr: &Expr,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<String, InterpreterError> {
        self.eval_text_arg(expr, event, event_binding, locals, step_path)
    }

    fn eval_text_arg(
        &self,
        expr: &Expr,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<String, InterpreterError> {
        self.eval_expr(expr, event, event_binding, locals, step_path)?
            .as_str()
            .map(str::to_string)
            .ok_or(InterpreterError::UnsupportedExpression)
    }

    fn eval_coerce_call(
        &self,
        function_name: &str,
        args: &[Expr],
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<serde_json::Value, InterpreterError> {
        let Some(function) = self.ir.coerce_functions.get(function_name) else {
            return Err(InterpreterError::UnsupportedExpression);
        };

        if args.len() != function.params.len() {
            return Err(InterpreterError::InvalidCoerceArity {
                function_name: function_name.to_string(),
                expected: function.params.len(),
                actual: args.len(),
            });
        }

        let mut named_args = BTreeMap::new();
        for (index, arg) in args.iter().enumerate() {
            let value = self.eval_expr(arg, event, event_binding, locals, step_path)?;
            let param = function
                .params
                .get(index)
                .ok_or(InterpreterError::UnsupportedExpression)?;
            if let Some(reason) = param.schema.explain_json_mismatch(&value, &self.ir.types) {
                return Err(InterpreterError::InvalidCoerceInput {
                    function_name: function_name.to_string(),
                    param_name: param.name.clone(),
                    reason,
                });
            }
            named_args.insert(param.name.clone(), value);
        }

        let outcome = self
            .coerce_executor
            .coerce(coerce::CoerceRequest {
                workflow_id: self.state.workflow_name.clone(),
                function_name: function_name.to_string(),
                args: named_args,
                idempotency_key: None,
                event_id: Some(event.event_id.clone()),
                step_path: step_path.map(str::to_string),
                backend: coerce::CoerceBackend::None,
                timeout_ms: None,
            })
            .map_err(|error| InterpreterError::CoerceExecutionFailed {
                function_name: function_name.to_string(),
                category: error.category,
                message: error.message,
            })?;

        let output = outcome
            .value
            .ok_or_else(|| InterpreterError::CoerceExecutionFailed {
                function_name: function_name.to_string(),
                category: coerce::CoerceErrorCategory::InternalError,
                message: "coerce executor returned no value".to_string(),
            })?;

        if let Some(reason) = function
            .output
            .explain_json_mismatch(&output, &self.ir.types)
        {
            return Err(InterpreterError::InvalidCoerceOutput {
                function_name: function_name.to_string(),
                reason,
            });
        }

        Ok(output)
    }

    fn eval_guard(
        &self,
        expr: &Expr,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<bool, InterpreterError> {
        self.eval_bool(expr, event, event_binding, locals, step_path)
    }

    fn eval_bool(
        &self,
        expr: &Expr,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
        step_path: Option<&str>,
    ) -> Result<bool, InterpreterError> {
        self.eval_expr(expr, event, event_binding, locals, step_path)?
            .as_bool()
            .ok_or(InterpreterError::GuardNotBoolean)
    }

    fn eval_ordered_comparison(
        &self,
        left: serde_json::Value,
        right: serde_json::Value,
        predicate: impl FnOnce(std::cmp::Ordering) -> bool,
    ) -> Result<serde_json::Value, InterpreterError> {
        let ordering = compare_json_values(&left, &right)?;
        Ok(serde_json::Value::Bool(predicate(ordering)))
    }

    fn eval_path(
        &self,
        path: &str,
        event: &queue::WorkflowEvent,
        event_binding: Option<&str>,
        locals: &BTreeMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, InterpreterError> {
        if let Some((local_name, rest)) = path.split_once('.') {
            if let Some(value) = locals.get(local_name) {
                return value
                    .pointer(&json_pointer(rest))
                    .cloned()
                    .ok_or(InterpreterError::UnsupportedExpression);
            }
        } else if let Some(value) = locals.get(path) {
            return Ok(value.clone());
        }

        if let Some(binding) = event_binding {
            if let Some(rest) = path
                .strip_prefix(binding)
                .and_then(|rest| rest.strip_prefix('.'))
            {
                return event
                    .payload
                    .pointer(&json_pointer(rest))
                    .cloned()
                    .ok_or_else(|| InterpreterError::MissingEventPath(path.to_string()));
            }
        }

        if let Some(rest) = path
            .strip_prefix("data")
            .and_then(|rest| rest.strip_prefix('.'))
        {
            let field_name = rest.split('.').next().unwrap_or(rest);
            let value = self.state.context.get(field_name).cloned().or_else(|| {
                self.ir
                    .context_schema
                    .get(field_name)
                    .and_then(default_value_for_schema)
            });

            if let Some(value) = value {
                if field_name == rest {
                    return Ok(value);
                }

                let nested_path = rest
                    .strip_prefix(field_name)
                    .and_then(|rest| rest.strip_prefix('.'))
                    .ok_or_else(|| InterpreterError::MissingDataPath(path.to_string()))?;
                return value
                    .pointer(&json_pointer(nested_path))
                    .cloned()
                    .ok_or_else(|| InterpreterError::MissingDataPath(path.to_string()));
            }

            return Err(InterpreterError::MissingDataPath(path.to_string()));
        }

        Err(InterpreterError::UnsupportedExpression)
    }
}

fn json_pointer(path: &str) -> String {
    let mut pointer = String::new();
    for segment in path.split('.') {
        pointer.push('/');
        pointer.push_str(&segment.replace('~', "~0").replace('/', "~1"));
    }
    pointer
}

fn set_json_path(
    root: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> Result<(), InterpreterError> {
    let mut current = root;
    let mut segments = path.split('.').peekable();
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            let object = current
                .as_object_mut()
                .ok_or(InterpreterError::UnsupportedExpression)?;
            object.insert(segment.to_string(), value);
            return Ok(());
        }

        let object = current
            .as_object_mut()
            .ok_or(InterpreterError::UnsupportedExpression)?;
        current = object
            .entry(segment.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    }

    Err(InterpreterError::UnsupportedExpression)
}

pub fn initial_context_from_ir(ir: &WorkflowIr) -> BTreeMap<String, serde_json::Value> {
    ir.context_initializers
        .iter()
        .filter_map(|(name, expr)| {
            static_initializer_value(expr).map(|value| (name.clone(), value))
        })
        .collect()
}

fn static_initializer_value(expr: &Expr) -> Option<serde_json::Value> {
    match expr {
        Expr::Literal { value } => Some(value.clone()),
        Expr::List { items } => items
            .iter()
            .map(static_initializer_value)
            .collect::<Option<Vec<_>>>()
            .map(serde_json::Value::Array),
        Expr::Object { fields } => {
            let mut object = serde_json::Map::new();
            for (name, value) in fields {
                object.insert(name.clone(), static_initializer_value(value)?);
            }
            Some(serde_json::Value::Object(object))
        }
        Expr::Path { .. }
        | Expr::Call { .. }
        | Expr::Eq { .. }
        | Expr::Neq { .. }
        | Expr::Lt { .. }
        | Expr::Lte { .. }
        | Expr::Gt { .. }
        | Expr::Gte { .. }
        | Expr::And { .. }
        | Expr::Or { .. }
        | Expr::Not { .. }
        | Expr::In { .. } => None,
    }
}

fn default_value_for_schema(
    schema: &armature_workflow::schema::Schema,
) -> Option<serde_json::Value> {
    match schema {
        armature_workflow::schema::Schema::Optional { .. }
        | armature_workflow::schema::Schema::Null => Some(serde_json::Value::Null),
        armature_workflow::schema::Schema::List { .. }
        | armature_workflow::schema::Schema::Set { .. } => {
            Some(serde_json::Value::Array(Vec::new()))
        }
        armature_workflow::schema::Schema::Map { .. }
        | armature_workflow::schema::Schema::Record { .. } => {
            Some(serde_json::Value::Object(serde_json::Map::new()))
        }
        armature_workflow::schema::Schema::Boolean => Some(serde_json::Value::Bool(false)),
        armature_workflow::schema::Schema::Int => {
            Some(serde_json::Value::Number(serde_json::Number::from(0)))
        }
        armature_workflow::schema::Schema::Float => {
            Some(serde_json::Value::Number(serde_json::Number::from(0)))
        }
        armature_workflow::schema::Schema::String
        | armature_workflow::schema::Schema::Time
        | armature_workflow::schema::Schema::Duration
        | armature_workflow::schema::Schema::Agent
        | armature_workflow::schema::Schema::Enum { .. }
        | armature_workflow::schema::Schema::Ref { .. }
        | armature_workflow::schema::Schema::Union { .. }
        | armature_workflow::schema::Schema::Literal { .. }
        | armature_workflow::schema::Schema::Json => None,
    }
}

fn initial_leaf_state(ir: &WorkflowIr) -> Option<String> {
    target_leaf_state(ir, &ir.statechart.initial)
}

fn target_leaf_state(ir: &WorkflowIr, target: &str) -> Option<String> {
    let (_, state) = find_state(&ir.statechart.states, target)?;
    Some(descend_initial(target, state))
}

fn entry_steps_for_target(
    ir: &WorkflowIr,
    target: &str,
) -> Option<Vec<Vec<armature_workflow::ir::Step>>> {
    let (_, state) = find_state(&ir.statechart.states, target)?;
    let mut entry_steps = Vec::new();
    collect_entry_steps_from_state(state, &mut entry_steps);
    Some(entry_steps)
}

fn collect_entry_steps_from_state(
    state: &State,
    entry_steps: &mut Vec<Vec<armature_workflow::ir::Step>>,
) {
    if !state.entry.is_empty() {
        entry_steps.push(state.entry.clone());
    }

    let Some(initial) = &state.initial else {
        return;
    };
    if let Some(child) = state.states.get(initial) {
        collect_entry_steps_from_state(child, entry_steps);
    }
}

fn descend_initial(name: &str, state: &State) -> String {
    let Some(initial) = &state.initial else {
        return name.to_string();
    };

    state
        .states
        .get_key_value(initial)
        .map(|(name, child)| descend_initial(name, child))
        .unwrap_or_else(|| name.to_string())
}

fn find_state<'a>(
    states: &'a BTreeMap<String, State>,
    target: &str,
) -> Option<(&'a str, &'a State)> {
    for (name, state) in states {
        if name == target {
            return Some((name, state));
        }
        if let Some(found) = find_state(&state.states, target) {
            return Some(found);
        }
    }
    None
}

fn state_chain<'a>(ir: &'a WorkflowIr, target: &str) -> Option<Vec<&'a State>> {
    let mut chain = Vec::new();
    if collect_state_chain(&ir.statechart.states, target, &mut chain) {
        Some(chain)
    } else {
        None
    }
}

fn collect_state_chain<'a>(
    states: &'a BTreeMap<String, State>,
    target: &str,
    chain: &mut Vec<&'a State>,
) -> bool {
    for (name, state) in states {
        chain.push(state);
        if name == target || collect_state_chain(&state.states, target, chain) {
            return true;
        }
        chain.pop();
    }
    false
}

fn compare_json_values(
    left: &serde_json::Value,
    right: &serde_json::Value,
) -> Result<std::cmp::Ordering, InterpreterError> {
    if let (Some(left), Some(right)) = (comparable_number(left), comparable_number(right)) {
        return left
            .partial_cmp(&right)
            .ok_or(InterpreterError::UnsupportedExpression);
    }

    if let (Some(left), Some(right)) = (left.as_f64(), right.as_f64()) {
        return left
            .partial_cmp(&right)
            .ok_or(InterpreterError::UnsupportedExpression);
    }

    if let (Some(left), Some(right)) = (left.as_str(), right.as_str()) {
        return Ok(left.cmp(right));
    }

    Err(InterpreterError::UnsupportedExpression)
}

fn stringify_template_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => value.to_string(),
    }
}

fn comparable_number(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().or_else(|| {
        value
            .as_str()
            .and_then(parse_duration_millis)
            .map(|millis| millis as f64)
    })
}

fn current_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn elapsed_millis_since(value: &serde_json::Value) -> Result<u64, InterpreterError> {
    if value.is_null() {
        return Ok(u64::MAX);
    }

    let Some(start) = value.as_u64() else {
        return Err(InterpreterError::UnsupportedExpression);
    };

    Ok(current_unix_millis().saturating_sub(start))
}

fn parse_duration_millis(value: &str) -> Option<u64> {
    let unit_start = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let amount = value[..unit_start].parse::<u64>().ok()?;
    let unit = &value[unit_start..];
    let multiplier = match unit {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    amount.checked_mul(multiplier)
}

fn case_pattern_matches(
    pattern: &armature_workflow::ir::CasePattern,
    value: &serde_json::Value,
) -> bool {
    match pattern {
        armature_workflow::ir::CasePattern::Wildcard => true,
        armature_workflow::ir::CasePattern::Literal { value: expected } => expected == value,
        armature_workflow::ir::CasePattern::Identifier { name } => {
            value.as_str().is_some_and(|actual| actual == name)
        }
        armature_workflow::ir::CasePattern::Matches { pattern } => value
            .as_str()
            .is_some_and(|actual| simple_glob_matches(pattern, actual)),
    }
}

fn simple_glob_matches(pattern: &str, actual: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == actual;
    }

    let starts_with_wildcard = pattern.starts_with('*');
    let ends_with_wildcard = pattern.ends_with('*');
    let parts: Vec<&str> = pattern.split('*').filter(|part| !part.is_empty()).collect();
    let mut position = 0usize;

    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if index == 0 && !starts_with_wildcard {
            if !actual.starts_with(part) {
                return false;
            }
            position = part.len();
            continue;
        }

        let Some(found_at) = actual[position..].find(part) else {
            return false;
        };
        position += found_at + part.len();
    }

    ends_with_wildcard
        || parts
            .last()
            .is_none_or(|last_part| actual.ends_with(last_part))
}

#[cfg(test)]
mod tests {
    use super::coerce::{
        BamlHttpCoerceExecutor, CoerceBackend, CoerceCallRecord, CoerceError, CoerceErrorCategory,
        CoerceExecutor, CoerceOutcome, CoerceRequest, CoerceStatus, DurableCoerceExecutor,
    };
    use super::log::{EffectStatus, WorkflowLogRecord};
    use super::queue::{EventStatus, WorkflowEvent};
    use super::state::WorkflowState;
    use super::storage::{
        AgentCompletionRecord, AgentInvocationRecord, AgentInvocationStatus, StorageError,
        WorkflowStore,
    };
    use super::{
        baml_runtime_summary, EventProcessingStatus, Interpreter, InterpreterError, WorkflowRuntime,
    };
    use serde_json::json;
    use std::cell::Cell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::rc::Rc;

    fn loopback_tests_enabled() -> bool {
        std::env::var_os("ARMATURE_RUN_LOOPBACK_TESTS").is_some()
    }

    fn minimal_interpreter() -> Interpreter {
        let source = include_str!("../../../examples/workflows/minimal.armature");
        let ir = armature_workflow::parse_source(source).expect("minimal source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        Interpreter::new(ir)
    }

    fn interpreter_from_source(source: &str) -> Interpreter {
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        Interpreter::new(ir)
    }

    fn event(event_type: &str, payload: serde_json::Value) -> WorkflowEvent {
        WorkflowEvent {
            event_id: "evt_1".to_string(),
            workflow_id: "Minimal".to_string(),
            event_type: event_type.to_string(),
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
        }
    }

    #[test]
    fn simple_glob_matching_is_anchored_without_wildcards() {
        assert!(super::simple_glob_matches("worker-*", "worker-1"));
        assert!(super::simple_glob_matches("*-done", "worker-done"));
        assert!(super::simple_glob_matches("worker", "worker"));
        assert!(!super::simple_glob_matches("worker", "worker-1"));
        assert!(!super::simple_glob_matches("worker-*", "quality-1"));
    }

    fn workflow_event(
        workflow_id: &str,
        event_id: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> WorkflowEvent {
        WorkflowEvent {
            event_id: event_id.to_string(),
            workflow_id: workflow_id.to_string(),
            event_type: event_type.to_string(),
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
        }
    }

    #[test]
    fn processes_minimal_event_into_state_and_context() {
        let mut interpreter = minimal_interpreter();
        let outcome = interpreter
            .process_event(&event("start", json!({"message": "ship it"})))
            .expect("event processes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.from_state, "waiting");
        assert_eq!(outcome.to_state, "complete");
        assert_eq!(
            interpreter.context().get("lastMessage"),
            Some(&json!("ship it"))
        );
        assert_eq!(interpreter.status(0).current_state, "complete");
        assert_eq!(
            interpreter.status(0).recent_transition.as_deref(),
            Some("waiting --start--> complete")
        );
    }

    #[test]
    fn expression_invariant_violation_rolls_back_interpreter_state() {
        let source = r#"
machine RuntimeInvariant
initial waiting

data {
  count int = 0
}

event go {}

state waiting {
  on go {
    assign data.count = 2
    stay
  }
}
"#;
        let mut ir = armature_workflow::parse_source(source).expect("source parses");
        ir.invariants
            .push(armature_workflow::ir::Invariant::Expression {
                name: "countAtMostOne".to_string(),
                expr: armature_workflow::expr::Expr::Lte {
                    left: Box::new(armature_workflow::expr::Expr::Path {
                        path: "data.count".to_string(),
                    }),
                    right: Box::new(armature_workflow::expr::Expr::Literal { value: json!(1) }),
                },
                span: None,
            });
        let mut interpreter = Interpreter::new(ir);

        let error = interpreter
            .process_event(&workflow_event(
                "RuntimeInvariant",
                "evt_go",
                "go",
                json!({}),
            ))
            .expect_err("invariant violation fails the transition");

        assert!(matches!(
            error,
            InterpreterError::InvariantViolated { ref name } if name == "countAtMostOne"
        ));
        assert_eq!(interpreter.context().get("count"), Some(&json!(0)));
        assert_eq!(interpreter.status(0).current_state, "waiting");
        assert_eq!(interpreter.status(0).recent_transition, None);
    }

    #[test]
    fn expression_invariant_must_evaluate_to_boolean() {
        let source = r#"
machine RuntimeInvariant
initial waiting

data {
  count int = 1
}

event go {}

state waiting {
  on go {
    stay
  }
}
"#;
        let mut ir = armature_workflow::parse_source(source).expect("source parses");
        ir.invariants
            .push(armature_workflow::ir::Invariant::Expression {
                name: "countIsBoolean".to_string(),
                expr: armature_workflow::expr::Expr::Path {
                    path: "data.count".to_string(),
                },
                span: None,
            });
        let mut interpreter = Interpreter::new(ir);

        let error = interpreter
            .process_event(&workflow_event(
                "RuntimeInvariant",
                "evt_go",
                "go",
                json!({}),
            ))
            .expect_err("non-boolean invariant fails the transition");

        assert!(matches!(
            error,
            InterpreterError::InvariantNotBoolean { ref name } if name == "countIsBoolean"
        ));
    }

    #[test]
    fn ignores_event_without_current_state_handler() {
        let mut interpreter = minimal_interpreter();
        let outcome = interpreter
            .process_event(&event("other", json!({})))
            .expect("unknown event is ignored");

        assert_eq!(outcome.status, EventProcessingStatus::Ignored);
        assert_eq!(outcome.from_state, "waiting");
        assert_eq!(outcome.to_state, "waiting");
        assert!(outcome.reason.is_some());
    }

    #[test]
    fn rejects_invalid_declared_event_payload() {
        let mut interpreter = minimal_interpreter();
        let error = interpreter
            .process_event(&event("start", json!({"message": 42})))
            .expect_err("payload should fail schema validation");

        assert!(matches!(
            error,
            InterpreterError::InvalidEventPayload { event_type, reason }
                if event_type == "start" && reason.contains("$.message expected string, got int")
        ));
    }

    #[test]
    fn rejects_event_payloads_with_extra_fields() {
        let mut interpreter = minimal_interpreter();
        let error = interpreter
            .process_event(&event(
                "start",
                json!({"message": "ship it", "misspelled": true}),
            ))
            .expect_err("payload with extra field should fail schema validation");

        assert!(matches!(
            error,
            InterpreterError::InvalidEventPayload { event_type, reason }
                if event_type == "start" && reason.contains("$.misspelled is not declared")
        ));
    }

    #[test]
    fn accepts_missing_optional_event_payload_fields() {
        let source = r#"
machine OptionalPayload
initial waiting

event go {
  required string
  optional int?
}

state waiting {
  on go as evt {
    goto done
  }
}

state done {
  final
}
"#;
        let mut interpreter = interpreter_from_source(source);
        let outcome = interpreter
            .process_event(&workflow_event(
                "OptionalPayload",
                "evt_optional",
                "go",
                json!({"required": "present"}),
            ))
            .expect("missing optional field is accepted");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.to_state, "done");
    }

    #[test]
    fn respects_event_handler_guards() {
        let source = r#"
machine Guarded
initial waiting

event go {
  count int
}

state waiting {
  on go as evt
    guard evt.count > 1
  {
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut interpreter = Interpreter::new(ir);
        let ignored = interpreter
            .process_event(&workflow_event(
                "Guarded",
                "evt_guard_1",
                "go",
                json!({"count": 1}),
            ))
            .expect("guard false is handled as ignored");
        assert_eq!(ignored.status, EventProcessingStatus::Ignored);
        assert_eq!(ignored.to_state, "waiting");

        let processed = interpreter
            .process_event(&workflow_event(
                "Guarded",
                "evt_guard_2",
                "go",
                json!({"count": 2}),
            ))
            .expect("guard true processes");
        assert_eq!(processed.status, EventProcessingStatus::Processed);
        assert_eq!(processed.to_state, "done");
    }

    #[test]
    fn sqlite_store_persists_state_events_and_logs() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let schema_version: String = store
            .connection()
            .query_row(
                "SELECT value FROM armature_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema version reads");
        assert_eq!(schema_version, "4");

        let mut context = BTreeMap::new();
        context.insert("lastMessage".to_string(), json!("persisted"));
        let state = WorkflowState {
            workflow_name: "Minimal".to_string(),
            current_state: "waiting".to_string(),
            context,
        };

        store.save_state(&state).expect("state saves");
        assert_eq!(
            store
                .load_state("Minimal")
                .expect("state loads")
                .expect("state exists"),
            state
        );

        let event = event("start", json!({"message": "hello"}));
        store.enqueue_event(&event).expect("event enqueues");
        assert_eq!(
            store
                .queued_event_summaries("Minimal", 10)
                .expect("queued summaries load"),
            vec![crate::status::QueuedEventSummary {
                event_id: "evt_1".to_string(),
                event_type: "start".to_string(),
                source: None,
                attempt_count: 0,
            }]
        );
        let dequeued = store
            .dequeue_next_event("Minimal")
            .expect("event dequeues")
            .expect("event exists");
        assert_eq!(dequeued.event_id, "evt_1");
        assert_eq!(dequeued.status, EventStatus::Processing);
        assert_eq!(dequeued.attempt_count, 1);
        store
            .update_event_status(&dequeued, EventStatus::Processed)
            .expect("event status updates");
        let events = store.events("Minimal", 10).expect("events load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, EventStatus::Processed);
        assert!(store
            .dequeue_next_event("Minimal")
            .expect("queue checks")
            .is_none());

        let record = WorkflowLogRecord::Effect {
            effect_id: "effect_1".to_string(),
            workflow_id: "Minimal".to_string(),
            transition_id: "transition_1".to_string(),
            idempotency_key: Some("Minimal:evt_1:effect_1".to_string()),
            effect: "assign".to_string(),
            category: super::effects::EffectCategory::Context,
            target: None,
            args: json!({}),
            required_capabilities: Vec::new(),
            status: EffectStatus::Succeeded,
            outcome: None,
        };
        store
            .append_log("Minimal", &record)
            .expect("log record appends");
        assert_eq!(
            store.log_records("Minimal").expect("log records load"),
            vec![record.clone()]
        );
        assert_eq!(
            store
                .recent_log_records("Minimal", 10)
                .expect("recent log records load"),
            vec![record]
        );
    }

    #[test]
    fn sqlite_store_persists_coerce_call_records() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut args = BTreeMap::new();
        args.insert("run".to_string(), json!({"id": "run_1"}));

        let successful = CoerceCallRecord {
            coerce_call_id: "coerce_1".to_string(),
            workflow_id: "Supervisor".to_string(),
            workflow_version: "version_1".to_string(),
            transition_id: Some("transition_1".to_string()),
            event_id: Some("event_1".to_string()),
            step_path: "watching.on.finished[0]".to_string(),
            function_name: "ClassifyRun".to_string(),
            idempotency_key: "Supervisor:event_1:ClassifyRun".to_string(),
            backend: CoerceBackend::BamlHttp {
                url: "http://127.0.0.1:2024".to_string(),
                baml_src_hash: Some("sha256:test".to_string()),
                runtime_mode: None,
            },
            args,
            status: CoerceStatus::Succeeded,
            http_status: Some(200),
            raw_response: Some(json!({"value": {"kind": "workerDone"}})),
            parsed_output: Some(json!({"kind": "workerDone"})),
            error: None,
            duration_ms: Some(42),
            created_at: "2026-05-23T00:00:00Z".to_string(),
        };

        store
            .append_coerce_call_attempt(&successful)
            .expect("successful coerce call appends");

        assert_eq!(
            store
                .find_successful_coerce_call("Supervisor", "Supervisor:event_1:ClassifyRun")
                .expect("successful coerce call loads"),
            Some(successful.clone())
        );
        assert_eq!(
            store
                .latest_coerce_calls("Supervisor", 10)
                .expect("latest coerce calls load"),
            vec![successful.clone()]
        );

        let mut failed = successful.clone();
        failed.coerce_call_id = "coerce_2".to_string();
        failed.idempotency_key = "Supervisor:event_2:ClassifyRun".to_string();
        failed.event_id = Some("event_2".to_string());
        failed.status = CoerceStatus::Failed;
        failed.http_status = Some(503);
        failed.raw_response = Some(json!({"error": "unavailable"}));
        failed.parsed_output = None;
        failed.error = Some("BAML server unavailable".to_string());
        failed.duration_ms = Some(7);

        store
            .append_coerce_call_attempt(&failed)
            .expect("failed coerce call appends");
        assert_eq!(
            store
                .latest_coerce_failures("Supervisor", 10)
                .expect("latest coerce failures load"),
            vec![failed]
        );
    }

    #[test]
    fn sqlite_store_rejects_duplicate_successful_coerce_idempotency_keys() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let record = CoerceCallRecord {
            coerce_call_id: "coerce_1".to_string(),
            workflow_id: "Supervisor".to_string(),
            workflow_version: "version_1".to_string(),
            transition_id: None,
            event_id: None,
            step_path: "watching.entry[0]".to_string(),
            function_name: "ChooseNextStep".to_string(),
            idempotency_key: "Supervisor:tick:ChooseNextStep".to_string(),
            backend: CoerceBackend::Fake,
            args: BTreeMap::new(),
            status: CoerceStatus::Succeeded,
            http_status: None,
            raw_response: None,
            parsed_output: Some(json!({"kind": "idle"})),
            error: None,
            duration_ms: None,
            created_at: "2026-05-23T00:00:00Z".to_string(),
        };

        store
            .append_coerce_call_attempt(&record)
            .expect("first successful coerce call appends");
        let mut duplicate = record;
        duplicate.coerce_call_id = "coerce_2".to_string();

        assert!(store.append_coerce_call_attempt(&duplicate).is_err());
    }

    #[test]
    fn baml_runtime_summary_projects_generated_stdio_records() {
        let record = CoerceCallRecord {
            coerce_call_id: "coerce_1".to_string(),
            workflow_id: "Supervisor".to_string(),
            workflow_version: "version_1".to_string(),
            transition_id: None,
            event_id: None,
            step_path: "watching.entry[0]".to_string(),
            function_name: "ChooseNextStep".to_string(),
            idempotency_key: "Supervisor:tick:ChooseNextStep".to_string(),
            backend: CoerceBackend::BamlGeneratedStdio {
                baml_src_hash: Some("sha256:baml".to_string()),
                runner_hash: Some("sha256:runner".to_string()),
                runtime_mode: None,
            },
            args: BTreeMap::new(),
            status: CoerceStatus::Succeeded,
            http_status: None,
            raw_response: None,
            parsed_output: Some(json!({"kind": "idle"})),
            error: None,
            duration_ms: Some(13),
            created_at: "2026-05-23T00:00:00Z".to_string(),
        };

        let summary = baml_runtime_summary(&[record]).expect("summary projects");

        assert_eq!(summary.mode, "generated_stdio");
        assert_eq!(summary.status, "observed");
        assert_eq!(summary.url, "stdio");
        assert_eq!(summary.baml_src_hash, Some("sha256:baml".to_string()));
        assert_eq!(summary.last_error, None);
        assert_eq!(summary.last_call_at, "2026-05-23T00:00:00Z");
    }

    #[test]
    fn baml_runtime_summary_projects_brokered_records() {
        let record = CoerceCallRecord {
            coerce_call_id: "coerce_1".to_string(),
            workflow_id: "Supervisor".to_string(),
            workflow_version: "version_1".to_string(),
            transition_id: None,
            event_id: None,
            step_path: "watching.entry[0]".to_string(),
            function_name: "ChooseNextStep".to_string(),
            idempotency_key: "Supervisor:tick:ChooseNextStep".to_string(),
            backend: CoerceBackend::BamlBrokered {
                request_id: "coerce_req_1".to_string(),
                baml_src_hash: Some("sha256:baml".to_string()),
            },
            args: BTreeMap::new(),
            status: CoerceStatus::Failed,
            http_status: None,
            raw_response: None,
            parsed_output: None,
            error: Some("broker unavailable".to_string()),
            duration_ms: Some(13),
            created_at: "2026-05-23T00:00:00Z".to_string(),
        };

        let summary = baml_runtime_summary(&[record]).expect("summary projects");

        assert_eq!(summary.mode, "brokered");
        assert_eq!(summary.status, "failed");
        assert_eq!(summary.url, "brokered:coerce_req_1");
        assert_eq!(summary.baml_src_hash, Some("sha256:baml".to_string()));
        assert_eq!(summary.last_error, Some("broker unavailable".to_string()));
        assert_eq!(summary.last_call_at, "2026-05-23T00:00:00Z");
    }

    #[test]
    fn baml_http_coerce_executor_posts_named_args_and_reads_json_output() {
        if !loopback_tests_enabled() {
            eprintln!(
                "skipping explicit BAML HTTP test; set ARMATURE_RUN_LOOPBACK_TESTS=1 to run it"
            );
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test server binds");
        let address = listener.local_addr().expect("test server address");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request accepted");
            let mut request = [0_u8; 4096];
            let bytes_read = std::io::Read::read(&mut stream, &mut request).expect("request reads");
            let request = String::from_utf8_lossy(&request[..bytes_read]);
            assert!(request.starts_with("POST /call/ClassifyRun "));
            assert!(request.contains(r#""message":"hello""#));

            let body = r#"{"kind":"workerDone","runId":"run_1"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            std::io::Write::write_all(&mut stream, response.as_bytes()).expect("response writes");
        });

        let executor = BamlHttpCoerceExecutor::new(format!("http://{address}"))
            .with_baml_src_hash(Some("sha256:test".to_string()))
            .with_timeout_ms(Some(1_000));
        let mut args = BTreeMap::new();
        args.insert("message".to_string(), json!("hello"));

        let outcome = executor
            .coerce(CoerceRequest {
                workflow_id: "Supervisor".to_string(),
                function_name: "ClassifyRun".to_string(),
                args,
                idempotency_key: Some("key_1".to_string()),
                event_id: Some("event_1".to_string()),
                step_path: Some("handler.0".to_string()),
                backend: CoerceBackend::None,
                timeout_ms: None,
            })
            .expect("BAML HTTP call succeeds");

        assert_eq!(outcome.status, CoerceStatus::Succeeded);
        assert_eq!(outcome.http_status, Some(200));
        assert_eq!(
            outcome.value,
            Some(json!({"kind": "workerDone", "runId": "run_1"}))
        );
        assert_eq!(
            outcome.backend,
            CoerceBackend::BamlHttp {
                url: format!("http://{address}"),
                baml_src_hash: Some("sha256:test".to_string()),
                runtime_mode: None,
            }
        );
        handle.join().expect("test server joins");
    }

    #[test]
    fn baml_http_coerce_executor_classifies_http_errors() {
        if !loopback_tests_enabled() {
            eprintln!(
                "skipping explicit BAML HTTP test; set ARMATURE_RUN_LOOPBACK_TESTS=1 to run it"
            );
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test server binds");
        let address = listener.local_addr().expect("test server address");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request accepted");
            let mut request = [0_u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut request).expect("request reads");
            let body = r#"{"error":"nope"}"#;
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            std::io::Write::write_all(&mut stream, response.as_bytes()).expect("response writes");
        });

        let error = BamlHttpCoerceExecutor::new(format!("http://{address}"))
            .with_timeout_ms(Some(1_000))
            .coerce(CoerceRequest {
                workflow_id: "Supervisor".to_string(),
                function_name: "ClassifyRun".to_string(),
                args: BTreeMap::new(),
                idempotency_key: None,
                event_id: None,
                step_path: None,
                backend: CoerceBackend::None,
                timeout_ms: None,
            })
            .expect_err("HTTP 500 should fail");

        assert_eq!(error.category, CoerceErrorCategory::BamlHttpError);
        assert_eq!(error.http_status, Some(500));
        handle.join().expect("test server joins");
    }

    #[derive(Debug)]
    struct CountingCoerceExecutor {
        calls: Rc<Cell<u32>>,
        output: serde_json::Value,
    }

    impl CoerceExecutor for CountingCoerceExecutor {
        fn coerce(&self, request: CoerceRequest) -> Result<CoerceOutcome, CoerceError> {
            self.calls.set(self.calls.get() + 1);
            Ok(CoerceOutcome {
                function_name: request.function_name,
                status: CoerceStatus::Succeeded,
                value: Some(self.output.clone()),
                backend: CoerceBackend::Fake,
                http_status: None,
                raw_response: None,
                error: None,
                duration_ms: Some(1),
            })
        }
    }

    #[derive(Debug)]
    struct FailingCoerceExecutor;

    impl CoerceExecutor for FailingCoerceExecutor {
        fn coerce(&self, _request: CoerceRequest) -> Result<CoerceOutcome, CoerceError> {
            Err(CoerceError::new(
                CoerceErrorCategory::BamlHttpError,
                "backend rejected request",
            ))
        }
    }

    fn coerce_request_for_test() -> CoerceRequest {
        let mut args = BTreeMap::new();
        args.insert("message".to_string(), json!("hello"));
        CoerceRequest {
            workflow_id: "Supervisor".to_string(),
            function_name: "ClassifyRun".to_string(),
            args,
            idempotency_key: Some("Supervisor:event_1:ClassifyRun".to_string()),
            event_id: Some("event_1".to_string()),
            step_path: Some("handler.0".to_string()),
            backend: CoerceBackend::None,
            timeout_ms: None,
        }
    }

    #[test]
    fn durable_coerce_executor_reuses_successful_records() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let calls = Rc::new(Cell::new(0));
        let executor = DurableCoerceExecutor::new(
            store.clone(),
            Box::new(CountingCoerceExecutor {
                calls: Rc::clone(&calls),
                output: json!("workerDone"),
            }),
            "version_1",
        );
        let request = coerce_request_for_test();

        let first = executor
            .coerce(request.clone())
            .expect("first call succeeds");
        let second = executor.coerce(request).expect("second call replays");

        assert_eq!(calls.get(), 1);
        assert_eq!(first.value, Some(json!("workerDone")));
        assert_eq!(second.value, Some(json!("workerDone")));
        let calls = store
            .latest_coerce_calls("Supervisor", 10)
            .expect("coerce calls load");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].status, CoerceStatus::Succeeded);
        assert_eq!(calls[0].parsed_output, Some(json!("workerDone")));
    }

    #[test]
    fn durable_coerce_executor_records_failed_attempts() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let executor =
            DurableCoerceExecutor::new(store.clone(), Box::new(FailingCoerceExecutor), "version_1");

        let error = executor
            .coerce(coerce_request_for_test())
            .expect_err("coerce call fails");

        assert_eq!(error.category, CoerceErrorCategory::BamlHttpError);
        let failures = store
            .latest_coerce_failures("Supervisor", 10)
            .expect("coerce failures load");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].status, CoerceStatus::Failed);
        assert!(failures[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("backend rejected request")));
    }

    #[test]
    fn sqlite_records_and_claims_native_agent_invocations() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let invocation = AgentInvocationRecord {
            workflow_id: "Harness".to_string(),
            invocation_id: "inv-1".to_string(),
            agent: "worker".to_string(),
            effect_id: "eff-1".to_string(),
            transition_id: "tr-1".to_string(),
            event_id: Some("evt-1".to_string()),
            idempotency_key: "Harness/evt-1/tr-1/start".to_string(),
            input: json!({"message": "ship it"}),
            requested_profile: Some("repo-writer".to_string()),
            resolved_profile: None,
            profile_enforcement: None,
            status: AgentInvocationStatus::Queued,
            claimed_by: None,
            claim_expires_at: None,
            provider: None,
            provider_run_id: None,
            run_dir: None,
            stdout_path: None,
            stderr_path: None,
            exit_code: None,
            error: None,
            created_at: "1".to_string(),
            updated_at: "1".to_string(),
        };

        store
            .insert_agent_invocation(&invocation)
            .expect("invocation inserts");
        assert_eq!(
            store
                .active_agent_invocation_counts("Harness")
                .expect("active counts load")
                .get("worker"),
            Some(&1)
        );

        let claimed = store
            .claim_agent_invocation("inv-1", "worker-process", "999")
            .expect("claim succeeds")
            .expect("invocation claimed");
        assert_eq!(claimed.claimed_by.as_deref(), Some("worker-process"));
        assert_eq!(claimed.status, AgentInvocationStatus::Claimed);
        assert!(store
            .claim_agent_invocation("inv-1", "second-process", "999")
            .expect("second claim is safe")
            .is_none());
    }

    #[test]
    fn sqlite_recovers_expired_agent_leases() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let invocation = AgentInvocationRecord {
            workflow_id: "Harness".to_string(),
            invocation_id: "inv-expired".to_string(),
            agent: "worker".to_string(),
            effect_id: "eff-1".to_string(),
            transition_id: "tr-1".to_string(),
            event_id: Some("evt-1".to_string()),
            idempotency_key: "Harness/evt-1/tr-1/start".to_string(),
            input: json!({"message": "ship it"}),
            requested_profile: Some("repo-writer".to_string()),
            resolved_profile: None,
            profile_enforcement: None,
            status: AgentInvocationStatus::Queued,
            claimed_by: None,
            claim_expires_at: None,
            provider: None,
            provider_run_id: None,
            run_dir: None,
            stdout_path: None,
            stderr_path: None,
            exit_code: None,
            error: None,
            created_at: "1".to_string(),
            updated_at: "1".to_string(),
        };
        store
            .insert_agent_invocation(&invocation)
            .expect("invocation inserts");
        store
            .claim_agent_invocation("inv-expired", "worker-process", "100")
            .expect("claim succeeds")
            .expect("invocation claimed");

        let recovered = store
            .recover_expired_agent_leases("Harness", "101")
            .expect("lease recovers");

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].status, AgentInvocationStatus::Claimed);
        let queued = store
            .queued_agent_invocations("Harness", 10)
            .expect("queued invocations load");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].status, AgentInvocationStatus::Queued);
        assert_eq!(queued[0].claimed_by, None);
        assert_eq!(queued[0].claim_expires_at, None);
    }

    #[test]
    fn sqlite_records_agent_completion_and_workflow_event_atomically() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let completion = AgentCompletionRecord {
            workflow_id: "Harness".to_string(),
            completion_id: "cmp-1".to_string(),
            invocation_id: "inv-1".to_string(),
            agent: "worker".to_string(),
            status: "succeeded".to_string(),
            summary: Some("done".to_string()),
            exit_code: Some(0),
            event_id: Some("evt-finished".to_string()),
            payload: json!({
                "id": "inv-1",
                "name": "worker",
                "status": "succeeded",
                "summary": "done",
                "exitCode": 0
            }),
            created_at: "1".to_string(),
        };
        let event = workflow_event(
            "Harness",
            "evt-finished",
            "finished",
            completion.payload.clone(),
        );

        assert!(store
            .record_agent_completion(&completion, &event)
            .expect("completion records"));

        let queued = store
            .dequeue_next_event("Harness")
            .expect("queue reads")
            .expect("completion event queued");
        assert_eq!(queued.event_type, "finished");
        assert_eq!(queued.payload["id"], "inv-1");
    }

    #[test]
    fn sqlite_records_duplicate_agent_completion_idempotently() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let completion = AgentCompletionRecord {
            workflow_id: "Harness".to_string(),
            completion_id: "cmp-1".to_string(),
            invocation_id: "inv-1".to_string(),
            agent: "worker".to_string(),
            status: "succeeded".to_string(),
            summary: Some("done".to_string()),
            exit_code: Some(0),
            event_id: Some("evt-finished".to_string()),
            payload: json!({
                "id": "inv-1",
                "name": "worker",
                "status": "succeeded",
                "summary": "done",
                "exitCode": 0
            }),
            created_at: "1".to_string(),
        };
        let event = workflow_event(
            "Harness",
            "evt-finished",
            "finished",
            completion.payload.clone(),
        );
        let duplicate_event = workflow_event(
            "Harness",
            "evt-duplicate",
            "finished",
            completion.payload.clone(),
        );

        assert!(store
            .record_agent_completion(&completion, &event)
            .expect("completion records"));
        assert!(!store
            .record_agent_completion(&completion, &duplicate_event)
            .expect("duplicate is ignored"));

        let completions = store
            .recent_agent_completions("Harness", 10)
            .expect("completions load");
        assert_eq!(completions.len(), 1);
        assert!(store
            .dequeue_next_event("Harness")
            .expect("queue reads")
            .is_some());
        assert!(store
            .dequeue_next_event("Harness")
            .expect("queue reads")
            .is_none());
    }

    #[test]
    fn sqlite_requeues_processing_events_for_recovery() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let event = workflow_event("Recoverable", "evt_recover", "tick", json!({}));

        store.enqueue_event(&event).expect("event enqueues");
        let processing = store
            .dequeue_next_event("Recoverable")
            .expect("event dequeues")
            .expect("event exists");
        assert_eq!(processing.status, EventStatus::Processing);
        assert_eq!(processing.attempt_count, 1);

        assert_eq!(
            store
                .recover_processing_events("Recoverable")
                .expect("processing events recover"),
            1
        );
        assert_eq!(
            store
                .queued_event_summaries("Recoverable", 10)
                .expect("queued summaries load")[0]
                .attempt_count,
            1
        );

        let retried = store
            .dequeue_next_event("Recoverable")
            .expect("event dequeues again")
            .expect("event exists again");
        assert_eq!(retried.status, EventStatus::Processing);
        assert_eq!(retried.attempt_count, 2);
    }

    #[test]
    fn sqlite_retry_event_requeues_retryable_statuses_only() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut failed = workflow_event("Retryable", "evt_failed", "tick", json!({}));
        failed.last_error = Some("temporary failure".to_string());
        let mut dead_lettered = workflow_event("Retryable", "evt_dead", "tick", json!({}));
        dead_lettered.last_error = Some("too many attempts".to_string());
        let queued = workflow_event("Retryable", "evt_queued", "tick", json!({}));

        store.enqueue_event(&failed).expect("failed event enqueues");
        store
            .update_event_status(&failed, EventStatus::Failed)
            .expect("failed event status updates");
        store
            .enqueue_event(&dead_lettered)
            .expect("dead-lettered event enqueues");
        store
            .update_event_status(&dead_lettered, EventStatus::DeadLettered)
            .expect("dead-lettered event status updates");
        store.enqueue_event(&queued).expect("queued event enqueues");

        let retried_failed = store
            .retry_event("Retryable", "evt_failed")
            .expect("failed event retries");
        assert_eq!(retried_failed.status, EventStatus::Queued);
        assert_eq!(retried_failed.attempt_count, 0);
        assert_eq!(retried_failed.last_error, None);

        let retried_dead = store
            .retry_event("Retryable", "evt_dead")
            .expect("dead-lettered event retries");
        assert_eq!(retried_dead.status, EventStatus::Queued);
        assert_eq!(retried_dead.attempt_count, 0);
        assert_eq!(retried_dead.last_error, None);

        let error = store
            .retry_event("Retryable", "evt_queued")
            .expect_err("queued event cannot be retried");
        assert!(matches!(
            error,
            StorageError::EventRetryNotAllowed {
                workflow_id,
                event_id,
                status,
            } if workflow_id == "Retryable" && event_id == "evt_queued" && status == "queued"
        ));
    }

    #[test]
    fn sqlite_event_status_updates_are_scoped_by_workflow() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let first = workflow_event("FirstWorkflow", "shared_event_id", "tick", json!({}));
        let second = workflow_event("SecondWorkflow", "shared_event_id", "tick", json!({}));

        store.enqueue_event(&first).expect("first event enqueues");
        store.enqueue_event(&second).expect("second event enqueues");

        let dequeued = store
            .dequeue_next_event("FirstWorkflow")
            .expect("first workflow dequeues")
            .expect("first event exists");
        assert_eq!(dequeued.status, EventStatus::Processing);
        store
            .update_event_status(&dequeued, EventStatus::Processed)
            .expect("first event status updates");

        assert_eq!(
            store
                .events("FirstWorkflow", 10)
                .expect("first events load")[0]
                .status,
            EventStatus::Processed
        );
        assert_eq!(
            store
                .events("SecondWorkflow", 10)
                .expect("second events load")[0]
                .status,
            EventStatus::Queued
        );
        assert!(store
            .dequeue_next_event("SecondWorkflow")
            .expect("second workflow queue checks")
            .is_some());
    }

    #[test]
    fn sqlite_migrates_global_event_identity_to_workflow_scoped_identity() {
        let path = std::env::temp_dir().join(format!(
            "armature-old-event-identity-{}.sqlite",
            ulid::Ulid::new()
        ));
        {
            let connection = rusqlite::Connection::open(&path).expect("legacy db opens");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE workflow_state (
                      workflow_id TEXT PRIMARY KEY NOT NULL,
                      workflow_name TEXT NOT NULL,
                      current_state TEXT NOT NULL,
                      context_json TEXT NOT NULL
                    );

                    CREATE TABLE workflow_events (
                      seq INTEGER PRIMARY KEY AUTOINCREMENT,
                      workflow_id TEXT NOT NULL,
                      event_id TEXT UNIQUE NOT NULL,
                      status TEXT NOT NULL,
                      event_json TEXT NOT NULL
                    );

                    CREATE INDEX workflow_events_pending
                      ON workflow_events(workflow_id, status, seq);

                    CREATE TABLE workflow_log (
                      seq INTEGER PRIMARY KEY AUTOINCREMENT,
                      workflow_id TEXT NOT NULL,
                      record_json TEXT NOT NULL
                    );
                    "#,
                )
                .expect("legacy schema creates");
        }

        let store = WorkflowStore::open(&path).expect("store migrates");
        store
            .enqueue_event(&workflow_event(
                "FirstWorkflow",
                "shared_event_id",
                "tick",
                json!({}),
            ))
            .expect("first event enqueues after migration");
        store
            .enqueue_event(&workflow_event(
                "SecondWorkflow",
                "shared_event_id",
                "tick",
                json!({}),
            ))
            .expect("second event enqueues after migration");

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_event_status_update_reports_missing_events() {
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let missing = workflow_event("MissingWorkflow", "missing_event", "tick", json!({}));

        let error = store
            .update_event_status(&missing, EventStatus::Processed)
            .expect_err("missing event update fails");

        assert!(matches!(
            error,
            StorageError::EventNotFound {
                workflow_id,
                event_id,
            } if workflow_id == "MissingWorkflow" && event_id == "missing_event"
        ));
    }

    #[test]
    fn sqlite_rejects_newer_schema_versions() {
        let path = std::env::temp_dir().join(format!(
            "armature-newer-schema-{}.sqlite",
            ulid::Ulid::new()
        ));
        {
            let connection = rusqlite::Connection::open(&path).expect("db opens");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE armature_meta (
                      key TEXT PRIMARY KEY NOT NULL,
                      value TEXT NOT NULL
                    );
                    INSERT INTO armature_meta (key, value)
                    VALUES ('schema_version', '999');
                    "#,
                )
                .expect("newer schema marker writes");
        }

        let error = match WorkflowStore::open(&path) {
            Ok(_) => panic!("newer schema should be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StorageError::UnsupportedSchemaVersion {
                found: 999,
                supported: 4
            }
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn runtime_processes_queued_event_and_persists_projection() {
        let source = include_str!("../../../examples/workflows/minimal.armature");
        let ir = armature_workflow::parse_source(source).expect("minimal source parses");
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");

        runtime
            .enqueue_event(&event("start", json!({"message": "persisted transition"})))
            .expect("event enqueues");

        let outcome = runtime
            .process_next_event()
            .expect("event processes")
            .expect("event was queued");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(runtime.status(0).current_state, "complete");
        assert_eq!(
            runtime
                .store()
                .load_state("Minimal")
                .expect("state loads")
                .expect("state exists")
                .current_state,
            "complete"
        );
        assert!(matches!(
            runtime
                .store()
                .log_records("Minimal")
                .expect("logs load")
                .as_slice(),
            [WorkflowLogRecord::Transition { .. }]
        ));
    }

    #[test]
    fn runtime_requeues_processing_event_on_startup() {
        let source = include_str!("../../../examples/workflows/minimal.armature");
        let ir = armature_workflow::parse_source(source).expect("minimal source parses");
        let store = WorkflowStore::open_in_memory().expect("store opens");
        store
            .enqueue_event(&event("start", json!({"message": "retry after restart"})))
            .expect("event enqueues");
        let stranded = store
            .dequeue_next_event("Minimal")
            .expect("event dequeues")
            .expect("event exists");
        assert_eq!(stranded.status, EventStatus::Processing);

        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime recovers");
        let outcome = runtime
            .process_next_event()
            .expect("recovered event processes")
            .expect("event was requeued");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.to_state, "complete");
        let events = runtime.store().events("Minimal", 10).expect("events load");
        assert_eq!(events[0].status, EventStatus::Processed);
        assert_eq!(events[0].attempt_count, 2);
    }

    #[test]
    fn runtime_persists_ignored_event_reason_on_event_record() {
        let source = r#"
machine GuardedRuntime
initial waiting

event go {
  count int
}

state waiting {
  on go as evt
    guard evt.count > 1
  {
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");

        runtime
            .enqueue_event(&workflow_event(
                "GuardedRuntime",
                "evt_ignored",
                "go",
                json!({"count": 1}),
            ))
            .expect("event enqueues");

        let outcome = runtime
            .process_next_event()
            .expect("ignored event processes")
            .expect("event was queued");

        assert_eq!(outcome.status, EventProcessingStatus::Ignored);
        let events = runtime
            .store()
            .events_by_status("GuardedRuntime", EventStatus::Ignored, 10)
            .expect("ignored events load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, EventStatus::Ignored);
        assert!(events[0]
            .last_error
            .as_deref()
            .is_some_and(|reason| reason.contains("no matching handler")));
    }

    #[test]
    fn runtime_marks_invariant_failures_without_persisting_mutated_state() {
        let source = r#"
machine RuntimeInvariant
initial waiting

data {
  count int = 0
}

event go {}

state waiting {
  on go {
    assign data.count = 2
    stay
  }
}

invariant countAtMostOne {
  assert data.count <= 1
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");

        runtime
            .enqueue_event(&workflow_event(
                "RuntimeInvariant",
                "evt_go",
                "go",
                json!({}),
            ))
            .expect("event enqueues");

        let error = runtime
            .process_next_event()
            .expect_err("invariant violation fails event");

        assert!(error.to_string().contains("invariant `countAtMostOne`"));
        let state = runtime
            .store()
            .load_state("RuntimeInvariant")
            .expect("state loads")
            .expect("state exists");
        assert_eq!(state.current_state, "waiting");
        assert_eq!(state.context.get("count"), Some(&json!(0)));

        let events = runtime
            .store()
            .events("RuntimeInvariant", 10)
            .expect("events load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, EventStatus::Failed);
        assert!(events[0]
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("invariant `countAtMostOne`")));

        assert!(matches!(
            runtime
                .store()
                .log_records("RuntimeInvariant")
                .expect("logs load")
                .as_slice(),
            [WorkflowLogRecord::Diagnostic { message }]
                if message.contains("invariant `countAtMostOne`")
        ));
    }

    #[test]
    fn runtime_enqueues_raised_events_after_transition() {
        let source = r#"
machine RaiseRuntime
initial waiting

event go {
  message string
}

event followUp {
  message string
}

state waiting {
  on go as evt {
    raise followUp {
      message evt.message
    }
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event(
                "RaiseRuntime",
                "evt_go",
                "go",
                json!({"message": "continue"}),
            ))
            .expect("event enqueues");

        let outcome = runtime
            .process_next_event()
            .expect("event processes")
            .expect("event was queued");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.to_state, "done");
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(outcome.effects[0].effect, "raise");
        assert_eq!(
            runtime.store().pending_event_count("RaiseRuntime").unwrap(),
            1
        );
        let queued = runtime
            .store()
            .dequeue_next_event("RaiseRuntime")
            .expect("raised event dequeues")
            .expect("raised event exists");
        assert_eq!(queued.event_type, "followUp");
        assert_eq!(queued.payload, json!({"message": "continue"}));
        assert_eq!(queued.causation_id.as_deref(), Some("evt_go"));
    }

    #[test]
    fn runtime_records_malformed_raise_as_failed_effect() {
        let source = r#"
machine RaiseRuntime
initial waiting

event go {}

event followUp {}

state waiting {
  on go {
    raise followUp {}
    goto done
  }
}

state done {
  final
}
"#;
        let mut ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        ir.statechart
            .states
            .get_mut("waiting")
            .expect("waiting state")
            .on
            .get_mut(0)
            .expect("go handler")
            .steps
            .get_mut(0)
            .expect("raise step")
            .args
            .remove("event");

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event("RaiseRuntime", "evt_go", "go", json!({})))
            .expect("event enqueues");

        let outcome = runtime
            .process_next_event()
            .expect("malformed raise is recorded as failed effect")
            .expect("event was queued");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.to_state, "done");
        assert_eq!(
            runtime.store().pending_event_count("RaiseRuntime").unwrap(),
            0
        );
        assert!(matches!(
            runtime
                .store()
                .log_records("RaiseRuntime")
                .expect("logs load")
                .as_slice(),
            [
                WorkflowLogRecord::Transition { .. },
                WorkflowLogRecord::Effect { status: EffectStatus::Intended, .. },
                WorkflowLogRecord::Effect {
                    status: EffectStatus::Failed,
                    outcome: Some(outcome),
                    ..
                },
            ] if outcome.error.as_deref().is_some_and(|error| error.contains("raised event effect is malformed"))
        ));
    }

    #[test]
    fn runtime_records_raise_enqueue_failure_as_failed_effect() {
        let source = r#"
machine RaiseRuntime
initial waiting

event go {}

event followUp {}

state waiting {
  on go {
    raise followUp {}
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        let store = WorkflowStore::open_in_memory().expect("store opens");
        store
            .connection()
            .execute_batch(
                r#"
                CREATE TRIGGER block_raised_event_insert
                BEFORE INSERT ON workflow_events
                WHEN NEW.event_id LIKE 'evt_raise_%'
                BEGIN
                  SELECT RAISE(ABORT, 'blocked raised event insert');
                END;
                "#,
            )
            .expect("trigger creates");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event("RaiseRuntime", "evt_go", "go", json!({})))
            .expect("event enqueues");

        let outcome = runtime
            .process_next_event()
            .expect("raise enqueue failure is recorded as failed effect")
            .expect("event was queued");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.to_state, "done");
        assert_eq!(
            runtime.store().pending_event_count("RaiseRuntime").unwrap(),
            0
        );
        assert!(matches!(
            runtime
                .store()
                .log_records("RaiseRuntime")
                .expect("logs load")
                .as_slice(),
            [
                WorkflowLogRecord::Transition { .. },
                WorkflowLogRecord::Effect { status: EffectStatus::Intended, .. },
                WorkflowLogRecord::Effect {
                    status: EffectStatus::Failed,
                    outcome: Some(outcome),
                    ..
                },
            ] if outcome.error.as_deref().is_some_and(|error| error.contains("failed to enqueue raised event"))
        ));
    }

    #[test]
    fn runtime_resumes_existing_state_instead_of_resetting() {
        let source = include_str!("../../../examples/workflows/minimal.armature");
        let ir = armature_workflow::parse_source(source).expect("minimal source parses");
        let store = WorkflowStore::open_in_memory().expect("store opens");
        store
            .save_state(&WorkflowState {
                workflow_name: "Minimal".to_string(),
                current_state: "complete".to_string(),
                context: BTreeMap::new(),
            })
            .expect("existing state saves");

        let runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");

        assert_eq!(runtime.status(0).current_state, "complete");
    }

    #[test]
    fn initial_state_name_descends_to_nested_initial_leaf() {
        let source = include_str!("../../../examples/workflows/spec-implementation.armature");
        let ir = armature_workflow::parse_source(source).expect("source parses");

        assert_eq!(crate::initial_state_name(&ir), "watching");
    }

    #[test]
    fn runtime_records_fake_effect_dispatches() {
        let source = r#"
machine Effects
initial waiting

agent director = thread("director")
agent worker = adapter("worker")
capability plan = adapter("implementationPlan")

event go {
  message string
}

state waiting {
  on go as evt {
    plan.markDone(evt.message)
    start worker {
      task evt.message
    }
    send director evt.message
    askHuman(evt.message)
    goto done
  }
}

state done {
  final
}
"#;
        let mut ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        ir.agents.get_mut("worker").expect("worker").max_active = Some(2);

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event(
                "Effects",
                "evt_effects",
                "go",
                json!({"message": "review this"}),
            ))
            .expect("event enqueues");

        let outcome = runtime
            .process_next_event()
            .expect("event processes")
            .expect("event was queued");

        assert_eq!(outcome.effects.len(), 4);
        assert_eq!(outcome.effects[0].effect, "plan.markDone");
        assert_eq!(outcome.effects[0].args["call_args"], json!(["review this"]));
        assert_eq!(outcome.effects[1].effect, "start");
        assert_eq!(outcome.effects[1].target.as_deref(), Some("worker"));
        assert_eq!(
            outcome.effects[1].args["input"],
            json!({"task": "review this"})
        );
        assert_eq!(outcome.effects[2].effect, "send");
        assert_eq!(outcome.effects[2].target.as_deref(), Some("director"));
        assert_eq!(outcome.effects[2].args["message"], json!("review this"));
        assert_eq!(outcome.effects[3].effect, "askHuman");
        assert_eq!(outcome.effects[3].args["reason"], json!("review this"));

        let records = runtime.store().log_records("Effects").expect("logs load");
        assert_eq!(records.len(), 9);
        let status = runtime.projected_status().expect("status projects");
        assert_eq!(status.active_invocations.len(), 1);
        assert_eq!(status.active_invocations[0].agent, "worker");
        assert_eq!(status.active_invocations[0].count, 1);
        assert_eq!(status.active_invocations[0].max, Some(2));
        assert_eq!(status.recent_effects[0].effect, "askHuman");
        assert!(status.recent_effects[0]
            .idempotency_key
            .as_deref()
            .is_some_and(|key| key.contains("Effects:evt_effects:")));
        assert_eq!(
            status.recent_effects[0].args["reason"],
            json!("review this")
        );
        assert!(matches!(records[0], WorkflowLogRecord::Transition { .. }));
        let mut intended_effect_ids = BTreeSet::new();
        let mut terminal_effect_ids = BTreeSet::new();
        for record in records.iter().skip(1) {
            let WorkflowLogRecord::Effect {
                effect_id, status, ..
            } = record
            else {
                panic!("expected effect record after transition");
            };
            match status {
                EffectStatus::Intended => {
                    intended_effect_ids.insert(effect_id.clone());
                }
                EffectStatus::Succeeded => {
                    assert!(
                        intended_effect_ids.contains(effect_id),
                        "effect outcome was recorded before intended effect"
                    );
                    terminal_effect_ids.insert(effect_id.clone());
                }
                EffectStatus::Dispatched => {
                    assert!(
                        intended_effect_ids.contains(effect_id),
                        "effect outcome was recorded before intended effect"
                    );
                    terminal_effect_ids.insert(effect_id.clone());
                }
                EffectStatus::Failed => {
                    panic!("fake/native dispatcher should succeed, got {status:?}");
                }
            }
        }
        assert_eq!(intended_effect_ids.len(), 4);
        assert_eq!(terminal_effect_ids, intended_effect_ids);
        assert!(records.iter().any(|record| matches!(
            record,
            WorkflowLogRecord::Effect {
                effect,
                category: super::effects::EffectCategory::AsyncInvocation,
                target: Some(target),
                idempotency_key: Some(idempotency_key),
                args,
                status: EffectStatus::Intended,
                ..
            } if effect == "start"
                && target == "worker"
                && idempotency_key.contains("Effects:evt_effects:")
                && args["input"] == json!({"task": "review this"})
        )));
    }

    struct RejectingDispatcher;

    impl super::effects::EffectDispatcher for RejectingDispatcher {
        fn dispatch(
            &mut self,
            request: super::effects::EffectRequest,
        ) -> Result<super::effects::EffectOutcome, super::effects::EffectError> {
            Err(super::effects::EffectError::Unsupported(format!(
                "blocked {}",
                request.effect
            )))
        }
    }

    struct FailedOutcomeDispatcher;

    impl super::effects::EffectDispatcher for FailedOutcomeDispatcher {
        fn dispatch(
            &mut self,
            request: super::effects::EffectRequest,
        ) -> Result<super::effects::EffectOutcome, super::effects::EffectError> {
            Ok(super::effects::EffectOutcome {
                effect_id: request.effect_id,
                status: super::effects::EffectOutcomeStatus::Failed,
                accepted: false,
                invocation_id: None,
                required_capabilities: request.required_capabilities,
                output: None,
                error: Some(format!("adapter reported failed {}", request.effect)),
                completed_at: None,
            })
        }
    }

    struct AcceptedStartDispatcher;

    impl super::effects::EffectDispatcher for AcceptedStartDispatcher {
        fn dispatch(
            &mut self,
            request: super::effects::EffectRequest,
        ) -> Result<super::effects::EffectOutcome, super::effects::EffectError> {
            Ok(super::effects::EffectOutcome {
                effect_id: request.effect_id,
                status: super::effects::EffectOutcomeStatus::Accepted,
                accepted: true,
                invocation_id: Some(request.idempotency_key),
                required_capabilities: request.required_capabilities,
                output: None,
                error: None,
                completed_at: None,
            })
        }
    }

    struct PolicyDenyingDispatcher;

    impl super::effects::EffectDispatcher for PolicyDenyingDispatcher {
        fn dispatch(
            &mut self,
            _request: super::effects::EffectRequest,
        ) -> Result<super::effects::EffectOutcome, super::effects::EffectError> {
            Err(super::effects::EffectError::CapabilityDenied {
                message: "effect requires denied capability `message_agents`".to_string(),
                required_capabilities: vec!["message_agents".to_string()],
            })
        }
    }

    #[test]
    fn runtime_uses_injected_effect_dispatcher() {
        let source = r#"
machine InjectedDispatcher
initial waiting

agent worker = adapter("worker")

event go {
  message string
}

state waiting {
  on go as evt {
    start worker {
      task evt.message
    }
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime =
            WorkflowRuntime::with_dispatcher(ir, store, Box::new(RejectingDispatcher))
                .expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event(
                "InjectedDispatcher",
                "evt_dispatcher",
                "go",
                json!({"message": "review this"}),
            ))
            .expect("event enqueues");

        runtime
            .process_next_event()
            .expect("event processes")
            .expect("event was queued");

        let records = runtime
            .store()
            .log_records("InjectedDispatcher")
            .expect("logs load");
        assert!(records.iter().any(|record| {
            matches!(
                record,
                WorkflowLogRecord::Effect {
                    effect,
                    status: EffectStatus::Failed,
                    outcome: Some(outcome),
                    ..
                } if effect == "start"
                    && outcome.error.as_deref().is_some_and(|error| error.contains("blocked start"))
            )
        }));
    }

    #[test]
    fn runtime_maps_failed_dispatch_outcomes_to_failed_effect_logs() {
        let source = r#"
machine FailedOutcome
initial waiting

agent director = adapter("director")

event go {
  message string
}

state waiting {
  on go as evt {
    send director evt.message
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime =
            WorkflowRuntime::with_dispatcher(ir, store, Box::new(FailedOutcomeDispatcher))
                .expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event(
                "FailedOutcome",
                "evt_failed_outcome",
                "go",
                json!({"message": "hello"}),
            ))
            .expect("event enqueues");

        let outcome = runtime
            .process_next_event()
            .expect("event processes")
            .expect("event was queued");
        assert_eq!(outcome.status, EventProcessingStatus::Processed);

        let status = runtime.projected_status().expect("status projects");
        assert_eq!(status.recent_effects[0].effect, "send");
        assert_eq!(status.recent_effects[0].status, EffectStatus::Failed);
        assert!(status.recent_effects[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("adapter reported failed send")));
        assert!(status
            .recent_failures
            .iter()
            .any(|failure| failure.contains("send failed: adapter reported failed send")));
    }

    #[test]
    fn accepted_start_outcomes_are_dispatched_and_count_active() {
        let source = r#"
machine AcceptedStart
initial waiting

agent worker = codingAgent() {
  maxActive 1
}

event go {
  message string
}

event finished {
  name string
}

state waiting {
  on go as evt {
    start worker {
      task evt.message
    }
    stay
  }

  on finished as run {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime =
            WorkflowRuntime::with_dispatcher(ir, store, Box::new(AcceptedStartDispatcher))
                .expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event(
                "AcceptedStart",
                "evt_start",
                "go",
                json!({"message": "hello"}),
            ))
            .expect("event enqueues");

        runtime
            .process_next_event()
            .expect("event processes")
            .expect("event was queued");

        let status = runtime.projected_status().expect("status projects");
        assert_eq!(status.recent_effects[0].effect, "start");
        assert_eq!(status.recent_effects[0].status, EffectStatus::Dispatched);
        assert_eq!(status.active_invocations.len(), 1);
        assert_eq!(status.active_invocations[0].agent, "worker");
        assert_eq!(status.active_invocations[0].count, 1);
    }

    #[test]
    fn active_projection_uses_latest_start_outcome_per_effect() {
        let source = r#"
machine ActiveProjectionLatest
initial waiting

agent worker = adapter("worker")

event go {}

state waiting {
  on go {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let state = WorkflowState {
            workflow_name: "ActiveProjectionLatest".to_string(),
            current_state: "waiting".to_string(),
            context: BTreeMap::new(),
        };
        store.save_state(&state).expect("state saves");

        let start_record =
            |status, outcome_status, accepted, error: Option<String>| WorkflowLogRecord::Effect {
                effect_id: "effect_start_worker".to_string(),
                workflow_id: "ActiveProjectionLatest".to_string(),
                transition_id: "transition_1".to_string(),
                idempotency_key: Some(
                    "ActiveProjectionLatest:evt_go:effect_start_worker".to_string(),
                ),
                effect: "start".to_string(),
                category: super::effects::EffectCategory::AsyncInvocation,
                target: Some("worker".to_string()),
                args: json!({}),
                required_capabilities: Vec::new(),
                status,
                outcome: Some(super::effects::EffectOutcome {
                    effect_id: "effect_start_worker".to_string(),
                    status: outcome_status,
                    accepted,
                    invocation_id: Some("worker-1".to_string()),
                    required_capabilities: Vec::new(),
                    output: None,
                    error,
                    completed_at: None,
                }),
            };
        let started = start_record(
            EffectStatus::Dispatched,
            super::effects::EffectOutcomeStatus::Accepted,
            true,
            None,
        );
        let completed = start_record(
            EffectStatus::Succeeded,
            super::effects::EffectOutcomeStatus::Succeeded,
            true,
            None,
        );

        store
            .append_log("ActiveProjectionLatest", &started)
            .expect("accepted outcome appends");
        store
            .append_log("ActiveProjectionLatest", &completed)
            .expect("succeeded outcome appends");

        let status =
            super::project_status(&ir, &store, "ActiveProjectionLatest").expect("status projects");
        assert_eq!(status.active_invocations.len(), 1);
        assert_eq!(status.active_invocations[0].agent, "worker");
        assert_eq!(status.active_invocations[0].count, 1);

        let failed = start_record(
            EffectStatus::Failed,
            super::effects::EffectOutcomeStatus::Failed,
            false,
            Some("adapter rejected after reconciliation".to_string()),
        );
        store
            .append_log("ActiveProjectionLatest", &failed)
            .expect("failed outcome appends");

        let status =
            super::project_status(&ir, &store, "ActiveProjectionLatest").expect("status projects");
        assert!(status.active_invocations.is_empty());
    }

    #[test]
    fn runtime_preserves_dispatch_error_required_capabilities() {
        let source = r#"
machine DispatchPolicy
initial waiting

agent worker = adapter("worker")

event go {
  message string
}

state waiting {
  on go as evt {
    start worker {
      task evt.message
    }
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime =
            WorkflowRuntime::with_dispatcher(ir, store, Box::new(PolicyDenyingDispatcher))
                .expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event(
                "DispatchPolicy",
                "evt_policy",
                "go",
                json!({"message": "review this"}),
            ))
            .expect("event enqueues");

        runtime
            .process_next_event()
            .expect("event processes")
            .expect("event was queued");

        let records = runtime
            .store()
            .log_records("DispatchPolicy")
            .expect("logs load");
        assert!(records.iter().any(|record| {
            matches!(
                record,
                WorkflowLogRecord::Effect {
                    effect,
                    status: EffectStatus::Failed,
                    outcome: Some(outcome),
                    ..
                } if effect == "start"
                    && outcome.required_capabilities == vec!["message_agents".to_string()]
            )
        }));

        let status = runtime.projected_status().expect("status projects");
        assert_eq!(
            status.policy_blockers,
            vec![
                "start requires message_agents: effect requires denied capability `message_agents`"
                    .to_string()
            ]
        );
        assert!(status
            .recent_failures
            .iter()
            .any(|failure| failure.contains("denied capability `message_agents`")));
    }

    #[test]
    fn projected_status_subtracts_processed_finished_events_from_active_invocations() {
        let source = r#"
machine ActiveProjection
initial waiting

agent worker = codingAgent()

event go {
  message string
}

event finished {
  name string
}

state waiting {
  on go as evt {
    start worker {
      task evt.message
    }
    stay
  }

  on finished as run {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event(
                "ActiveProjection",
                "evt_go",
                "go",
                json!({"message": "do work"}),
            ))
            .expect("event enqueues");
        runtime
            .process_next_event()
            .expect("go processes")
            .expect("go was queued");

        let status = runtime.projected_status().expect("status projects");
        assert_eq!(status.active_invocations.len(), 1);
        assert_eq!(status.active_invocations[0].agent, "worker");
        assert_eq!(status.active_invocations[0].count, 1);

        runtime
            .enqueue_event(&workflow_event(
                "ActiveProjection",
                "evt_finished",
                "finished",
                json!({"name": "worker-1"}),
            ))
            .expect("event enqueues");
        runtime
            .process_next_event()
            .expect("finished processes")
            .expect("finished was queued");

        let status = runtime.projected_status().expect("status projects");
        assert!(status.active_invocations.is_empty());
    }

    #[test]
    fn finished_event_matching_allows_hyphenated_agent_names() {
        let source = r#"
machine HyphenatedAgent
initial waiting

agent worker-team = codingAgent()
agent worker = codingAgent()

event go {
  message string
}

event finished {
  name string
}

state waiting {
  on go as evt {
    start worker-team {
      task evt.message
    }
    stay
  }

  on finished as run {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");
        runtime
            .enqueue_event(&workflow_event(
                "HyphenatedAgent",
                "evt_go",
                "go",
                json!({"message": "do work"}),
            ))
            .expect("event enqueues");
        runtime
            .process_next_event()
            .expect("go processes")
            .expect("go was queued");

        let status = runtime.projected_status().expect("status projects");
        assert_eq!(status.active_invocations.len(), 1);
        assert_eq!(status.active_invocations[0].agent, "worker-team");

        runtime
            .enqueue_event(&workflow_event(
                "HyphenatedAgent",
                "evt_finished",
                "finished",
                json!({"name": "worker-team-1"}),
            ))
            .expect("event enqueues");
        runtime
            .process_next_event()
            .expect("finished processes")
            .expect("finished was queued");

        let status = runtime.projected_status().expect("status projects");
        assert!(status.active_invocations.is_empty());
    }

    #[test]
    fn runtime_blocks_start_effects_when_agent_is_at_max_active() {
        let source = r#"
machine ActiveLimit
initial waiting

agent worker = codingAgent() {
  maxActive 1
}

event go {
  message string
}

event finished {
  name string
}

state waiting {
  on go as evt {
    start worker {
      task evt.message
    }
    stay
  }

  on finished as run {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let store = WorkflowStore::open_in_memory().expect("store opens");
        let mut runtime = WorkflowRuntime::new(ir, store).expect("runtime initializes");

        for (event_id, message) in [("evt_go_1", "first"), ("evt_go_2", "second")] {
            runtime
                .enqueue_event(&workflow_event(
                    "ActiveLimit",
                    event_id,
                    "go",
                    json!({ "message": message }),
                ))
                .expect("event enqueues");
            runtime
                .process_next_event()
                .expect("event processes")
                .expect("event was queued");
        }

        let status = runtime.projected_status().expect("status projects");
        assert_eq!(status.active_invocations.len(), 1);
        assert_eq!(status.active_invocations[0].agent, "worker");
        assert_eq!(status.active_invocations[0].count, 1);
        assert_eq!(status.active_invocations[0].max, Some(1));
        assert!(status.current_blockers.is_empty());
        assert!(status
            .current_effect_failures
            .iter()
            .any(|failure| failure.contains("start failed: agent `worker` is at maxActive 1")));

        let records = runtime
            .store()
            .log_records("ActiveLimit")
            .expect("logs load");
        assert!(records.iter().any(|record| {
            matches!(
                record,
                WorkflowLogRecord::Effect {
                    effect,
                    status: EffectStatus::Failed,
                    outcome: Some(outcome),
                    ..
                } if effect == "start"
                    && outcome.error.as_deref().is_some_and(|error| error.contains("maxActive 1"))
            )
        }));
        assert!(records.iter().any(|record| {
            matches!(
                record,
                WorkflowLogRecord::Effect {
                    effect,
                    status: EffectStatus::Failed,
                    outcome: Some(outcome),
                    ..
                } if effect == "start"
                    && outcome.error.as_deref().is_some_and(|error| !error.contains("not supported by this adapter"))
            )
        }));
    }

    #[test]
    fn executes_case_arm_effects_and_transition() {
        let source = r#"
machine CaseRuntime
initial waiting

agent director = thread("director")

event run {
  name string
}

state waiting {
  on run as evt {
    case evt.name {
      matches "worker-*" -> {
        send director "worker completed"
        goto done
      }

      _ -> {
        stay
      }
    }
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut interpreter = Interpreter::new(ir);
        let outcome = interpreter
            .process_event(&workflow_event(
                "CaseRuntime",
                "evt_case",
                "run",
                json!({"name": "worker-1"}),
            ))
            .expect("case event processes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.to_state, "done");
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(outcome.effects[0].effect, "send");
        assert_eq!(outcome.effects[0].target.as_deref(), Some("director"));
    }

    #[test]
    fn descends_initial_children_and_uses_parent_handlers() {
        let source = r#"
machine Nested
initial running

event tick {}
event reset {}

state running {
  initial watching

  on tick {
    goto done
  }

  state watching {}
}

state done {
  on reset {
    goto running
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut interpreter = Interpreter::new(ir);
        assert_eq!(interpreter.status(0).current_state, "watching");

        let tick = interpreter
            .process_event(&workflow_event("Nested", "evt_tick", "tick", json!({})))
            .expect("parent handler processes from child");
        assert_eq!(tick.status, EventProcessingStatus::Processed);
        assert_eq!(tick.from_state, "watching");
        assert_eq!(tick.to_state, "done");

        let reset = interpreter
            .process_event(&workflow_event("Nested", "evt_reset", "reset", json!({})))
            .expect("compound transition descends to child");
        assert_eq!(reset.status, EventProcessingStatus::Processed);
        assert_eq!(reset.from_state, "done");
        assert_eq!(reset.to_state, "watching");
    }

    #[test]
    fn executes_entry_actions_after_transition() {
        let source = r#"
machine EntryActions
initial waiting

agent director = thread("director")

event go {}

state waiting {
  on go {
    goto choosing
  }
}

state choosing {
  entry {
    send director "entered choosing"
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut interpreter = Interpreter::new(ir);
        let outcome = interpreter
            .process_event(&workflow_event("EntryActions", "evt_go", "go", json!({})))
            .expect("entry action runs");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.from_state, "waiting");
        assert_eq!(outcome.to_state, "done");
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(outcome.effects[0].effect, "send");
        assert_eq!(outcome.effects[0].target.as_deref(), Some("director"));
    }

    #[test]
    fn executes_always_transition_after_handled_event() {
        let source = r#"
machine AlwaysRuntime
initial waiting

data {
  ready bool
}

event go {
  ready bool
}

state waiting {
  on go as evt {
    assign data.ready = evt.ready
    stay
  }

  always
    guard data.ready == true
  {
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);
        let mut interpreter = Interpreter::new(ir);
        let outcome = interpreter
            .process_event(&workflow_event(
                "AlwaysRuntime",
                "evt_go",
                "go",
                json!({"ready": true}),
            ))
            .expect("always transition executes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.from_state, "waiting");
        assert_eq!(outcome.to_state, "done");
        assert_eq!(interpreter.context().get("ready"), Some(&json!(true)));
    }

    #[test]
    fn evaluates_let_bindings_in_later_steps() {
        let source = r#"
machine Locals
initial waiting

agent worker = codingAgent()

event go {
  id string
}

state waiting {
  on go as evt {
    let next = {
      workItemId evt.id
      message "ship"
    }

    case next.message {
      ship -> {
        start worker {
          task next.workItemId
          message next.message
        }
        goto done
      }
    }
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut interpreter = Interpreter::new(ir);
        let outcome = interpreter
            .process_event(&workflow_event(
                "Locals",
                "evt_go",
                "go",
                json!({"id": "W1"}),
            ))
            .expect("local binding evaluates");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.to_state, "done");
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(
            outcome.effects[0].args["input"],
            json!({"task": "W1", "message": "ship"})
        );
    }

    #[test]
    fn evaluates_fake_coerce_outputs() {
        let source = r#"
machine FakeCoerce
initial waiting

agent quality = codingAgent()

event finished {
  id string
  name string
}

class RunSummary {
  id string
  name string
}

class RunClassification {
  kind string
  workItemId string
}

coerce classifyRun(run RunSummary) -> RunClassification {
  model "fake"
  prompt """
  classify
  """
}

state waiting {
  on finished as run {
    let classification = coerce classifyRun({
      id run.id
      name run.name
    })

    case classification.kind {
      WorkerComplete -> {
        start quality {
          task classification.workItemId
        }
        goto done
      }
    }
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut fake_outputs = BTreeMap::new();
        fake_outputs.insert(
            "classifyRun".to_string(),
            json!({"kind": "WorkerComplete", "workItemId": "W1"}),
        );
        let mut interpreter = Interpreter::new(ir).with_fake_coerce_outputs(fake_outputs);
        let outcome = interpreter
            .process_event(&workflow_event(
                "FakeCoerce",
                "evt_finished",
                "finished",
                json!({"id": "run-1", "name": "worker-1"}),
            ))
            .expect("fake coerce output evaluates");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.to_state, "done");
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(outcome.effects[0].effect, "start");
        assert_eq!(outcome.effects[0].target.as_deref(), Some("quality"));
        assert_eq!(outcome.effects[0].args["input"], json!({"task": "W1"}));
    }

    #[test]
    fn rejects_fake_coerce_outputs_that_do_not_match_schema() {
        let source = r#"
machine BadFakeCoerce
initial waiting

event go {
  message string
}

class Classification {
  kind string
}

coerce classify(message string) -> Classification {
  prompt """
  classify
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.message)
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut fake_outputs = BTreeMap::new();
        fake_outputs.insert("classify".to_string(), json!({"kind": 42}));
        let mut interpreter = Interpreter::new(ir).with_fake_coerce_outputs(fake_outputs);
        let error = interpreter
            .process_event(&workflow_event(
                "BadFakeCoerce",
                "evt_go",
                "go",
                json!({"message": "hello"}),
            ))
            .expect_err("invalid fake coerce output is rejected");

        assert!(matches!(
            error,
            InterpreterError::InvalidCoerceOutput { function_name, reason }
                if function_name == "classify" && reason.contains("$.kind expected string, got int")
        ));
    }

    #[test]
    fn rejects_coerce_inputs_that_do_not_match_schema() {
        let source = r#"
machine BadCoerceInput
initial waiting

event go {
  count json
}

class Classification {
  kind string
}

coerce classify(message string) -> Classification {
  prompt """
  classify
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.count)
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut fake_outputs = BTreeMap::new();
        fake_outputs.insert("classify".to_string(), json!({"kind": "ok"}));
        let mut interpreter = Interpreter::new(ir).with_fake_coerce_outputs(fake_outputs);
        let error = interpreter
            .process_event(&workflow_event(
                "BadCoerceInput",
                "evt_go",
                "go",
                json!({"count": 42}),
            ))
            .expect_err("invalid coerce input is rejected");

        assert!(matches!(
            error,
            InterpreterError::InvalidCoerceInput {
                function_name,
                param_name,
                reason
            } if function_name == "classify"
                && param_name == "message"
                && reason.contains("$ expected string, got int")
        ));
    }

    #[test]
    fn spec_fixture_idle_path_can_choose_and_start_worker_with_fakes() {
        let source = include_str!("../../../examples/workflows/spec-implementation.armature");
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut fake_calls = BTreeMap::new();
        fake_calls.insert("plan.snapshot".to_string(), json!("plan text"));
        let mut fake_coerce = BTreeMap::new();
        fake_coerce.insert(
            "chooseNextStep".to_string(),
            json!({
                "action": "StartWorker",
                "workItemId": "W1",
                "message": "Implement W1",
                "reason": ""
            }),
        );

        let mut interpreter = Interpreter::new(ir)
            .with_fake_call_outputs(fake_calls)
            .with_fake_coerce_outputs(fake_coerce);
        assert_eq!(interpreter.status(0).current_state, "watching");

        let outcome = interpreter
            .process_event(&workflow_event(
                "specImplementation",
                "evt_idle",
                "idle",
                json!({"activeRuns": 0, "unfinishedItems": 1}),
            ))
            .expect("idle path executes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.from_state, "watching");
        assert_eq!(outcome.to_state, "watching");
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(outcome.effects[0].effect, "start");
        assert_eq!(outcome.effects[0].target.as_deref(), Some("worker"));
        assert_eq!(
            outcome.effects[0].args["input"],
            json!({"task": "W1", "message": "Implement W1"})
        );
        assert!(interpreter
            .context()
            .get("lastIdleNudgeAt")
            .and_then(|value| value.as_u64())
            .is_some());
    }

    #[test]
    fn spec_fixture_worker_complete_starts_quality_with_fakes() {
        let source = include_str!("../../../examples/workflows/spec-implementation.armature");
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut fake_coerce = BTreeMap::new();
        fake_coerce.insert(
            "classifyRun".to_string(),
            json!({
                "kind": "WorkerComplete",
                "workItemId": "W1",
                "reason": ""
            }),
        );

        let mut interpreter = Interpreter::new(ir).with_fake_coerce_outputs(fake_coerce);
        assert_eq!(interpreter.status(0).current_state, "watching");

        let outcome = interpreter
            .process_event(&workflow_event(
                "specImplementation",
                "evt_worker_done",
                "finished",
                json!({
                    "id": "run-worker-1",
                    "name": "worker-1",
                    "status": "succeeded",
                    "stdoutTail": "done",
                    "stderrTail": "",
                    "exitCode": 0
                }),
            ))
            .expect("worker completion path executes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.from_state, "watching");
        assert_eq!(outcome.to_state, "watching");
        assert_eq!(
            interpreter.context().get("seenRuns"),
            Some(&json!(["run-worker-1"]))
        );
        assert_eq!(outcome.effects.len(), 2);
        assert_eq!(outcome.effects[0].effect, "plan.markReadyForQuality");
        assert_eq!(outcome.effects[0].args["call_args"], json!(["W1"]));
        assert_eq!(outcome.effects[1].effect, "start");
        assert_eq!(outcome.effects[1].target.as_deref(), Some("quality"));
        assert_eq!(
            outcome.effects[1].args["input"],
            json!({"task": "W1", "message": "Review completed worker task."})
        );
    }

    #[test]
    fn spec_fixture_duplicate_finished_run_is_ignored() {
        let source = include_str!("../../../examples/workflows/spec-implementation.armature");
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut fake_coerce = BTreeMap::new();
        fake_coerce.insert(
            "classifyRun".to_string(),
            json!({
                "kind": "WorkerComplete",
                "workItemId": "W1",
                "reason": ""
            }),
        );

        let mut interpreter = Interpreter::new(ir).with_fake_coerce_outputs(fake_coerce);
        let first = interpreter
            .process_event(&workflow_event(
                "specImplementation",
                "evt_worker_first",
                "finished",
                json!({
                    "id": "run-worker-1",
                    "name": "worker-1",
                    "status": "succeeded",
                    "stdoutTail": "done",
                    "stderrTail": "",
                    "exitCode": 0
                }),
            ))
            .expect("first worker completion processes");
        assert_eq!(first.status, EventProcessingStatus::Processed);
        assert_eq!(
            interpreter.context().get("seenRuns"),
            Some(&json!(["run-worker-1"]))
        );

        let duplicate = interpreter
            .process_event(&workflow_event(
                "specImplementation",
                "evt_worker_duplicate",
                "finished",
                json!({
                    "id": "run-worker-1",
                    "name": "worker-1",
                    "status": "succeeded",
                    "stdoutTail": "done again",
                    "stderrTail": "",
                    "exitCode": 0
                }),
            ))
            .expect("duplicate event is ignored");

        assert_eq!(duplicate.status, EventProcessingStatus::Ignored);
        assert_eq!(duplicate.from_state, "watching");
        assert_eq!(duplicate.to_state, "watching");
        assert!(duplicate.effects.is_empty());
        assert_eq!(
            interpreter.context().get("seenRuns"),
            Some(&json!(["run-worker-1"]))
        );
    }

    #[test]
    fn spec_fixture_worker_failed_interpolates_director_message() {
        let source = include_str!("../../../examples/workflows/spec-implementation.armature");
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut fake_coerce = BTreeMap::new();
        fake_coerce.insert(
            "classifyRun".to_string(),
            json!({
                "kind": "WorkerFailed",
                "workItemId": "W1",
                "reason": "missing migration"
            }),
        );

        let mut interpreter = Interpreter::new(ir).with_fake_coerce_outputs(fake_coerce);
        let outcome = interpreter
            .process_event(&workflow_event(
                "specImplementation",
                "evt_worker_failed",
                "finished",
                json!({
                    "id": "run-worker-failed",
                    "name": "worker-1",
                    "status": "failed",
                    "stdoutTail": "",
                    "stderrTail": "missing migration",
                    "exitCode": 1
                }),
            ))
            .expect("worker failure path executes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.to_state, "watching");
        assert_eq!(
            interpreter.context().get("seenRuns"),
            Some(&json!(["run-worker-failed"]))
        );
        assert_eq!(outcome.effects.len(), 2);
        assert_eq!(outcome.effects[0].effect, "plan.markBlocked");
        assert_eq!(
            outcome.effects[0].args["call_args"],
            json!(["W1", "missing migration"])
        );
        assert_eq!(outcome.effects[1].effect, "send");
        assert_eq!(outcome.effects[1].target.as_deref(), Some("director"));
        assert!(outcome.effects[1].args["message"]
            .as_str()
            .is_some_and(|message| message.contains("Worker failed: missing migration")));
    }

    #[test]
    fn spec_fixture_quality_passed_can_finish_via_entry_decision() {
        let source = include_str!("../../../examples/workflows/spec-implementation.armature");
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut fake_calls = BTreeMap::new();
        fake_calls.insert("plan.snapshot".to_string(), json!("all work complete"));
        let mut fake_coerce = BTreeMap::new();
        fake_coerce.insert(
            "classifyRun".to_string(),
            json!({
                "kind": "QualityPassed",
                "workItemId": "W1",
                "reason": ""
            }),
        );
        fake_coerce.insert(
            "chooseNextStep".to_string(),
            json!({
                "action": "Done",
                "workItemId": null,
                "message": null,
                "reason": "complete"
            }),
        );

        let mut interpreter = Interpreter::new(ir)
            .with_fake_call_outputs(fake_calls)
            .with_fake_coerce_outputs(fake_coerce);
        assert_eq!(interpreter.status(0).current_state, "watching");

        let outcome = interpreter
            .process_event(&workflow_event(
                "specImplementation",
                "evt_quality_passed",
                "finished",
                json!({
                    "id": "run-quality-1",
                    "name": "quality-1",
                    "status": "succeeded",
                    "stdoutTail": "accepted",
                    "stderrTail": "",
                    "exitCode": 0
                }),
            ))
            .expect("quality completion path executes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.from_state, "watching");
        assert_eq!(outcome.to_state, "done");
        assert_eq!(
            interpreter.context().get("seenRuns"),
            Some(&json!(["run-quality-1"]))
        );
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(outcome.effects[0].effect, "plan.markDone");
        assert_eq!(outcome.effects[0].args["call_args"], json!(["W1"]));
    }

    #[test]
    fn spec_fixture_quality_failed_blocks_and_asks_human() {
        let source = include_str!("../../../examples/workflows/spec-implementation.armature");
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut fake_coerce = BTreeMap::new();
        fake_coerce.insert(
            "classifyRun".to_string(),
            json!({
                "kind": "QualityFailed",
                "workItemId": "W1",
                "reason": "review found missing tests"
            }),
        );

        let mut interpreter = Interpreter::new(ir).with_fake_coerce_outputs(fake_coerce);
        assert_eq!(interpreter.status(0).current_state, "watching");

        let outcome = interpreter
            .process_event(&workflow_event(
                "specImplementation",
                "evt_quality_failed",
                "finished",
                json!({
                    "id": "run-quality-failed",
                    "name": "quality-1",
                    "status": "failed",
                    "stdoutTail": "needs tests",
                    "stderrTail": "",
                    "exitCode": 1
                }),
            ))
            .expect("quality failure path executes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(outcome.from_state, "watching");
        assert_eq!(outcome.to_state, "watching");
        assert_eq!(
            interpreter.context().get("seenRuns"),
            Some(&json!(["run-quality-failed"]))
        );
        assert_eq!(outcome.effects.len(), 2);
        assert_eq!(outcome.effects[0].effect, "plan.markBlocked");
        assert_eq!(
            outcome.effects[0].args["call_args"],
            json!(["W1", "review found missing tests"])
        );
        assert_eq!(outcome.effects[1].effect, "askHuman");
        assert_eq!(
            outcome.effects[1].args["reason"],
            json!("review found missing tests")
        );
    }

    #[test]
    fn evaluates_builtin_calls_and_data_defaults() {
        let source = r#"
machine Builtins
initial waiting

data {
  seen string[] = []
  lastIdleNudgeAt time? = nil
}

event finished {
  id string
}

event idle {}

state waiting {
  on finished as run
    guard !(run.id in data.seen)
  {
    assign data.seen = data.seen.append(run.id)
  }

  on idle
    guard elapsedSince(data.lastIdleNudgeAt) >= 2m
  {
    assign data.lastIdleNudgeAt = now()
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut interpreter = Interpreter::new(ir);
        let finished = interpreter
            .process_event(&workflow_event(
                "Builtins",
                "evt_finished",
                "finished",
                json!({"id": "run-1"}),
            ))
            .expect("append call evaluates");
        assert_eq!(finished.status, EventProcessingStatus::Processed);
        assert_eq!(interpreter.context().get("seen"), Some(&json!(["run-1"])));

        let idle = interpreter
            .process_event(&workflow_event("Builtins", "evt_idle", "idle", json!({})))
            .expect("elapsedSince and now evaluate");
        assert_eq!(idle.status, EventProcessingStatus::Processed);
        assert_eq!(idle.to_state, "done");
        assert!(interpreter
            .context()
            .get("lastIdleNudgeAt")
            .and_then(|value| value.as_u64())
            .is_some());
    }

    #[test]
    fn evaluates_collection_map_and_text_helpers() {
        let source = r#"
machine Helpers
initial waiting

data {
  seen string[] = ["old"]
  names map<string, string> = {}
  first string? = nil
  found string? = nil
  hasRun bool = false
  count int = 0
}

event go {
  id string
  message string
}

state waiting {
  on go as evt
    guard text.contains(evt.message, "ready")
  {
    assign data.seen = list.append(data.seen, evt.id)
    assign data.seen = data.seen.remove("old")
    assign data.first = list.first(data.seen)
    assign data.names = map.set(data.names, evt.id, text.trim(evt.message))
    assign data.found = map.get(data.names, evt.id)
    assign data.hasRun = list.contains(data.seen, evt.id) && map.containsKey(data.names, evt.id) && text.startsWith(text.trim(evt.message), "ready")
    assign data.count = list.length(data.seen)
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut interpreter = Interpreter::new(ir);
        let outcome = interpreter
            .process_event(&workflow_event(
                "Helpers",
                "evt_go",
                "go",
                json!({"id": "run-1", "message": "  ready now  "}),
            ))
            .expect("helper workflow processes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(interpreter.context().get("seen"), Some(&json!(["run-1"])));
        assert_eq!(interpreter.context().get("first"), Some(&json!("run-1")));
        assert_eq!(
            interpreter.context().get("names"),
            Some(&json!({"run-1": "ready now"}))
        );
        assert_eq!(
            interpreter.context().get("found"),
            Some(&json!("ready now"))
        );
        assert_eq!(interpreter.context().get("hasRun"), Some(&json!(true)));
        assert_eq!(interpreter.context().get("count"), Some(&json!(1)));

        let status = interpreter.status(0);
        assert_eq!(status.data_summary.get("seen"), Some(&json!(1)));
        assert_eq!(
            status.data_summary.get("names"),
            Some(&json!({"fields": 1}))
        );
        assert_eq!(status.data_summary.get("first"), Some(&json!("run-1")));
    }

    #[test]
    fn initializes_context_from_data_initializers() {
        let source = r#"
machine Initializers
initial waiting

data {
  count int = 7
  labels string[] = ["ready"]
}

event go {}

state waiting {
  on go {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let interpreter = Interpreter::new(ir);

        assert_eq!(interpreter.context().get("count"), Some(&json!(7)));
        assert_eq!(interpreter.context().get("labels"), Some(&json!(["ready"])));
    }

    #[test]
    fn reads_and_writes_nested_data_paths() {
        let source = r#"
machine NestedData
initial waiting

class UserState {
  status string
}

data {
  user UserState = { status "todo" }
}

event go {}

state waiting {
  on go
    guard data.user.status == "todo"
  {
    assign data.user.status = "done"
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let report = armature_workflow::validate_ir(&ir);
        assert!(report.is_ok(), "{:#?}", report.diagnostics);

        let mut interpreter = Interpreter::new(ir);
        let outcome = interpreter
            .process_event(&workflow_event("NestedData", "evt_go", "go", json!({})))
            .expect("nested data path processes");

        assert_eq!(outcome.status, EventProcessingStatus::Processed);
        assert_eq!(
            interpreter.context().get("user"),
            Some(&json!({"status": "done"}))
        );
    }
}
