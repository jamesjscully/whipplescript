//! Minimal Pi RPC JSONL client.

use std::{
    collections::{BTreeMap, VecDeque},
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use serde_json::{json, Map, Value};

use crate::{
    native_lifecycle::{normalize_pi_rpc_event, AgentTurnLifecycleKind},
    provider::{
        AdapterSurface, CancellationDepth, NativeProviderAdapter, NativeProviderArtifactRef,
        NativeProviderBoundaryError, NativeProviderCancellation, NativeProviderEvent,
        NativeProviderEventKind, NativeProviderTurnRequest, ProviderCapability, ProviderKind,
    },
};

#[derive(Debug)]
pub enum PiRpcError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Protocol(String),
    Remote(Value),
    Timeout(String),
}

impl From<std::io::Error> for PiRpcError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for PiRpcError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PiRpcState {
    pub session_id: Option<String>,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub is_streaming: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PiRpcToolPolicy {
    pub tools: Vec<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub extension_refs: Vec<String>,
    pub skill_refs: Vec<String>,
    pub no_session: bool,
}

impl PiRpcToolPolicy {
    pub fn to_cli_args(&self) -> Vec<String> {
        let mut args = vec!["--mode".to_owned(), "rpc".to_owned()];
        if self.no_session {
            args.push("--no-session".to_owned());
        }
        if let Some(provider) = &self.provider {
            args.push("--provider".to_owned());
            args.push(provider.clone());
        }
        if let Some(model) = &self.model {
            args.push("--model".to_owned());
            args.push(model.clone());
        }
        if !self.tools.is_empty() {
            args.push("--tools".to_owned());
            args.push(self.tools.join(","));
        }
        for extension_ref in &self.extension_refs {
            args.push("--extension".to_owned());
            args.push(extension_ref.clone());
        }
        for skill_ref in &self.skill_refs {
            args.push("--skill".to_owned());
            args.push(skill_ref.clone());
        }
        args
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PiRpcPolicyError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PiRpcPolicyInput<'a> {
    pub profile: Option<&'a str>,
    pub required_capabilities: &'a [String],
    pub workspace_policy: &'a str,
    pub approval_mode: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub extension_refs: &'a [String],
    pub skill_refs: &'a [String],
}

pub fn build_pi_rpc_tool_policy(
    input: PiRpcPolicyInput<'_>,
) -> Result<PiRpcToolPolicy, PiRpcPolicyError> {
    let Some(profile) = input.profile else {
        return Err(pi_policy_error(
            "missing_profile",
            "Pi RPC runs require a WhippleScript profile",
        ));
    };
    let mut tools = match profile {
        "repo-reader" => pi_strings(&["read"]),
        "repo-writer" => pi_strings(&["read", "edit", "write"]),
        other => {
            return Err(pi_policy_error(
                "unsupported_profile",
                format!("profile `{other}` is not mapped to a Pi tool policy"),
            ));
        }
    };
    let mut needs_approval = false;
    for capability in input.required_capabilities {
        match capability.as_str() {
            "repo.read" | "agent.tell" => {}
            "repo.write" => {
                require_writer(profile, input.workspace_policy, "repo.write")?;
                needs_approval = true;
                pi_insert_unique(&mut tools, "edit");
                pi_insert_unique(&mut tools, "write");
            }
            "command.run" => {
                require_writer(profile, input.workspace_policy, "command.run")?;
                needs_approval = true;
                pi_insert_unique(&mut tools, "bash");
            }
            other => {
                return Err(pi_policy_error(
                    "unsupported_capability",
                    format!("capability `{other}` is not mapped to a Pi tool policy"),
                ));
            }
        }
    }
    if needs_approval && input.approval_mode.is_none() {
        return Err(pi_policy_error(
            "missing_approval",
            "destructive Pi tools require an explicit approval mode",
        ));
    }
    if let Some(approval_mode) = input.approval_mode {
        match approval_mode {
            "auto" | "manual" | "accept_edits" => {}
            other => {
                return Err(pi_policy_error(
                    "unsupported_approval_mode",
                    format!("approval mode `{other}` is not supported for Pi"),
                ));
            }
        }
    }
    validate_optional_identifier("provider", input.provider)?;
    validate_optional_identifier("model", input.model)?;
    validate_refs("extension", input.extension_refs)?;
    validate_refs("skill", input.skill_refs)?;
    tools.sort();
    Ok(PiRpcToolPolicy {
        tools,
        provider: input.provider.map(str::to_owned),
        model: input.model.map(str::to_owned),
        extension_refs: input.extension_refs.to_vec(),
        skill_refs: input.skill_refs.to_vec(),
        no_session: true,
    })
}

fn require_writer(
    profile: &str,
    workspace_policy: &str,
    capability: &str,
) -> Result<(), PiRpcPolicyError> {
    if profile != "repo-writer" {
        return Err(pi_policy_error(
            "profile_denied",
            format!("profile `{profile}` cannot grant capability `{capability}`"),
        ));
    }
    match workspace_policy {
        "shared" | "per_effect_worktree" | "per_issue_worktree" => Ok(()),
        "read_only" => Err(pi_policy_error(
            "workspace_denied",
            format!("capability `{capability}` cannot run in a read-only workspace"),
        )),
        other => Err(pi_policy_error(
            "unsupported_workspace_policy",
            format!("workspace policy `{other}` is not supported for Pi writes"),
        )),
    }
}

fn validate_optional_identifier(kind: &str, value: Option<&str>) -> Result<(), PiRpcPolicyError> {
    if let Some(value) = value {
        if value.trim().is_empty() || value.contains(char::is_whitespace) {
            return Err(pi_policy_error(
                format!("invalid_{kind}"),
                format!("Pi {kind} must be a non-empty identifier without whitespace"),
            ));
        }
    }
    Ok(())
}

fn validate_refs(kind: &str, refs: &[String]) -> Result<(), PiRpcPolicyError> {
    for reference in refs {
        if reference.trim().is_empty() || reference.contains('\n') || reference.contains('\r') {
            return Err(pi_policy_error(
                format!("invalid_{kind}_ref"),
                format!("Pi {kind} reference must be a non-empty single-line value"),
            ));
        }
    }
    Ok(())
}

fn pi_policy_error(code: impl Into<String>, message: impl Into<String>) -> PiRpcPolicyError {
    PiRpcPolicyError {
        code: code.into(),
        message: message.into(),
    }
}

fn pi_strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn pi_insert_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|item| item == value) {
        values.push(value.to_owned());
    }
}

