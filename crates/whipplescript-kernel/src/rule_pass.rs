//! The rule-pass orchestration, lifted host-agnostic (DR-0033 chunk 4 groundwork).
//!
//! `step_instance_generic` drives the native `dev`-loop rule fixpoint
//! (project_tracker_issues + match/lower/commit) over ONE held `RuntimeKernel<S>`,
//! where `S` unifies the runtime / coordination / work-items surfaces (native
//! `NativeStores`; the DO's `DoSqliteStore`). This is the piece the instance step
//! machine drives; it lives in the wasm-clean kernel so the DO host can call it.
//! The native CLI keeps the thin `step_instance` wrapper that builds the handle.

#![allow(clippy::too_many_arguments)]

use std::path::Path;

use serde_json::{json, Value};
use whipplescript_core::Severity;
use whipplescript_parser::IrProgram;
use whipplescript_store::coordination::Coordination;
use whipplescript_store::items::WorkItems;
use whipplescript_store::{
    DiagnosticRecord, EffectCancellation, EffectCancellationRequest, RuleCommit,
    RuleCommitRevisionGuard, RuntimeStore, StoreError,
};

use crate::idempotency_key;
use crate::lowering::{
    BranchReport, OwnedDependency, OwnedEffect, OwnedFact, OwnedLowering, OwnedWorkflowTerminal,
};
use crate::rule_lowering::{
    json_from_str, lower_rule, ready_contexts, stable_hash_hex, GuardReport,
};
use crate::RuntimeKernel;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StepReport {
    pub instance_id: String,
    pub committed_rules: usize,
    pub facts_created: usize,
    pub facts_consumed: usize,
    pub effects_created: usize,
    pub guard_reports: Vec<GuardReport>,
    pub branch_reports: Vec<BranchReport>,
}

