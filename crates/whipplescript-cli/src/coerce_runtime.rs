//! CLI-side wiring for native (real-LLM) `coerce`: configuration and credential
//! resolution from the environment, the `ureq` HTTP transport, and turning a
//! declared `coerce` function + its arguments into a rendered prompt and an
//! output JSON Schema.
//!
//! The pure request/response logic and JSON-Schema synthesis live in
//! `whipplescript_kernel::coerce_native`; this module only supplies the parts
//! that need the environment (credentials), the network (`ureq`), and the
//! program IR (prompt template + output type).
//!
//! Activation is opt-in: with `WHIPPLESCRIPT_COERCE_PROVIDER` unset, coerce uses
//! the fixture path (so `dev`/`worker`/tests are unchanged). When it is set but
//! credentials are missing, resolution returns `Err` — a clear binding-time
//! failure — rather than silently degrading to a fixture.

use std::time::Duration;

use serde_json::Value;
use whipplescript_kernel::coerce_native::{
    CoerceProvider, CoerceTransport, CoerceTransportError, HttpRequest, HttpResponse,
};

/// Resolved configuration for a real coerce call.
pub struct NativeCoerceConfig {
    pub provider: CoerceProvider,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub timeout: Duration,
    /// `Some(account_id)` when the OpenAI credential is the Codex OAuth token, so
    /// the kernel targets the codex backend (SSE) instead of `api.openai.com`.
    pub codex_account_id: Option<String>,
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The `model = "..."` from `~/.codex/config.toml` (the model the codex CLI is
/// configured to use), so the codex coerce path tracks the user's config rather
/// than a hard-coded default. Shared with the agent-turn app-server path.
pub(crate) fn codex_config_model() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home)
        .join(".codex")
        .join("config.toml");
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        // Match the top-level `model = "..."` (not `model_reasoning_effort`, etc.).
        let Some(rest) = line.strip_prefix("model") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    None
}

/// Resolve native coerce configuration from the environment. `Ok(None)` selects
/// the fixture path; `Err` is a binding-time credential failure.
pub fn resolve_native_coerce_config() -> Result<Option<NativeCoerceConfig>, String> {
    resolve_native_coerce_config_for(None)
}

pub fn resolve_native_coerce_config_for(
    provider_override: Option<&str>,
) -> Result<Option<NativeCoerceConfig>, String> {
    let Some(provider_name) = provider_override
        .map(str::to_owned)
        .or_else(|| env_nonempty("WHIPPLESCRIPT_COERCE_PROVIDER"))
    else {
        return Ok(None);
    };
    if provider_name.eq_ignore_ascii_case("fixture") {
        return Ok(None);
    }
    let provider = parse_provider(&provider_name)?;
    let (api_key, source) = resolve_credential_with_source(provider)
        .ok_or_else(|| missing_credential_message(provider))?;
    if provider == CoerceProvider::Anthropic
        && whipplescript_kernel::coerce_native::is_anthropic_oauth_token(&api_key)
    {
        // Anthropic coerce uses a console API key only (decided 2026-06-23, Jack):
        // reusing a Claude Code OAuth token for the API is a terms gray area.
        return Err(
            "Anthropic coerce requires a console API key (`sk-ant-api...`), not a Claude Code \
             OAuth token (`sk-ant-oat...`); set ANTHROPIC_API_KEY or run `whip auth set anthropic <key>`"
                .to_owned(),
        );
    }
    // The OpenAI Codex OAuth token routes to the codex backend (SSE) with the
    // account id from `~/.codex/auth.json`.
    let codex_account_id = (provider == CoerceProvider::OpenAi
        && source == CredentialSource::CodexOAuth)
        .then(codex_account_id)
        .flatten();
    let base_url = env_nonempty("WHIPPLESCRIPT_COERCE_BASE_URL").unwrap_or_else(|| {
        if codex_account_id.is_some() {
            "https://chatgpt.com".to_owned()
        } else {
            provider.default_base_url().to_owned()
        }
    });
    // Model is not hard-coded: `WHIPPLESCRIPT_COERCE_MODEL` wins; otherwise the
    // codex path reads `~/.codex/config.toml`; the standard path requires the env.
    let model = env_nonempty("WHIPPLESCRIPT_COERCE_MODEL")
        .or_else(|| codex_account_id.as_ref().and_then(|_| codex_config_model()))
        .ok_or_else(|| {
            if codex_account_id.is_some() {
                "no coerce model: set WHIPPLESCRIPT_COERCE_MODEL, or set `model` in \
                 ~/.codex/config.toml"
                    .to_owned()
            } else {
                "no coerce model: set WHIPPLESCRIPT_COERCE_MODEL to the provider model id"
                    .to_owned()
            }
        })?;
    let max_tokens = env_nonempty("WHIPPLESCRIPT_COERCE_MAX_TOKENS")
        .and_then(|value| value.parse().ok())
        .unwrap_or(4096);
    let timeout_secs = env_nonempty("WHIPPLESCRIPT_COERCE_TIMEOUT_SECS")
        .and_then(|value| value.parse().ok())
        .unwrap_or(120);
    Ok(Some(NativeCoerceConfig {
        provider,
        base_url,
        api_key,
        model,
        max_tokens,
        timeout: Duration::from_secs(timeout_secs),
        codex_account_id,
    }))
}

