//! Live provider model client for the owned brokered harness (DR-0024).
//!
//! Generalizes the single-shot `coerce` client (`coerce_native`) into a
//! multi-turn tool-use client: it serializes the running conversation + the tool
//! specs into a provider request, posts it through the shared [`CoerceTransport`]
//! seam, and parses the reply into a normalized [`ModelReply`] (free text plus any
//! tool calls). The pure request-build / response-parse functions are
//! unit-testable with a fake transport; the CLI supplies the real `ureq`
//! transport and resolved credentials (live calls are credential-gated).
//!
//! Slice-1 scope: OpenAI Responses and Anthropic Messages (non-streaming). The
//! Codex OAuth SSE backend (function-call items over an event stream) is a
//! follow-on; `coerce_native`'s `assemble_responses_sse` is the starting point.

use serde_json::{json, Map, Value};

use crate::coerce_native::{
    CoerceProvider, CoerceTransport, CoerceTransportError, HttpRequest, HttpResponse,
};
use crate::harness_loop::{
    ChatMessage, HarnessModelClient, HarnessModelError, HttpModelClient, ModelCallMachine,
    ModelReply, ToolCall, ToolSpec,
};
use crate::sansio::run_to_completion;

/// Cap on a provider control-plane error string crossing into a turn failure
/// (matches the coerce path; DR-0024 lets operational errors cross redaction).
const PROVIDER_ERROR_CAP: usize = 300;

/// The context window (tokens) of a provider model, for the conversation-compaction
/// trigger (context-assembly Phase 4). This is a **model capability**, derived from
/// the provider + model id — never an operator config knob. The numbers are the
/// window whip's requests actually get (e.g. Claude is 200k standard; the 1M-context
/// beta requires an opt-in header whip does not send, so it is not claimed here).
/// Unknown models fall back to the conservative default.
pub fn model_context_window(provider: CoerceProvider, model: &str) -> u64 {
    let model = model.to_ascii_lowercase();
    match provider {
        // Claude models are 200k standard context.
        CoerceProvider::Anthropic => 200_000,
        CoerceProvider::OpenAi => {
            if model.contains("gpt-4.1") {
                1_000_000
            } else if model.contains("gpt-4o") || model.contains("gpt-4-turbo") {
                128_000
            } else if model.starts_with('o') || model.contains("-o1") || model.contains("-o3") {
                // o1 / o3 / o4 reasoning models.
                200_000
            } else {
                128_000
            }
        }
    }
}

/// A live model client over one provider API. The CLI builds this with a
/// `ureq`-backed transport and a resolved API key + model.
pub struct RealHarnessModelClient<'a, T: CoerceTransport + ?Sized> {
    transport: &'a T,
    provider: CoerceProvider,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u64,
    /// Stable cache key for this turn-thread (Decision 7): the run/effect id.
    /// Sent as `prompt_cache_key` on OpenAI; Anthropic caches by prefix hash
    /// (via `cache_control` breakpoints) and does not use it.
    cache_key: Option<String>,
}

impl<'a, T: CoerceTransport + ?Sized> RealHarnessModelClient<'a, T> {
    pub fn new(
        transport: &'a T,
        provider: CoerceProvider,
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        max_tokens: u64,
        cache_key: Option<String>,
    ) -> Self {
        Self {
            transport,
            provider,
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.into(),
            max_tokens,
            cache_key,
        }
    }
}

impl<T: CoerceTransport + ?Sized> HttpModelClient for RealHarnessModelClient<'_, T> {
    fn build_request(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> HttpRequest {
        build_request(
            self.provider,
            &self.base_url,
            &self.api_key,
            &self.model,
            self.max_tokens,
            self.cache_key.as_deref(),
            messages,
            tools,
        )
    }

    fn parse_response(
        &self,
        response: Result<HttpResponse, CoerceTransportError>,
    ) -> Result<ModelReply, HarnessModelError> {
        map_transport_response(self.provider, response)
    }

    fn context_window(&self) -> u64 {
        model_context_window(self.provider, &self.model)
    }
}

