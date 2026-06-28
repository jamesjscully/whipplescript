//! Provider-native structured-output `coerce` client.
//!
//! This is the real LLM integration behind the `coerce` effect (the fixture
//! path stays in `coerce.rs`). It calls each provider's NATIVE structured-output
//! feature directly:
//!
//! - **OpenAI**: POST the Responses endpoint with a `text.format` JSON-schema
//!   constraint (`type: "json_schema"`).
//! - **Anthropic**: POST the Messages endpoint with a single tool whose
//!   `input_schema` is the output schema and a forced `tool_choice` (tool-use
//!   forces a structured argument object).
//!
//! Everything here is pure and unit/mock-testable: request construction and
//! response parsing are plain functions, and the only side effect — the socket
//! write — lives behind the [`CoerceTransport`] trait. The CLI supplies the real
//! `ureq`-backed transport (kernel stays network-free); tests inject a fake
//! transport. The live network call is credential-gated (see `whip auth`).

use crate::coerce::{CoerceClient, CoerceRequest, CoerceResult, CoerceStatus};
use serde_json::{json, Map, Value};
use whipplescript_parser::{IrClassField, IrPrimitiveType, IrSchema, IrType};

/// Which provider API the native client targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoerceProvider {
    OpenAi,
    Anthropic,
}

impl CoerceProvider {
    /// Default API base URL (overridable for the Codex backend or a mock).
    pub fn default_base_url(self) -> &'static str {
        match self {
            CoerceProvider::OpenAi => "https://api.openai.com",
            CoerceProvider::Anthropic => "https://api.anthropic.com",
        }
    }
}

/// A transport-agnostic HTTP request. The kernel builds these; the CLI's
/// `ureq` transport executes them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

/// A transport-agnostic HTTP response (status code + decoded JSON body).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Value,
}

/// Why a transport call did not yield a response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoerceTransportError {
    /// The request exceeded its deadline.
    Timeout,
    /// Any other transport-level failure (connect/TLS/decode), redacted message.
    Transport(String),
}

/// The single side effect: POST a request and decode a JSON response. The real
/// impl lives in the CLI (`ureq`); tests inject a fake.
pub trait CoerceTransport {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, CoerceTransportError>;
}

// -- JSON Schema synthesis ------------------------------------------------

/// Bound on schema recursion so a cyclic class ref (`class Node { child Node }`)
/// cannot infinitely recurse; beyond it the subtree is left unconstrained.
const MAX_SCHEMA_DEPTH: usize = 16;

/// Build a JSON Schema `Value` for a whip `IrType`, resolving `Ref`s against the
/// program's schema registry. Used to constrain the provider's structured output.
pub fn json_schema_for_type(ty: &IrType, schemas: &[IrSchema]) -> Value {
    schema_with_depth(ty, schemas, 0)
}

fn schema_with_depth(ty: &IrType, schemas: &[IrSchema], depth: usize) -> Value {
    if depth > MAX_SCHEMA_DEPTH {
        return json!({});
    }
    match ty {
        IrType::Primitive(primitive) => primitive_schema(primitive),
        IrType::LiteralString(literal) => json!({ "type": "string", "const": literal }),
        IrType::Ref(name) => schema_for_ref(name, schemas, depth),
        // An AgentRef is a string-named agent at the boundary.
        IrType::AgentRef(agents) => {
            json!({ "type": "string", "enum": agents.clone() })
        }
        IrType::Object(fields) => object_schema(fields, schemas, depth),
        // `Missing`/optional: allow the value to be null so absence validates
        // even though strict mode keeps the property `required`.
        IrType::Optional(inner) => nullable(schema_with_depth(inner, schemas, depth + 1)),
        IrType::Array(inner) => json!({
            "type": "array",
            "items": schema_with_depth(inner, schemas, depth + 1),
        }),
        // A map is an object with homogeneous additional properties.
        IrType::Map(value_ty) => json!({
            "type": "object",
            "additionalProperties": schema_with_depth(value_ty, schemas, depth + 1),
        }),
        IrType::Union(variants) => union_schema(variants, schemas, depth),
    }
}

/// A union of only string literals collapses to a single `enum` (cleaner and
/// more widely supported than `anyOf` of `const`s); a mixed union stays `anyOf`.
fn union_schema(variants: &[IrType], schemas: &[IrSchema], depth: usize) -> Value {
    let literals: Option<Vec<&str>> = variants
        .iter()
        .map(|variant| match variant {
            IrType::LiteralString(literal) => Some(literal.as_str()),
            _ => None,
        })
        .collect();
    match literals {
        Some(values) if !values.is_empty() => json!({ "type": "string", "enum": values }),
        _ => json!({
            "anyOf": variants
                .iter()
                .map(|variant| schema_with_depth(variant, schemas, depth + 1))
                .collect::<Vec<_>>(),
        }),
    }
}

