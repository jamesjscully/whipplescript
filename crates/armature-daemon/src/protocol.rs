use armature_core::{ProcessState, RunId, RunRecord};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonRequest {
    Inspect,
    Runs,
    StartTask { name: String },
    CancelRun { run_id: String },
    ServiceStart { name: String },
    ServiceStop { name: String },
    ServiceRestart { name: String },
    ReloadConfig,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResponse {
    pub config_version: String,
    pub socket_path: String,
    pub pid_path: String,
    pub services: Vec<RuntimeServiceStatus>,
    pub active_runs: Vec<RunRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeServiceStatus {
    pub name: String,
    pub configured_enabled: bool,
    pub stop_override: bool,
    pub state: ProcessState,
    pub supervision_state: String,
    pub active_run_id: Option<RunId>,
    pub last_error: Option<String>,
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
    Runs { runs: Vec<RunRecord> },
    StartedRun { run_id: RunId },
}