impl<T: CoerceTransport + ?Sized> HarnessModelClient for RealHarnessModelClient<'_, T> {
    fn next(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelReply, HarnessModelError> {
        // One model call as a sans-IO step machine: prepare (`build_request`) →
        // `NeedsIo(Http)` → finish (`parse_response`), driven to completion
        // synchronously via the transport. Identical to a direct
        // build_request → post → parse_response.
        let mut machine = ModelCallMachine::new(self, messages, tools);
        run_to_completion(&mut machine, self.transport)
    }
}

/// A model client that owns only the provider config — no transport — so a host
/// that performs the HTTP itself drives it purely (DR-0033). This is the agent
/// counterpart to coerce's `build_coerce_call_parts`: the durable-object host
/// builds one from its secrets plane and drives it through the `HttpModelClient`
/// trait (`build_request` → its own `fetch` → `parse_response`) via the
/// `BrokeredTurnMachine`, never calling `next`/`run_to_completion`. It shares the
/// exact request-build / response-parse logic the native
/// [`RealHarnessModelClient`] uses, so the wire format is identical across hosts.
pub struct MessagesApiClient {
    provider: CoerceProvider,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u64,
    /// Stable per-turn-thread cache key (Decision 7). The durable-object host
    /// constructs this client once per object (not per turn), so it has no
    /// per-effect id at construction and passes `None` for now; the Anthropic
    /// `cache_control` breakpoint still applies. Wiring the per-turn key through
    /// the DO agent path is a later-phase follow-up.
    cache_key: Option<String>,
}

impl MessagesApiClient {
    pub fn new(
        provider: CoerceProvider,
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        max_tokens: u64,
        cache_key: Option<String>,
    ) -> Self {
        Self {
            provider,
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.into(),
            max_tokens,
            cache_key,
        }
    }
}