fn primitive_schema(primitive: &IrPrimitiveType) -> Value {
    match primitive {
        IrPrimitiveType::String => json!({ "type": "string" }),
        IrPrimitiveType::Int => json!({ "type": "integer" }),
        IrPrimitiveType::Float => json!({ "type": "number" }),
        IrPrimitiveType::Bool => json!({ "type": "boolean" }),
        IrPrimitiveType::Null => json!({ "type": "null" }),
        // Durations/times serialize as strings at the boundary; media types are
        // out-of-band references the model returns as strings.
        IrPrimitiveType::Duration
        | IrPrimitiveType::Time
        | IrPrimitiveType::Image
        | IrPrimitiveType::Audio
        | IrPrimitiveType::Pdf
        | IrPrimitiveType::Video => json!({ "type": "string" }),
    }
}

fn schema_for_ref(name: &str, schemas: &[IrSchema], depth: usize) -> Value {
    for schema in schemas {
        match schema {
            IrSchema::Enum(enum_decl) if enum_decl.name == name => {
                // A payload-carrying (sum-type) enum lowers each data variant to a
                // generated class `<Enum>.<Variant>` (with a synthesized `variant`
                // const discriminant). Emit `anyOf` over the variants so the provider
                // can construct payloads, not just a bare variant name; a variant with
                // no generated class (a bare tag) stays a string `const`. A fully-bare
                // enum collapses to a single string `enum` (cleaner, widely supported).
                let mut alternatives = Vec::with_capacity(enum_decl.variants.len());
                let mut any_payload = false;
                for variant in &enum_decl.variants {
                    let class_name = format!("{}.{}", enum_decl.name, variant);
                    match find_class_fields(&class_name, schemas) {
                        Some(fields) => {
                            any_payload = true;
                            alternatives.push(object_schema(fields, schemas, depth + 1));
                        }
                        None => alternatives.push(json!({ "type": "string", "const": variant })),
                    }
                }
                return if any_payload {
                    json!({ "anyOf": alternatives })
                } else {
                    json!({ "type": "string", "enum": enum_decl.variants.clone() })
                };
            }
            IrSchema::Class(class_decl) if class_decl.name == name => {
                return object_schema(&class_decl.fields, schemas, depth);
            }
            _ => {}
        }
    }
    // Unknown ref (e.g. a built-in): leave unconstrained rather than reject — the
    // compiler already validates references, so this is a defensive fallback.
    json!({})
}

/// Look up a class declaration's fields by name (used to resolve a sum-type enum's
/// generated `<Enum>.<Variant>` payload classes).
fn find_class_fields<'a>(name: &str, schemas: &'a [IrSchema]) -> Option<&'a [IrClassField]> {
    schemas.iter().find_map(|schema| match schema {
        IrSchema::Class(class_decl) if class_decl.name == name => {
            Some(class_decl.fields.as_slice())
        }
        _ => None,
    })
}

fn object_schema(fields: &[IrClassField], schemas: &[IrSchema], depth: usize) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in fields {
        properties.insert(
            field.name.clone(),
            schema_with_depth(&field.ty, schemas, depth + 1),
        );
        // Strict structured-output mode (OpenAI `strict: true`) requires every
        // property in `required`; a genuinely-optional field expresses absence
        // through nullability (`anyOf [T, null]`, see the `Optional` arm), not by
        // omission from `required`.
        required.push(Value::String(field.name.clone()));
    }
    json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": required,
        "additionalProperties": false,
    })
}

fn nullable(schema: Value) -> Value {
    json!({ "anyOf": [schema, { "type": "null" }] })
}

/// The provider structured-output root must be a JSON object. For an output type
/// that is not object-shaped (enum, primitive, array, union), wrap it in a
/// single `value` property; the caller unwraps `.value` on parse.
///
/// Returns `(object_schema, wrapped)`.
pub fn output_schema_envelope(ty: &IrType, schemas: &[IrSchema]) -> (Value, bool) {
    let resolved_object = match ty {
        IrType::Object(_) => true,
        IrType::Ref(name) => schema_ref_is_class(name, schemas),
        _ => false,
    };
    if resolved_object {
        (json_schema_for_type(ty, schemas), false)
    } else {
        let inner = json_schema_for_type(ty, schemas);
        let wrapped = json!({
            "type": "object",
            "properties": { "value": inner },
            "required": ["value"],
            "additionalProperties": false,
        });
        (wrapped, true)
    }
}

