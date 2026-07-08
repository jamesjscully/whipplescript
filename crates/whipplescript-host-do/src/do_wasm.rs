//! The `#[wasm_bindgen]` boundary the Cloudflare Worker shell imports (DR-0033
//! chunk 5c). wasm32-only — a thin JS↔Rust wrapper over
//! [`DurableInstance`](crate::do_worker::DurableInstance)`::create`/`step`/`status`,
//! carrying no orchestration of its own:
//!
//! - a [`DoSql`] backed by the isolate's `state.storage.sql` (via the JS
//!   `DoSqlBridge` object the shell passes in),
//! - JSON marshalling of the step protocol (`fetch` request out, response in), and
//! - error surfacing across the boundary.
//!
//! The TS shell drives it: `WasmDurableInstance.create(bridge, program, input,
//! principal)`, then loop `step(responseJson)` — on `{"kind":"needs_http", …}` do the
//! `fetch` and pass the result back, on `{"kind":"terminal"|"parked"|"failed"}` stop.
//! Provider creds flow in via `create`'s `coerce_config_json` / `agent_config_json`
//! (from DO secrets), so the store-only, effect-free, coerce, AND agent-turn paths
//! run on the deployed surface. The remaining agent seam is tools: a live turn that
//! requests tools needs a tool-executor sidecar (the async-tool boundary).

use wasm_bindgen::prelude::*;

use crate::do_store::SqlValue;
use whipplescript_kernel::coerce_native::CoerceProvider;
use whipplescript_kernel::harness_model::MessagesApiClient;
use whipplescript_kernel::sansio::{HttpResponse, TransportError};

use crate::do_instance::CoerceProviderConfig;
use crate::do_store::DoSql;
use crate::do_worker::{DurableEffectPorts, DurableInstance, DurableStepOutcome};

#[wasm_bindgen]
extern "C" {
    /// The JS object the shell implements over `state.storage.sql`: run a statement
    /// (returns the changed-row count) or a query (returns rows as a JSON string).
    /// Params arrive as a JSON array of `null | number | string`.
    pub type DoSqlBridge;

    #[wasm_bindgen(method, catch)]
    fn exec(this: &DoSqlBridge, sql: &str, params_json: &str) -> Result<f64, JsValue>;

    #[wasm_bindgen(method, catch)]
    fn query(this: &DoSqlBridge, sql: &str, params_json: &str) -> Result<String, JsValue>;
}

/// [`DoSql`] over the JS `DoSqlBridge`. `SqlValue` marshals as JSON scalars.
struct JsDoSql {
    bridge: DoSqlBridge,
}

fn params_to_json(params: &[SqlValue]) -> String {
    let values: Vec<serde_json::Value> = params
        .iter()
        .map(|value| match value {
            SqlValue::Null => serde_json::Value::Null,
            SqlValue::Int(number) => serde_json::Value::from(*number),
            SqlValue::Text(text) => serde_json::Value::from(text.clone()),
        })
        .collect();
    serde_json::Value::from(values).to_string()
}

fn parse_rows(rows_json: &str) -> Result<Vec<Vec<SqlValue>>, String> {
    let rows: serde_json::Value =
        serde_json::from_str(rows_json).map_err(|error| error.to_string())?;
    let rows = rows
        .as_array()
        .ok_or("DoSqlBridge.query must return a JSON array of rows")?;
    rows.iter()
        .map(|row| {
            let cells = row
                .as_array()
                .ok_or("each row must be a JSON array".to_owned())?;
            Ok(cells
                .iter()
                .map(|cell| {
                    if cell.is_null() {
                        SqlValue::Null
                    } else if let Some(number) = cell.as_i64() {
                        SqlValue::Int(number)
                    } else if let Some(number) = cell.as_f64() {
                        SqlValue::Int(number as i64)
                    } else {
                        SqlValue::Text(cell.as_str().map(str::to_owned).unwrap_or_default())
                    }
                })
                .collect())
        })
        .collect()
}