impl HttpModelClient for MessagesApiClient {
    fn build_request(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> HttpRequest {
        build_request(
            self.provider,
            &self.base_url,
            &self.api_key,
            &self.model,
            self.max_tokens,
            self.cache_key.as_deref(),
            messages,
            tools,
        )
    }

    fn parse_response(
        &self,
        response: Result<HttpResponse, CoerceTransportError>,
    ) -> Result<ModelReply, HarnessModelError> {
        map_transport_response(self.provider, response)
    }

    fn context_window(&self) -> u64 {
        model_context_window(self.provider, &self.model)
    }
}

/// Map a transport outcome to a model reply: parse a delivered response, or lift a
/// transport failure to the matching [`HarnessModelError`]. Shared by every
/// [`HttpModelClient`] so the timeout/transport mapping cannot drift between hosts.
fn map_transport_response(
    provider: CoerceProvider,
    response: Result<HttpResponse, CoerceTransportError>,
) -> Result<ModelReply, HarnessModelError> {
    match response {
        Ok(response) => parse_response(provider, response.status, &response.body),
        Err(CoerceTransportError::Timeout) => Err(HarnessModelError::Timeout),
        Err(CoerceTransportError::Transport(message)) => Err(HarnessModelError::Transport(message)),
    }
}

// -- request construction -------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn build_request(
    provider: CoerceProvider,
    base_url: &str,
    api_key: &str,
    model: &str,
    max_tokens: u64,
    cache_key: Option<&str>,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> HttpRequest {
    match provider {
        CoerceProvider::Anthropic => {
            // Anthropic caches by prefix hash via `cache_control` breakpoints, so
            // the stable-key intent (Decision 7) is carried by the breakpoint, not
            // an explicit key.
            build_anthropic_request(base_url, api_key, model, max_tokens, messages, tools)
        }
        CoerceProvider::OpenAi => {
            build_openai_request(base_url, api_key, model, cache_key, messages, tools)
        }
    }
}

fn build_anthropic_request(
    base_url: &str,
    api_key: &str,
    model: &str,
    max_tokens: u64,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> HttpRequest {
    let (system, msgs) = anthropic_messages(messages);
    let tool_defs: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect();
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("max_tokens".into(), json!(max_tokens));
    if let Some(system) = system {
        // Cache breakpoint at the end of the system prompt (Decision 7). The
        // deterministic assembler makes [tools, system] a byte-stable prefix, so
        // marking the system block `ephemeral` caches that prefix and lets it be
        // reused across the turn's model steps. Messages append after the
        // breakpoint and are not part of this cached prefix.
        body.insert(
            "system".into(),
            json!([{
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" },
            }]),
        );
    }
    body.insert("messages".into(), json!(msgs));
    body.insert("tools".into(), json!(tool_defs));
    HttpRequest {
        url: format!("{base_url}/v1/messages"),
        headers: vec![
            ("x-api-key".into(), api_key.to_owned()),
            ("anthropic-version".into(), "2023-06-01".into()),
            ("content-type".into(), "application/json".into()),
        ],
        body: Value::Object(body),
    }
}

/// Serialize the conversation into Anthropic's (system, messages[]) shape.
fn anthropic_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<Value>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        match message {
            ChatMessage::System(text) => system_parts.push(text.clone()),
            ChatMessage::User { text, images } => {
                // Text-only user messages keep the plain-string content shape
                // (byte-stable requests → provider cache stability); images
                // switch to content blocks (pi-conformance §6).
                if images.is_empty() {
                    out.push(json!({ "role": "user", "content": text }));
                } else {
                    let mut content: Vec<Value> = vec![json!({ "type": "text", "text": text })];
                    for image in images {
                        content.push(json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": image.media_type,
                                "data": image.data_base64,
                            },
                        }));
                    }
                    out.push(json!({ "role": "user", "content": content }));
                }
            }
            ChatMessage::Assistant { text, tool_calls } => {
                let mut content: Vec<Value> = Vec::new();
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                for call in tool_calls {
                    content.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments,
                    }));
                }
                out.push(json!({ "role": "assistant", "content": content }));
            }
            ChatMessage::ToolResults(results) => {
                let content: Vec<Value> = results
                    .iter()
                    .map(|result| {
                        json!({
                            "type": "tool_result",
                            "tool_use_id": result.tool_call_id,
                            "content": result.content,
                            "is_error": result.is_error,
                        })
                    })
                    .collect();
                out.push(json!({ "role": "user", "content": content }));
            }
        }
    }
    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };
    (system, out)
}

fn build_openai_request(
    base_url: &str,
    api_key: &str,
    model: &str,
    cache_key: Option<&str>,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> HttpRequest {
    let input = openai_input(messages);
    let tool_defs: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            })
        })
        .collect();
    let mut body = json!({
        "model": model,
        "input": input,
        "tools": tool_defs,
    });
    if let Some(key) = cache_key {
        // Stable per-turn-thread cache key (Decision 7): the run/effect id, held
        // constant across the turn's model steps so the server serves the growing
        // request prefix from cache instead of re-reading it each round.
        body["prompt_cache_key"] = json!(key);
    }
    HttpRequest {
        url: format!("{base_url}/v1/responses"),
        headers: vec![
            ("authorization".into(), format!("Bearer {api_key}")),
            ("content-type".into(), "application/json".into()),
        ],
        body,
    }
}

