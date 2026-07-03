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
//! Coerce provider creds flow in via `create`'s `coerce_config_json` (from DO
//! secrets), so the store-only, effect-free, AND coerce paths run on the deployed
//! surface; the messages-API agent model client is the remaining follow-on seam.

use wasm_bindgen::prelude::*;

use crate::do_store::SqlValue;
use whipplescript_kernel::coerce_native::CoerceProvider;
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
        DurableStepOutcome::Parked => serde_json::json!({ "kind": "parked" }),
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

/// The durable-object instance as the Worker shell sees it.
#[wasm_bindgen]
pub struct WasmDurableInstance {
    inner: DurableInstance<JsDoSql>,
}

#[wasm_bindgen]
impl WasmDurableInstance {
    /// Compile `program` and create + start a fresh instance over the JS-backed DO
    /// SQLite. Called once when the object is first addressed. `coerce_config_json`
    /// (optional) carries the provider creds a `coerce` effect needs, from DO
    /// secrets: `{"provider":"anthropic"|"openai","base_url","api_key","model",
    /// "max_tokens"}`. The messages-API agent model client is a follow-on seam.
    pub fn create(
        bridge: DoSqlBridge,
        program: &str,
        input: &str,
        principal: &str,
        coerce_config_json: Option<String>,
    ) -> Result<WasmDurableInstance, JsValue> {
        let coerce = match coerce_config_json {
            Some(json) => Some(parse_coerce_config(&json).map_err(|e| JsValue::from_str(&e))?),
            None => None,
        };
        let inner = DurableInstance::create(
            JsDoSql { bridge },
            program,
            input,
            principal,
            DurableEffectPorts {
                coerce,
                ..DurableEffectPorts::default()
            },
        )
        .map_err(|error| JsValue::from_str(&error))?;
        Ok(Self { inner })
    }

    /// Advance the instance one HTTP round. Pass `undefined`/`null` on the first
    /// call, then the previous `needs_http` request's `fetch` result as JSON.
    /// Returns the next `DurableStepOutcome` as JSON.
    pub fn step(&mut self, response_json: Option<String>) -> Result<String, JsValue> {
        let incoming = parse_incoming(response_json)?;
        Ok(outcome_to_json(&self.inner.step(incoming)))
    }

    /// The instance's durable status (`"running"` / `"completed"` / …).
    pub fn status(&self) -> Result<String, JsValue> {
        self.inner
            .status()
            .map(|status| status.unwrap_or_default())
            .map_err(|error| JsValue::from_str(&format!("{error:?}")))
    }
}
