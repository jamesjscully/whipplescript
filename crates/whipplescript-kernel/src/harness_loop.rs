//! Owned brokered agent harness: the pure tool-use loop driver.
//!
//! Slice 1 of DR-0024 (see spec/owned-harness-loop-contract.md). This module is
//! the network-free, store-free *spine* of the brokered turn: it drives a model
//! through a tool-use loop where the KERNEL executes each requested tool (I1,
//! brokering) rather than delegating the whole turn to a provider harness.
//!
//! Two side effects are factored behind traits so this stays unit-testable:
//!   - [`HarnessModelClient`] makes one model call (the CLI supplies a real
//!     `ureq`-backed provider client; tests inject a fake).
//!   - [`ToolExecutor`] runs one tool request against the workspace (the CLI
//!     supplies the file-store-bounded executor; tests inject a fake).
//!
//! The driver returns a [`BrokeredTurnOutcome`] whose `observations` are the
//! in-turn stream events. Per the DR-0024 boundary corollary they are
//! evidence-grade only: the kernel runner records them as evidence and never
//! derives a rule-matchable fact from them (I2, leaf-ness). Only the single
//! terminal becomes a fact (layer 3).

use serde_json::{json, Value};

/// A model-facing tool: its name, a one-line description, and the JSON Schema for
/// its arguments. Built from the file-tool set in slice 1.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// One tool invocation the model requested.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCall {
    /// Provider-assigned call id, used to correlate the result back.
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Terminal status of a single tool execution. Anti-idempotence is intended: a
/// failed tool result is informative to the model (it retries), not a turn
/// failure (DR-0024 boundary corollary).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolStatus {
    Ok,
    Error,
}

/// The result of executing one tool, fed back to the model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolOutcome {
    pub status: ToolStatus,
    /// The content returned to the model (already bounded/truncated by the
    /// executor; full output is recoverable by event reference in later slices).
    pub content: String,
}

/// The single tool side effect: run one requested tool against the workspace.
/// The real impl lives in the CLI (file-store bounded); tests inject a fake.
pub trait ToolExecutor {
    fn execute(&self, call: &ToolCall) -> ToolOutcome;
}

/// One message in the model conversation the driver maintains.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChatMessage {
    System(String),
    User(String),
    /// An assistant turn: free text plus any tool calls it requested.
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    /// The results of the assistant's tool calls, correlated by call id.
    ToolResults(Vec<ToolResultMsg>),
}

/// A tool result as it appears back in the conversation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolResultMsg {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: String,
    pub is_error: bool,
}

/// One model reply: any text, any tool calls, and usage metadata. A reply with no
/// tool calls is final and ends the loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelReply {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Value,
}

impl ModelReply {
    pub fn is_final(&self) -> bool {
        self.tool_calls.is_empty()
    }
}

/// Why a model call did not produce a reply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessModelError {
    Timeout,
    /// A control-plane / provider error (usage-limit, auth, model-not-found). The
    /// message is operational metadata and may cross the redaction boundary
    /// (capped + scrubbed by the caller), per DR-0024.
    Provider(String),
    /// Any other transport-level failure (connect/TLS/decode), redacted message.
    Transport(String),
}

/// The single model side effect: one model call given the conversation so far and
/// the available tools. The real impl lives in the CLI; tests inject a fake.
pub trait HarnessModelClient {
    fn next(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelReply, HarnessModelError>;
}

/// An in-turn stream event. Evidence-grade only (I2): the kernel runner records
/// each as evidence; none derives a rule-matchable fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoopObservation {
    /// A model call was made at this 0-based step.
    ModelRequest { step: usize },
    /// The model requested a tool.
    ToolRequested { call_id: String, name: String },
    /// The kernel executed the tool and got this status.
    ToolResult {
        call_id: String,
        name: String,
        status: ToolStatus,
    },
}

/// Terminal status of a brokered turn (layer 3). Maps to the existing
/// agent.turn.* lifecycle terminal kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnStatus {
    Completed,
    Failed,
    TimedOut,
}