impl DoSql for JsDoSql {
    fn execute(&self, sql: &str, params: &[SqlValue]) -> Result<u64, String> {
        let count = self
            .bridge
            .exec(sql, &params_to_json(params))
            .map_err(|error| format!("{error:?}"))?;
        Ok(count as u64)
    }

    fn query(&self, sql: &str, params: &[SqlValue]) -> Result<Vec<Vec<SqlValue>>, String> {
        let rows_json = self
            .bridge
            .query(sql, &params_to_json(params))
            .map_err(|error| format!("{error:?}"))?;
        parse_rows(&rows_json)
    }
}

/// The step response the shell hands back: `{"body": <json>, "status": <n>}` for a
/// completed `fetch`, or `{"error": "timeout" | <message>}` for a transport failure.
fn parse_incoming(
    response_json: Option<String>,
) -> Result<Option<Result<HttpResponse, TransportError>>, JsValue> {
    let Some(json) = response_json else {
        return Ok(None);
    };
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|error| JsValue::from_str(&error.to_string()))?;
    if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
        let transport = if error == "timeout" {
            TransportError::Timeout
        } else {
            TransportError::Transport(error.to_owned())
        };
        return Ok(Some(Err(transport)));
    }
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(200) as u16;
    let body = value
        .get("body")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Ok(Some(Ok(HttpResponse { status, body })))
}

/// Marshal a [`DurableStepOutcome`] to the JSON the shell branches on.
fn outcome_to_json(outcome: &DurableStepOutcome) -> String {
    let value = match outcome {
        DurableStepOutcome::NeedsHttp(request) => serde_json::json!({
            "kind": "needs_http",
            "request": {
                "url": request.url,
                "headers": request.headers,
                "body": request.body,
            },
        }),
        DurableStepOutcome::Terminal => serde_json::json!({ "kind": "terminal" }),
        DurableStepOutcome::Parked { next_due_unix_ms } => serde_json::json!({
            "kind": "parked",
            "next_due_unix_ms": next_due_unix_ms,
        }),
        DurableStepOutcome::Failed(message) => {
            serde_json::json!({ "kind": "failed", "message": message })
        }
    };
    value.to_string()
}

/// Parse the DO-secret coerce config JSON into a `CoerceProviderConfig`.
fn parse_coerce_config(json: &str) -> Result<CoerceProviderConfig, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let provider = match value.get("provider").and_then(serde_json::Value::as_str) {
        Some("anthropic") => CoerceProvider::Anthropic,
        Some("openai") => CoerceProvider::OpenAi,
        other => return Err(format!("unknown coerce provider: {other:?}")),
    };
    let field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    Ok(CoerceProviderConfig {
        provider,
        provider_name: field("provider").unwrap_or_else(|| "coerce".to_owned()),
        base_url: field("base_url").ok_or("coerce config needs base_url")?,
        api_key: field("api_key").ok_or("coerce config needs api_key")?,
        model: field("model").ok_or("coerce config needs model")?,
        max_tokens: value
            .get("max_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1024) as u32,
    })
}

/// Parse the DO-secret agent-model config JSON into a `MessagesApiClient` — the
/// same `{provider, base_url, api_key, model, max_tokens}` shape as the coerce
/// config (an agent turn is a multi-round messages/responses call). The client is
/// transport-free: the shell performs each round's `fetch`.
fn parse_agent_config(json: &str) -> Result<MessagesApiClient, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let provider = match value.get("provider").and_then(serde_json::Value::as_str) {
        Some("anthropic") => CoerceProvider::Anthropic,
        Some("openai") => CoerceProvider::OpenAi,
        other => return Err(format!("unknown agent provider: {other:?}")),
    };
    let field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    Ok(MessagesApiClient::new(
        provider,
        field("api_key").ok_or("agent config needs api_key")?,
        field("model").ok_or("agent config needs model")?,
        field("base_url").ok_or("agent config needs base_url")?,
        value
            .get("max_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(4096),
        // Constructed once per durable object, so no per-turn id here; the
        // Anthropic cache_control breakpoint still applies (Decision 7).
        None,
    ))
}