pub trait PiRpcTransport {
    fn write_line(&mut self, line: &str) -> Result<(), PiRpcError>;
    fn read_line(&mut self) -> Result<String, PiRpcError>;
    /// Wait up to `wait` for the next line; `Ok(None)` means the window
    /// elapsed with the peer still alive (DR-0035 Decision 4 inactivity
    /// clock). The default keeps blocking semantics for transports without a
    /// clock (test fakes).
    fn read_line_timeout(
        &mut self,
        wait: std::time::Duration,
    ) -> Result<Option<String>, PiRpcError> {
        let _ = wait;
        self.read_line().map(Some)
    }
}

pub struct PiRpcClient<T> {
    transport: T,
    next_id: u64,
    events: VecDeque<Value>,
}

impl<T: PiRpcTransport> PiRpcClient<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_id: 1,
            events: VecDeque::new(),
        }
    }

    pub fn request(&mut self, command_type: &str, params: Value) -> Result<Value, PiRpcError> {
        self.request_inner(command_type, params, None)
    }

    /// `request` bounded by a wait budget (DR-0035 Decision 4): a peer that
    /// never answers the RPC yields a Timeout error instead of blocking the
    /// worker thread through turn start.
    pub fn request_timeout(
        &mut self,
        command_type: &str,
        params: Value,
        wait: std::time::Duration,
    ) -> Result<Value, PiRpcError> {
        self.request_inner(command_type, params, Some(wait))
    }

    fn request_inner(
        &mut self,
        command_type: &str,
        params: Value,
        wait: Option<std::time::Duration>,
    ) -> Result<Value, PiRpcError> {
        let id = format!("ws-{}", self.next_id);
        self.next_id += 1;
        let request = build_request(&id, command_type, params);
        self.transport.write_line(&request.to_string())?;
        loop {
            let line = match wait {
                Some(window) => self.transport.read_line_timeout(window)?.ok_or_else(|| {
                    PiRpcError::Timeout(format!(
                        "no response to `{command_type}` within the start budget"
                    ))
                })?,
                None => self.transport.read_line()?,
            };
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line)?;
            if is_matching_response(&message, &id, command_type) {
                if message.get("success").and_then(Value::as_bool) == Some(false) {
                    return Err(PiRpcError::Remote(message));
                }
                return Ok(message.get("data").cloned().unwrap_or(Value::Null));
            }
            if message.get("type").and_then(Value::as_str) == Some("response") {
                return Err(PiRpcError::Protocol(
                    "received response for unexpected Pi RPC request".to_owned(),
                ));
            }
            self.events.push_back(message);
        }
    }

    pub fn get_state(&mut self) -> Result<PiRpcState, PiRpcError> {
        let data = self.request("get_state", Value::Null)?;
        Self::parse_state(data)
    }

    /// `get_state` bounded by the start budget (DR-0035 Decision 4).
    pub fn get_state_timeout(
        &mut self,
        wait: std::time::Duration,
    ) -> Result<PiRpcState, PiRpcError> {
        let data = self.request_timeout("get_state", Value::Null, wait)?;
        Self::parse_state(data)
    }

    fn parse_state(data: Value) -> Result<PiRpcState, PiRpcError> {
        Ok(PiRpcState {
            session_id: data
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            model_provider: data
                .pointer("/model/provider")
                .and_then(Value::as_str)
                .map(str::to_owned),
            model_id: data
                .pointer("/model/id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            is_streaming: data.get("isStreaming").and_then(Value::as_bool),
        })
    }

    pub fn abort_current_operation(&mut self) -> Result<(), PiRpcError> {
        self.request("abort", Value::Null).map(|_| ())
    }

    pub fn pop_event(&mut self) -> Option<Value> {
        self.events.pop_front()
    }

    pub fn read_event(&mut self) -> Result<Value, PiRpcError> {
        loop {
            let line = self.transport.read_line()?;
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line)?;
            if message.get("type").and_then(Value::as_str) == Some("response") {
                return Err(PiRpcError::Protocol(
                    "received Pi RPC response while waiting for event".to_owned(),
                ));
            }
            return Ok(message);
        }
    }

    /// `read_event` bounded by the inactivity clock: `Ok(None)` means the
    /// window elapsed without a frame (the peer is alive but silent).
    pub fn read_event_timeout(
        &mut self,
        wait: std::time::Duration,
    ) -> Result<Option<Value>, PiRpcError> {
        loop {
            let Some(line) = self.transport.read_line_timeout(wait)? else {
                return Ok(None);
            };
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line)?;
            if message.get("type").and_then(Value::as_str) == Some("response") {
                return Err(PiRpcError::Protocol(
                    "received Pi RPC response while waiting for event".to_owned(),
                ));
            }
            return Ok(Some(message));
        }
    }

    pub fn into_transport(self) -> T {
        self.transport
    }
}

fn build_request(id: &str, command_type: &str, params: Value) -> Value {
    let mut object = Map::new();
    object.insert("id".to_owned(), Value::String(id.to_owned()));
    object.insert("type".to_owned(), Value::String(command_type.to_owned()));
    if let Some(params) = params.as_object() {
        for (key, value) in params {
            if key != "id" && key != "type" {
                object.insert(key.clone(), value.clone());
            }
        }
    } else if !params.is_null() {
        object.insert("params".to_owned(), params);
    }
    Value::Object(object)
}

fn is_matching_response(message: &Value, id: &str, command_type: &str) -> bool {
    message.get("type").and_then(Value::as_str) == Some("response")
        && message.get("id").and_then(Value::as_str) == Some(id)
        && message.get("command").and_then(Value::as_str) == Some(command_type)
}

pub fn summarize_pi_rpc_events(events: &[Value]) -> Value {
    let mut counts = Map::new();
    for event in events {
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let count = counts.get(event_type).and_then(Value::as_u64).unwrap_or(0) + 1;
        counts.insert(event_type.to_owned(), json!(count));
    }
    json!({ "event_counts": counts })
}

