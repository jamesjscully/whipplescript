use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{EventId, RunId, TriggerId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionPolicy {
    Allow,
    Reject,
    Restart,
    QueueOne,
    QueueAll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRouting {
    Manual,
    Schedule,
    Watch,
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOrigin {
    Task,
    Service,
    HealthCheck,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Idle,
    Starting,
    Running,
    Stopping,
    Exited,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerOutcome {
    Started,
    Rejected,
    Queued,
    Coalesced,
    Superseded,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDefinition {
    pub schedule: Option<String>,
    pub watch: Vec<String>,
    pub on: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDefinition {
    pub name: String,
    pub run: String,
    pub triggers: TriggerDefinition,
    pub admission: AdmissionPolicy,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisionPolicy {
    pub restart: Option<String>,
    pub max_restarts: Option<u32>,
    pub within: Option<String>,
    pub backoff: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDefinition {
    pub name: String,
    pub run: String,
    pub enabled: bool,
    pub supervision: SupervisionPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: EventId,
    pub event_type: String,
    pub payload: Value,
    pub routing: EventRouting,
    pub config_version: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: RunId,
    pub name: String,
    pub origin: RunOrigin,
    pub state: ProcessState,
    pub config_version: Option<String>,
    pub event_id: Option<EventId>,
    pub run_directory: Option<String>,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRecord {
    pub run_id: RunId,
    pub stdout_path: String,
    pub stderr_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerRecord {
    pub id: TriggerId,
    pub task_name: String,
    pub event_id: Option<EventId>,
    pub event_type: String,
    pub routing: EventRouting,
    pub admission: AdmissionPolicy,
    pub outcome: TriggerOutcome,
    pub run_id: Option<RunId>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub tasks: Vec<TaskDefinition>,
    pub services: Vec<ServiceDefinition>,
    pub active_runs: Vec<RunRecord>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        AdmissionPolicy, EventRecord, EventRouting, ProcessState, RunOrigin, RunRecord,
        TriggerOutcome,
    };
    use crate::{EventId, RunId};

    #[test]
    fn serializes_event_records_with_generic_payloads() {
        let event = EventRecord {
            id: EventId::parse("evt_01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            event_type: "tool.run.completed".to_string(),
            payload: json!({ "ok": true }),
            routing: EventRouting::Event,
            config_version: Some("cfg_123".to_string()),
            source: Some("tool-events".to_string()),
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["payload"]["ok"], json!(true));
    }

    #[test]
    fn serializes_run_state_enums_as_snake_case() {
        let run = RunRecord {
            id: RunId::parse("run_01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            name: "status".to_string(),
            origin: RunOrigin::Task,
            state: ProcessState::Running,
            config_version: None,
            event_id: None,
            run_directory: None,
            stdout_path: None,
            stderr_path: None,
        };
        let value = serde_json::to_value(run).unwrap();
        assert_eq!(value["origin"], "task");
        assert_eq!(value["state"], "running");
        assert_eq!(
            serde_json::to_value(AdmissionPolicy::QueueOne).unwrap(),
            "queue_one"
        );
        assert_eq!(
            serde_json::to_value(TriggerOutcome::Superseded).unwrap(),
            "superseded"
        );
    }
}