/// The outcome of a brokered turn: exactly one terminal, plus the in-turn stream
/// observations the runner records as evidence.
#[derive(Clone, Debug)]
pub struct BrokeredTurnOutcome {
    pub status: TurnStatus,
    /// Final model text on success; the error reason on failure/timeout.
    pub summary: String,
    /// Number of model calls made.
    pub steps: usize,
    pub observations: Vec<LoopObservation>,
    pub usage: Value,
}

/// The initial prompt for a brokered turn (slice 1 minimal projection: a system
/// prompt + the turn input as the first user message).
#[derive(Clone, Debug)]
pub struct BrokeredTurnInput {
    pub system: String,
    pub user: String,
    pub tools: Vec<ToolSpec>,
    /// Hard safety bound on model calls for slice 1. The governing budget
    /// (counter) is slice 2; this just prevents an unbounded loop.
    pub max_steps: usize,
    /// Resume-from-projection (slice 6): when non-empty, the loop continues from
    /// this persisted transcript instead of starting fresh from system+user. A
    /// dangling final tool-call (crash between request and result) is tolerated.
    pub resume_from: Vec<ChatMessage>,
}

/// Drive a brokered tool-use loop to a single terminal.
///
/// The loop is the model's control flow (I2/I3): each iteration makes one model
/// call, and for every requested tool the KERNEL executes it via `executor` and
/// feeds the result back (I1, brokering). The conversation grows by an assistant
/// message then a tool-results message each round. The loop ends when the model
/// replies with no tool calls (Completed), a model call errors (Failed), or the
/// step bound is hit (TimedOut).
pub fn run_brokered_loop<C, E>(
    client: &C,
    executor: &E,
    input: &BrokeredTurnInput,
    checkpoint: &mut dyn FnMut(&[ChatMessage]),
) -> BrokeredTurnOutcome
where
    C: HarnessModelClient + ?Sized,
    E: ToolExecutor + ?Sized,
{
    let mut messages = if input.resume_from.is_empty() {
        vec![
            ChatMessage::System(input.system.clone()),
            ChatMessage::User(input.user.clone()),
        ]
    } else {
        // Resume-from-projection: continue from the persisted transcript, dropping
        // a dangling final tool-call (a crash between request and result) so the
        // model re-decides rather than the loop deadlocking on an unanswered call.
        sanitize_resume_messages(input.resume_from.clone())
    };
    // Persist the (possibly resumed) starting context so a crash before the first
    // model call still leaves a transcript to resume from.
    checkpoint(&messages);
    let mut observations: Vec<LoopObservation> = Vec::new();
    let mut usage = Value::Null;

    for step in 0..input.max_steps {
        // Compact the projected context before each model call: the durable
        // observation stream is untouched (the runner records every step); only
        // what the model re-reads is bounded (DR-0024 boundary corollary).
        messages = compact_context(messages, COMPACT_MAX_MESSAGES, COMPACT_KEEP_RECENT);
        observations.push(LoopObservation::ModelRequest { step });
        let reply = match client.next(&messages, &input.tools) {
            Ok(reply) => reply,
            Err(error) => {
                return BrokeredTurnOutcome {
                    status: match error {
                        HarnessModelError::Timeout => TurnStatus::TimedOut,
                        _ => TurnStatus::Failed,
                    },
                    summary: model_error_summary(&error),
                    steps: step + 1,
                    observations,
                    usage,
                };
            }
        };
        usage = merge_usage(usage, reply.usage.clone());

        if reply.is_final() {
            return BrokeredTurnOutcome {
                status: TurnStatus::Completed,
                summary: reply.text,
                steps: step + 1,
                observations,
                usage,
            };
        }

        // The model requested tools: record the assistant turn, then broker each
        // tool through the kernel executor and feed the results back.
        messages.push(ChatMessage::Assistant {
            text: reply.text.clone(),
            tool_calls: reply.tool_calls.clone(),
        });
        let mut results = Vec::with_capacity(reply.tool_calls.len());
        for call in &reply.tool_calls {
            observations.push(LoopObservation::ToolRequested {
                call_id: call.id.clone(),
                name: call.name.clone(),
            });
            let outcome = executor.execute(call);
            observations.push(LoopObservation::ToolResult {
                call_id: call.id.clone(),
                name: call.name.clone(),
                status: outcome.status,
            });
            results.push(ToolResultMsg {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                content: outcome.content,
                is_error: matches!(outcome.status, ToolStatus::Error),
            });
        }
        messages.push(ChatMessage::ToolResults(results));
        // Persist the transcript after the step so a crash mid-turn leaves a
        // projection to resume from (DR-0024 resume-from-projection).
        checkpoint(&messages);
    }

    BrokeredTurnOutcome {
        status: TurnStatus::TimedOut,
        summary: format!("brokered turn exceeded {} model steps", input.max_steps),
        steps: input.max_steps,
        observations,
        usage,
    }
}