pub fn summarize_pi_rpc_evidence(
    state: &PiRpcState,
    events: &[Value],
    terminal: Option<&Value>,
) -> Value {
    let mut summary = summarize_pi_rpc_events(events);
    let terminal_payload = terminal
        .map(|terminal| {
            json!({
                "type": terminal.get("type").and_then(Value::as_str),
                "status": terminal.get("status").and_then(Value::as_str),
                "result_shape": terminal.get("result").map(pi_json_shape),
                "error_shape": terminal.get("error").map(pi_json_shape),
            })
        })
        .unwrap_or(Value::Null);
    let object = summary
        .as_object_mut()
        .expect("Pi RPC summary starts as an object");
    object.insert("session_id".to_owned(), json!(state.session_id));
    object.insert("model_provider".to_owned(), json!(state.model_provider));
    object.insert("model_id".to_owned(), json!(state.model_id));
    object.insert("is_streaming".to_owned(), json!(state.is_streaming));
    object.insert(
        "terminal_type".to_owned(),
        terminal
            .and_then(|terminal| terminal.get("type").and_then(Value::as_str))
            .map(Value::from)
            .unwrap_or(Value::Null),
    );
    object.insert("terminal_payload".to_owned(), terminal_payload);
    summary
}

fn pi_json_shape(value: &Value) -> Value {
    match value {
        Value::Null => json!({"type":"null"}),
        Value::Bool(_) => json!({"type":"bool"}),
        Value::Number(_) => json!({"type":"number"}),
        Value::String(value) => json!({"type":"string","chars":value.chars().count()}),
        Value::Array(values) => json!({"type":"array","items":values.len()}),
        Value::Object(object) => json!({"type":"object","keys":object.len()}),
    }
}

pub struct PiRpcAdapter<T> {
    provider_id: String,
    capability: ProviderCapability,
    client: PiRpcClient<T>,
    state: Option<PiRpcState>,
    sequence: u64,
    // DR-0035 Decision 4: the inactivity wall clock. When no frame arrives
    // within this window, the adapter synthesizes the TimedOut terminal.
    inactivity_budget: std::time::Duration,
    // Idle time accumulated across empty poll slices; reset on every frame.
    idle_elapsed: std::time::Duration,
}

impl<T: PiRpcTransport> PiRpcAdapter<T> {
    pub fn new(
        provider_id: impl Into<String>,
        capability: ProviderCapability,
        client: PiRpcClient<T>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            capability,
            client,
            state: None,
            sequence: 0,
            inactivity_budget: std::time::Duration::from_secs(300),
            idle_elapsed: std::time::Duration::ZERO,
        }
    }

    pub fn with_inactivity_budget(mut self, budget: std::time::Duration) -> Self {
        self.inactivity_budget = budget;
        self
    }

    pub fn into_client(self) -> PiRpcClient<T> {
        self.client
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    /// The synthesized inactivity terminal (DR-0035 Decision 4 T2): a silent
    /// or closed peer still yields exactly one terminal.
    fn inactivity_timeout_event(&mut self, run_id: &str, reason: &str) -> NativeProviderEvent {
        NativeProviderEvent {
            provider_id: self.provider_id.clone(),
            run_id: run_id.to_owned(),
            event_kind: NativeProviderEventKind::TimedOut,
            provider_event_type: "whip.native.inactivity_timeout".to_owned(),
            provider_session_id: self
                .state
                .as_ref()
                .and_then(|state| state.session_id.clone()),
            provider_turn_id: None,
            sequence: Some(self.next_sequence()),
            evidence: json!({
                "reason": reason,
                "inactivity_budget_seconds": self.inactivity_budget.as_secs(),
            }),
            artifacts: Vec::new(),
        }
    }

    fn boundary_error(
        &self,
        code: impl Into<String>,
        message: impl Into<String>,
        recoverable: bool,
        evidence: Value,
    ) -> NativeProviderBoundaryError {
        NativeProviderBoundaryError {
            provider_id: self.provider_id.clone(),
            surface: AdapterSurface::PiRpc,
            code: code.into(),
            message: message.into(),
            recoverable,
            evidence,
        }
    }

    fn map_error(&self, code: &'static str, error: PiRpcError) -> NativeProviderBoundaryError {
        let (message, recoverable, evidence) = match error {
            PiRpcError::Io(error) => (
                format!("Pi RPC I/O error: {error}"),
                true,
                json!({"kind": "io"}),
            ),
            PiRpcError::Json(error) => (
                format!("Pi RPC emitted invalid JSON: {error}"),
                true,
                json!({"kind": "json"}),
            ),
            PiRpcError::Protocol(message) => (message, true, json!({"kind": "protocol"})),
            PiRpcError::Remote(error) => (
                "Pi RPC returned a remote error".to_owned(),
                true,
                json!({"kind": "remote", "shape": pi_json_shape(&error)}),
            ),
            PiRpcError::Timeout(message) => (message, true, json!({"kind": "timeout"})),
        };
        self.boundary_error(code, message, recoverable, evidence)
    }

    fn ensure_pi_request(
        &self,
        request: &NativeProviderTurnRequest,
    ) -> Result<(), NativeProviderBoundaryError> {
        if request.provider_id != self.provider_id {
            return Err(self.boundary_error(
                "provider_id_mismatch",
                "Pi adapter received a request for a different provider id",
                false,
                request.to_json_redacted(),
            ));
        }
        if request.provider_kind != ProviderKind::Pi || request.surface != AdapterSurface::PiRpc {
            return Err(self.boundary_error(
                "surface_mismatch",
                "Pi adapter only accepts pi_rpc requests",
                false,
                request.to_json_redacted(),
            ));
        }
        Ok(())
    }

    fn event_from_message(&mut self, run_id: &str, message: Value) -> Option<NativeProviderEvent> {
        let observation = normalize_pi_rpc_event(&message)?;
        let state = self.state.clone().unwrap_or(PiRpcState {
            session_id: observation.provider_session_id.clone(),
            model_provider: None,
            model_id: None,
            is_streaming: None,
        });
        let terminal = observation.terminal.then_some(&message);
        Some(NativeProviderEvent {
            provider_id: self.provider_id.clone(),
            run_id: run_id.to_owned(),
            event_kind: native_event_kind(observation.kind),
            provider_event_type: observation.provider_event_type,
            provider_session_id: observation
                .provider_session_id
                .or_else(|| state.session_id.clone()),
            provider_turn_id: observation.provider_turn_id,
            sequence: Some(self.next_sequence()),
            evidence: json!({
                "provider_payload_shape": observation.provider_payload_shape,
                "rpc_evidence": summarize_pi_rpc_evidence(&state, std::slice::from_ref(&message), terminal),
                "provider_error": observation.provider_error,
            }),
            artifacts: artifact_refs_from_pi_message(&message),
        })
    }
}