/// Serialize the conversation into the OpenAI Responses `input[]` shape, mapping
/// assistant tool calls to `function_call` items and results to
/// `function_call_output` items (correlated by call id).
fn openai_input(messages: &[ChatMessage]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        match message {
            ChatMessage::System(text) => {
                out.push(json!({ "role": "system", "content": text }));
            }
            ChatMessage::User { text, images } => {
                // Text-only stays a plain string (cache stability); images use
                // Responses content parts with data-URL `input_image` entries
                // (pi-conformance §6).
                if images.is_empty() {
                    out.push(json!({ "role": "user", "content": text }));
                } else {
                    let mut content: Vec<Value> = vec![json!({
                        "type": "input_text",
                        "text": text,
                    })];
                    for image in images {
                        content.push(json!({
                            "type": "input_image",
                            "image_url": format!(
                                "data:{};base64,{}",
                                image.media_type, image.data_base64
                            ),
                        }));
                    }
                    out.push(json!({ "role": "user", "content": content }));
                }
            }
            ChatMessage::Assistant { text, tool_calls } => {
                if !text.is_empty() {
                    out.push(json!({ "role": "assistant", "content": text }));
                }
                for call in tool_calls {
                    out.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments.to_string(),
                    }));
                }
            }
            ChatMessage::ToolResults(results) => {
                for result in results {
                    // The Responses `function_call_output` item has no error flag
                    // (unlike Anthropic's `tool_result.is_error`), so a failed tool
                    // call is marked in-band: prefix the output text so the model
                    // sees the failure (pi-conformance §5).
                    let output = if result.is_error {
                        format!("error: {}", result.content)
                    } else {
                        result.content.clone()
                    };
                    out.push(json!({
                        "type": "function_call_output",
                        "call_id": result.tool_call_id,
                        "output": output,
                    }));
                }
            }
        }
    }
    out
}

// -- response parsing -----------------------------------------------------

fn parse_response(
    provider: CoerceProvider,
    status: u16,
    body: &Value,
) -> Result<ModelReply, HarnessModelError> {
    if !(200..300).contains(&status) {
        return Err(HarnessModelError::Provider(provider_error_excerpt(body)));
    }
    match provider {
        CoerceProvider::Anthropic => Ok(parse_anthropic_response(body)),
        CoerceProvider::OpenAi => Ok(parse_openai_response(body)),
    }
}

fn parse_anthropic_response(body: &Value) -> ModelReply {
    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    if let Some(blocks) = body.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(part) = block.get("text").and_then(Value::as_str) {
                        text.push_str(part);
                    }
                }
                Some("tool_use") => {
                    tool_calls.push(ToolCall {
                        id: block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        arguments: block.get("input").cloned().unwrap_or(Value::Null),
                    });
                }
                _ => {}
            }
        }
    }
    ModelReply {
        text,
        tool_calls,
        usage: body.get("usage").cloned().unwrap_or(Value::Null),
    }
}