/// Compact when the projected context exceeds this many messages.
const COMPACT_MAX_MESSAGES: usize = 40;
/// Messages at the tail kept verbatim through compaction (the recent window).
const COMPACT_KEEP_RECENT: usize = 12;

/// Compact the projected context (DR-0024 boundary corollary, slice 5).
///
/// Two-tier eviction: when the context exceeds `max_messages`, older
/// `ToolResults` are elided to a short reference while the System message, the
/// first User message, and the last `keep_recent` messages are kept verbatim.
/// Tool *call* records (the assistant turns) are preserved so the model retains
/// what it did, only the bulky results are dropped. Idempotent: re-compacting an
/// already-elided context changes nothing (anti-thrashing). This transforms only
/// the model's working context; the durable observation stream is unaffected.
pub fn compact_context(
    messages: Vec<ChatMessage>,
    max_messages: usize,
    keep_recent: usize,
) -> Vec<ChatMessage> {
    let len = messages.len();
    if len <= max_messages {
        return messages;
    }
    let keep_from = len.saturating_sub(keep_recent);
    messages
        .into_iter()
        .enumerate()
        .map(|(index, message)| {
            // Anchors: the System message and the first User message (indices 0
            // and 1 in the loop's construction) and the recent tail survive.
            let is_anchor = index < 2;
            let in_recent_window = index >= keep_from;
            if is_anchor || in_recent_window {
                return message;
            }
            match message {
                ChatMessage::ToolResults(results) => {
                    ChatMessage::ToolResults(results.into_iter().map(elide_tool_result).collect())
                }
                other => other,
            }
        })
        .collect()
}

/// Serialize a transcript to JSON for durable persistence (no serde derive dep).
pub fn chat_messages_to_json(messages: &[ChatMessage]) -> Value {
    Value::Array(messages.iter().map(chat_message_to_json).collect())
}

fn chat_message_to_json(message: &ChatMessage) -> Value {
    match message {
        ChatMessage::System(text) => json!({ "role": "system", "text": text }),
        ChatMessage::User(text) => json!({ "role": "user", "text": text }),
        ChatMessage::Assistant { text, tool_calls } => json!({
            "role": "assistant",
            "text": text,
            "tool_calls": tool_calls
                .iter()
                .map(|call| json!({ "id": call.id, "name": call.name, "arguments": call.arguments }))
                .collect::<Vec<_>>(),
        }),
        ChatMessage::ToolResults(results) => json!({
            "role": "tool_results",
            "results": results
                .iter()
                .map(|result| json!({
                    "tool_call_id": result.tool_call_id,
                    "tool_name": result.tool_name,
                    "content": result.content,
                    "is_error": result.is_error,
                }))
                .collect::<Vec<_>>(),
        }),
    }
}

/// Parse a transcript persisted by [`chat_messages_to_json`]. Unknown shapes are
/// skipped (best-effort recovery).
pub fn chat_messages_from_json(value: &Value) -> Vec<ChatMessage> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(chat_message_from_json).collect())
        .unwrap_or_default()
}