impl<T: PiRpcTransport> NativeProviderAdapter for PiRpcAdapter<T> {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn capability(&self) -> &ProviderCapability {
        &self.capability
    }

    fn start_turn(
        &mut self,
        request: NativeProviderTurnRequest,
    ) -> Result<NativeProviderEvent, NativeProviderBoundaryError> {
        self.ensure_pi_request(&request)?;
        let extension_refs = string_array_option(&request.provider_options, "extension_refs");
        let skill_refs = string_array_option(&request.provider_options, "skill_refs");
        let policy = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: request.profile.as_deref(),
            required_capabilities: &request.required_capabilities,
            workspace_policy: &request.workspace_policy,
            approval_mode: request
                .provider_options
                .get("approval_mode")
                .and_then(Value::as_str),
            provider: request
                .provider_options
                .get("provider")
                .and_then(Value::as_str),
            model: request
                .provider_options
                .get("model")
                .and_then(Value::as_str),
            extension_refs: &extension_refs,
            skill_refs: &skill_refs,
        })
        .map_err(|error| {
            self.boundary_error(error.code, error.message, false, request.to_json_redacted())
        })?;
        // Turn-start RPCs are bounded by the inactivity budget (DR-0035
        // Decision 4): a hung peer fails the start instead of pinning the
        // worker thread.
        let budget = self.inactivity_budget;
        let state = self
            .client
            .get_state_timeout(budget)
            .map_err(|error| self.map_error("pi_get_state_failed", error))?;
        self.state = Some(state);
        self.client
            .request_timeout(
                "prompt",
                pi_prompt_request(&request.prompt_json, &policy),
                budget,
            )
            .map_err(|error| self.map_error("pi_prompt_failed", error))?;
        while let Some(event) = self.client.pop_event() {
            if let Some(native) = self.event_from_message(&request.run_id, event) {
                return Ok(native);
            }
        }
        Ok(NativeProviderEvent {
            provider_id: self.provider_id.clone(),
            run_id: request.run_id,
            event_kind: NativeProviderEventKind::Started,
            provider_event_type: "prompt".to_owned(),
            provider_session_id: self
                .state
                .as_ref()
                .and_then(|state| state.session_id.clone()),
            provider_turn_id: None,
            sequence: Some(self.next_sequence()),
            evidence: json!({
                "prompt_accepted": true,
                "policy": {
                    "tools": policy.tools,
                    "provider": policy.provider,
                    "model": policy.model,
                    "extension_refs": policy.extension_refs,
                    "skill_refs": policy.skill_refs,
                    "no_session": policy.no_session,
                },
            }),
            artifacts: vec![],
        })
    }

    fn next_event(
        &mut self,
        run_id: &str,
    ) -> Result<Option<NativeProviderEvent>, NativeProviderBoundaryError> {
        if let Some(event) = self.client.pop_event() {
            return Ok(self.event_from_message(run_id, event));
        }
        // The wait is sliced (DR-0035 Decision 5) so the driver regains control
        // between slices to act on cancellation requests; the inactivity clock
        // accumulates across empty slices and fires at the full budget.
        let slice = self
            .inactivity_budget
            .min(std::time::Duration::from_secs(1));
        match self.client.read_event_timeout(slice) {
            Ok(Some(event)) => {
                self.idle_elapsed = std::time::Duration::ZERO;
                Ok(self.event_from_message(run_id, event))
            }
            Ok(None) => {
                self.idle_elapsed += slice;
                if self.idle_elapsed >= self.inactivity_budget {
                    self.idle_elapsed = std::time::Duration::ZERO;
                    // Window elapsed with no frame: the inactivity clock fires.
                    Ok(Some(self.inactivity_timeout_event(
                        run_id,
                        "inactivity_budget_exhausted",
                    )))
                } else {
                    Ok(None)
                }
            }
            // The stream closed with no terminal: same backstop, distinct reason.
            Err(PiRpcError::Timeout(_)) => {
                Ok(Some(self.inactivity_timeout_event(run_id, "stream_closed")))
            }
            Err(error) => Err(self.map_error("pi_event_read_failed", error)),
        }
    }

    fn cancel_turn(
        &mut self,
        cancellation: NativeProviderCancellation,
    ) -> Result<NativeProviderEvent, NativeProviderBoundaryError> {
        if !CancellationDepth::NativeStop.allows(cancellation.requested_depth) {
            return Err(self.boundary_error(
                "unsupported_cancellation_depth",
                format!(
                    "Pi RPC adapter cannot satisfy `{}` cancellation",
                    cancellation.requested_depth.as_str()
                ),
                false,
                json!({"requested_depth": cancellation.requested_depth.as_str()}),
            ));
        }
        self.client
            .abort_current_operation()
            .map_err(|error| self.map_error("pi_abort_failed", error))?;
        Ok(NativeProviderEvent {
            provider_id: self.provider_id.clone(),
            run_id: cancellation.run_id,
            event_kind: NativeProviderEventKind::Diagnostic,
            provider_event_type: "abort".to_owned(),
            provider_session_id: cancellation.provider_session_id.or_else(|| {
                self.state
                    .as_ref()
                    .and_then(|state| state.session_id.clone())
            }),
            provider_turn_id: cancellation.provider_turn_id,
            sequence: Some(self.next_sequence()),
            evidence: json!({
                "acknowledged": true,
                "requested_depth": cancellation.requested_depth.as_str(),
                "reason_shape": pi_json_shape(&Value::String(cancellation.reason)),
            }),
            artifacts: vec![],
        })
    }
}

fn pi_prompt_request(prompt: &Value, policy: &PiRpcToolPolicy) -> Value {
    let mut payload = Map::new();
    payload.insert("message".to_owned(), Value::String(pi_prompt_text(prompt)));
    payload.insert("tools".to_owned(), json!(policy.tools));
    if let Some(provider) = policy.provider.as_deref() {
        payload.insert("provider".to_owned(), json!(provider));
    }
    if let Some(model) = policy.model.as_deref() {
        payload.insert("model".to_owned(), json!(model));
    }
    if !policy.extension_refs.is_empty() {
        payload.insert("extension_refs".to_owned(), json!(policy.extension_refs));
    }
    if !policy.skill_refs.is_empty() {
        payload.insert("skill_refs".to_owned(), json!(policy.skill_refs));
    }
    Value::Object(payload)
}

