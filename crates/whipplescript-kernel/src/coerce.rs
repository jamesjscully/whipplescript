//! Coerce effect provider abstraction.
//!
//! `run_coerce` (see `lib.rs`) drives a coerce effect through a `CoerceClient`.
//! The in-tree client is `FakeCoerceClient`, used by the fixture provider and
//! tests. The real, provider-native structured-output client (OpenAI Responses /
//! Anthropic Messages) is a separate, credential-gated build; the earlier
//! bridge-server placeholders (`HttpCoerceClient`/`ManagedCoerceService`) were a
//! fictional design and have been removed (no real provider implements a
//! `/coerce` POST). See `spec/coerce.md`.

use serde_json::{json, Value};

/// The coercion-config fingerprint folded into `schema.coerce` effect
/// admission keys (DR-0014 amendment; spec/std-coercion.md "Idempotency And
/// Replay"): H(provider_kind, provider_id, backend, model). Hosts compute it
/// at kernel construction — native from the resolved coerce config, the
/// durable object from `coerce_config_json` — and the fixture path uses the
/// literal `"fixture"` instead so tests stay deterministic. Credentials are
/// deliberately excluded: a rotated key must not rekey admissions.
pub fn coercion_config_fingerprint(
    provider_kind: &str,
    provider_id: &str,
    backend: &str,
    model: &str,
) -> String {
    crate::idempotency_key(&[
        "coercion-config",
        provider_kind,
        provider_id,
        backend,
        model,
    ])
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoerceRequest {
    pub function_name: String,
    pub arguments_json: String,
    pub output_type: String,
    pub generated_coerce_source_hash: String,
    pub input_schema_hash: String,
    pub output_schema_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoerceStatus {
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoerceResult {
    pub status: CoerceStatus,
    pub value_json: Option<String>,
    pub error_json: Option<String>,
    pub summary: String,
    pub transcript: String,
    pub usage_json: String,
}

pub trait CoerceClient {
    fn coerce(&self, request: &CoerceRequest) -> CoerceResult;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeCoerceClient {
    result: CoerceResult,
}

impl FakeCoerceClient {
    pub fn succeeds(value_json: impl Into<String>) -> Self {
        Self {
            result: CoerceResult {
                status: CoerceStatus::Succeeded,
                value_json: Some(value_json.into()),
                error_json: None,
                summary: "coerce succeeded".to_owned(),
                transcript: "fake coerce transcript\n".to_owned(),
                usage_json: r#"{"input_tokens":1,"output_tokens":1}"#.to_owned(),
            },
        }
    }

    pub fn fails(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            result: CoerceResult {
                status: CoerceStatus::Failed,
                value_json: None,
                error_json: Some(
                    json!({
                        "reason": reason,
                        "recoverable": true,
                    })
                    .to_string(),
                ),
                summary: "coerce failed".to_owned(),
                transcript: "fake coerce failure transcript\n".to_owned(),
                usage_json: r#"{"input_tokens":1,"output_tokens":0}"#.to_owned(),
            },
        }
    }

    pub fn times_out(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            result: CoerceResult {
                status: CoerceStatus::TimedOut,
                value_json: None,
                error_json: Some(
                    json!({
                        "reason": reason,
                        "recoverable": true,
                    })
                    .to_string(),
                ),
                summary: "coerce timed out".to_owned(),
                transcript: "fake coerce timeout transcript\n".to_owned(),
                usage_json: r#"{"input_tokens":1,"output_tokens":0}"#.to_owned(),
            },
        }
    }
}

impl CoerceClient for FakeCoerceClient {
    fn coerce(&self, request: &CoerceRequest) -> CoerceResult {
        let mut result = self.result.clone();
        let request_json = json!({
            "function_name": request.function_name,
            "arguments": json_from_str(&request.arguments_json),
            "output_type": request.output_type,
        });
        result.transcript.push_str(&request_json.to_string());
        result
    }
}

fn json_from_str(source: &str) -> Value {
    serde_json::from_str(source).unwrap_or_else(|_| Value::String(source.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_coerce_client_returns_typed_value() {
        let client = FakeCoerceClient::succeeds(r#"{"status":"Accept"}"#);
        let result = client.coerce(&CoerceRequest {
            function_name: "reviewWork".to_owned(),
            arguments_json: r#"{"summary":"done"}"#.to_owned(),
            output_type: "WorkReview".to_owned(),
            generated_coerce_source_hash: "coerce".to_owned(),
            input_schema_hash: "input".to_owned(),
            output_schema_hash: "output".to_owned(),
        });

        assert_eq!(result.status, CoerceStatus::Succeeded);
        assert_eq!(result.value_json.as_deref(), Some(r#"{"status":"Accept"}"#));
        assert!(result.transcript.contains("reviewWork"));
    }
}
