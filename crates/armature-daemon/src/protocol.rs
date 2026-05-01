use armature_core::{EventRecord, ProcessState, RunId, RunRecord, TriggerRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonRequest {
    Inspect,
    Events,
    Triggers,
    Runs,
    StartTask {
        name: String,
        source_run_id: Option<RunId>,
        parent_event_id: Option<armature_core::EventId>,
        correlation_id: Option<String>,
    },
    EmitEvent {
        event_type: String,
        payload: Value,
        source: Option<String>,
        source_run_id: Option<RunId>,
        parent_event_id: Option<armature_core::EventId>,
        correlation_id: Option<String>,
    },
    CancelRun {
        run_id: String,
    },
    LockAcquire {
        name: String,
        ttl_ms: u64,
        owner_pid: u32,
        owner_id: String,
        reason: Option<String>,
    },
    LockRenew {
        name: String,
        token: String,
        ttl_ms: u64,
    },
    LockRelease {
        name: String,
        token: String,
    },
    LockStatus,
    ServiceStart {
        name: String,
    },
    ServiceStop {
        name: String,
    },
    ServiceRestart {
        name: String,
    },
    ReloadConfig,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResponse {
    pub config_version: String,
    pub socket_path: String,
    pub pid_path: String,
    pub services: Vec<RuntimeServiceStatus>,
    pub tasks: Vec<RuntimeTaskStatus>,
    pub active_runs: Vec<RunRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeServiceStatus {
    pub name: String,
    pub configured_enabled: bool,
    pub stop_override: bool,
    pub state: ProcessState,
    pub supervision_state: String,
    pub health: Option<RuntimeHealthStatus>,
    pub active_run_id: Option<RunId>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeHealthStatus {
    pub state: String,
    pub active_run_id: Option<RunId>,
    pub last_run_id: Option<RunId>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTaskStatus {
    pub name: String,
    pub admission: String,
    pub active_run_ids: Vec<RunId>,
    pub queued_triggers: usize,
    pub schedule_active: bool,
    pub watch_active: bool,
    pub event_trigger: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualLockRecord {
    pub name: String,
    pub owner_pid: u32,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default = "legacy_lock_token")]
    pub token: String,
    pub acquired_at_ms: i64,
    #[serde(default)]
    pub renewed_at_ms: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub manual: bool,
}

fn legacy_lock_token() -> String {
    "legacy".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DaemonResponse {
    Ok { payload: ResponsePayload },
    Error { kind: String, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponsePayload {
    Empty,
    Inspect(InspectResponse),
    Events { events: Vec<EventRecord> },
    Triggers { triggers: Vec<TriggerRecord> },
    Runs { runs: Vec<RunRecord> },
    StartedRun { run_id: RunId },
    LockAcquired { lock: ManualLockRecord },
    LockRenewed { lock: ManualLockRecord },
    Locks { locks: Vec<ManualLockRecord> },
}