fn chat_message_from_json(value: &Value) -> Option<ChatMessage> {
    let text = |v: &Value| {
        v.get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match value.get("role").and_then(Value::as_str)? {
        "system" => Some(ChatMessage::System(text(value))),
        "user" => Some(ChatMessage::User(text(value))),
        "assistant" => {
            let tool_calls = value
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(|calls| {
                    calls
                        .iter()
                        .map(|call| ToolCall {
                            id: call
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            name: call
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            arguments: call.get("arguments").cloned().unwrap_or(Value::Null),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(ChatMessage::Assistant {
                text: text(value),
                tool_calls,
            })
        }
        "tool_results" => {
            let results = value
                .get("results")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .map(|row| ToolResultMsg {
                            tool_call_id: row
                                .get("tool_call_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            tool_name: row
                                .get("tool_name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            content: row
                                .get("content")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            is_error: row
                                .get("is_error")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(ChatMessage::ToolResults(results))
        }
        _ => None,
    }
}

/// Prepare a resumed transcript: drop a dangling final assistant tool-call that
/// has no following results (a crash between request and execution), so the model
/// re-decides on resume instead of the loop waiting on an unanswered call.
/// Anti-idempotence makes this safe: a re-issued edit that already applied is
/// just an informative error.
fn sanitize_resume_messages(mut messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    if let Some(ChatMessage::Assistant { tool_calls, .. }) = messages.last() {
        if !tool_calls.is_empty() {
            messages.pop();
        }
    }
    messages
}

/// Replace a tool result's content with a short reference. Idempotent.
fn elide_tool_result(result: ToolResultMsg) -> ToolResultMsg {
    if result.content.starts_with("[elided") {
        return result;
    }
    let content = format!(
        "[elided: {} result, {} bytes — recoverable from the durable log]",
        result.tool_name,
        result.content.len()
    );
    ToolResultMsg { content, ..result }
}

fn model_error_summary(error: &HarnessModelError) -> String {
    match error {
        HarnessModelError::Timeout => "model call timed out".to_string(),
        HarnessModelError::Provider(message) => format!("provider error: {message}"),
        HarnessModelError::Transport(message) => format!("transport error: {message}"),
    }
}

/// Accumulate usage objects across model calls by summing shared numeric keys.
/// Non-numeric or absent keys fall back to the latest value.
fn merge_usage(acc: Value, next: Value) -> Value {
    match (acc, next) {
        (Value::Null, next) => next,
        (acc, Value::Null) => acc,
        (Value::Object(mut acc_map), Value::Object(next_map)) => {
            for (key, value) in next_map {
                let merged = match (acc_map.get(&key), &value) {
                    (Some(Value::Number(a)), Value::Number(b)) => match (a.as_i64(), b.as_i64()) {
                        (Some(a), Some(b)) => json!(a + b),
                        _ => value.clone(),
                    },
                    _ => value.clone(),
                };
                acc_map.insert(key, merged);
            }
            Value::Object(acc_map)
        }
        (_, next) => next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A fake model client that replays a scripted sequence of replies/errors.
    struct ScriptedClient {
        replies: RefCell<std::collections::VecDeque<Result<ModelReply, HarnessModelError>>>,
        seen_tools: RefCell<bool>,
    }

    impl ScriptedClient {
        fn new(replies: Vec<Result<ModelReply, HarnessModelError>>) -> Self {
            Self {
                replies: RefCell::new(replies.into_iter().collect()),
                seen_tools: RefCell::new(false),
            }
        }
    }

    impl HarnessModelClient for ScriptedClient {
        fn next(
            &self,
            _messages: &[ChatMessage],
            tools: &[ToolSpec],
        ) -> Result<ModelReply, HarnessModelError> {
            if !tools.is_empty() {
                *self.seen_tools.borrow_mut() = true;
            }
            self.replies
                .borrow_mut()
                .pop_front()
                .expect("scripted client ran out of replies")
        }
    }

    /// A fake executor recording every brokered call.
    struct RecordingExecutor {
        calls: RefCell<Vec<ToolCall>>,
        outcome: ToolOutcome,
    }

    impl RecordingExecutor {
        fn new(outcome: ToolOutcome) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                outcome,
            }
        }
    }

    impl ToolExecutor for RecordingExecutor {
        fn execute(&self, call: &ToolCall) -> ToolOutcome {
            self.calls.borrow_mut().push(call.clone());
            self.outcome.clone()
        }
    }

    fn final_reply(text: &str) -> ModelReply {
        ModelReply {
            text: text.to_string(),
            tool_calls: Vec::new(),
            usage: json!({ "output_tokens": 5 }),
        }
    }

    fn tool_reply(id: &str, name: &str) -> ModelReply {
        ModelReply {
            text: String::new(),
            tool_calls: vec![ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: json!({ "path": "README.md" }),
            }],
            usage: json!({ "output_tokens": 3 }),
        }
    }

    fn input(max_steps: usize) -> BrokeredTurnInput {
        BrokeredTurnInput {
            system: "you are a coding agent".to_string(),
            user: "read the readme".to_string(),
            tools: vec![ToolSpec {
                name: "read".to_string(),
                description: "read a file".to_string(),
                input_schema: json!({ "type": "object" }),
            }],
            max_steps,
            resume_from: Vec::new(),
        }
    }

    /// A no-op checkpoint for tests that do not exercise persistence.
    fn no_checkpoint() -> impl FnMut(&[ChatMessage]) {
        |_messages: &[ChatMessage]| {}
    }

    #[test]
    fn completes_without_tools() {
        let client = ScriptedClient::new(vec![Ok(final_reply("done"))]);
        let exec = RecordingExecutor::new(ToolOutcome {
            status: ToolStatus::Ok,
            content: String::new(),
        });
        let outcome = run_brokered_loop(&client, &exec, &input(8), &mut no_checkpoint());
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(outcome.summary, "done");
        assert_eq!(outcome.steps, 1);
        assert!(exec.calls.borrow().is_empty());
        // One model_request observation, no tool observations.
        assert_eq!(
            outcome.observations,
            vec![LoopObservation::ModelRequest { step: 0 }]
        );
        // The model was offered the tools.
        assert!(*client.seen_tools.borrow());
    }

    #[test]
    fn brokers_a_tool_then_completes() {
        let client = ScriptedClient::new(vec![
            Ok(tool_reply("call_1", "read")),
            Ok(final_reply("the readme says hi")),
        ]);
        let exec = RecordingExecutor::new(ToolOutcome {
            status: ToolStatus::Ok,
            content: "hi".to_string(),
        });
        let outcome = run_brokered_loop(&client, &exec, &input(8), &mut no_checkpoint());

        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(outcome.summary, "the readme says hi");
        assert_eq!(outcome.steps, 2);

        // Brokering (I1): the executor ran exactly the requested tool.
        let calls = exec.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read");

        // The observation stream: a tool_result is always preceded by its
        // tool_requested, and both sit between model_requests.
        assert_eq!(
            outcome.observations,
            vec![
                LoopObservation::ModelRequest { step: 0 },
                LoopObservation::ToolRequested {
                    call_id: "call_1".to_string(),
                    name: "read".to_string(),
                },
                LoopObservation::ToolResult {
                    call_id: "call_1".to_string(),
                    name: "read".to_string(),
                    status: ToolStatus::Ok,
                },
                LoopObservation::ModelRequest { step: 1 },
            ]
        );
    }

    #[test]
    fn brokering_invariant_every_result_follows_a_request() {
        // Two tool calls in one reply, then finish: each result must be preceded
        // by its request, and every result corresponds to an executor call.
        let client = ScriptedClient::new(vec![
            Ok(ModelReply {
                text: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "a".into(),
                        name: "read".into(),
                        arguments: json!({}),
                    },
                    ToolCall {
                        id: "b".into(),
                        name: "ls".into(),
                        arguments: json!({}),
                    },
                ],
                usage: Value::Null,
            }),
            Ok(final_reply("ok")),
        ]);
        let exec = RecordingExecutor::new(ToolOutcome {
            status: ToolStatus::Ok,
            content: String::new(),
        });
        let outcome = run_brokered_loop(&client, &exec, &input(8), &mut no_checkpoint());
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(exec.calls.borrow().len(), 2);

        let mut pending: std::collections::HashSet<String> = std::collections::HashSet::new();
        for obs in &outcome.observations {
            match obs {
                LoopObservation::ToolRequested { call_id, .. } => {
                    pending.insert(call_id.clone());
                }
                LoopObservation::ToolResult { call_id, .. } => {
                    assert!(
                        pending.contains(call_id),
                        "tool_result for {call_id} had no preceding tool_requested"
                    );
                }
                LoopObservation::ModelRequest { .. } => {}
            }
        }
    }

    #[test]
    fn provider_error_fails_the_turn() {
        let client =
            ScriptedClient::new(vec![Err(HarnessModelError::Provider("usage limit".into()))]);
        let exec = RecordingExecutor::new(ToolOutcome {
            status: ToolStatus::Ok,
            content: String::new(),
        });
        let outcome = run_brokered_loop(&client, &exec, &input(8), &mut no_checkpoint());
        assert_eq!(outcome.status, TurnStatus::Failed);
        assert!(outcome.summary.contains("usage limit"));
    }

    #[test]
    fn timeout_error_times_out_the_turn() {
        let client = ScriptedClient::new(vec![Err(HarnessModelError::Timeout)]);
        let exec = RecordingExecutor::new(ToolOutcome {
            status: ToolStatus::Ok,
            content: String::new(),
        });
        let outcome = run_brokered_loop(&client, &exec, &input(8), &mut no_checkpoint());
        assert_eq!(outcome.status, TurnStatus::TimedOut);
    }

    #[test]
    fn exhausting_steps_times_out() {
        // The model keeps requesting tools forever; the step bound must stop it.
        let client = ScriptedClient::new(vec![
            Ok(tool_reply("c1", "read")),
            Ok(tool_reply("c2", "read")),
        ]);
        let exec = RecordingExecutor::new(ToolOutcome {
            status: ToolStatus::Ok,
            content: String::new(),
        });
        let outcome = run_brokered_loop(&client, &exec, &input(2), &mut no_checkpoint());
        assert_eq!(outcome.status, TurnStatus::TimedOut);
        assert_eq!(outcome.steps, 2);
    }

    fn tool_result(tag: &str) -> ChatMessage {
        ChatMessage::ToolResults(vec![ToolResultMsg {
            tool_call_id: tag.to_string(),
            tool_name: "read".to_string(),
            content: format!("big content for {tag}"),
            is_error: false,
        }])
    }

    #[test]
    fn compaction_is_a_noop_under_threshold() {
        let messages = vec![
            ChatMessage::System("s".into()),
            ChatMessage::User("u".into()),
            tool_result("a"),
        ];
        let out = compact_context(messages.clone(), 40, 12);
        assert_eq!(out, messages);
    }

    #[test]
    fn compaction_elides_old_tool_results_but_keeps_anchors_and_recent() {
        let mut messages = vec![
            ChatMessage::System("s".into()),
            ChatMessage::User("u".into()),
        ];
        for i in 0..30 {
            messages.push(tool_result(&format!("r{i}")));
        }
        let len = messages.len();
        let out = compact_context(messages, 10, 5);
        assert_eq!(out.len(), len, "compaction elides content, not messages");

        // System + first User anchors are verbatim.
        assert_eq!(out[0], ChatMessage::System("s".into()));
        assert_eq!(out[1], ChatMessage::User("u".into()));

        // A middle (old) tool result is elided to a reference.
        if let ChatMessage::ToolResults(results) = &out[4] {
            assert!(results[0].content.starts_with("[elided"));
        } else {
            panic!("expected tool results at index 4");
        }

        // The most recent entry is kept verbatim.
        if let ChatMessage::ToolResults(results) = out.last().expect("last") {
            assert!(results[0].content.starts_with("big content"));
        } else {
            panic!("expected tool results at the tail");
        }
    }

    #[test]
    fn compaction_is_idempotent_anti_thrashing() {
        let mut messages = vec![
            ChatMessage::System("s".into()),
            ChatMessage::User("u".into()),
        ];
        for i in 0..30 {
            messages.push(tool_result(&format!("r{i}")));
        }
        let once = compact_context(messages, 10, 5);
        let twice = compact_context(once.clone(), 10, 5);
        assert_eq!(once, twice, "re-compacting an elided context is a no-op");
    }

    #[test]
    fn resume_continues_from_persisted_transcript_without_rerunning_tools() {
        let client = ScriptedClient::new(vec![Ok(final_reply("resumed and done"))]);
        let exec = RecordingExecutor::new(ToolOutcome {
            status: ToolStatus::Ok,
            content: String::new(),
        });
        let mut input = input(8);
        input.resume_from = vec![
            ChatMessage::System("s".into()),
            ChatMessage::User("u".into()),
            ChatMessage::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    id: "1".into(),
                    name: "read".into(),
                    arguments: json!({}),
                }],
            },
            ChatMessage::ToolResults(vec![ToolResultMsg {
                tool_call_id: "1".into(),
                tool_name: "read".into(),
                content: "x".into(),
                is_error: false,
            }]),
        ];
        let outcome = run_brokered_loop(&client, &exec, &input, &mut no_checkpoint());
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(outcome.summary, "resumed and done");
        // The already-applied tool is NOT re-run on resume.
        assert!(exec.calls.borrow().is_empty());
    }

    #[test]
    fn resume_drops_a_dangling_tool_call() {
        let messages = vec![
            ChatMessage::System("s".into()),
            ChatMessage::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    id: "1".into(),
                    name: "read".into(),
                    arguments: json!({}),
                }],
            },
        ];
        let out = sanitize_resume_messages(messages);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], ChatMessage::System(_)));
    }

    #[test]
    fn transcript_json_round_trips() {
        let messages = vec![
            ChatMessage::System("s".into()),
            ChatMessage::User("u".into()),
            ChatMessage::Assistant {
                text: "t".into(),
                tool_calls: vec![ToolCall {
                    id: "1".into(),
                    name: "read".into(),
                    arguments: json!({ "path": "a" }),
                }],
            },
            ChatMessage::ToolResults(vec![ToolResultMsg {
                tool_call_id: "1".into(),
                tool_name: "read".into(),
                content: "c".into(),
                is_error: false,
            }]),
        ];
        let json = chat_messages_to_json(&messages);
        assert_eq!(chat_messages_from_json(&json), messages);
    }

    #[test]
    fn checkpoint_is_invoked_during_the_loop() {
        let client =
            ScriptedClient::new(vec![Ok(tool_reply("c1", "read")), Ok(final_reply("done"))]);
        let exec = RecordingExecutor::new(ToolOutcome {
            status: ToolStatus::Ok,
            content: "r".into(),
        });
        let count = std::cell::Cell::new(0usize);
        let mut checkpoint = |_messages: &[ChatMessage]| count.set(count.get() + 1);
        let outcome = run_brokered_loop(&client, &exec, &input(8), &mut checkpoint);
        assert_eq!(outcome.status, TurnStatus::Completed);
        // Once for the initial context, once after the tool round.
        assert!(count.get() >= 2, "checkpoint should fire per step");
    }

    #[test]
    fn merge_usage_sums_shared_numeric_keys() {
        let merged = merge_usage(
            json!({ "input_tokens": 10, "output_tokens": 5 }),
            json!({ "input_tokens": 3, "output_tokens": 7 }),
        );
        assert_eq!(merged, json!({ "input_tokens": 13, "output_tokens": 12 }));
    }
}
