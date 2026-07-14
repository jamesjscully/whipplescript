//! Runtime-owned projection for the Durable Object host surface.
//!
//! The Worker shell must not reverse-engineer WhippleScript's SQL into a second
//! receipt format. This module folds the admitted command and durable runtime
//! rows through the public `whipplescript.host.v1` pointer schema. It is generic
//! over `RuntimeStore`, so native SQLite-backed tests exercise the same code the
//! wasm boundary calls over DO SQLite.

use serde::Serialize;
use serde_json::{json, Value};
use whipplescript_kernel::host_protocol::{
    AnswerHumanAskCommand, EventPosition, LabeledHumanAsk, LabeledRuntimeEvent,
    RuntimeEvidencePointer, StartTurnCommand, TurnReceipt, TurnStatus, HOST_PROTOCOL,
};
use whipplescript_kernel::idempotency_key;
use whipplescript_store::{EvidenceRecord, NewEvent, RuntimeStore, StoreError};

#[derive(Clone, Debug, Serialize)]
pub struct HostedOutputFieldFlow {
    pub field: String,
    pub reads: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HostedTurnProjection {
    pub runtime_evidence_pointers: Vec<RuntimeEvidencePointer>,
    pub pending_human: Option<LabeledHumanAsk>,
    pub receipt: Option<TurnReceipt>,
    /// Typed, host-published token counts for product metering. The opaque
    /// `usage_ref` remains the authoritative runtime evidence pointer; this is
    /// only its deliberately narrow billing projection.
    pub usage_observation: Option<HostedUsageObservation>,
    pub output_flow_signature: Vec<HostedOutputFieldFlow>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct HostedUsageObservation {
    pub usage_ref: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug)]
pub struct HostedHumanAnswerContext {
    pub command: StartTurnCommand,
    pub call_id: String,
}

pub fn validate_human_answer_context<S: RuntimeStore>(
    store: &S,
    answer: &AnswerHumanAskCommand,
) -> Result<HostedHumanAnswerContext, String> {
    answer.validate().map_err(|error| error.to_string())?;
    let item = store
        .get_inbox_item(&answer.ask_ref)
        .map_err(store_error)?
        .ok_or_else(|| "human ask was not found".to_owned())?;
    if item.instance_id != answer.instance_ref {
        return Err("human answer does not match the pending instance".to_owned());
    }
    let command_id = item
        .effect_id
        .as_deref()
        .ok_or_else(|| "human ask has no suspended turn".to_owned())?;
    let effect = store
        .list_effects(&answer.instance_ref)
        .map_err(store_error)?
        .into_iter()
        .find(|effect| effect.effect_id == command_id)
        .ok_or_else(|| "human ask turn was not found".to_owned())?;
    let command: StartTurnCommand =
        serde_json::from_str(&effect.input_json).map_err(|error| error.to_string())?;
    command.validate().map_err(|error| error.to_string())?;
    if command.instance_ref != answer.instance_ref || command.policy != answer.policy {
        return Err("human answer changed the suspended turn or policy epoch".to_owned());
    }
    let waiting = store
        .list_events(&answer.instance_ref)
        .map_err(store_error)?
        .into_iter()
        .rev()
        .find(|event| {
            event.event_type == "agent.turn.awaiting_human"
                && json_field_equals(&event.payload_json, "inbox_item_id", &answer.ask_ref)
        })
        .ok_or_else(|| "human ask has no suspension event".to_owned())?;
    let call_id = serde_json::from_str::<Value>(&waiting.payload_json)
        .ok()
        .and_then(|value| {
            value
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|call_id| !call_id.is_empty())
        .ok_or_else(|| "human ask suspension omitted its call id".to_owned())?;
    Ok(HostedHumanAnswerContext { command, call_id })
}

pub fn current_position<S: RuntimeStore>(
    store: &S,
    instance_id: &str,
) -> Result<EventPosition, StoreError> {
    let sequence = store
        .list_events(instance_id)?
        .last()
        .map_or(0, |event| event.sequence);
    Ok(EventPosition {
        instance_ref: instance_id.to_owned(),
        sequence: u64::try_from(sequence).map_err(|_| {
            StoreError::Conflict("runtime event position cannot be negative".to_owned())
        })?,
    })
}

pub fn project_host_turn<S: RuntimeStore>(
    store: &mut S,
    instance_id: &str,
    command_id: &str,
) -> Result<HostedTurnProjection, String> {
    let effect = store
        .list_effects(instance_id)
        .map_err(store_error)?
        .into_iter()
        .find(|effect| effect.effect_id == command_id)
        .ok_or_else(|| "host turn was not found".to_owned())?;
    let command: StartTurnCommand =
        serde_json::from_str(&effect.input_json).map_err(|error| error.to_string())?;
    command.validate().map_err(|error| error.to_string())?;
    if command.instance_ref != instance_id || command.command_id != command_id {
        return Err("host turn command does not match its durable identity".to_owned());
    }

    let label_ref = format!("whip:label:{}", command.policy.envelope_hash);
    let mut events = store.list_events(instance_id).map_err(store_error)?;
    let first_turn_event = events
        .iter()
        .find(|event| event_mentions_command(&event.payload_json, command_id))
        .map_or_else(
            || events.last().map_or(0, |event| event.sequence),
            |event| event.sequence,
        );
    let mut pointers = events
        .iter()
        .filter(|event| {
            event.sequence >= first_turn_event
                && (event_mentions_command(&event.payload_json, command_id)
                    || event.event_type == "host.turn.receipt")
        })
        .map(|event| {
            Ok(RuntimeEvidencePointer::Event(LabeledRuntimeEvent {
                protocol: HOST_PROTOCOL.to_owned(),
                command_id: command_id.to_owned(),
                position: EventPosition {
                    instance_ref: instance_id.to_owned(),
                    sequence: positive_sequence(event.sequence)?,
                },
                policy: command.policy.clone(),
                kind: event.event_type.clone(),
                label_ref: label_ref.clone(),
                evidence_ref: format!("whip:event:{}", event.event_id),
                payload_ref: Some(format!("whip:event:{}:payload", event.event_id)),
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    for event in events
        .iter()
        .filter(|event| event.event_type == "host.human.answer.receipt")
    {
        let answer_receipt: whipplescript_kernel::host_protocol::HumanAnswerReceipt =
            serde_json::from_str(&event.payload_json)
                .map_err(|error| format!("invalid durable human answer receipt: {error}"))?;
        if answer_receipt.turn_command_id == command_id {
            pointers.push(RuntimeEvidencePointer::HumanAnswer(answer_receipt));
        }
    }

    let pending_item = store
        .list_inbox_items(Some("pending"))
        .map_err(store_error)?
        .into_iter()
        .find(|item| {
            item.instance_id == instance_id && item.effect_id.as_deref() == Some(command_id)
        });
    if let Some(item) = pending_item {
        let waiting = events
            .iter()
            .rev()
            .find(|event| {
                event.event_type == "agent.turn.awaiting_human"
                    && json_field_equals(&event.payload_json, "inbox_item_id", &item.inbox_item_id)
            })
            .ok_or_else(|| "pending human ask has no suspension event".to_owned())?;
        let evidence = store
            .list_evidence(instance_id)
            .map_err(store_error)?
            .into_iter()
            .find(|evidence| {
                evidence.kind == "human.ask"
                    && json_field_equals(
                        &evidence.metadata_json,
                        "inbox_item_id",
                        &item.inbox_item_id,
                    )
            })
            .ok_or_else(|| "pending human ask has no evidence".to_owned())?;
        let ask = LabeledHumanAsk {
            protocol: HOST_PROTOCOL.to_owned(),
            ask_ref: item.inbox_item_id,
            command_id: command_id.to_owned(),
            instance_ref: instance_id.to_owned(),
            policy: command.policy.clone(),
            position: EventPosition {
                instance_ref: instance_id.to_owned(),
                sequence: positive_sequence(waiting.sequence)?,
            },
            label_ref,
            evidence_ref: format!("whip:evidence:{}", evidence.evidence_id),
            question: item.prompt,
            choices: serde_json::from_str(&item.choices_json).map_err(|error| error.to_string())?,
            freeform_allowed: item.freeform_allowed,
        };
        pointers.push(RuntimeEvidencePointer::HumanAsk(ask.clone()));
        return Ok(HostedTurnProjection {
            runtime_evidence_pointers: pointers,
            pending_human: Some(ask),
            receipt: None,
            usage_observation: None,
            output_flow_signature: output_flows(&command),
        });
    }

    let Some(run) = store
        .list_runs(instance_id)
        .map_err(store_error)?
        .into_iter()
        .rev()
        .find(|run| run.effect_id == command_id)
    else {
        return Ok(HostedTurnProjection {
            runtime_evidence_pointers: pointers,
            pending_human: None,
            receipt: None,
            usage_observation: None,
            output_flow_signature: output_flows(&command),
        });
    };
    let Some(status) = turn_status(&run.status) else {
        return Ok(HostedTurnProjection {
            runtime_evidence_pointers: pointers,
            pending_human: None,
            receipt: None,
            usage_observation: None,
            output_flow_signature: output_flows(&command),
        });
    };

    let usage_ref = ensure_evidence(
        store,
        &command,
        &run.run_id,
        "host.turn.usage",
        &run.metadata_json,
    )?;
    let usage_observation = project_usage(&run.metadata_json, &usage_ref)?;
    let guarantee = json!({
        "protocol": HOST_PROTOCOL,
        "policy": command.policy,
        "actor_ref": command.actor_ref,
        "package_version_ref": command.package_version_ref,
        "resources": command.resources,
        "images": command.input.images,
        "provider_binding_ref": command.provider_binding,
        "placement_ceiling_ref": command.placement_ceiling_ref,
        "guarantees": [
            "signed_policy_identity_verified",
            "package_ifc_checked_under_verified_envelope",
            "instance_package_policy_binding_verified",
            "resource_provider_placement_handles_governed",
            "tool_surface_pinned_to_package",
            "resource_and_secret_bodies_resolved_after_admission"
        ],
        "dynamic": [],
        "workspace_cut": "unwitnessed"
    })
    .to_string();
    let guarantee_report_ref = ensure_evidence(
        store,
        &command,
        &run.run_id,
        "host.turn.guarantee",
        &guarantee,
    )?;
    let output_handle =
        matches!(status, TurnStatus::Completed).then(|| format!("whip:run:{}:output", run.run_id));
    let marker_payload = json!({
        "command_id": command.command_id,
        "run_ref": command.run_ref,
        "status": status,
        "output_handle": output_handle,
        "usage_ref": usage_ref,
        "guarantee_report_ref": guarantee_report_ref,
        "workspace_cut_ref": Value::Null,
    })
    .to_string();
    let marker = events
        .iter()
        .find(|event| {
            event.event_type == "host.turn.receipt"
                && json_field_equals(&event.payload_json, "command_id", command_id)
        })
        .map(|event| (event.event_id.clone(), event.sequence))
        .map_or_else(
            || {
                store
                    .append_event(NewEvent {
                        instance_id,
                        event_type: "host.turn.receipt",
                        payload_json: &marker_payload,
                        source: "host-do",
                        causation_id: Some(&run.run_id),
                        correlation_id: Some(command_id),
                        idempotency_key: Some(&idempotency_key(&[
                            instance_id,
                            command_id,
                            "host-turn-receipt",
                        ])),
                    })
                    .map(|event| (event.event_id, event.sequence))
                    .map_err(store_error)
            },
            Ok,
        )?;
    let receipt = TurnReceipt {
        protocol: HOST_PROTOCOL.to_owned(),
        command_id: command_id.to_owned(),
        run_ref: command.run_ref.clone(),
        instance_ref: instance_id.to_owned(),
        policy: command.policy.clone(),
        terminal_position: EventPosition {
            instance_ref: instance_id.to_owned(),
            sequence: positive_sequence(marker.1)?,
        },
        status,
        output_handle,
        usage_ref,
        guarantee_report_ref,
        workspace_cut_ref: None,
    };
    receipt
        .validate_for(&command)
        .map_err(|error| error.to_string())?;
    if !events.iter().any(|event| event.event_id == marker.0) {
        events = store.list_events(instance_id).map_err(store_error)?;
        if let Some(event) = events.iter().find(|event| event.event_id == marker.0) {
            pointers.push(RuntimeEvidencePointer::Event(LabeledRuntimeEvent {
                protocol: HOST_PROTOCOL.to_owned(),
                command_id: command_id.to_owned(),
                position: receipt.terminal_position.clone(),
                policy: command.policy.clone(),
                kind: event.event_type.clone(),
                label_ref,
                evidence_ref: format!("whip:event:{}", event.event_id),
                payload_ref: Some(format!("whip:event:{}:payload", event.event_id)),
            }));
        }
    }
    pointers.push(RuntimeEvidencePointer::TurnReceipt(receipt.clone()));
    Ok(HostedTurnProjection {
        runtime_evidence_pointers: pointers,
        pending_human: None,
        receipt: Some(receipt),
        usage_observation,
        output_flow_signature: output_flows(&command),
    })
}

fn project_usage(
    metadata_json: &str,
    usage_ref: &str,
) -> Result<Option<HostedUsageObservation>, String> {
    let metadata: Value = serde_json::from_str(metadata_json).map_err(|error| error.to_string())?;
    let Some(usage) = metadata.get("usage").filter(|usage| usage.is_object()) else {
        return Ok(None);
    };
    let tokens = |primary: &str, alias: &str| {
        usage
            .get(primary)
            .or_else(|| usage.get(alias))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Ok(Some(HostedUsageObservation {
        usage_ref: usage_ref.to_owned(),
        input_tokens: tokens("input_tokens", "prompt_tokens"),
        output_tokens: tokens("output_tokens", "completion_tokens"),
    }))
}

fn output_flows(command: &StartTurnCommand) -> Vec<HostedOutputFieldFlow> {
    let reads = command
        .resources
        .iter()
        .chain(command.input.images.iter())
        .map(|resource| resource.handle.clone())
        .collect::<Vec<_>>();
    ["assistant_text", "tool_calls"]
        .into_iter()
        .map(|field| HostedOutputFieldFlow {
            field: field.to_owned(),
            reads: reads.clone(),
        })
        .collect()
}

fn ensure_evidence<S: RuntimeStore>(
    store: &S,
    command: &StartTurnCommand,
    run_id: &str,
    kind: &str,
    metadata_json: &str,
) -> Result<String, String> {
    if let Some(existing) = store
        .list_evidence_for_subject("run", run_id)
        .map_err(store_error)?
        .into_iter()
        .find(|item| {
            item.kind == kind && item.correlation_id.as_deref() == Some(&command.command_id)
        })
    {
        return Ok(format!("whip:evidence:{}", existing.evidence_id));
    }
    let evidence_id = store
        .record_evidence(EvidenceRecord {
            instance_id: &command.instance_ref,
            kind,
            subject_type: "run",
            subject_id: run_id,
            causation_id: Some(&command.command_id),
            correlation_id: Some(&command.command_id),
            summary: None,
            metadata_json,
        })
        .map_err(store_error)?;
    Ok(format!("whip:evidence:{evidence_id}"))
}

fn turn_status(status: &str) -> Option<TurnStatus> {
    match status {
        "completed" | "succeeded" => Some(TurnStatus::Completed),
        "failed" => Some(TurnStatus::Failed),
        "timed_out" => Some(TurnStatus::TimedOut),
        "cancelled" => Some(TurnStatus::Cancelled),
        _ => None,
    }
}

fn event_mentions_command(payload: &str, command_id: &str) -> bool {
    serde_json::from_str::<Value>(payload)
        .ok()
        .is_some_and(|value| value_mentions(&value, command_id))
}

fn value_mentions(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value == needle,
        Value::Array(values) => values.iter().any(|value| value_mentions(value, needle)),
        Value::Object(values) => values.values().any(|value| value_mentions(value, needle)),
        _ => false,
    }
}

fn json_field_equals(payload: &str, field: &str, expected: &str) -> bool {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|value| value.get(field).and_then(Value::as_str).map(str::to_owned))
        .as_deref()
        == Some(expected)
}

fn positive_sequence(sequence: i64) -> Result<u64, String> {
    u64::try_from(sequence)
        .ok()
        .filter(|sequence| *sequence > 0)
        .ok_or_else(|| "runtime event position must be positive".to_owned())
}

fn store_error(error: StoreError) -> String {
    format!("{error:?}")
}
