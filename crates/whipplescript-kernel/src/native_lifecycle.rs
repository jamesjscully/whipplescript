//! Native provider lifecycle normalization.

use serde_json::{json, Value};

use crate::claude_agent_sdk::ClaudeSidecarEvent;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentTurnLifecycleKind {
    Started,
    Streamed,
    ToolRequested,
    ArtifactCaptured,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

impl AgentTurnLifecycleKind {
    pub fn event_type(self) -> &'static str {
        match self {
            Self::Started => "agent.turn.started",
            Self::Streamed => "agent.turn.streamed",
            Self::ToolRequested => "agent.turn.tool_requested",
            Self::ArtifactCaptured => "agent.turn.artifact_captured",
            Self::Completed => "agent.turn.completed",
            Self::Failed => "agent.turn.failed",
            Self::TimedOut => "agent.turn.timed_out",
            Self::Cancelled => "agent.turn.cancelled",
        }
    }

    pub fn status(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Streamed => "streamed",
            Self::ToolRequested => "tool_requested",
            Self::ArtifactCaptured => "artifact_captured",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::TimedOut | Self::Cancelled
        )
    }

    /// Per spec/agent-harness.md, the only durable, rule-matchable lifecycle
    /// facts are `agent.turn.started/completed/failed/timed_out/cancelled`. The
    /// in-turn observations `streamed`/`tool_requested`/`artifact_captured` are
    /// EVIDENCE only — turn-internal activity that is inspectable but never an
    /// event-sourced fact that later rules pattern-match (the compiler enforces
    /// the matching ban via `validate_evidence_fact_not_matched`; this keeps the
    /// storage side honest so they never inflate the fact set in the first place).
    pub fn derives_rule_matchable_fact(self) -> bool {
        !matches!(
            self,
            Self::Streamed | Self::ToolRequested | Self::ArtifactCaptured
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeAgentTurnObservation {
    pub kind: AgentTurnLifecycleKind,
    pub provider_event_type: String,
    pub provider_session_id: Option<String>,
    pub provider_turn_id: Option<String>,
    pub terminal: bool,
    pub provider_payload_shape: Value,
}

impl NativeAgentTurnObservation {
    pub fn fixture(
        kind: AgentTurnLifecycleKind,
        provider_event_type: impl Into<String>,
        provider_session_id: Option<String>,
        provider_turn_id: Option<String>,
        provider_payload_shape: Value,
    ) -> Self {
        Self {
            kind,
            provider_event_type: provider_event_type.into(),
            provider_session_id,
            provider_turn_id,
            terminal: kind.terminal(),
            provider_payload_shape,
        }
    }

    fn new(kind: AgentTurnLifecycleKind, provider_event_type: impl Into<String>) -> Self {
        Self {
            kind,
            provider_event_type: provider_event_type.into(),
            provider_session_id: None,
            provider_turn_id: None,
            terminal: kind.terminal(),
            provider_payload_shape: Value::Null,
        }
    }

    fn session_id(mut self, session_id: Option<String>) -> Self {
        self.provider_session_id = session_id;
        self
    }

    fn turn_id(mut self, turn_id: Option<String>) -> Self {
        self.provider_turn_id = turn_id;
        self
    }

    fn payload_shape(mut self, payload: &Value) -> Self {
        self.provider_payload_shape = json_shape(payload);
        self
    }
}

pub fn normalize_codex_app_server_event(message: &Value) -> Option<NativeAgentTurnObservation> {
    let method = message.get("method").and_then(Value::as_str)?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let kind = match method {
        "turn/started" => AgentTurnLifecycleKind::Started,
        "turn/completed" => codex_terminal_kind(params),
        "turn/diff/updated" | "item/fileChange/patchUpdated" => {
            AgentTurnLifecycleKind::ArtifactCaptured
        }
        "item/tool/call" | "item/tool/requestUserInput" => AgentTurnLifecycleKind::ToolRequested,
        "item/started" | "item/completed" => AgentTurnLifecycleKind::Streamed,
        method if method.contains("/requestApproval") => AgentTurnLifecycleKind::ToolRequested,
        _ => return None,
    };
    Some(
        NativeAgentTurnObservation::new(kind, method)
            .session_id(
                params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
            .turn_id(
                params
                    .get("turnId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
            .payload_shape(params),
    )
}

fn codex_terminal_kind(params: &Value) -> AgentTurnLifecycleKind {
    match params
        .get("status")
        .or_else(|| params.pointer("/turn/status"))
        .or_else(|| params.get("reason"))
        .and_then(Value::as_str)
    {
        Some("cancelled" | "interrupted" | "canceled") => AgentTurnLifecycleKind::Cancelled,
        Some("failed" | "error") => AgentTurnLifecycleKind::Failed,
        Some("timed_out" | "timeout") => AgentTurnLifecycleKind::TimedOut,
        _ => AgentTurnLifecycleKind::Completed,
    }
}

pub fn normalize_claude_agent_sdk_event(
    event: &ClaudeSidecarEvent,
) -> Option<NativeAgentTurnObservation> {
    let kind = match event.event_type.as_str() {
        "claude.session.started" => AgentTurnLifecycleKind::Started,
        "claude.stream.message" => AgentTurnLifecycleKind::Streamed,
        "claude.tool.requested" | "claude.hook.event" => AgentTurnLifecycleKind::ToolRequested,
        "claude.artifact.captured" => AgentTurnLifecycleKind::ArtifactCaptured,
        "claude.turn.completed" => AgentTurnLifecycleKind::Completed,
        "claude.turn.failed" => AgentTurnLifecycleKind::Failed,
        "claude.turn.cancelled" => AgentTurnLifecycleKind::Cancelled,
        _ => return None,
    };
    Some(
        NativeAgentTurnObservation::new(kind, &event.event_type)
            .session_id(
                event
                    .payload
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
            .turn_id(Some(event.run_id.clone()))
            .payload_shape(&event.payload),
    )
}

pub fn normalize_pi_rpc_event(message: &Value) -> Option<NativeAgentTurnObservation> {
    let event_type = message.get("type").and_then(Value::as_str)?;
    let kind = match event_type {
        "turn_start" => AgentTurnLifecycleKind::Started,
        "message_start" | "message_end" => AgentTurnLifecycleKind::Streamed,
        "tool_call" | "tool_result" => AgentTurnLifecycleKind::ToolRequested,
        "artifact" | "artifact_captured" => AgentTurnLifecycleKind::ArtifactCaptured,
        "turn_end" => pi_terminal_kind(message),
        "agent_start" | "agent_end" => AgentTurnLifecycleKind::Streamed,
        _ => return None,
    };
    Some(
        NativeAgentTurnObservation::new(kind, event_type)
            .session_id(
                message
                    .get("sessionId")
                    .or_else(|| message.pointer("/message/sessionId"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
            .turn_id(
                message
                    .get("turnId")
                    .or_else(|| message.pointer("/message/turnId"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
            .payload_shape(message),
    )
}

fn pi_terminal_kind(message: &Value) -> AgentTurnLifecycleKind {
    match message
        .pointer("/message/stopReason")
        .or_else(|| message.get("stopReason"))
        .and_then(Value::as_str)
    {
        Some("aborted" | "cancelled" | "canceled") => AgentTurnLifecycleKind::Cancelled,
        Some("error" | "failed") => AgentTurnLifecycleKind::Failed,
        Some("timeout" | "timed_out") => AgentTurnLifecycleKind::TimedOut,
        _ if message.pointer("/message/errorMessage").is_some() => AgentTurnLifecycleKind::Failed,
        _ => AgentTurnLifecycleKind::Completed,
    }
}

fn json_shape(value: &Value) -> Value {
    match value {
        Value::Null => json!({"type":"null"}),
        Value::Bool(_) => json!({"type":"bool"}),
        Value::Number(_) => json!({"type":"number"}),
        Value::String(value) => json!({"type":"string","chars":value.chars().count()}),
        Value::Array(values) => json!({"type":"array","items":values.len()}),
        Value::Object(object) => json!({"type":"object","keys":object.len()}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_codex_started_diff_tool_and_cancelled_terminal() {
        let started = normalize_codex_app_server_event(&json!({
            "method": "turn/started",
            "params": {"threadId": "thread-1", "turnId": "turn-1"},
        }))
        .expect("started normalizes");
        assert_eq!(started.kind, AgentTurnLifecycleKind::Started);
        assert_eq!(started.provider_session_id.as_deref(), Some("thread-1"));
        assert_eq!(started.provider_turn_id.as_deref(), Some("turn-1"));

        let diff = normalize_codex_app_server_event(&json!({
            "method": "turn/diff/updated",
            "params": {"diff": "secret diff"},
        }))
        .expect("diff normalizes");
        assert_eq!(diff.kind, AgentTurnLifecycleKind::ArtifactCaptured);
        assert!(!diff
            .provider_payload_shape
            .to_string()
            .contains("secret diff"));

        let tool = normalize_codex_app_server_event(&json!({
            "method": "item/commandExecution/requestApproval",
            "params": {"command": "cat secret.txt"},
        }))
        .expect("tool normalizes");
        assert_eq!(tool.kind, AgentTurnLifecycleKind::ToolRequested);

        let terminal = normalize_codex_app_server_event(&json!({
            "method": "turn/completed",
            "params": {"status": "interrupted"},
        }))
        .expect("terminal normalizes");
        assert_eq!(terminal.kind, AgentTurnLifecycleKind::Cancelled);
        assert!(terminal.terminal);
    }

    #[test]
    fn normalizes_claude_terminal_events() {
        let event = ClaudeSidecarEvent {
            event_type: "claude.turn.failed".to_owned(),
            run_id: "run-1".to_owned(),
            payload: json!({"session_id": "session-1", "error": "secret"}),
        };

        let observation = normalize_claude_agent_sdk_event(&event).expect("event normalizes");

        assert_eq!(observation.kind, AgentTurnLifecycleKind::Failed);
        assert_eq!(
            observation.provider_session_id.as_deref(),
            Some("session-1")
        );
        assert_eq!(observation.provider_turn_id.as_deref(), Some("run-1"));
        assert!(!observation
            .provider_payload_shape
            .to_string()
            .contains("secret"));
    }

    #[test]
    fn normalizes_claude_and_pi_artifact_events() {
        let claude = normalize_claude_agent_sdk_event(&ClaudeSidecarEvent {
            event_type: "claude.artifact.captured".to_owned(),
            run_id: "run-1".to_owned(),
            payload: json!({"session_id": "session-1", "content": "secret"}),
        })
        .expect("claude artifact normalizes");
        assert_eq!(claude.kind, AgentTurnLifecycleKind::ArtifactCaptured);
        assert!(!claude.provider_payload_shape.to_string().contains("secret"));

        let pi = normalize_pi_rpc_event(&json!({
            "type": "artifact",
            "message": {
                "sessionId": "session-1",
                "turnId": "turn-1",
                "content": "secret",
            },
        }))
        .expect("pi artifact normalizes");
        assert_eq!(pi.kind, AgentTurnLifecycleKind::ArtifactCaptured);
        assert_eq!(pi.provider_session_id.as_deref(), Some("session-1"));
        assert_eq!(pi.provider_turn_id.as_deref(), Some("turn-1"));
        assert!(!pi.provider_payload_shape.to_string().contains("secret"));
    }

    #[test]
    fn normalizes_pi_aborted_turn_end() {
        let observation = normalize_pi_rpc_event(&json!({
            "type": "turn_end",
            "message": {
                "role": "assistant",
                "stopReason": "aborted",
                "content": [{"type": "text", "text": "secret"}],
            },
        }))
        .expect("event normalizes");

        assert_eq!(observation.kind, AgentTurnLifecycleKind::Cancelled);
        assert!(observation.terminal);
        assert!(!observation
            .provider_payload_shape
            .to_string()
            .contains("secret"));
    }
}