fn schema_ref_is_class(name: &str, schemas: &[IrSchema]) -> bool {
    schemas.iter().any(|schema| match schema {
        IrSchema::Class(class_decl) => class_decl.name == name,
        IrSchema::Enum(_) => false,
    })
}

// -- Request construction (pure) -----------------------------------------

/// Codex-backend (ChatGPT-plan OAuth) parameters. When present on an OpenAI
/// call, the request targets `chatgpt.com/backend-api/codex/responses` with the
/// codex headers and an SSE-streamed Responses body, instead of the standard
/// `api.openai.com/v1/responses` JSON endpoint. Validated 2026-06-23: the codex
/// backend honors `text.format` json_schema structured outputs.
#[derive(Clone, Copy, Debug)]
pub struct CodexAuth<'a> {
    pub account_id: &'a str,
    pub session_id: &'a str,
}

/// Inputs needed to build a provider request, independent of transport/creds.
#[derive(Clone, Debug)]
pub struct CoerceCall<'a> {
    pub provider: CoerceProvider,
    pub base_url: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    /// Fully interpolated prompt (arguments already substituted).
    pub prompt: &'a str,
    /// Object-rooted output schema (see [`output_schema_envelope`]).
    pub output_schema: &'a Value,
    /// A stable name for the schema/tool (the output type name).
    pub schema_name: &'a str,
    /// Upper bound on output tokens (Anthropic requires `max_tokens`).
    pub max_tokens: u32,
    /// When set (OpenAI + a ChatGPT-plan OAuth token), use the codex backend.
    pub codex: Option<CodexAuth<'a>>,
}

/// Build the HTTP request for a coerce call. Auth headers are included so the
/// transport stays a dumb pipe.
pub fn build_request(call: &CoerceCall<'_>) -> HttpRequest {
    match call.provider {
        CoerceProvider::OpenAi => match call.codex {
            Some(codex) => build_codex_request(call, codex),
            None => build_openai_request(call),
        },
        CoerceProvider::Anthropic => build_anthropic_request(call),
    }
}

/// Codex-backend request: the Responses-API body the codex CLI sends, with the
/// `text.format` structured-output constraint. Always streams (the endpoint is
/// SSE); the transport assembles the `response.completed` payload.
fn build_codex_request(call: &CoerceCall<'_>, codex: CodexAuth<'_>) -> HttpRequest {
    let body = json!({
        "model": call.model,
        "instructions": "Respond only with a value matching the required output schema.",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": call.prompt }],
        }],
        "tools": [],
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "store": false,
        "stream": true,
        "text": {
            "format": {
                "type": "json_schema",
                "name": sanitize_name(call.schema_name),
                "schema": call.output_schema,
                "strict": is_strict_compatible(call.output_schema),
            }
        },
    });
    HttpRequest {
        url: format!(
            "{}/backend-api/codex/responses",
            call.base_url.trim_end_matches('/')
        ),
        headers: vec![
            (
                "authorization".to_owned(),
                format!("Bearer {}", call.api_key),
            ),
            ("chatgpt-account-id".to_owned(), codex.account_id.to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("accept".to_owned(), "text/event-stream".to_owned()),
            (
                "openai-beta".to_owned(),
                "responses=experimental".to_owned(),
            ),
            ("originator".to_owned(), "codex_cli_rs".to_owned()),
            ("session_id".to_owned(), codex.session_id.to_owned()),
        ],
        body,
    }
}