fn missing_credential_message(provider: CoerceProvider) -> String {
    match provider {
        CoerceProvider::Anthropic => {
            "coerce provider `anthropic` needs a console API key: set ANTHROPIC_API_KEY or run \
             `whip auth set anthropic <key>`"
                .to_owned()
        }
        CoerceProvider::OpenAi => {
            "coerce provider `openai` needs a credential: set OPENAI_API_KEY, run \
             `whip auth set openai <key>`, or sign in with `codex login`"
                .to_owned()
        }
    }
}

/// The Codex account id (`chatgpt-account-id` header) from `~/.codex/auth.json`.
fn codex_account_id() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home).join(".codex").join("auth.json");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    json.get("tokens")
        .and_then(|tokens| tokens.get("account_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn parse_provider(name: &str) -> Result<CoerceProvider, String> {
    match name {
        "openai" => Ok(CoerceProvider::OpenAi),
        "anthropic" => Ok(CoerceProvider::Anthropic),
        other => Err(format!(
            "unknown WHIPPLESCRIPT_COERCE_PROVIDER `{other}` (expected `openai` or `anthropic`)"
        )),
    }
}

/// Where a resolved coerce credential came from (for `whip auth status`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialSource {
    /// An environment variable (named).
    Env(&'static str),
    /// The `whip auth set` config file.
    Stored,
    /// The Codex OAuth token in `~/.codex/auth.json`.
    CodexOAuth,
}

impl CredentialSource {
    pub fn label(self) -> String {
        match self {
            CredentialSource::Env(name) => format!("env:{name}"),
            CredentialSource::Stored => "stored (whip auth)".to_owned(),
            CredentialSource::CodexOAuth => "~/.codex/auth.json".to_owned(),
        }
    }
}

/// Resolve the coerce credential and report where it came from, in precedence
/// order: environment variable, then `whip auth` stored config, then (OpenAI
/// only) the Codex OAuth token. `None` means no credential is available.
pub fn resolve_credential_with_source(
    provider: CoerceProvider,
) -> Option<(String, CredentialSource)> {
    match provider {
        CoerceProvider::Anthropic => env_nonempty("ANTHROPIC_API_KEY")
            .map(|key| (key, CredentialSource::Env("ANTHROPIC_API_KEY")))
            .or_else(|| {
                crate::auth::stored_credential("anthropic")
                    .map(|key| (key, CredentialSource::Stored))
            }),
        CoerceProvider::OpenAi => env_nonempty("OPENAI_API_KEY")
            .map(|key| (key, CredentialSource::Env("OPENAI_API_KEY")))
            .or_else(|| {
                crate::auth::stored_credential("openai").map(|key| (key, CredentialSource::Stored))
            })
            .or_else(|| codex_oauth_token().map(|key| (key, CredentialSource::CodexOAuth))),
    }
}

/// Best-effort read of the Codex OAuth access token from `~/.codex/auth.json`.
/// Tries the common shapes; returns `None` if the file or token is absent.
fn codex_oauth_token() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home).join(".codex").join("auth.json");
    let text = std::fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    json.get("tokens")
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .or_else(|| json.get("access_token").and_then(Value::as_str))
        .or_else(|| json.get("OPENAI_API_KEY").and_then(Value::as_str))
        .map(str::to_owned)
}