fn pi_prompt_text(prompt: &Value) -> String {
    prompt
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| prompt.to_string())
}

fn string_array_option(options: &BTreeMap<String, Value>, key: &str) -> Vec<String> {
    options
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn native_event_kind(kind: AgentTurnLifecycleKind) -> NativeProviderEventKind {
    match kind {
        AgentTurnLifecycleKind::Started => NativeProviderEventKind::Started,
        AgentTurnLifecycleKind::Streamed => NativeProviderEventKind::Streamed,
        AgentTurnLifecycleKind::ToolRequested => NativeProviderEventKind::ToolRequested,
        AgentTurnLifecycleKind::ArtifactCaptured => NativeProviderEventKind::ArtifactCaptured,
        AgentTurnLifecycleKind::Completed => NativeProviderEventKind::Completed,
        AgentTurnLifecycleKind::Failed => NativeProviderEventKind::Failed,
        AgentTurnLifecycleKind::TimedOut => NativeProviderEventKind::TimedOut,
        AgentTurnLifecycleKind::Cancelled => NativeProviderEventKind::Cancelled,
    }
}

fn artifact_refs_from_pi_message(message: &Value) -> Vec<NativeProviderArtifactRef> {
    artifact_values(message)
        .into_iter()
        .chain(message.get("message").into_iter().flat_map(artifact_values))
        .filter_map(|artifact| artifact_ref_from_value(artifact, "pi", "provider://pi/artifacts"))
        .collect()
}

fn artifact_values(payload: &Value) -> Vec<&Value> {
    ["artifacts", "artifact_refs"]
        .into_iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_array))
        .flatten()
        .collect()
}

fn artifact_ref_from_value(
    artifact: &Value,
    provider: &str,
    fallback_base: &str,
) -> Option<NativeProviderArtifactRef> {
    let uri = string_field(artifact, &["uri", "ref", "artifact_ref"])
        .or_else(|| {
            string_field(artifact, &["path"]).map(|path| format!("{provider}-artifact://{path}"))
        })
        .or_else(|| {
            string_field(artifact, &["id", "artifact_id"]).map(|id| format!("{fallback_base}/{id}"))
        })?;
    Some(NativeProviderArtifactRef {
        artifact_id: string_field(artifact, &["artifact_id", "id"]),
        kind: string_field(artifact, &["kind"]).unwrap_or_else(|| "artifact".to_owned()),
        uri,
        content_hash: string_field(artifact, &["content_hash", "hash"]),
        mime_type: string_field(artifact, &["mime_type", "mime"]),
        required: artifact
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
}

pub struct StdioPiRpcTransport {
    _child: Child,
    stdin: ChildStdin,
    // Lines arrive via a reader thread so reads can carry a timeout
    // (DR-0035 Decision 4): a blocked pipe no longer pins the worker thread.
    lines: std::sync::mpsc::Receiver<std::io::Result<String>>,
}

impl StdioPiRpcTransport {
    pub fn spawn(command: &str, args: &[&str]) -> Result<Self, PiRpcError> {
        let mut builder = Command::new(command);
        builder
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        crate::harness::strip_control_plane_secrets(&mut builder);
        let mut child = builder.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            PiRpcError::Protocol("Pi RPC process did not expose stdin".to_owned())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            PiRpcError::Protocol("Pi RPC process did not expose stdout".to_owned())
        })?;
        Ok(Self {
            _child: child,
            stdin,
            lines: spawn_line_reader(stdout),
        })
    }

    fn closed_error() -> PiRpcError {
        PiRpcError::Timeout("Pi RPC stdout closed before response".to_owned())
    }
}

fn spawn_line_reader(stdout: ChildStdout) -> std::sync::mpsc::Receiver<std::io::Result<String>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if sender.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                    break;
                }
            }
        }
    });
    receiver
}