fn build_openai_request(call: &CoerceCall<'_>) -> HttpRequest {
    let body = json!({
        "model": call.model,
        "input": [{ "role": "user", "content": call.prompt }],
        "text": {
            "format": {
                "type": "json_schema",
                "name": sanitize_name(call.schema_name),
                "schema": call.output_schema,
                // Strict mode (guaranteed schema adherence) requires every object
                // to have `additionalProperties: false`; a whip `Map` lowers to a
                // schema-valued `additionalProperties`, which strict mode forbids,
                // so enable strict only when the schema is compatible.
                "strict": is_strict_compatible(call.output_schema),
            }
        },
    });
    HttpRequest {
        url: format!("{}/v1/responses", call.base_url.trim_end_matches('/')),
        headers: vec![
            (
                "authorization".to_owned(),
                format!("Bearer {}", call.api_key),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ],
        body,
    }
}

fn build_anthropic_request(call: &CoerceCall<'_>) -> HttpRequest {
    let tool_name = format!("emit_{}", sanitize_name(call.schema_name));
    let body = json!({
        "model": call.model,
        "max_tokens": call.max_tokens,
        "tools": [{
            "name": tool_name,
            "description": "Return the coerced value as structured arguments.",
            "input_schema": call.output_schema,
        }],
        "tool_choice": { "type": "tool", "name": tool_name },
        "messages": [{ "role": "user", "content": call.prompt }],
    });
    // Anthropic coerce uses a console API key only (`x-api-key`). Reusing a
    // Claude Code OAuth token (`sk-ant-oat*`) for the API is a terms gray area
    // (decided 2026-06-23, Jack), so the credential resolver rejects those before
    // we get here — this path always carries a real key.
    HttpRequest {
        url: format!("{}/v1/messages", call.base_url.trim_end_matches('/')),
        headers: vec![
            ("x-api-key".to_owned(), call.api_key.to_owned()),
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
        ],
        body,
    }
}

/// Whether a token is a Claude Code / `ant auth login` OAuth token (`sk-ant-oat*`)
/// rather than a console API key. Anthropic coerce requires a console key, so the
/// credential resolver uses this to reject OAuth tokens with a clear message.
pub fn is_anthropic_oauth_token(token: &str) -> bool {
    token.starts_with("sk-ant-oat")
}

/// Whether a JSON Schema satisfies OpenAI strict structured-output rules: every
/// object's `additionalProperties` must be `false` (never a sub-schema). A whip
/// `Map` produces a schema-valued `additionalProperties`, which is incompatible.
fn is_strict_compatible(schema: &Value) -> bool {
    match schema {
        Value::Object(object) => {
            if let Some(additional) = object.get("additionalProperties") {
                if !matches!(additional, Value::Bool(false)) {
                    return false;
                }
            }
            object
                .iter()
                .filter(|(key, _)| key.as_str() != "additionalProperties")
                .all(|(_, value)| is_strict_compatible(value))
        }
        Value::Array(items) => items.iter().all(is_strict_compatible),
        _ => true,
    }
}

/// Provider names/tool names allow only `[a-zA-Z0-9_-]`; whip type names already
/// satisfy this, but sanitize defensively.
fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "value".to_owned()
    } else {
        cleaned
    }
}

// -- Response parsing (pure) ---------------------------------------------

/// Parse a provider response into a [`CoerceResult`]. `wrapped` indicates the
/// schema used the `value`-envelope and the structured payload must be unwrapped.
pub fn parse_response(
    provider: CoerceProvider,
    response: &HttpResponse,
    wrapped: bool,
) -> CoerceResult {
    if !(200..300).contains(&response.status) {
        return failed_result(
            format!("provider returned HTTP {}", response.status),
            provider_error_excerpt(&response.body),
        );
    }
    match provider {
        CoerceProvider::OpenAi => parse_openai_response(response, wrapped),
        CoerceProvider::Anthropic => parse_anthropic_response(response, wrapped),
    }
}

fn parse_openai_response(response: &HttpResponse, wrapped: bool) -> CoerceResult {
    // The structured payload is a JSON string in the assistant message's output
    // text. Prefer the convenience `output_text`, then walk `output[].content[]`.
    let text = openai_output_text(&response.body);
    let Some(text) = text else {
        return failed_result(
            "provider response contained no output text".to_owned(),
            provider_error_excerpt(&response.body),
        );
    };
    finalize_structured(&text_to_value(&text), wrapped, openai_usage(&response.body))
}

fn parse_anthropic_response(response: &HttpResponse, wrapped: bool) -> CoerceResult {
    // The structured payload is the `input` of the forced tool_use content block.
    let tool_input = response
        .body
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| {
            content
                .iter()
                .find(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        })
        .and_then(|block| block.get("input").cloned());
    let Some(tool_input) = tool_input else {
        return failed_result(
            "provider response contained no tool_use block".to_owned(),
            provider_error_excerpt(&response.body),
        );
    };
    finalize_structured(&tool_input, wrapped, anthropic_usage(&response.body))
}

/// Unwrap the `value` envelope when needed and emit a success result.
fn finalize_structured(structured: &Value, wrapped: bool, usage: Value) -> CoerceResult {
    let value = if wrapped {
        match structured.get("value") {
            Some(inner) => inner.clone(),
            None => {
                return failed_result(
                    "structured output missing wrapped `value`".to_owned(),
                    Some(structured.to_string()),
                );
            }
        }
    } else {
        structured.clone()
    };
    CoerceResult {
        status: CoerceStatus::Succeeded,
        value_json: Some(value.to_string()),
        error_json: None,
        summary: "coerce succeeded".to_owned(),
        transcript: format!("structured output:\n{}\n", value),
        usage_json: usage.to_string(),
    }
}