/// The host-agnostic rule pass (DR-0033 instance-scheduler lift): the fixpoint of
/// `project_tracker_issues` + rule matching/lowering/commit, run over ONE held store
/// handle instead of re-opening per operation. `S` unifies the runtime,
/// coordination, and work-items surfaces — natively `NativeStores`, on the DO the
/// one `DoSqliteStore`.
pub fn step_instance_generic<S: RuntimeStore + Coordination + WorkItems>(
    kernel: &mut RuntimeKernel<S>,
    instance_id: &str,
    ir: &IrProgram,
    source_path: Option<&Path>,
    active_version_guard: Option<&str>,
) -> Result<StepReport, StoreError> {
    let mut report = StepReport {
        instance_id: instance_id.to_owned(),
        ..StepReport::default()
    };
    let mut made_progress = true;
    while made_progress {
        made_progress = false;
        let status = kernel
            .store()
            .status(instance_id)?
            .ok_or_else(|| StoreError::Conflict("instance does not exist".to_owned()))?;
        if status.instance.status != "running" {
            break;
        }
        if let Some(active_version_guard) = active_version_guard {
            if status.instance.version_id != active_version_guard {
                return Err(StoreError::Conflict(format!(
                    "active version changed during step from {active_version_guard} to {}; rerun `whip step` with the active program",
                    status.instance.version_id
                )));
            }
        }
        let active_version_id = status.instance.version_id;
        let active_revision_epoch = status.instance.revision_epoch;
        let active_revision_epoch_key = active_revision_epoch.to_string();
        project_tracker_issues(kernel, instance_id, ir)?;
        let events = kernel.store().list_events(instance_id)?;
        let facts = kernel.store().list_facts(instance_id)?;
        let all_facts = kernel.store().list_facts_including_consumed(instance_id)?;
        let effects = kernel.store().list_effects(instance_id)?;
        let started_event_id = events
            .iter()
            .find(|event| event.event_type == "external.started")
            .map(|event| event.event_id.clone());

        'rules: for rule in &ir.rules {
            let ready = ready_contexts(ir, rule, &facts, &effects, started_event_id.as_deref());
            report.guard_reports.extend(ready.guard_reports);
            for context in ready.contexts {
                let lowering = lower_rule(
                    instance_id,
                    &active_version_id,
                    &active_revision_epoch_key,
                    ir,
                    rule,
                    &context,
                    &all_facts,
                    &effects,
                    source_path,
                );
                report
                    .branch_reports
                    .extend(lowering.branch_reports.iter().cloned());
                if !lowering.errors.is_empty() {
                    let message = format!(
                        "rule `{}` lowering failed: {}",
                        rule.name,
                        lowering.errors.join("; ")
                    );
                    kernel.store().record_diagnostic(DiagnosticRecord {
                        instance_id: Some(instance_id),
                        program_id: None,
                        program_version_id: Some(&active_version_id),
                        severity: Severity::Error,
                        code: Some("rule.lowering.unresolved"),
                        message: &message,
                        source_span_json: None,
                        subject_type: Some("rule"),
                        subject_id: Some(&rule.name),
                        event_id: None,
                        effect_id: None,
                        run_id: None,
                        assertion_id: None,
                        evidence_ids_json: "[]",
                        artifact_ids_json: "[]",
                        causation_id: context.trigger_event_id.as_deref(),
                        correlation_id: context.identity.as_deref(),
                        idempotency_key: Some(&idempotency_key(&[
                            instance_id,
                            &active_version_id,
                            &active_revision_epoch_key,
                            &rule.name,
                            "lowering-error",
                            &lowering.errors.join("|"),
                        ])),
                    })?;
                    return Err(StoreError::Conflict(message));
                }
                if lowering.facts.is_empty()
                    && lowering.consumed_fact_ids.is_empty()
                    && lowering.effects.is_empty()
                    && lowering.dependencies.is_empty()
                    && lowering.terminal.is_none()
                    && lowering.internal_fail.is_none()
                {
                    continue;
                }
                // 503 auto-fail: an unhandled effect failure in a self-terminating
                // flow routes to the generic kernel failed terminal (no typed
                // `failure` payload), distinct from the typed terminal commit path.
                // The generated `flowfail` block carries nothing else, so handle it
                // before the normal commit. fail_instance_internal transitions
                // running -> failed; the loop then exits (status != running).
                if let Some(reason) = lowering.internal_fail.clone() {
                    let fail_key = idempotency_key(&[
                        instance_id,
                        &active_version_id,
                        &active_revision_epoch_key,
                        &rule.name,
                        context.identity.as_deref().unwrap_or("started"),
                        "flow-autofail",
                        &reason,
                    ]);
                    let event =
                        kernel.fail_instance_internal(instance_id, &reason, Some(&fail_key));
                    match event {
                        Ok(_) => {
                            report.committed_rules += 1;
                            // A workflow terminal auto-releases every held lease
                            // (spec/coordination.md), the same as the typed path.
                            release_holder_resources_on_terminal(kernel.store_mut(), instance_id);
                            made_progress = true;
                            break 'rules;
                        }
                        Err(StoreError::Conflict(_)) => {
                            // Already failed (idempotent re-fire) — nothing to do.
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                }
                let consumed_fact_ids = lowering
                    .consumed_fact_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                let new_facts = lowering
                    .facts
                    .iter()
                    .map(OwnedFact::as_new_fact)
                    .collect::<Vec<_>>();
                let new_effects = lowering
                    .effects
                    .iter()
                    .map(OwnedEffect::as_new_effect)
                    .collect::<Vec<_>>();
                let new_dependencies = lowering
                    .dependencies
                    .iter()
                    .map(OwnedDependency::as_new_dependency)
                    .collect::<Vec<_>>();
                let terminal = lowering
                    .terminal
                    .as_ref()
                    .map(OwnedWorkflowTerminal::as_workflow_terminal);
                let lowering_key = lowering_idempotency_key(&lowering);
                let commit_key = idempotency_key(&[
                    instance_id,
                    &active_version_id,
                    &active_revision_epoch_key,
                    &rule.name,
                    context.identity.as_deref().unwrap_or("started"),
                    &lowering_key,
                ]);
                let event = kernel.commit_rule_with_revision_guard(
                    RuleCommit {
                        instance_id,
                        rule: &rule.name,
                        trigger_event_id: context.trigger_event_id.as_deref(),
                        facts: &new_facts,
                        consumed_fact_ids: &consumed_fact_ids,
                        effects: &new_effects,
                        dependencies: &new_dependencies,
                        terminal,
                        idempotency_key: Some(&commit_key),
                    },
                    RuleCommitRevisionGuard {
                        program_version_id: &active_version_id,
                        revision_epoch: active_revision_epoch,
                    },
                );
                match event {
                    Ok(committed) => {
                        report.committed_rules += 1;
                        report.facts_created += new_facts.len();
                        report.facts_consumed += consumed_fact_ids.len();
                        report.effects_created += new_effects.len();
                        // Holder-lifetime bound (spec/coordination.md): an
                        // instance reaching a workflow terminal auto-releases
                        // every lease it held.
                        if lowering.terminal.is_some() {
                            release_holder_resources_on_terminal(kernel.store_mut(), instance_id);
                        }
                        apply_rule_cancels(
                            kernel,
                            instance_id,
                            &rule.name,
                            &lowering.cancels,
                            &committed.event_id,
                        )?;
                        made_progress = true;
                        break 'rules;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(report)
}

/// Holder-lifetime release on terminal (spec/coordination.md principle 3 +
/// spec/work-queues.md): an instance that reaches ANY terminal — a rule-driven
/// `complete`/`fail` OR an operator `cancel` — drops every workspace-scoped
/// resource it held: coordination leases AND builtin-queue claims. Coordination
/// leases also have a TTL crash net, but queue claims do NOT, so this terminal
/// release is the only automatic recovery for a claim held by a dead instance.
/// Releasing both from one place keeps the terminal paths from forgetting a
/// resource type. Best-effort: a cleanup failure degrades to leaving the
/// resource (lease TTL backstops it) rather than failing an already-committed
/// terminal.
pub fn release_holder_resources_on_terminal<S: Coordination + WorkItems>(
    store: &mut S,
    instance_id: &str,
) {
    let _ = Coordination::release_all_for_holder(store, instance_id);
    let _ = WorkItems::release_claims_for_holder(store, instance_id);
}

/// Projects ready work items from declared builtin queues into
/// instance-local `tracker.issue.ready` facts, and retires projections whose
/// items are no longer ready. The tracker is the source of truth; the run
/// store holds a cache keyed (queue, id).
pub fn project_tracker_issues<S: RuntimeStore + WorkItems>(
    kernel: &mut RuntimeKernel<S>,
    instance_id: &str,
    ir: &IrProgram,
) -> Result<(), StoreError> {
    if ir.trackers.is_empty() {
        return Ok(());
    }
    for queue in &ir.trackers {
        if queue.provider != "builtin" {
            continue;
        }
        // Keep a projection alive while this instance holds the claim: the
        // dispatching rule's multi-stage chain needs its trigger fact until
        // the item is finished or released. Re-fires are idempotent (effect
        // ids are identity-derived), matching the engine's existing idiom.
        let ready = WorkItems::list_items(kernel.store(), Some(&queue.name), None)?
            .into_iter()
            .filter(|item| {
                (item.status == "open" && item.claimed_by.is_none())
                    || (item.status == "in_progress"
                        && item.claimed_by.as_deref() == Some(instance_id))
            })
            .collect::<Vec<_>>();
        let existing = kernel
            .store()
            .list_facts(instance_id)?
            .into_iter()
            .filter(|fact| fact.name == "tracker.issue.ready")
            .filter(|fact| {
                json_from_str(&fact.value_json)
                    .get("queue")
                    .and_then(Value::as_str)
                    == Some(queue.name.as_str())
            })
            .collect::<Vec<_>>();
        let ready_prefixes = ready
            .iter()
            .map(|item| format!("{}:{}:", queue.name, item.id))
            .collect::<Vec<_>>();
        for item in &ready {
            let prefix = format!("{}:{}:", queue.name, item.id);
            if existing.iter().any(|fact| fact.key.starts_with(&prefix)) {
                continue;
            }
            // Salt the key with the item's update generation: a released
            // item re-projects as a fresh fact instead of colliding with
            // its retired predecessor.
            let key = format!("{prefix}{}", stable_hash_hex(&item.updated_at));
            let value_json = json!({
                "queue": queue.name,
                "id": item.id,
                "title": item.title,
                "body": item.body,
                "status": item.status,
                "labels": item.labels,
                "metadata": item.metadata,
            })
            .to_string();
            // Salt with updated_at: a released item re-projects as a fresh
            // fact generation instead of colliding with its retired one.
            kernel.derive_fact(
                instance_id,
                "tracker.issue.ready",
                &key,
                &value_json,
                None,
                Some(&idempotency_key(&[
                    instance_id,
                    "tracker.issue.ready",
                    &key,
                    &item.updated_at,
                ])),
            )?;
        }
        for fact in existing {
            if !ready_prefixes
                .iter()
                .any(|prefix| fact.key.starts_with(prefix))
            {
                kernel.store_mut().retire_fact(instance_id, &fact.fact_id)?;
            }
        }
    }
    Ok(())
}

pub fn lowering_idempotency_key(lowering: &OwnedLowering) -> String {
    let mut ids = Vec::new();
    ids.extend(lowering.facts.iter().map(|fact| fact.fact_id.as_str()));
    ids.extend(
        lowering
            .consumed_fact_ids
            .iter()
            .map(|fact_id| fact_id.as_str()),
    );
    ids.extend(
        lowering
            .effects
            .iter()
            .map(|effect| effect.effect_id.as_str()),
    );
    ids.extend(
        lowering
            .dependencies
            .iter()
            .map(|dependency| dependency.dependency_id.as_str()),
    );
    if let Some(terminal) = &lowering.terminal {
        ids.push(terminal.idempotency_key.as_str());
    }
    idempotency_key(&ids)
}

/// Applies `cancel <binding>` operations committed by a rule: pending
/// effects terminal-cancel; running effects get a cancellation request (a
/// request, not a result); already-terminal effects are a recorded no-op.
pub fn apply_rule_cancels<S: RuntimeStore>(
    kernel: &mut RuntimeKernel<S>,
    instance_id: &str,
    rule_name: &str,
    effect_ids: &[String],
    causation_event_id: &str,
) -> Result<(), StoreError> {
    for effect_id in effect_ids {
        let status = kernel
            .store()
            .list_effects(instance_id)?
            .into_iter()
            .find(|effect| &effect.effect_id == effect_id)
            .map(|effect| effect.status);
        match status.as_deref() {
            Some("running") => {
                let _ = kernel
                    .store_mut()
                    .request_effect_cancellation(EffectCancellationRequest {
                        instance_id,
                        effect_id,
                        revision_id: None,
                        reason: Some("cancelled by rule"),
                        requested_by: rule_name,
                        causation_event_id: Some(causation_event_id),
                        idempotency_key: Some(&idempotency_key(&[
                            instance_id,
                            effect_id,
                            rule_name,
                            "rule-cancel-request",
                        ])),
                    });
            }
            Some("completed") | Some("failed") | Some("timed_out") | Some("cancelled") => {
                // No-op with evidence: cancelling settled work is legal.
                kernel.store().record_diagnostic(DiagnosticRecord {
                    instance_id: Some(instance_id),
                    program_id: None,
                    program_version_id: None,
                    severity: Severity::Info,
                    code: Some("cancel.noop"),
                    message: &format!(
                        "rule `{rule_name}` cancelled effect `{effect_id}` after it reached a terminal status"
                    ),
                    source_span_json: None,
                    subject_type: Some("effect"),
                    subject_id: Some(effect_id),
                    event_id: Some(causation_event_id),
                    effect_id: Some(effect_id),
                    run_id: None,
                    assertion_id: None,
                    evidence_ids_json: "[]",
                    artifact_ids_json: "[]",
                    causation_id: Some(causation_event_id),
                    correlation_id: None,
                    idempotency_key: Some(&idempotency_key(&[
                        instance_id,
                        effect_id,
                        rule_name,
                        "rule-cancel-noop",
                    ])),
                })?;
            }
            Some(_) => {
                kernel.cancel_effect(EffectCancellation {
                    instance_id,
                    effect_id,
                    reason: Some("cancelled by rule"),
                    idempotency_key: Some(&idempotency_key(&[
                        instance_id,
                        effect_id,
                        rule_name,
                        "rule-cancel",
                    ])),
                })?;
            }
            None => {}
        }
    }
    Ok(())
}