/// The single network side effect, backed by `ureq` (synchronous — consistent
/// with how the worker drains effects serially; concurrency comes from running
/// effects on a worker thread pool, not async).
pub struct UreqCoerceTransport {
    agent: ureq::Agent,
}

impl UreqCoerceTransport {
    pub fn new(timeout: Duration) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(timeout)
            .user_agent("whipplescript-coerce")
            .build();
        Self { agent }
    }
}

impl CoerceTransport for UreqCoerceTransport {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, CoerceTransportError> {
        let mut builder = self.agent.post(&request.url);
        for (name, value) in &request.headers {
            builder = builder.set(name, value);
        }
        // We know the response is an SSE stream from the request's own Accept
        // header — don't rely on the server's (sometimes absent) content-type.
        let expect_sse = request
            .headers
            .iter()
            .any(|(name, value)| name == "accept" && value.contains("event-stream"));
        match builder.send_json(request.body.clone()) {
            Ok(response) => Ok(read_response(response, expect_sse)),
            // A non-2xx status is still a structured (JSON) response — parse it
            // for the provider error message, never as SSE.
            Err(ureq::Error::Status(_, response)) => Ok(read_response(response, false)),
            Err(ureq::Error::Transport(transport)) => {
                let message = transport.to_string();
                if message.to_ascii_lowercase().contains("timed out")
                    || message.to_ascii_lowercase().contains("timeout")
                {
                    Err(CoerceTransportError::Timeout)
                } else {
                    Err(CoerceTransportError::Transport(message))
                }
            }
        }
    }
}

fn read_response(response: ureq::Response, expect_sse: bool) -> HttpResponse {
    let status = response.status();
    if expect_sse {
        // Codex backend: assemble the SSE Responses stream into a `response`-shaped
        // object the kernel parser reads. (The server's content-type header is
        // unreliable here, so we key off the request's Accept instead.)
        let raw = response.into_string().unwrap_or_default();
        return HttpResponse {
            status,
            body: assemble_responses_sse(&raw),
        };
    }
    let body = response.into_json::<Value>().unwrap_or(Value::Null);
    HttpResponse { status, body }
}