fn openai_output_text(body: &Value) -> Option<String> {
    if let Some(text) = body.get("output_text").and_then(Value::as_str) {
        if !text.is_empty() {
            return Some(text.to_owned());
        }
    }
    let output = body.get("output").and_then(Value::as_array)?;
    for item in output {
        if let Some(content) = item.get("content").and_then(Value::as_array) {
            for block in content {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    return Some(text.to_owned());
                }
            }
        }
    }
    None
}

/// The structured text may itself be a JSON object string, or (rarely) already a
/// JSON value; parse leniently, falling back to a string scalar.
fn text_to_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))
}

fn openai_usage(body: &Value) -> Value {
    let usage = body.get("usage");
    json!({
        "input_tokens": usage.and_then(|u| u.get("input_tokens")).and_then(Value::as_u64).unwrap_or(0),
        "output_tokens": usage.and_then(|u| u.get("output_tokens")).and_then(Value::as_u64).unwrap_or(0),
    })
}

fn anthropic_usage(body: &Value) -> Value {
    let usage = body.get("usage");
    json!({
        "input_tokens": usage.and_then(|u| u.get("input_tokens")).and_then(Value::as_u64).unwrap_or(0),
        "output_tokens": usage.and_then(|u| u.get("output_tokens")).and_then(Value::as_u64).unwrap_or(0),
    })
}

/// Pull a short, redactable error message from a provider error body without
/// leaking the whole payload.
fn provider_error_excerpt(body: &Value) -> Option<String> {
    body.get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .map(|message| message.chars().take(300).collect())
}

fn failed_result(reason: String, provider_error: Option<String>) -> CoerceResult {
    let mut error = json!({
        "reason": reason,
        "recoverable": true,
    });
    if let Some(provider_error) = provider_error {
        error["provider_error"] = Value::String(provider_error);
    }
    CoerceResult {
        status: CoerceStatus::Failed,
        value_json: None,
        error_json: Some(error.to_string()),
        summary: "coerce failed".to_owned(),
        transcript: "native coerce failure\n".to_owned(),
        usage_json: r#"{"input_tokens":0,"output_tokens":0}"#.to_owned(),
    }
}

// -- The native client ----------------------------------------------------

/// A real coerce client that talks to a provider's structured-output API over an
/// injected [`CoerceTransport`]. Holds the already-interpolated prompt and the
/// object-rooted output schema; `CoerceRequest` arguments are only used for the
/// transcript (the prompt already embeds them).
pub struct NativeCoerceClient<'a, T: CoerceTransport> {
    pub provider: CoerceProvider,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub prompt: String,
    pub output_schema: Value,
    pub wrapped: bool,
    pub schema_name: String,
    pub max_tokens: u32,
    /// `(account_id, session_id)` when using the codex backend (OpenAI only).
    pub codex: Option<(String, String)>,
    pub transport: &'a T,
}

