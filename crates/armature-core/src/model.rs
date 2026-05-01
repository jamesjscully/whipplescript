use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{EventId, RunId, TriggerId};

pub const DEFAULT_EVENT_SOURCE: &str = "unknown";

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
    Adhoc,
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

#[derive(Debug, Clone, PartialEq)]
pub struct EventRecord {
    pub id: EventId,
    pub event_type: String,
    pub time: String,
    pub payload: Value,
    pub routing: EventRouting,
    pub config_version: Option<String>,
    pub source: Option<String>,
    pub source_run_id: Option<RunId>,
    pub parent_event_id: Option<EventId>,
    pub correlation_id: Option<String>,
}

impl Serialize for EventRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct EventRecordSerde<'a> {
            id: &'a EventId,
            #[serde(rename = "type")]
            event_type: &'a str,
            time: &'a str,
            #[serde(rename = "event_type")]
            legacy_event_type: &'a str,
            payload: &'a Value,
            routing: &'a EventRouting,
            config_version: &'a Option<String>,
            source: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            source_run_id: &'a Option<RunId>,
            #[serde(skip_serializing_if = "Option::is_none")]
            parent_event_id: &'a Option<EventId>,
            #[serde(skip_serializing_if = "Option::is_none")]
            correlation_id: &'a Option<String>,
        }

        EventRecordSerde {
            id: &self.id,
            event_type: &self.event_type,
            time: &self.time,
            legacy_event_type: &self.event_type,
            payload: &self.payload,
            routing: &self.routing,
            config_version: &self.config_version,
            source: self.source.as_deref().unwrap_or(DEFAULT_EVENT_SOURCE),
            source_run_id: &self.source_run_id,
            parent_event_id: &self.parent_event_id,
            correlation_id: &self.correlation_id,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EventRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct EventRecordSerde {
            id: EventId,
            #[serde(default, rename = "type")]
            event_type: Option<String>,
            #[serde(default)]
            time: String,
            #[serde(default)]
            #[serde(rename = "event_type")]
            legacy_event_type: Option<String>,
            payload: Value,
            routing: EventRouting,
            config_version: Option<String>,
            source: Option<String>,
            #[serde(default)]
            source_run_id: Option<RunId>,
            #[serde(default)]
            parent_event_id: Option<EventId>,
            #[serde(default)]
            correlation_id: Option<String>,
        }

        let decoded = EventRecordSerde::deserialize(deserializer)?;
        let event_type = decoded
            .event_type
            .or(decoded.legacy_event_type)
            .ok_or_else(|| serde::de::Error::missing_field("type"))?;
        Ok(Self {
            id: decoded.id,
            event_type,
            time: decoded.time,
            payload: decoded.payload,
            routing: decoded.routing,
            config_version: decoded.config_version,
            source: decoded.source,
            source_run_id: decoded.source_run_id,
            parent_event_id: decoded.parent_event_id,
            correlation_id: decoded.correlation_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: RunId,
    pub name: String,
    #[serde(default)]
    pub command: String,
    pub origin: RunOrigin,
    pub state: ProcessState,
    #[serde(default)]
    pub start_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    #[serde(default)]
    pub killed: bool,
    pub config_version: Option<String>,
    pub event_id: Option<EventId>,
    #[serde(default, rename = "restartOf", skip_serializing_if = "Option::is_none")]
    pub restart_of: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
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
        TriggerOutcome, DEFAULT_EVENT_SOURCE,
    };
    use crate::{EventId, RunId};

    #[test]
    fn serializes_event_records_with_generic_payloads() {
        let event = EventRecord {
            id: EventId::parse("evt_01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            event_type: "tool.run.completed".to_string(),
            time: "2026-04-29T12:00:00Z".to_string(),
            payload: json!({ "ok": true }),
            routing: EventRouting::Event,
            config_version: Some("cfg_123".to_string()),
            source: Some("tool-events".to_string()),
            source_run_id: Some(RunId::parse("run_01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap()),
            parent_event_id: None,
            correlation_id: Some("corr-123".to_string()),
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "tool.run.completed");
        assert_eq!(value["event_type"], "tool.run.completed");
        assert_eq!(value["time"], "2026-04-29T12:00:00Z");
        assert_eq!(value["payload"]["ok"], json!(true));
        assert_eq!(value["source_run_id"], "run_01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(value["correlation_id"], "corr-123");
    }

    #[test]
    fn serializes_event_source_as_non_null_for_legacy_records() {
        let event = EventRecord {
            id: EventId::parse("evt_01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            event_type: "tool.run.completed".to_string(),
            time: "2026-04-29T12:00:00Z".to_string(),
            payload: json!({}),
            routing: EventRouting::Event,
            config_version: None,
            source: None,
            source_run_id: None,
            parent_event_id: None,
            correlation_id: None,
        };

        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["source"], DEFAULT_EVENT_SOURCE);
    }

    #[test]
    fn deserializes_legacy_event_type_field() {
        let event: EventRecord = serde_json::from_value(json!({
            "id": "evt_01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "event_type": "tool.run.completed",
            "time": "2026-04-29T12:00:00Z",
            "payload": {},
            "routing": "event",
            "config_version": null,
            "source": "tool-events",
            "source_run_id": "run_01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "parent_event_id": "evt_01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "correlation_id": "corr-123"
        }))
        .unwrap();

        assert_eq!(event.event_type, "tool.run.completed");
        assert_eq!(
            event.source_run_id.as_ref().map(RunId::as_str),
            Some("run_01ARZ3NDEKTSV4RRFFQ69G5FAV")
        );
        assert_eq!(
            event.parent_event_id.as_ref().map(EventId::as_str),
            Some("evt_01ARZ3NDEKTSV4RRFFQ69G5FAV")
        );
        assert_eq!(event.correlation_id.as_deref(), Some("corr-123"));
    }

    #[test]
    fn serializes_run_state_enums_as_snake_case() {
        let run = RunRecord {
            id: RunId::parse("run_01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            name: "status".to_string(),
            command: "cargo test".to_string(),
            origin: RunOrigin::Task,
            state: ProcessState::Running,
            start_time: "2026-04-29T12:00:00Z".to_string(),
            end_time: None,
            exit_code: None,
            signal: None,
            killed: false,
            config_version: None,
            event_id: None,
            restart_of: None,
            attempt: None,
            run_directory: None,
            stdout_path: None,
            stderr_path: None,
        };
        let value = serde_json::to_value(run).unwrap();
        assert_eq!(value["origin"], "task");
        assert_eq!(value["state"], "running");
        assert!(value.get("restartOf").is_none());
        assert!(value.get("attempt").is_none());
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
