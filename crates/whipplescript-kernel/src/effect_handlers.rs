//! Host-agnostic effect-handler cores (DR-0033 chunk 5b).
//!
//! The store-only effect handlers, lifted out of the CLI so BOTH host bindings
//! can execute effects over their held store handle: the native `InstanceDriver`
//! dispatches them over `RuntimeKernel<NativeStores>`, the DO's `DoInstanceDriver`
//! over `RuntimeKernel<DoSqliteStore>`. Each core settles one ready effect to its
//! terminal synchronously (no external I/O), reading only its `EffectConfig`
//! (host-neutral) — so it runs identically on both hosts. HTTP-bearing effects
//! (coerce/agent) and the recursion handlers are lifted separately.

use serde_json::{json, Value};

use whipplescript_store::{
    ClaimableEffect, EffectCompletion, RunStart, RuntimeStore, StoreError, StoredEvent,
};

use crate::effect_config::EffectConfig;
use crate::idempotency_key;
use crate::loft::{FakeLoftClient, LoftAction, LoftEffectRequest};
use crate::rule_lowering::json_from_str;
use crate::LoftEffectExecution;
use crate::RuntimeKernel;

/// `event.emit`: ingest a durable event, settle the effect, and derive the
/// `event.emit.succeeded` + `<event_type>` facts (kernel methods only).
pub fn run_event_effect_generic<S: RuntimeStore>(
    kernel: &mut RuntimeKernel<S>,
    instance_id: &str,
    effect: &ClaimableEffect,
    config: &EffectConfig,
) -> Result<StoredEvent, StoreError> {
    let input = json_from_str(&effect.input_json);
    let event_type = input
        .get("event_type")
        .and_then(Value::as_str)
        .or(effect.target.as_deref())
        .unwrap_or("event.emitted");
    let payload = input
        .get("payload")
        .cloned()
        .unwrap_or_else(|| json!({"effect_id": effect.effect_id, "event_type": event_type}));
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "event-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "event-lease"]);
    kernel.start_run(RunStart {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: &config.provider,
        worker_id: "whip-worker",
        lease_id: &lease_id,
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: &json!({
            "event_type": event_type,
            "input": input,
        })
        .to_string(),
    })?;

    let emitted = kernel.ingest_external_event(
        instance_id,
        event_type,
        &payload.to_string(),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            event_type,
            "event.emit",
        ])),
    )?;
    let metadata_json = json!({
        "event_type": event_type,
        "event_id": emitted.event_id,
        "input": input,
        "value": payload,
    })
    .to_string();
    let terminal = kernel.complete_run(EffectCompletion {
        instance_id,
        effect_id: &effect.effect_id,
        run_id: &run_id,
        provider: &config.provider,
        worker_id: "whip-worker",
        status: "completed",
        exit_code: Some(0),
        summary: Some("fixture event emitted"),
        metadata_json: &metadata_json,
        idempotency_key: Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "terminal",
        ])),
    })?;
    let mut emitted_value = payload.as_object().cloned().unwrap_or_default();
    emitted_value.insert(
        "event_id".to_owned(),
        Value::String(emitted.event_id.clone()),
    );
    emitted_value.insert(
        "event_type".to_owned(),
        Value::String(event_type.to_owned()),
    );
    emitted_value.insert("payload".to_owned(), payload.clone());
    let value_json = json!({
        "effect_id": effect.effect_id,
        "run_id": run_id,
        "event_id": emitted.event_id,
        "event_type": event_type,
        "status": "completed",
        "value": Value::Object(emitted_value),
        "summary": "fixture event emitted",
    })
    .to_string();
    kernel.derive_fact(
        instance_id,
        "event.emit.succeeded",
        &effect.effect_id,
        &value_json,
        Some(&terminal.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            "event.emit.succeeded",
        ])),
    )?;
    kernel.derive_fact(
        instance_id,
        event_type,
        &effect.effect_id,
        &value_json,
        Some(&emitted.event_id),
        Some(&idempotency_key(&[
            instance_id,
            &effect.effect_id,
            event_type,
            "fact",
        ])),
    )?;
    Ok(terminal)
}

/// `loft.claim`: claim a Loft issue via the (fixture) loft client + settle.
pub fn run_loft_effect_generic<S: RuntimeStore>(
    kernel: &mut RuntimeKernel<S>,
    instance_id: &str,
    effect: &ClaimableEffect,
    config: &EffectConfig,
) -> Result<whipplescript_store::StoredEvent, StoreError> {
    let input = json_from_str(&effect.input_json);
    let issue_id = input
        .pointer("/issue/issue/id")
        .and_then(Value::as_str)
        .or_else(|| input.pointer("/issue/issue_id").and_then(Value::as_str))
        .unwrap_or("issue-fixture")
        .to_owned();
    let request = LoftEffectRequest {
        action: LoftAction::Claim,
        issue_id: issue_id.clone(),
        lease_id: None,
        claim_ready: true,
        issue_version: None,
        actor: Some("whip-worker".to_owned()),
        lease_duration_seconds: Some(3600),
        command_id: idempotency_key(&[instance_id, &effect.effect_id, "loft-command"]),
        note: None,
        target_status: None,
        evidence_json: None,
        evidence_kind: None,
        evidence_artifact: None,
        evidence_data_path: None,
        resource_intent_json: None,
        release_after_failure: false,
        expect_heads: Vec::new(),
        metadata_json: effect.input_json.clone(),
    };
    let client = if config.outcome_failed {
        FakeLoftClient::fails("fixture loft failure")
    } else {
        FakeLoftClient::succeeds(
            json!({
                "issue": {
                    "id": issue_id,
                    "title": "Fixture Loft issue",
                    "body": "Fixture body"
                },
                "lease": {
                    "id": idempotency_key(&[instance_id, &effect.effect_id, "loft-lease-value"])
                }
            })
            .to_string(),
        )
    };
    let run_id = idempotency_key(&[instance_id, &effect.effect_id, "loft-run"]);
    let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "loft-lease"]);
    kernel.run_loft_effect(
        LoftEffectExecution {
            instance_id,
            effect_id: &effect.effect_id,
            run_id: &run_id,
            provider: &config.provider,
            worker_id: "whip-worker",
            lease_id: &lease_id,
            lease_expires_at: "2030-01-01T00:00:00Z",
            request: &request,
        },
        &client,
    )
}