/// Collapse a Responses-API SSE stream into a `response`-shaped object the
/// kernel's OpenAI parser can read. The structured output is carried by the
/// `response.output_text.delta` events (the codex `response.completed` payload's
/// `output[]` does not always include the assembled text), so prefer the
/// concatenated deltas and attach usage from the completed event.
fn assemble_responses_sse(raw: &str) -> Value {
    let mut completed: Option<Value> = None;
    let mut deltas = String::new();
    for line in raw.lines() {
        let Some(payload) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("response.completed") => completed = event.get("response").cloned(),
            Some("response.output_text.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    deltas.push_str(delta);
                }
            }
            _ => {}
        }
    }
    if !deltas.is_empty() {
        let usage = completed
            .as_ref()
            .and_then(|response| response.get("usage"))
            .cloned()
            .unwrap_or(Value::Null);
        return serde_json::json!({ "output_text": deltas, "usage": usage });
    }
    // No text deltas: fall back to the completed response object (the kernel
    // parser walks output[].content[].text).
    completed.unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use whipplescript_kernel::coerce_native::{build_coerce_call_parts, render_coerce_prompt};

    #[test]
    fn assemble_responses_sse_prefers_delta_text_and_keeps_usage() {
        let raw = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"{\\\"v\\\":1}\"}\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":2},\"output\":[]}}\n",
            "data: [DONE]\n",
        );
        let body = assemble_responses_sse(raw);
        assert_eq!(body["output_text"].as_str(), Some("{\"v\":1}"));
        assert_eq!(body["usage"]["input_tokens"], 3);
    }

    #[test]
    fn anthropic_oauth_token_is_recognized_for_rejection() {
        // Anthropic coerce requires a console key; the resolver uses this to
        // reject OAuth tokens (a terms gray area), not to route them.
        assert!(whipplescript_kernel::coerce_native::is_anthropic_oauth_token("sk-ant-oat01-abc"));
        assert!(
            !whipplescript_kernel::coerce_native::is_anthropic_oauth_token("sk-ant-api03-real")
        );
    }

    #[test]
    fn parse_provider_accepts_known_and_rejects_unknown() {
        assert!(matches!(
            parse_provider("openai"),
            Ok(CoerceProvider::OpenAi)
        ));
        assert!(matches!(
            parse_provider("anthropic"),
            Ok(CoerceProvider::Anthropic)
        ));
        assert!(parse_provider("gemini").is_err());
    }

    #[test]
    fn render_substitutes_arguments_and_output_format() {
        let arguments = json!({ "summary": "shipped the feature", "count": 3 });
        let schema = json!({ "type": "object" });
        let rendered = render_coerce_prompt(
            "Classify: {{ summary }} ({{ count }})\n{{ ctx.output_format }}",
            &arguments,
            Some(&schema),
        );
        assert!(rendered.contains("shipped the feature"));
        assert!(rendered.contains("(3)"));
        // The output-format token embeds the JSON Schema.
        assert!(rendered.contains("JSON Schema"));
    }

    #[test]
    fn render_preserves_unknown_interpolation() {
        let rendered = render_coerce_prompt("hi {{ missing }}", &json!({}), None);
        assert_eq!(rendered, "hi {{ missing }}");
    }

    #[test]
    fn render_supports_dotted_access() {
        let arguments = json!({ "item": { "title": "Bug" } });
        let rendered = render_coerce_prompt("Title: {{ item.title }}", &arguments, None);
        assert_eq!(rendered, "Title: Bug");
    }

    #[test]
    fn build_call_parts_resolves_prompt_and_schema() {
        let source = r#"
@service
workflow Coerce

enum Verdict { Yes No }
coerce judge(summary string) -> Verdict {
  prompt """markdown
  Decide: {{ summary }}
  {{ ctx.output_format }}
  """
}

output result R
class R { v string }
signal go.now { x string }
rule j
  when go.now as g
=> { complete result { v "ok" } }
"#;
        let ir = whipplescript_parser::compile_program(source)
            .ir
            .expect("compiles");
        // Positional args (`arg0`) are mapped to the declared parameter name so
        // `{{ summary }}` resolves — this is how a `coerce judge("…")` call lowers.
        let arguments = json!({ "arg0": "looks good" });
        let (prompt, schema, wrapped, name) =
            build_coerce_call_parts(&ir, "judge", &arguments).expect("parts");
        assert!(
            prompt.contains("looks good"),
            "positional arg0 should bind to `summary`: {prompt}"
        );
        assert!(wrapped, "an enum output is wrapped in a value envelope");
        assert_eq!(name, "Verdict");
        assert_eq!(schema["properties"]["value"]["enum"][0], "Yes");
    }

    #[test]
    fn build_call_parts_rejects_unknown_function() {
        let source = r#"
@service
workflow Coerce
output result R
class R { v string }
signal go.now { x string }
rule j
  when go.now as g
=> { complete result { v "ok" } }
"#;
        let ir = whipplescript_parser::compile_program(source)
            .ir
            .expect("compiles");
        assert!(build_coerce_call_parts(&ir, "missing", &json!({})).is_err());
    }
}
