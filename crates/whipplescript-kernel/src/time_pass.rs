//! The due-time pass, lifted from the native `dev` loop so the DO host can run
//! it too (DR-0033 Phase 6): complete due `timer.wait` effects and expire
//! deadline-passed effects, all through the threaded `RuntimeStore` handle.
//! `now` is injected (ISO-8601 UTC) — the pass never reads wall time itself, so
//! it honors both the native virtual clock and the DO's host-supplied instant.

use serde_json::json;
use whipplescript_store::{EffectCancellationRequest, EffectCompletion, RunStart, StoreResult};

use crate::{idempotency_key, RuntimeKernel};
use whipplescript_store::RuntimeStore;

/// What one due-time pass did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TimePassReport {
    pub timers_fired: usize,
    pub deadlines_expired: usize,
    pub terminal_events: Vec<String>,
}

/// Complete due timers and expire deadline-passed effects for one instance.
/// Mirrors the native `dev` loop's time pass byte-for-byte; the CLI delegates
/// here.
pub fn resolve_due_time_effects<S: RuntimeStore>(
    kernel: &mut RuntimeKernel<S>,
    instance_id: &str,
    now: &str,
) -> StoreResult<TimePassReport> {
    let mut report = TimePassReport::default();
    let due = kernel.store().due_time_effects(instance_id, now)?;
    for effect in due {
        // A `lease.acquire … wait <duration>` carries a creation-anchored
        // `timeout_seconds` purely to bound its contention retry, so it surfaces
        // here once the wait elapses. Its terminal is `contended` (give up), not a
        // timeout/expiry — the coordination handler on the effect pass owns that
        // completion, so leave it for the handler rather than expiring it here.
        if effect.kind == "lease.acquire" {
            continue;
        }
        if effect.kind == "timer.wait" {
            let run_id = idempotency_key(&[instance_id, &effect.effect_id, "timer-run"]);
            let lease_id = idempotency_key(&[instance_id, &effect.effect_id, "timer-lease"]);
            kernel.start_run(RunStart {
                instance_id,
                effect_id: &effect.effect_id,
                run_id: &run_id,
                provider: "timer",
                worker_id: "whip-timer",
                lease_id: &lease_id,
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: &json!({
                    "duration_seconds": effect.timeout_seconds,
                })
                .to_string(),
            })?;
            let terminal = kernel.complete_run(EffectCompletion {
                instance_id,
                effect_id: &effect.effect_id,
                run_id: &run_id,
                provider: "timer",
                worker_id: "whip-timer",
                status: "completed",
                exit_code: Some(0),
                summary: Some("timer fired"),
                metadata_json: &json!({
                    "duration_seconds": effect.timeout_seconds,
                })
                .to_string(),
                idempotency_key: Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "timer-terminal",
                ])),
            })?;
            let value_json = json!({
                "effect_id": effect.effect_id,
                "run_id": run_id,
                "status": "completed",
                "fired": true,
                "duration_seconds": effect.timeout_seconds,
            })
            .to_string();
            kernel.derive_fact(
                instance_id,
                "timer.fired",
                &effect.effect_id,
                &value_json,
                Some(&terminal.event_id),
                Some(&idempotency_key(&[
                    instance_id,
                    &effect.effect_id,
                    "timer.fired",
                ])),
            )?;
            report.timers_fired += 1;
            report.terminal_events.push(terminal.event_id);
            continue;
        }
        // Deadline expiry: running effects time out at the run level and get
        // a cancellation request; never-run effects expire directly.
        let running_run = kernel
            .store()
            .list_runs(instance_id)?
            .into_iter()
            .find(|run| run.effect_id == effect.effect_id && run.status == "running");
        let terminal_event_id = match running_run {
            Some(run) => {
                let terminal = kernel.timeout_run(EffectCompletion {
                    instance_id,
                    effect_id: &effect.effect_id,
                    run_id: &run.run_id,
                    provider: &run.provider,
                    worker_id: &run.worker_id,
                    status: "timed_out",
                    exit_code: None,
                    summary: Some("deadline exceeded"),
                    metadata_json: &json!({
                        "timeout_seconds": effect.timeout_seconds,
                        "reason": "deadline exceeded",
                    })
                    .to_string(),
                    idempotency_key: Some(&idempotency_key(&[
                        instance_id,
                        &effect.effect_id,
                        "deadline-terminal",
                    ])),
                })?;
                let _ = kernel
                    .store_mut()
                    .request_effect_cancellation(EffectCancellationRequest {
                        instance_id,
                        effect_id: &effect.effect_id,
                        revision_id: None,
                        reason: Some("deadline exceeded"),
                        requested_by: "deadline",
                        causation_event_id: Some(&terminal.event_id),
                        idempotency_key: Some(&idempotency_key(&[
                            instance_id,
                            &effect.effect_id,
                            "deadline-cancel-request",
                        ])),
                    });
                terminal.event_id
            }
            None => {
                let terminal = kernel.store_mut().expire_effect(
                    instance_id,
                    &effect.effect_id,
                    Some(&idempotency_key(&[
                        instance_id,
                        &effect.effect_id,
                        "deadline-terminal",
                    ])),
                )?;
                terminal.event_id
            }
        };
        let value_json = json!({
            "effect_id": effect.effect_id,
            "status": "timed_out",
            "reason": "deadline exceeded",
            "timeout_seconds": effect.timeout_seconds,
        })
        .to_string();
        kernel.derive_fact(
            instance_id,
            "effect.timed_out",
            &effect.effect_id,
            &value_json,
            Some(&terminal_event_id),
            Some(&idempotency_key(&[
                instance_id,
                &effect.effect_id,
                "effect.timed_out",
            ])),
        )?;
        report.deadlines_expired += 1;
        report.terminal_events.push(terminal_event_id);
    }
    Ok(report)
}