impl<T: CoerceTransport> CoerceClient for NativeCoerceClient<'_, T> {
    fn coerce(&self, _request: &CoerceRequest) -> CoerceResult {
        let call = CoerceCall {
            provider: self.provider,
            base_url: &self.base_url,
            api_key: &self.api_key,
            model: &self.model,
            prompt: &self.prompt,
            output_schema: &self.output_schema,
            schema_name: &self.schema_name,
            max_tokens: self.max_tokens,
            codex: self
                .codex
                .as_ref()
                .map(|(account_id, session_id)| CodexAuth {
                    account_id,
                    session_id,
                }),
        };
        let request = build_request(&call);
        match self.transport.post(&request) {
            Ok(response) => parse_response(self.provider, &response, self.wrapped),
            Err(CoerceTransportError::Timeout) => CoerceResult {
                status: CoerceStatus::TimedOut,
                value_json: None,
                error_json: Some(
                    json!({ "reason": "coerce request timed out", "recoverable": true })
                        .to_string(),
                ),
                summary: "coerce timed out".to_owned(),
                transcript: "native coerce timeout\n".to_owned(),
                usage_json: r#"{"input_tokens":0,"output_tokens":0}"#.to_owned(),
            },
            Err(CoerceTransportError::Transport(message)) => {
                failed_result(format!("transport error: {message}"), None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use whipplescript_parser::{IrClass, IrEnum};

    fn span() -> whipplescript_parser::SourceSpan {
        whipplescript_parser::SourceSpan { start: 0, end: 0 }
    }

    fn work_status_enum() -> IrSchema {
        IrSchema::Enum(IrEnum {
            name: "WorkStatus".to_owned(),
            variants: vec!["Accepted".to_owned(), "Rejected".to_owned()],
            span: span(),
        })
    }

    fn work_review_class() -> IrSchema {
        IrSchema::Class(IrClass {
            name: "WorkReview".to_owned(),
            fields: vec![
                IrClassField {
                    name: "status".to_owned(),
                    ty: IrType::Ref("WorkStatus".to_owned()),
                    is_key: false,
                    presence_condition: None,
                    span: span(),
                },
                IrClassField {
                    name: "reason".to_owned(),
                    ty: IrType::Primitive(IrPrimitiveType::String),
                    is_key: false,
                    presence_condition: None,
                    span: span(),
                },
                IrClassField {
                    name: "score".to_owned(),
                    ty: IrType::Optional(Box::new(IrType::Primitive(IrPrimitiveType::Int))),
                    is_key: false,
                    presence_condition: None,
                    span: span(),
                },
            ],
            span: span(),
        })
    }

    #[test]
    fn class_schema_is_strict_with_nullable_optionals() {
        let schemas = vec![work_status_enum(), work_review_class()];
        let (schema, wrapped) =
            output_schema_envelope(&IrType::Ref("WorkReview".to_owned()), &schemas);
        assert!(!wrapped, "a class output is already object-rooted");
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("present")
            .iter()
            .map(|v| v.as_str().expect("present"))
            .collect();
        // Strict mode: every property is required, including the optional one.
        assert!(required.contains(&"status"));
        assert!(required.contains(&"reason"));
        assert!(required.contains(&"score"));
        // The optional field expresses absence via nullability, not omission.
        assert!(
            schema["properties"]["score"]["anyOf"]
                .as_array()
                .expect("present")
                .iter()
                .any(|variant| variant["type"] == "null"),
            "optional field is nullable"
        );
        assert_eq!(schema["properties"]["status"]["enum"][0], "Accepted");
    }

    #[test]
    fn sum_type_enum_schema_is_anyof_of_variant_objects() {
        // A payload-carrying enum's generated `<Enum>.<Variant>` classes must drive an
        // anyOf of object schemas, not a string `enum` (which drops the payload — the
        // verified latent bug). A bare variant with no class stays a string const.
        let variant_field = |value: &str| IrClassField {
            name: "variant".to_owned(),
            ty: IrType::LiteralString(value.to_owned()),
            is_key: false,
            presence_condition: None,
            span: span(),
        };
        let payload_field = |name: &str, ty: IrType| IrClassField {
            name: name.to_owned(),
            ty,
            is_key: false,
            presence_condition: None,
            span: span(),
        };
        let schemas = vec![
            IrSchema::Enum(IrEnum {
                name: "ReviewOutcome".to_owned(),
                variants: vec![
                    "Approved".to_owned(),
                    "Rejected".to_owned(),
                    "Blocked".to_owned(),
                ],
                span: span(),
            }),
            IrSchema::Class(IrClass {
                name: "ReviewOutcome.Approved".to_owned(),
                fields: vec![
                    variant_field("Approved"),
                    payload_field("score", IrType::Primitive(IrPrimitiveType::Float)),
                ],
                span: span(),
            }),
            IrSchema::Class(IrClass {
                name: "ReviewOutcome.Rejected".to_owned(),
                fields: vec![
                    variant_field("Rejected"),
                    payload_field("reason", IrType::Primitive(IrPrimitiveType::String)),
                ],
                span: span(),
            }),
        ];
        let schema = json_schema_for_type(&IrType::Ref("ReviewOutcome".to_owned()), &schemas);
        let alts = schema["anyOf"].as_array().expect("anyOf present");
        assert_eq!(alts.len(), 3, "one alternative per variant");
        // The Approved payload field survives (the bug dropped it to a string enum).
        let approved = alts
            .iter()
            .find(|alt| alt["properties"]["variant"]["const"] == "Approved")
            .expect("Approved object alternative");
        assert_eq!(approved["type"], "object");
        assert_eq!(approved["additionalProperties"], false);
        assert_eq!(approved["properties"]["score"]["type"], "number");
        // The bare `Blocked` variant (no generated class) is a string const.
        assert!(
            alts.iter().any(|alt| alt["const"] == "Blocked"),
            "bare variant is a string const: {alts:?}"
        );

        // A fully-bare enum still collapses to a single string `enum`.
        let bare =
            json_schema_for_type(&IrType::Ref("WorkStatus".to_owned()), &[work_status_enum()]);
        assert_eq!(bare["type"], "string");
        assert_eq!(bare["enum"][0], "Accepted");
    }

    #[test]
    fn non_object_output_is_wrapped_in_a_value_envelope() {
        let schemas = vec![work_status_enum()];
        let (schema, wrapped) =
            output_schema_envelope(&IrType::Ref("WorkStatus".to_owned()), &schemas);
        assert!(wrapped, "an enum output must be wrapped");
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["value"]["enum"][1], "Rejected");
    }

    #[test]
    fn openai_request_carries_json_schema_format_and_bearer() {
        let schema = json!({ "type": "object" });
        let call = CoerceCall {
            provider: CoerceProvider::OpenAi,
            base_url: "https://api.openai.com",
            api_key: "sk-test",
            model: "gpt-4o",
            prompt: "Classify this",
            output_schema: &schema,
            schema_name: "WorkReview",
            max_tokens: 1024,
            codex: None,
        };
        let request = build_request(&call);
        assert_eq!(request.url, "https://api.openai.com/v1/responses");
        assert_eq!(request.body["text"]["format"]["type"], "json_schema");
        assert_eq!(request.body["text"]["format"]["name"], "WorkReview");
        assert!(request
            .headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer sk-test"));
    }

    #[test]
    fn codex_request_targets_backend_with_codex_headers_and_streams() {
        let schema = json!({ "type": "object" });
        let call = CoerceCall {
            provider: CoerceProvider::OpenAi,
            base_url: "https://chatgpt.com",
            api_key: "oauth-jwt",
            model: "gpt-5.5",
            prompt: "Classify this",
            output_schema: &schema,
            schema_name: "WorkReview",
            max_tokens: 1024,
            codex: Some(CodexAuth {
                account_id: "acct-1",
                session_id: "sess-1",
            }),
        };
        let request = build_request(&call);
        assert_eq!(
            request.url,
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(request.body["stream"], true);
        assert_eq!(request.body["text"]["format"]["type"], "json_schema");
        // Codex input is message-shaped, not a bare string.
        assert_eq!(request.body["input"][0]["type"], "message");
        let header = |name: &str| {
            request
                .headers
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(header("authorization").as_deref(), Some("Bearer oauth-jwt"));
        assert_eq!(header("chatgpt-account-id").as_deref(), Some("acct-1"));
        assert_eq!(header("session_id").as_deref(), Some("sess-1"));
        assert_eq!(header("originator").as_deref(), Some("codex_cli_rs"));
        assert_eq!(header("accept").as_deref(), Some("text/event-stream"));
        assert_eq!(
            header("openai-beta").as_deref(),
            Some("responses=experimental")
        );
    }

    #[test]
    fn anthropic_request_forces_a_single_tool() {
        let schema = json!({ "type": "object" });
        let call = CoerceCall {
            provider: CoerceProvider::Anthropic,
            base_url: "https://api.anthropic.com",
            api_key: "key",
            model: "claude-opus-4-8",
            prompt: "Classify this",
            output_schema: &schema,
            schema_name: "WorkReview",
            max_tokens: 1024,
            codex: None,
        };
        let request = build_request(&call);
        assert_eq!(request.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(request.body["tool_choice"]["type"], "tool");
        assert_eq!(request.body["tool_choice"]["name"], "emit_WorkReview");
        assert_eq!(request.body["tools"][0]["name"], "emit_WorkReview");
        assert!(request
            .headers
            .iter()
            .any(|(k, v)| k == "x-api-key" && v == "key"));
        // Anthropic coerce uses x-api-key only — never the OAuth bearer header.
        assert!(!request.headers.iter().any(|(k, _)| k == "authorization"));
    }

    #[test]
    fn strict_disabled_for_map_valued_schema() {
        // A class with a Map field renders `additionalProperties` as a sub-schema.
        let schemas = vec![IrSchema::Class(IrClass {
            name: "Bag".to_owned(),
            fields: vec![IrClassField {
                name: "items".to_owned(),
                ty: IrType::Map(Box::new(IrType::Primitive(IrPrimitiveType::String))),
                is_key: false,
                presence_condition: None,
                span: span(),
            }],
            span: span(),
        })];
        let (schema, _) = output_schema_envelope(&IrType::Ref("Bag".to_owned()), &schemas);
        let call = CoerceCall {
            provider: CoerceProvider::OpenAi,
            base_url: "https://api.openai.com",
            api_key: "k",
            model: "gpt-4o",
            prompt: "p",
            output_schema: &schema,
            schema_name: "Bag",
            max_tokens: 256,
            codex: None,
        };
        let request = build_request(&call);
        assert_eq!(
            request.body["text"]["format"]["strict"], false,
            "a Map-valued schema disables strict mode"
        );
        // A plain class stays strict.
        let plain = json!({ "type": "object", "additionalProperties": false });
        assert!(is_strict_compatible(&plain));
    }

    #[test]
    fn parse_openai_extracts_structured_output() {
        let response = HttpResponse {
            status: 200,
            body: json!({
                "output": [{
                    "content": [{ "type": "output_text", "text": "{\"status\":\"Accepted\",\"reason\":\"ok\"}" }]
                }],
                "usage": { "input_tokens": 12, "output_tokens": 5 }
            }),
        };
        let result = parse_response(CoerceProvider::OpenAi, &response, false);
        assert_eq!(result.status, CoerceStatus::Succeeded);
        let value: Value =
            serde_json::from_str(result.value_json.as_ref().expect("present")).expect("present");
        assert_eq!(value["status"], "Accepted");
        assert_eq!(
            result.usage_json,
            r#"{"input_tokens":12,"output_tokens":5}"#
        );
    }

    #[test]
    fn parse_anthropic_unwraps_value_envelope() {
        let response = HttpResponse {
            status: 200,
            body: json!({
                "content": [{ "type": "tool_use", "name": "emit_WorkStatus", "input": { "value": "Accepted" } }],
                "usage": { "input_tokens": 8, "output_tokens": 2 }
            }),
        };
        let result = parse_response(CoerceProvider::Anthropic, &response, true);
        assert_eq!(result.status, CoerceStatus::Succeeded);
        assert_eq!(result.value_json.as_deref(), Some("\"Accepted\""));
    }

    #[test]
    fn non_2xx_is_a_failure_with_provider_error() {
        let response = HttpResponse {
            status: 429,
            body: json!({ "error": { "message": "rate limited" } }),
        };
        let result = parse_response(CoerceProvider::OpenAi, &response, false);
        assert_eq!(result.status, CoerceStatus::Failed);
        let error: Value =
            serde_json::from_str(result.error_json.as_ref().expect("present")).expect("present");
        assert_eq!(error["provider_error"], "rate limited");
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

    #[test]
    fn native_client_drives_transport_and_parses() {
        let transport = FakeTransport {
            response: Ok(HttpResponse {
                status: 200,
                body: json!({
                    "content": [{ "type": "tool_use", "input": { "status": "Accepted", "reason": "ok" } }],
                    "usage": { "input_tokens": 1, "output_tokens": 1 }
                }),
            }),
            seen: RefCell::new(None),
        };
        let client = NativeCoerceClient {
            provider: CoerceProvider::Anthropic,
            base_url: "https://api.anthropic.com".to_owned(),
            api_key: "key".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            prompt: "Classify this".to_owned(),
            output_schema: json!({ "type": "object" }),
            wrapped: false,
            schema_name: "WorkReview".to_owned(),
            max_tokens: 1024,
            codex: None,
            transport: &transport,
        };
        let request = CoerceRequest {
            function_name: "reviewWork".to_owned(),
            arguments_json: "{}".to_owned(),
            output_type: "WorkReview".to_owned(),
            generated_coerce_source_hash: "h".to_owned(),
            input_schema_hash: "i".to_owned(),
            output_schema_hash: "o".to_owned(),
        };
        let result = client.coerce(&request);
        assert_eq!(result.status, CoerceStatus::Succeeded);
        assert!(transport.seen.borrow().is_some(), "transport was invoked");
    }

    #[test]
    fn native_client_maps_timeout() {
        let transport = FakeTransport {
            response: Err(CoerceTransportError::Timeout),
            seen: RefCell::new(None),
        };
        let client = NativeCoerceClient {
            provider: CoerceProvider::OpenAi,
            base_url: "https://api.openai.com".to_owned(),
            api_key: "k".to_owned(),
            model: "gpt-4o".to_owned(),
            prompt: "p".to_owned(),
            output_schema: json!({ "type": "object" }),
            wrapped: false,
            schema_name: "X".to_owned(),
            max_tokens: 256,
            codex: None,
            transport: &transport,
        };
        let request = CoerceRequest {
            function_name: "f".to_owned(),
            arguments_json: "{}".to_owned(),
            output_type: "X".to_owned(),
            generated_coerce_source_hash: "h".to_owned(),
            input_schema_hash: "i".to_owned(),
            output_schema_hash: "o".to_owned(),
        };
        assert_eq!(client.coerce(&request).status, CoerceStatus::TimedOut);
    }
}