fn parse_openai_response(body: &Value) -> ModelReply {
    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    if let Some(items) = body.get("output").and_then(Value::as_array) {
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("function_call") => {
                    tool_calls.push(ToolCall {
                        id: item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        arguments: item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                            .unwrap_or(Value::Null),
                    });
                }
                Some("message") => {
                    if let Some(content) = item.get("content").and_then(Value::as_array) {
                        for part in content {
                            if let Some(t) = part.get("text").and_then(Value::as_str) {
                                text.push_str(t);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Convenience field some responses include.
    if text.is_empty() {
        if let Some(t) = body.get("output_text").and_then(Value::as_str) {
            text.push_str(t);
        }
    }
    ModelReply {
        text,
        tool_calls,
        usage: body.get("usage").cloned().unwrap_or(Value::Null),
    }
}

/// Pull a capped, single-line control-plane error message from a provider body.
fn provider_error_excerpt(body: &Value) -> String {
    let message = body
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .unwrap_or("provider returned a non-success status");
    let mut excerpt: String = message.chars().take(PROVIDER_ERROR_CAP).collect();
    if message.chars().count() > PROVIDER_ERROR_CAP {
        excerpt.push('…');
    }
    excerpt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness_loop::ToolResultMsg;
    use std::cell::RefCell;

    #[test]
    fn context_window_is_derived_from_the_model_not_configured() {
        // Claude: 200k standard (the 1M beta is not claimed since whip does not send
        // the opt-in header).
        assert_eq!(
            model_context_window(CoerceProvider::Anthropic, "claude-opus-4-8"),
            200_000
        );
        // OpenAI families map to their real windows.
        assert_eq!(
            model_context_window(CoerceProvider::OpenAi, "gpt-4o"),
            128_000
        );
        assert_eq!(
            model_context_window(CoerceProvider::OpenAi, "gpt-4.1"),
            1_000_000
        );
        assert_eq!(model_context_window(CoerceProvider::OpenAi, "o3"), 200_000);
        // An unrecognized OpenAI model takes the conservative family default.
        assert_eq!(
            model_context_window(CoerceProvider::OpenAi, "some-future-model"),
            128_000
        );
    }

    struct FakeTransport {
        response: Result<HttpResponse, CoerceTransportError>,
        seen: RefCell<Option<HttpRequest>>,
    }

    impl CoerceTransport for FakeTransport {
        fn post(&self, request: &HttpRequest) -> Result<HttpResponse, CoerceTransportError> {
            *self.seen.borrow_mut() = Some(request.clone());
            self.response.clone()
        }
    }

    fn convo() -> Vec<ChatMessage> {
        vec![
            ChatMessage::System("be helpful".into()),
            ChatMessage::user_text("read the file"),
            ChatMessage::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: json!({ "path": "a.txt" }),
                }],
            },
            ChatMessage::ToolResults(vec![ToolResultMsg {
                tool_call_id: "call_1".into(),
                tool_name: "read".into(),
                content: "hello".into(),
                is_error: false,
            }]),
        ]
    }

    fn tool_specs() -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "read".into(),
            description: "read a file".into(),
            input_schema: json!({ "type": "object" }),
        }]
    }

    #[test]
    fn anthropic_request_shape_serializes_conversation_and_tools() {
        let req = build_anthropic_request(
            "https://api.anthropic.com",
            "sk-ant-api-key",
            "claude-opus-4-8",
            4096,
            &convo(),
            &tool_specs(),
        );
        assert_eq!(req.url, "https://api.anthropic.com/v1/messages");
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "x-api-key" && v == "sk-ant-api-key"));
        // System is a single text block carrying the end-of-prompt cache
        // breakpoint (Decision 7); the text the model sees is unchanged.
        assert_eq!(req.body["system"][0]["type"], json!("text"));
        assert_eq!(req.body["system"][0]["text"], json!("be helpful"));
        let msgs = req.body["messages"].as_array().expect("messages");
        // user, assistant(tool_use), user(tool_result)
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1]["role"], json!("assistant"));
        assert_eq!(msgs[1]["content"][0]["type"], json!("tool_use"));
        assert_eq!(msgs[1]["content"][0]["id"], json!("call_1"));
        assert_eq!(msgs[2]["content"][0]["type"], json!("tool_result"));
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], json!("call_1"));
        assert_eq!(req.body["tools"][0]["name"], json!("read"));
        // No forced tool_choice: the model chooses when to stop.
        assert!(req.body.get("tool_choice").is_none());
    }

    #[test]
    fn anthropic_parse_extracts_text_and_tool_calls() {
        let body = json!({
            "content": [
                { "type": "text", "text": "let me look" },
                { "type": "tool_use", "id": "tc1", "name": "read", "input": { "path": "x" } }
            ],
            "usage": { "output_tokens": 9 }
        });
        let reply = parse_anthropic_response(&body);
        assert_eq!(reply.text, "let me look");
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].name, "read");
        assert_eq!(reply.tool_calls[0].arguments, json!({ "path": "x" }));
        assert!(!reply.is_final());
    }

    #[test]
    fn openai_request_maps_tool_calls_and_results() {
        let req = build_openai_request(
            "https://api.openai.com",
            "sk-key",
            "gpt-5.5",
            None,
            &convo(),
            &tool_specs(),
        );
        assert_eq!(req.url, "https://api.openai.com/v1/responses");
        let input = req.body["input"].as_array().expect("input");
        // system, user, function_call, function_call_output
        assert!(input
            .iter()
            .any(|i| i["type"] == json!("function_call") && i["call_id"] == json!("call_1")));
        assert!(
            input
                .iter()
                .any(|i| i["type"] == json!("function_call_output")
                    && i["call_id"] == json!("call_1"))
        );
        assert_eq!(req.body["tools"][0]["type"], json!("function"));
    }

    #[test]
    fn openai_tool_result_error_is_marked_in_the_output_text() {
        // The Responses wire has no is_error field, so the failure marker rides
        // in-band; a successful result stays verbatim (pi-conformance §5).
        let messages = vec![ChatMessage::ToolResults(vec![
            ToolResultMsg {
                tool_call_id: "call_ok".into(),
                tool_name: "read".into(),
                content: "hello".into(),
                is_error: false,
            },
            ToolResultMsg {
                tool_call_id: "call_err".into(),
                tool_name: "read".into(),
                content: "read of `x` failed".into(),
                is_error: true,
            },
        ])];
        let input = openai_input(&messages);
        assert_eq!(input[0]["output"], json!("hello"));
        assert_eq!(input[1]["output"], json!("error: read of `x` failed"));
    }

    #[test]
    fn anthropic_user_images_emit_base64_source_blocks() {
        // pi-conformance §6: an image-bearing user message becomes content
        // blocks; a text-only one keeps the plain-string shape (cache stability).
        let messages = vec![
            ChatMessage::user_text("plain"),
            ChatMessage::User {
                text: "what is this?".into(),
                images: vec![crate::harness_loop::ImageBlock {
                    media_type: "image/png".into(),
                    data_base64: "aGVsbG8=".into(),
                }],
            },
        ];
        let (_, msgs) = anthropic_messages(&messages);
        assert_eq!(msgs[0]["content"], json!("plain"));
        assert_eq!(
            msgs[1]["content"][0],
            json!({ "type": "text", "text": "what is this?" })
        );
        assert_eq!(
            msgs[1]["content"][1],
            json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/png",
                    "data": "aGVsbG8=",
                },
            })
        );
    }

    #[test]
    fn openai_user_images_emit_input_image_parts() {
        // pi-conformance §6: Responses content parts with a data-URL
        // `input_image`; a text-only user message stays a plain string.
        let messages = vec![
            ChatMessage::user_text("plain"),
            ChatMessage::User {
                text: "what is this?".into(),
                images: vec![crate::harness_loop::ImageBlock {
                    media_type: "image/png".into(),
                    data_base64: "aGVsbG8=".into(),
                }],
            },
        ];
        let input = openai_input(&messages);
        assert_eq!(input[0]["content"], json!("plain"));
        assert_eq!(
            input[1]["content"][0],
            json!({ "type": "input_text", "text": "what is this?" })
        );
        assert_eq!(
            input[1]["content"][1],
            json!({
                "type": "input_image",
                "image_url": "data:image/png;base64,aGVsbG8=",
            })
        );
    }

    #[test]
    fn cache_breakpoints_and_stable_key_follow_decision_7() {
        // Anthropic: the system prompt is sent as a content block carrying a
        // `cache_control` breakpoint at its end (the stable [tools, system] prefix).
        let anthropic = build_anthropic_request(
            "https://api.anthropic.com",
            "k",
            "m",
            4096,
            &convo(),
            &tool_specs(),
        );
        let system = anthropic.body["system"]
            .as_array()
            .expect("system rendered as cache-controllable blocks");
        assert_eq!(
            system.last().expect("a system block")["cache_control"]["type"],
            json!("ephemeral")
        );

        // OpenAI: a stable per-turn-thread key rides as `prompt_cache_key` when
        // supplied, and is absent otherwise (no key => no field, not null).
        let with_key = build_openai_request(
            "https://api.openai.com",
            "k",
            "m",
            Some("turn-42"),
            &convo(),
            &tool_specs(),
        );
        assert_eq!(with_key.body["prompt_cache_key"], json!("turn-42"));
        let without_key = build_openai_request(
            "https://api.openai.com",
            "k",
            "m",
            None,
            &convo(),
            &tool_specs(),
        );
        assert!(without_key.body.get("prompt_cache_key").is_none());
    }

    #[test]
    fn openai_parse_extracts_function_call() {
        let body = json!({
            "output": [
                { "type": "function_call", "call_id": "c9", "name": "ls", "arguments": "{\"path\":\".\"}" }
            ]
        });
        let reply = parse_openai_response(&body);
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].id, "c9");
        assert_eq!(reply.tool_calls[0].arguments, json!({ "path": "." }));
    }

    #[test]
    fn non_success_status_is_a_provider_error() {
        let transport = FakeTransport {
            response: Ok(HttpResponse {
                status: 429,
                body: json!({ "error": { "message": "rate limit exceeded" } }),
            }),
            seen: RefCell::new(None),
        };
        let client = RealHarnessModelClient::new(
            &transport,
            CoerceProvider::Anthropic,
            "k",
            "m",
            "https://api.anthropic.com",
            4096,
            None,
        );
        let err = client
            .next(&convo(), &tool_specs())
            .expect_err("provider error");
        match err {
            HarnessModelError::Provider(message) => assert!(message.contains("rate limit")),
            other => panic!("expected provider error, got {other:?}"),
        }
    }

    #[test]
    fn timeout_maps_to_timeout() {
        let transport = FakeTransport {
            response: Err(CoerceTransportError::Timeout),
            seen: RefCell::new(None),
        };
        let client = RealHarnessModelClient::new(
            &transport,
            CoerceProvider::OpenAi,
            "k",
            "m",
            "https://api.openai.com",
            4096,
            None,
        );
        assert_eq!(
            client.next(&convo(), &tool_specs()),
            Err(HarnessModelError::Timeout)
        );
        // sanity: a request was actually built and sent
        assert!(transport.seen.borrow().is_some());
    }

    #[test]
    fn messages_api_client_builds_and_parses_without_a_transport() {
        // The durable-object path: no transport, the host does the fetch. The
        // config-only client must produce the same request and parse a reply.
        let client = MessagesApiClient::new(
            CoerceProvider::Anthropic,
            "sk-ant-key",
            "claude-opus-4-8",
            "https://api.anthropic.com",
            4096,
            None,
        );
        let request = client.build_request(&convo(), &tool_specs());
        assert_eq!(request.url, "https://api.anthropic.com/v1/messages");
        assert!(request
            .headers
            .iter()
            .any(|(k, v)| k == "x-api-key" && v == "sk-ant-key"));

        let reply = client
            .parse_response(Ok(HttpResponse {
                status: 200,
                body: json!({ "content": [ { "type": "text", "text": "final" } ] }),
            }))
            .expect("reply");
        assert_eq!(reply.text, "final");
        assert!(reply.is_final());

        assert_eq!(
            client.parse_response(Err(CoerceTransportError::Timeout)),
            Err(HarnessModelError::Timeout)
        );
    }

    #[test]
    fn final_reply_has_no_tool_calls() {
        let transport = FakeTransport {
            response: Ok(HttpResponse {
                status: 200,
                body: json!({ "content": [ { "type": "text", "text": "done" } ] }),
            }),
            seen: RefCell::new(None),
        };
        let client = RealHarnessModelClient::new(
            &transport,
            CoerceProvider::Anthropic,
            "k",
            "m",
            "https://api.anthropic.com",
            4096,
            None,
        );
        let reply = client.next(&convo(), &tool_specs()).expect("reply");
        assert_eq!(reply.text, "done");
        assert!(reply.is_final());
    }
}