/// The durable-object instance as the Worker shell sees it.
#[wasm_bindgen]
pub struct WasmDurableInstance {
    inner: DurableInstance<JsDoSql>,
}

#[wasm_bindgen]
impl WasmDurableInstance {
    /// Compile `program` and create + start a fresh instance over the JS-backed DO
    /// SQLite. Called once when the object is first addressed. Both config args are
    /// optional and carry provider creds from DO secrets, same JSON shape
    /// `{"provider":"anthropic"|"openai","base_url","api_key","model","max_tokens"}`:
    /// `coerce_config_json` for `coerce` effects, `agent_config_json` for the
    /// (multi-round) `agent.tell` turn. A live agent turn with tools also needs a
    /// tool executor over an HTTP sidecar (the remaining async-tool seam).
    pub fn create(
        bridge: DoSqlBridge,
        program: &str,
        input: &str,
        principal: &str,
        coerce_config_json: Option<String>,
        agent_config_json: Option<String>,
        project_context_json: Option<String>,
    ) -> Result<WasmDurableInstance, JsValue> {
        // Deploy-shipped project instructions: `[{"path": ..., "content": ...}]`
        // in injection order (context-assembly Phase 3 item 4).
        let project_context: Vec<(String, String)> = match project_context_json {
            Some(json) => serde_json::from_str::<Vec<serde_json::Value>>(&json)
                .map_err(|error| JsValue::from_str(&error.to_string()))?
                .into_iter()
                .filter_map(|doc| {
                    let path = doc.get("path")?.as_str()?.to_owned();
                    let content = doc.get("content")?.as_str()?.to_owned();
                    Some((path, content))
                })
                .collect(),
            None => Vec::new(),
        };
        let coerce = match coerce_config_json {
            Some(json) => Some(parse_coerce_config(&json).map_err(|e| JsValue::from_str(&e))?),
            None => None,
        };
        let agent_model: Option<Box<dyn whipplescript_kernel::harness_loop::HttpModelClient>> =
            match agent_config_json {
                Some(json) => Some(Box::new(
                    parse_agent_config(&json).map_err(|e| JsValue::from_str(&e))?,
                )),
                None => None,
            };
        let inner = DurableInstance::create(
            JsDoSql { bridge },
            program,
            input,
            principal,
            DurableEffectPorts {
                coerce,
                agent_model,
                ..DurableEffectPorts::default()
            },
            &project_context,
        )
        .map_err(|error| JsValue::from_str(&error))?;
        Ok(Self { inner })
    }

    /// Advance the instance one HTTP round. Pass `undefined`/`null` on the first
    /// call, then the previous `needs_http` request's `fetch` result as JSON.
    /// `now_unix_ms` is the host's clock (`Date.now()`), injected so the core
    /// never reads wall time (DR-0033 Phase 6 — timers/deadlines resolve
    /// against it, and `parked.next_due_unix_ms` names the next wake-up).
    /// Returns the next `DurableStepOutcome` as JSON.
    pub fn step(
        &mut self,
        response_json: Option<String>,
        now_unix_ms: f64,
    ) -> Result<String, JsValue> {
        let incoming = parse_incoming(response_json)?;
        Ok(outcome_to_json(
            &self.inner.step(incoming, now_unix_ms as i64),
        ))
    }

    /// The instance's durable status (`"running"` / `"completed"` / …).
    pub fn status(&self) -> Result<String, JsValue> {
        self.inner
            .status()
            .map(|status| status.unwrap_or_default())
            .map_err(|error| JsValue::from_str(&format!("{error:?}")))
    }
}