impl PiRpcTransport for StdioPiRpcTransport {
    fn write_line(&mut self, line: &str) -> Result<(), PiRpcError> {
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_line(&mut self) -> Result<String, PiRpcError> {
        match self.lines.recv() {
            Ok(Ok(line)) => Ok(line),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => Err(Self::closed_error()),
        }
    }

    fn read_line_timeout(
        &mut self,
        wait: std::time::Duration,
    ) -> Result<Option<String>, PiRpcError> {
        match self.lines.recv_timeout(wait) {
            Ok(Ok(line)) => Ok(Some(line)),
            Ok(Err(error)) => Err(error.into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(Self::closed_error()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::provider::{builtin_provider_capabilities, NativeProviderAdapter};

    #[derive(Default)]
    struct FakeTransport {
        writes: Vec<String>,
        reads: VecDeque<Result<String, String>>,
    }

    impl FakeTransport {
        fn with_reads(reads: &[&str]) -> Self {
            Self {
                writes: Vec::new(),
                reads: reads.iter().map(|line| Ok((*line).to_owned())).collect(),
            }
        }
    }

    impl PiRpcTransport for FakeTransport {
        fn write_line(&mut self, line: &str) -> Result<(), PiRpcError> {
            self.writes.push(line.to_owned());
            Ok(())
        }

        fn read_line(&mut self) -> Result<String, PiRpcError> {
            match self.reads.pop_front() {
                Some(Ok(line)) => Ok(line),
                Some(Err(message)) => Err(PiRpcError::Timeout(message)),
                None => Err(PiRpcError::Timeout("fake timeout".to_owned())),
            }
        }
    }

    fn pi_capability() -> ProviderCapability {
        builtin_provider_capabilities()
            .into_iter()
            .find(|capability| {
                capability.provider_kind == ProviderKind::Pi
                    && capability.surface == AdapterSurface::PiRpc
            })
            .expect("pi capability")
    }

    fn native_pi_request() -> NativeProviderTurnRequest {
        NativeProviderTurnRequest {
            provider_id: "pi-main".to_owned(),
            provider_kind: ProviderKind::Pi,
            surface: AdapterSurface::PiRpc,
            run_id: "run-1".to_owned(),
            effect_id: "effect-1".to_owned(),
            agent: "pi".to_owned(),
            profile: Some("repo-writer".to_owned()),
            prompt_json: json!("inspect the repo"),
            workspace_policy: "shared".to_owned(),
            required_capabilities: vec!["repo.write".to_owned()],
            cancellation_depth: CancellationDepth::NativeStop,
            artifact_policy: "manifest".to_owned(),
            credential_ref: Some("secret:pi".to_owned()),
            provider_options: BTreeMap::from([
                ("approval_mode".to_owned(), json!("manual")),
                ("provider".to_owned(), json!("openai-codex")),
                ("model".to_owned(), json!("gpt-5.5")),
                ("extension_refs".to_owned(), json!(["./pi-extension.js"])),
            ]),
        }
    }

    #[test]
    fn client_sends_request_and_returns_matching_data() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"get_state","success":true,"data":{"sessionId":"sess_1"}}"#,
        ]);
        let mut client = PiRpcClient::new(transport);

        let result = client
            .request("get_state", Value::Null)
            .expect("request succeeds");

        assert_eq!(
            result.get("sessionId").and_then(Value::as_str),
            Some("sess_1")
        );
        let transport = client.into_transport();
        let request: Value = serde_json::from_str(&transport.writes[0]).expect("request json");
        assert_eq!(request.get("id").and_then(Value::as_str), Some("ws-1"));
        assert_eq!(
            request.get("type").and_then(Value::as_str),
            Some("get_state")
        );
    }

    #[test]
    fn client_flattens_object_params_without_overriding_id_or_type() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"send_message","success":true,"data":{}}"#,
        ]);
        let mut client = PiRpcClient::new(transport);

        client
            .request(
                "send_message",
                json!({"id":"bad","type":"bad","message":"hello"}),
            )
            .expect("request succeeds");

        let transport = client.into_transport();
        let request: Value = serde_json::from_str(&transport.writes[0]).expect("request json");
        assert_eq!(request.get("id").and_then(Value::as_str), Some("ws-1"));
        assert_eq!(
            request.get("type").and_then(Value::as_str),
            Some("send_message")
        );
        assert_eq!(
            request.get("message").and_then(Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn client_buffers_events_before_matching_response() {
        let transport = FakeTransport::with_reads(&[
            r#"{"type":"message","data":{"shape":"redacted"}}"#,
            r#"{"id":"ws-1","type":"response","command":"get_state","success":true,"data":{"sessionId":"sess_1"}}"#,
        ]);
        let mut client = PiRpcClient::new(transport);

        client
            .request("get_state", Value::Null)
            .expect("request succeeds");

        let event = client.pop_event().expect("event queued");
        assert_eq!(event.get("type").and_then(Value::as_str), Some("message"));
    }

    #[test]
    fn get_state_extracts_session_and_model_metadata() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"get_state","success":true,"data":{"sessionId":"sess_1","model":{"provider":"openai-codex","id":"gpt-5.5"},"isStreaming":false}}"#,
        ]);
        let mut client = PiRpcClient::new(transport);

        let state = client.get_state().expect("state succeeds");

        assert_eq!(state.session_id.as_deref(), Some("sess_1"));
        assert_eq!(state.model_provider.as_deref(), Some("openai-codex"));
        assert_eq!(state.model_id.as_deref(), Some("gpt-5.5"));
        assert_eq!(state.is_streaming, Some(false));
    }

    #[test]
    fn client_sends_abort_command() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"abort","success":true}"#,
        ]);
        let mut client = PiRpcClient::new(transport);

        client
            .abort_current_operation()
            .expect("abort command succeeds");

        let transport = client.into_transport();
        let request: Value = serde_json::from_str(&transport.writes[0]).expect("request json");
        assert_eq!(request.get("id").and_then(Value::as_str), Some("ws-1"));
        assert_eq!(request.get("type").and_then(Value::as_str), Some("abort"));
    }

    #[test]
    fn client_reports_malformed_response() {
        let transport = FakeTransport::with_reads(&["not-json"]);
        let mut client = PiRpcClient::new(transport);

        let error = client
            .request("get_state", Value::Null)
            .expect_err("malformed response fails");

        assert!(matches!(error, PiRpcError::Json(_)));
    }

    #[test]
    fn client_reports_remote_error_response() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"get_state","success":false,"error":{"code":"auth_missing"}}"#,
        ]);
        let mut client = PiRpcClient::new(transport);

        let error = client
            .request("get_state", Value::Null)
            .expect_err("remote error fails");

        assert!(
            matches!(error, PiRpcError::Remote(error) if error.pointer("/error/code").and_then(Value::as_str) == Some("auth_missing"))
        );
    }

    #[test]
    fn client_rejects_unexpected_response() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"other","type":"response","command":"get_state","success":true,"data":{}}"#,
        ]);
        let mut client = PiRpcClient::new(transport);

        let error = client
            .request("get_state", Value::Null)
            .expect_err("unexpected response fails");

        assert!(matches!(error, PiRpcError::Protocol(_)));
    }

    #[test]
    fn client_reports_transport_timeout() {
        let transport = FakeTransport::default();
        let mut client = PiRpcClient::new(transport);

        let error = client
            .request("get_state", Value::Null)
            .expect_err("timeout fails");

        assert!(matches!(error, PiRpcError::Timeout(_)));
    }

    #[test]
    fn event_summary_counts_types_without_payloads() {
        let summary = summarize_pi_rpc_events(&[
            json!({"type":"message","data":{"content":"secret"}}),
            json!({"type":"message","data":{"content":"secret2"}}),
            json!({"type":"tool_call","data":{"arguments":{"token":"secret"}}}),
        ]);

        assert_eq!(
            summary
                .pointer("/event_counts/message")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            summary
                .pointer("/event_counts/tool_call")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert!(!summary.to_string().contains("secret"));
    }

    #[test]
    fn evidence_summary_redacts_terminal_and_event_payloads() {
        let state = PiRpcState {
            session_id: Some("sess_1".to_owned()),
            model_provider: Some("openai-codex".to_owned()),
            model_id: Some("gpt-5.5".to_owned()),
            is_streaming: Some(false),
        };
        let events = vec![json!({"type":"message","data":{"content":"secret"}})];
        let terminal = json!({
            "type": "completed",
            "status": "success",
            "result": "secret final text",
            "error": {"token": "secret"},
        });

        let summary = summarize_pi_rpc_evidence(&state, &events, Some(&terminal));

        assert_eq!(
            summary.get("session_id").and_then(Value::as_str),
            Some("sess_1")
        );
        assert_eq!(
            summary
                .pointer("/event_counts/message")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            summary
                .pointer("/terminal_payload/result_shape/chars")
                .and_then(Value::as_u64),
            Some("secret final text".chars().count() as u64)
        );
        assert_eq!(
            summary
                .pointer("/terminal_payload/error_shape/keys")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert!(!summary.to_string().contains("secret"));
    }

    #[test]
    fn policy_maps_reader_profile_to_read_tool() {
        let policy = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: Some("repo-reader"),
            required_capabilities: &["repo.read".to_owned()],
            workspace_policy: "read_only",
            approval_mode: None,
            provider: Some("openai-codex"),
            model: Some("gpt-5.5"),
            extension_refs: &[],
            skill_refs: &[],
        })
        .expect("reader policy maps");

        assert_eq!(policy.tools, pi_strings(&["read"]));
        assert_eq!(policy.provider.as_deref(), Some("openai-codex"));
        assert_eq!(policy.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(
            policy.to_cli_args(),
            pi_strings(&[
                "--mode",
                "rpc",
                "--no-session",
                "--provider",
                "openai-codex",
                "--model",
                "gpt-5.5",
                "--tools",
                "read",
            ])
        );
    }

    #[test]
    fn policy_maps_writer_profile_to_edit_and_bash_tools() {
        let policy = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: Some("repo-writer"),
            required_capabilities: &["repo.write".to_owned(), "command.run".to_owned()],
            workspace_policy: "shared",
            approval_mode: Some("manual"),
            provider: None,
            model: None,
            extension_refs: &["./pi-extension.js".to_owned()],
            skill_refs: &["./skills/review.md".to_owned()],
        })
        .expect("writer policy maps");

        assert_eq!(policy.tools, pi_strings(&["bash", "edit", "read", "write"]));
        assert_eq!(
            policy.to_cli_args(),
            pi_strings(&[
                "--mode",
                "rpc",
                "--no-session",
                "--tools",
                "bash,edit,read,write",
                "--extension",
                "./pi-extension.js",
                "--skill",
                "./skills/review.md",
            ])
        );
    }

    #[test]
    fn policy_rejects_forbidden_tool_for_reader_profile() {
        let error = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: Some("repo-reader"),
            required_capabilities: &["command.run".to_owned()],
            workspace_policy: "shared",
            approval_mode: Some("manual"),
            provider: None,
            model: None,
            extension_refs: &[],
            skill_refs: &[],
        })
        .expect_err("reader command denied");

        assert_eq!(error.code, "profile_denied");
    }

    #[test]
    fn policy_rejects_write_in_read_only_workspace() {
        let error = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: Some("repo-writer"),
            required_capabilities: &["repo.write".to_owned()],
            workspace_policy: "read_only",
            approval_mode: Some("manual"),
            provider: None,
            model: None,
            extension_refs: &[],
            skill_refs: &[],
        })
        .expect_err("read only workspace denied");

        assert_eq!(error.code, "workspace_denied");
    }

    #[test]
    fn policy_rejects_unsupported_workspace_policy() {
        let error = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: Some("repo-writer"),
            required_capabilities: &["repo.write".to_owned()],
            workspace_policy: "remote_sandbox",
            approval_mode: Some("manual"),
            provider: None,
            model: None,
            extension_refs: &[],
            skill_refs: &[],
        })
        .expect_err("remote sandbox requires explicit workspace implementation");

        assert_eq!(error.code, "unsupported_workspace_policy");
    }

    #[test]
    fn policy_rejects_destructive_tool_without_approval() {
        let error = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: Some("repo-writer"),
            required_capabilities: &["repo.write".to_owned()],
            workspace_policy: "shared",
            approval_mode: None,
            provider: None,
            model: None,
            extension_refs: &[],
            skill_refs: &[],
        })
        .expect_err("approval required");

        assert_eq!(error.code, "missing_approval");
    }

    #[test]
    fn policy_rejects_invalid_model_provider_and_refs() {
        let provider_error = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: Some("repo-reader"),
            required_capabilities: &["repo.read".to_owned()],
            workspace_policy: "read_only",
            approval_mode: None,
            provider: Some("bad provider"),
            model: None,
            extension_refs: &[],
            skill_refs: &[],
        })
        .expect_err("provider invalid");
        assert_eq!(provider_error.code, "invalid_provider");

        let model_error = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: Some("repo-reader"),
            required_capabilities: &["repo.read".to_owned()],
            workspace_policy: "read_only",
            approval_mode: None,
            provider: None,
            model: Some(" "),
            extension_refs: &[],
            skill_refs: &[],
        })
        .expect_err("model invalid");
        assert_eq!(model_error.code, "invalid_model");

        let extension_error = build_pi_rpc_tool_policy(PiRpcPolicyInput {
            profile: Some("repo-reader"),
            required_capabilities: &["repo.read".to_owned()],
            workspace_policy: "read_only",
            approval_mode: None,
            provider: None,
            model: None,
            extension_refs: &["bad\nextension".to_owned()],
            skill_refs: &[],
        })
        .expect_err("extension invalid");
        assert_eq!(extension_error.code, "invalid_extension_ref");
    }

    #[test]
    fn native_adapter_starts_prompt_and_preserves_policy_payload_shape() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"get_state","success":true,"data":{"sessionId":"session-1","model":{"provider":"openai-codex","id":"gpt-5.5"},"isStreaming":false}}"#,
            r#"{"type":"turn_start","sessionId":"session-1","turnId":"turn-1"}"#,
            r#"{"id":"ws-2","type":"response","command":"prompt","success":true,"data":{}}"#,
        ]);
        let client = PiRpcClient::new(transport);
        let mut adapter = PiRpcAdapter::new("pi-main", pi_capability(), client);

        let event = adapter
            .start_turn(native_pi_request())
            .expect("turn starts");

        assert_eq!(event.event_kind, NativeProviderEventKind::Started);
        assert_eq!(event.provider_session_id.as_deref(), Some("session-1"));
        assert_eq!(event.provider_turn_id.as_deref(), Some("turn-1"));
        let transport = adapter.into_client().into_transport();
        let prompt: Value = serde_json::from_str(&transport.writes[1]).expect("prompt json");
        assert_eq!(prompt.get("type").and_then(Value::as_str), Some("prompt"));
        assert_eq!(
            prompt.get("message").and_then(Value::as_str),
            Some("inspect the repo")
        );
        assert_eq!(
            prompt.get("provider").and_then(Value::as_str),
            Some("openai-codex")
        );
        assert_eq!(prompt.get("model").and_then(Value::as_str), Some("gpt-5.5"));
        assert_eq!(
            prompt
                .get("tools")
                .and_then(Value::as_array)
                .expect("tools")
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec!["edit", "read", "write"]
        );
    }

    #[test]
    fn native_adapter_maps_pi_remote_prompt_error_without_raw_message() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"get_state","success":true,"data":{"sessionId":"session-1"}}"#,
            r#"{"id":"ws-2","type":"response","command":"prompt","success":false,"error":{"code":"model_denied","message":"token sk-never-print leaked"}}"#,
        ]);
        let client = PiRpcClient::new(transport);
        let mut adapter = PiRpcAdapter::new("pi-main", pi_capability(), client);

        let error = adapter
            .start_turn(native_pi_request())
            .expect_err("remote prompt error is mapped");

        assert_eq!(error.code, "pi_prompt_failed");
        assert!(error.recoverable);
        assert_eq!(
            error.evidence.get("kind").and_then(Value::as_str),
            Some("remote")
        );
        let redacted = error.to_json_redacted().to_string();
        assert!(!redacted.contains("sk-never-print"));
        assert!(!redacted.contains("model_denied"));
    }

    #[test]
    fn native_adapter_streams_pi_events_without_raw_payload_content() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"get_state","success":true,"data":{"sessionId":"session-1"}}"#,
            r#"{"id":"ws-2","type":"response","command":"prompt","success":true,"data":{}}"#,
            r#"{"type":"message_start","message":{"sessionId":"session-1","turnId":"turn-1","content":[{"text":"secret"}]}}"#,
            r#"{"type":"turn_end","message":{"sessionId":"session-1","turnId":"turn-1","stopReason":"done","content":[{"text":"secret final"}]}}"#,
        ]);
        let client = PiRpcClient::new(transport);
        let mut adapter = PiRpcAdapter::new("pi-main", pi_capability(), client);
        let start = adapter
            .start_turn(native_pi_request())
            .expect("turn starts");
        assert_eq!(start.event_kind, NativeProviderEventKind::Started);

        let streamed = adapter
            .next_event("run-1")
            .expect("event reads")
            .expect("stream event");
        assert_eq!(streamed.event_kind, NativeProviderEventKind::Streamed);
        assert!(!streamed.to_json_redacted().to_string().contains("secret"));

        let terminal = adapter
            .next_event("run-1")
            .expect("event reads")
            .expect("terminal event");
        assert_eq!(terminal.event_kind, NativeProviderEventKind::Completed);
        assert!(terminal.event_kind.is_terminal());
    }

    #[test]
    fn native_adapter_captures_pi_artifact_refs_without_raw_content() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"get_state","success":true,"data":{"sessionId":"session-1"}}"#,
            r#"{"id":"ws-2","type":"response","command":"prompt","success":true,"data":{}}"#,
            r#"{"type":"artifact","message":{"sessionId":"session-1","turnId":"turn-1","artifacts":[{"id":"artifact-1","kind":"file","path":"outputs/result.txt","mime":"text/plain","hash":"sha256:def456","required":true,"content":"secret artifact bytes"}]}}"#,
        ]);
        let client = PiRpcClient::new(transport);
        let mut adapter = PiRpcAdapter::new("pi-main", pi_capability(), client);
        adapter
            .start_turn(native_pi_request())
            .expect("turn starts");

        let artifact = adapter
            .next_event("run-1")
            .expect("event reads")
            .expect("artifact event");

        assert_eq!(
            artifact.event_kind,
            NativeProviderEventKind::ArtifactCaptured
        );
        assert_eq!(artifact.provider_session_id.as_deref(), Some("session-1"));
        assert_eq!(artifact.provider_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(artifact.artifacts.len(), 1);
        assert_eq!(
            artifact.artifacts[0].artifact_id.as_deref(),
            Some("artifact-1")
        );
        assert_eq!(artifact.artifacts[0].kind, "file");
        assert_eq!(
            artifact.artifacts[0].uri,
            "pi-artifact://outputs/result.txt"
        );
        assert_eq!(
            artifact.artifacts[0].content_hash.as_deref(),
            Some("sha256:def456")
        );
        assert!(artifact.artifacts[0].required);
        assert!(!artifact
            .to_json_redacted()
            .to_string()
            .contains("secret artifact bytes"));
    }

    #[test]
    fn native_adapter_abort_ack_is_non_terminal_until_pi_turn_end() {
        let transport = FakeTransport::with_reads(&[
            r#"{"id":"ws-1","type":"response","command":"get_state","success":true,"data":{"sessionId":"session-1"}}"#,
            r#"{"type":"turn_start","sessionId":"session-1","turnId":"turn-1"}"#,
            r#"{"id":"ws-2","type":"response","command":"prompt","success":true,"data":{}}"#,
            r#"{"id":"ws-3","type":"response","command":"abort","success":true,"data":{}}"#,
            r#"{"type":"turn_end","message":{"sessionId":"session-1","turnId":"turn-1","stopReason":"aborted"}}"#,
        ]);
        let client = PiRpcClient::new(transport);
        let mut adapter = PiRpcAdapter::new("pi-main", pi_capability(), client);
        adapter
            .start_turn(native_pi_request())
            .expect("turn starts");

        let ack = adapter
            .cancel_turn(NativeProviderCancellation {
                run_id: "run-1".to_owned(),
                provider_session_id: None,
                provider_turn_id: None,
                requested_depth: CancellationDepth::NativeStop,
                reason: "revision changed".to_owned(),
            })
            .expect("abort acknowledged");
        assert_eq!(ack.event_kind, NativeProviderEventKind::Diagnostic);
        assert!(!ack.event_kind.is_terminal());

        let terminal = adapter
            .next_event("run-1")
            .expect("event reads")
            .expect("terminal event");
        assert_eq!(terminal.event_kind, NativeProviderEventKind::Cancelled);

        let transport = adapter.into_client().into_transport();
        let abort: Value = serde_json::from_str(&transport.writes[2]).expect("abort json");
        assert_eq!(abort.get("type").and_then(Value::as_str), Some("abort"));
    }
}
