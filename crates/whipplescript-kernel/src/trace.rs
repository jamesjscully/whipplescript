//! Trace-conformance checks for abstract runtime lifecycle events.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyPredicate {
    Succeeds,
    Fails,
    Completes,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectStatus {
    Queued,
    Blocked,
    Claimed,
    Running,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

impl EffectStatus {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::TimedOut | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyEdge {
    pub upstream_effect_id: String,
    pub predicate: DependencyPredicate,
    pub downstream_effect_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceEvent {
    EffectCreated {
        effect_id: String,
        status: EffectStatus,
    },
    DependencyCreated(DependencyEdge),
    EffectClaimed {
        effect_id: String,
    },
    RunStarted {
        run_id: String,
        effect_id: String,
    },
    LeaseExpired {
        run_id: String,
        effect_id: String,
    },
    EffectTerminal {
        run_id: String,
        effect_id: String,
        status: EffectStatus,
    },
    ProviderDiagnostic {
        run_id: String,
        effect_id: String,
        provider: String,
        status: EffectStatus,
        summary: String,
        diagnostics_json: String,
    },
    EffectBlocked {
        effect_id: String,
        status: Option<String>,
        reason: String,
    },
    EffectCancelled {
        effect_id: String,
    },
    RevisionActivated {
        revision_id: String,
        from_version_id: String,
        to_version_id: String,
        from_epoch: i64,
        to_epoch: i64,
        cancellation_policy: String,
        terminal_cancel_effects: Vec<String>,
        request_cancel_effects: Vec<String>,
    },
    EffectCancellationRequested {
        effect_id: String,
        revision_id: Option<String>,
        reason: Option<String>,
        requested_by: String,
    },
    InstancePaused,
    InstanceResumed,
    InstanceCancelled,
    InstanceFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRecord {
    pub sequence: u64,
    pub event: TraceEvent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceViolation {
    pub sequence: u64,
    pub message: String,
}

#[derive(Default)]
struct TraceState {
    effects: BTreeMap<String, EffectStatus>,
    run_effects: BTreeMap<String, String>,
    live_runs: BTreeSet<String>,
    stale_runs: BTreeSet<String>,
    terminal_effects: BTreeSet<String>,
    cancel_requested_effects: BTreeSet<String>,
    dependencies: Vec<DependencyEdge>,
    revision_epoch: i64,
    cancelled: bool,
    paused: bool,
}

pub fn check_trace(records: &[TraceRecord]) -> Result<(), TraceViolation> {
    let mut state = TraceState::default();
    for (expected_sequence, record) in (1..).zip(records.iter()) {
        if record.sequence != expected_sequence {
            return Err(TraceViolation {
                sequence: record.sequence,
                message: format!(
                    "event sequence gap: expected {expected_sequence}, got {}",
                    record.sequence
                ),
            });
        }

        check_record(&mut state, record)?;
    }

    Ok(())
}

fn check_record(state: &mut TraceState, record: &TraceRecord) -> Result<(), TraceViolation> {
    match &record.event {
        TraceEvent::EffectCreated { effect_id, status } => {
            if state.effects.contains_key(effect_id) {
                return violation(record, format!("effect {effect_id} was created twice"));
            }
            state.effects.insert(effect_id.clone(), status.clone());
        }
        TraceEvent::DependencyCreated(edge) => {
            if !state.effects.contains_key(&edge.upstream_effect_id) {
                return violation(
                    record,
                    format!(
                        "dependency references unknown upstream {}",
                        edge.upstream_effect_id
                    ),
                );
            }
            if !state.effects.contains_key(&edge.downstream_effect_id) {
                return violation(
                    record,
                    format!(
                        "dependency references unknown downstream {}",
                        edge.downstream_effect_id
                    ),
                );
            }
            state.dependencies.push(edge.clone());
        }
        TraceEvent::EffectClaimed { effect_id } => {
            if state.cancelled {
                return violation(record, "effect claimed after instance cancellation");
            }
            if state.paused {
                return violation(record, "effect claimed while instance is paused");
            }
            let Some(status) = state.effects.get(effect_id) else {
                return violation(record, format!("unknown effect {effect_id} claimed"));
            };
            if *status != EffectStatus::Queued {
                return violation(
                    record,
                    format!("effect {effect_id} claimed from non-queued status {status:?}"),
                );
            }
            if state.cancel_requested_effects.contains(effect_id) {
                return violation(
                    record,
                    format!("effect {effect_id} claimed after cancellation request"),
                );
            }
            if let Some(edge) = first_unsatisfied_dependency(state, effect_id) {
                return violation(
                    record,
                    format!(
                        "effect {effect_id} claimed before dependency on {} was satisfied",
                        edge.upstream_effect_id
                    ),
                );
            }
            state
                .effects
                .insert(effect_id.clone(), EffectStatus::Claimed);
        }
        TraceEvent::RunStarted { run_id, effect_id } => {
            if state.cancelled {
                return violation(record, "run started after instance cancellation");
            }
            if state.run_effects.contains_key(run_id) {
                return violation(record, format!("run {run_id} was started twice"));
            }
            let Some(status) = state.effects.get(effect_id) else {
                return violation(
                    record,
                    format!("run started for unknown effect {effect_id}"),
                );
            };
            if *status != EffectStatus::Claimed {
                return violation(
                    record,
                    format!("run started for effect {effect_id} in status {status:?}"),
                );
            }
            state
                .effects
                .insert(effect_id.clone(), EffectStatus::Running);
            state.run_effects.insert(run_id.clone(), effect_id.clone());
            state.live_runs.insert(run_id.clone());
        }
        TraceEvent::LeaseExpired { run_id, effect_id } => {
            let Some(run_effect_id) = state.run_effects.get(run_id) else {
                return violation(record, format!("lease expired for unknown run {run_id}"));
            };
            if run_effect_id != effect_id {
                return violation(
                    record,
                    format!("lease expired for run {run_id} on wrong effect {effect_id}"),
                );
            }
            if !state.live_runs.remove(run_id) {
                return violation(record, format!("lease expired for non-live run {run_id}"));
            }
            let Some(status) = state.effects.get(effect_id) else {
                return violation(
                    record,
                    format!("lease expired for unknown effect {effect_id}"),
                );
            };
            if *status != EffectStatus::Running {
                return violation(
                    record,
                    format!("lease expired for effect {effect_id} in status {status:?}"),
                );
            }
            state.stale_runs.insert(run_id.clone());
            state
                .effects
                .insert(effect_id.clone(), EffectStatus::Queued);
        }
        TraceEvent::EffectTerminal {
            run_id,
            effect_id,
            status,
        } => {
            if !status.is_terminal() {
                return violation(
                    record,
                    format!("terminal event used non-terminal status {status:?}"),
                );
            }
            if !state.effects.contains_key(effect_id) {
                return violation(
                    record,
                    format!("terminal event for unknown effect {effect_id}"),
                );
            }
            if state.terminal_effects.contains(effect_id) {
                return violation(
                    record,
                    format!("duplicate terminal event for effect {effect_id}"),
                );
            }
            let Some(run_effect_id) = state.run_effects.get(run_id) else {
                return violation(record, format!("terminal event for unknown run {run_id}"));
            };
            if run_effect_id != effect_id {
                return violation(
                    record,
                    format!("terminal event for run {run_id} on wrong effect {effect_id}"),
                );
            }
            if state.stale_runs.contains(run_id) {
                return violation(record, format!("terminal event from stale run {run_id}"));
            }
            if !state.live_runs.remove(run_id) {
                return violation(record, format!("terminal event for non-live run {run_id}"));
            }
            state.terminal_effects.insert(effect_id.clone());
            state.cancel_requested_effects.remove(effect_id);
            state.effects.insert(effect_id.clone(), status.clone());
        }
        TraceEvent::ProviderDiagnostic {
            run_id,
            effect_id,
            status,
            diagnostics_json,
            ..
        } => {
            if !status.is_terminal() {
                return violation(
                    record,
                    format!("provider diagnostic used non-terminal status {status:?}"),
                );
            }
            let Some(run_effect_id) = state.run_effects.get(run_id) else {
                return violation(
                    record,
                    format!("provider diagnostic for unknown run {run_id}"),
                );
            };
            if run_effect_id != effect_id {
                return violation(
                    record,
                    format!("provider diagnostic for run {run_id} on wrong effect {effect_id}"),
                );
            }
            if state.stale_runs.contains(run_id) {
                return violation(
                    record,
                    format!("provider diagnostic from stale run {run_id}"),
                );
            }
            if !state.live_runs.contains(run_id) {
                return violation(
                    record,
                    format!("provider diagnostic for non-live run {run_id}"),
                );
            }
            let Some(effect_status) = state.effects.get(effect_id) else {
                return violation(
                    record,
                    format!("provider diagnostic for unknown effect {effect_id}"),
                );
            };
            if *effect_status != EffectStatus::Running {
                return violation(
                    record,
                    format!(
                        "provider diagnostic for effect {effect_id} in status {effect_status:?}"
                    ),
                );
            }
            if serde_json::from_str::<serde_json::Value>(diagnostics_json).is_err() {
                return violation(record, "provider diagnostic metadata is not valid JSON");
            }
        }
        TraceEvent::EffectBlocked {
            effect_id,
            status: blocked_status,
            ..
        } => {
            let Some(status) = state.effects.get(effect_id) else {
                return violation(record, format!("blocked unknown effect {effect_id}"));
            };
            if status.is_terminal() || *status == EffectStatus::Running {
                return violation(
                    record,
                    format!("effect {effect_id} blocked from invalid status {status:?}"),
                );
            }
            if matches!(blocked_status.as_deref(), Some("blocked_by_dependency"))
                && first_unsatisfied_dependency(state, effect_id).is_none()
            {
                return violation(
                    record,
                    format!(
                        "effect {effect_id} marked blocked_by_dependency without an unsatisfied dependency"
                    ),
                );
            }
            state
                .effects
                .insert(effect_id.clone(), EffectStatus::Blocked);
        }
        TraceEvent::EffectCancelled { effect_id } => {
            let Some(status) = state.effects.get(effect_id) else {
                return violation(record, format!("cancelled unknown effect {effect_id}"));
            };
            if status.is_terminal() {
                return violation(
                    record,
                    format!("effect {effect_id} cancelled from terminal status {status:?}"),
                );
            }
            state.terminal_effects.insert(effect_id.clone());
            state.cancel_requested_effects.remove(effect_id);
            state
                .effects
                .insert(effect_id.clone(), EffectStatus::Cancelled);
        }
        TraceEvent::RevisionActivated {
            revision_id,
            from_epoch,
            to_epoch,
            cancellation_policy,
            ..
        } => {
            if revision_id.is_empty() {
                return violation(record, "revision activation has empty revision id");
            }
            if *from_epoch != state.revision_epoch {
                return violation(
                    record,
                    format!(
                        "revision activation from epoch {from_epoch} but trace is at epoch {}",
                        state.revision_epoch
                    ),
                );
            }
            if *to_epoch <= *from_epoch {
                return violation(
                    record,
                    format!("revision activation did not advance epoch {from_epoch}->{to_epoch}"),
                );
            }
            if !matches!(
                cancellation_policy.as_str(),
                "keep" | "cancel_queued" | "request_running"
            ) {
                return violation(
                    record,
                    format!("unknown revision cancellation policy {cancellation_policy}"),
                );
            }
            state.revision_epoch = *to_epoch;
        }
        TraceEvent::EffectCancellationRequested {
            effect_id,
            revision_id,
            requested_by,
            ..
        } => {
            if revision_id.as_deref() == Some("") {
                return violation(record, "cancellation request has empty revision id");
            }
            if requested_by.is_empty() {
                return violation(record, "cancellation request has empty requester");
            }
            let Some(status) = state.effects.get(effect_id) else {
                return violation(
                    record,
                    format!("cancellation requested for unknown effect {effect_id}"),
                );
            };
            if *status != EffectStatus::Running {
                return violation(
                    record,
                    format!("cancellation requested for effect {effect_id} in status {status:?}"),
                );
            }
            if !state.cancel_requested_effects.insert(effect_id.clone()) {
                return violation(
                    record,
                    format!("duplicate cancellation request for effect {effect_id}"),
                );
            }
        }
        TraceEvent::InstancePaused => {
            state.paused = true;
        }
        TraceEvent::InstanceResumed => {
            if state.cancelled {
                return violation(record, "cancelled instance resumed");
            }
            state.paused = false;
        }
        TraceEvent::InstanceCancelled => {
            state.cancelled = true;
            state.paused = true;
        }
        // A generic internal failure is a terminal; replay records it like any
        // other terminal and reprojects identically (no extra trace invariant).
        TraceEvent::InstanceFailed => {}
    }

    Ok(())
}

fn first_unsatisfied_dependency<'a>(
    state: &'a TraceState,
    effect_id: &str,
) -> Option<&'a DependencyEdge> {
    state
        .dependencies
        .iter()
        .filter(|edge| edge.downstream_effect_id == effect_id)
        .find(|edge| !dependency_satisfied(state, edge))
}

fn dependency_satisfied(state: &TraceState, edge: &DependencyEdge) -> bool {
    let Some(status) = state.effects.get(&edge.upstream_effect_id) else {
        return false;
    };

    match edge.predicate {
        DependencyPredicate::Succeeds => *status == EffectStatus::Completed,
        DependencyPredicate::Fails => {
            matches!(status, EffectStatus::Failed | EffectStatus::TimedOut)
        }
        DependencyPredicate::Completes => status.is_terminal(),
    }
}

fn violation<T>(record: &TraceRecord, message: impl Into<String>) -> Result<T, TraceViolation> {
    Err(TraceViolation {
        sequence: record.sequence,
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect_created(sequence: u64, effect_id: &str) -> TraceRecord {
        TraceRecord {
            sequence,
            event: TraceEvent::EffectCreated {
                effect_id: effect_id.to_owned(),
                status: EffectStatus::Queued,
            },
        }
    }

    fn claim(sequence: u64, effect_id: &str) -> TraceRecord {
        TraceRecord {
            sequence,
            event: TraceEvent::EffectClaimed {
                effect_id: effect_id.to_owned(),
            },
        }
    }

    fn start(sequence: u64, effect_id: &str) -> TraceRecord {
        TraceRecord {
            sequence,
            event: TraceEvent::RunStarted {
                run_id: format!("run-{effect_id}"),
                effect_id: effect_id.to_owned(),
            },
        }
    }

    fn expire_lease(sequence: u64, effect_id: &str) -> TraceRecord {
        TraceRecord {
            sequence,
            event: TraceEvent::LeaseExpired {
                run_id: format!("run-{effect_id}"),
                effect_id: effect_id.to_owned(),
            },
        }
    }

    fn terminal(sequence: u64, effect_id: &str, status: EffectStatus) -> TraceRecord {
        TraceRecord {
            sequence,
            event: TraceEvent::EffectTerminal {
                run_id: format!("run-{effect_id}"),
                effect_id: effect_id.to_owned(),
                status,
            },
        }
    }

    fn cancellation_request(sequence: u64, effect_id: &str) -> TraceRecord {
        TraceRecord {
            sequence,
            event: TraceEvent::EffectCancellationRequested {
                effect_id: effect_id.to_owned(),
                revision_id: Some("rev-a".to_owned()),
                reason: Some("workflow revision".to_owned()),
                requested_by: "workflow.revision".to_owned(),
            },
        }
    }

    fn revision_activated(sequence: u64, from_epoch: i64, to_epoch: i64) -> TraceRecord {
        TraceRecord {
            sequence,
            event: TraceEvent::RevisionActivated {
                revision_id: format!("rev-{to_epoch}"),
                from_version_id: format!("version-{from_epoch}"),
                to_version_id: format!("version-{to_epoch}"),
                from_epoch,
                to_epoch,
                cancellation_policy: "request_running".to_owned(),
                terminal_cancel_effects: Vec::new(),
                request_cancel_effects: Vec::new(),
            },
        }
    }

    fn diagnostic(sequence: u64, effect_id: &str, status: EffectStatus) -> TraceRecord {
        TraceRecord {
            sequence,
            event: TraceEvent::ProviderDiagnostic {
                run_id: format!("run-{effect_id}"),
                effect_id: effect_id.to_owned(),
                provider: "test".to_owned(),
                status,
                summary: "provider failed".to_owned(),
                diagnostics_json: r#"{"error":"boom"}"#.to_owned(),
            },
        }
    }

    fn dependency_block(sequence: u64, effect_id: &str) -> TraceRecord {
        TraceRecord {
            sequence,
            event: TraceEvent::EffectBlocked {
                effect_id: effect_id.to_owned(),
                status: Some("blocked_by_dependency".to_owned()),
                reason: "effect dependencies are not satisfied".to_owned(),
            },
        }
    }

    #[test]
    fn accepts_claim_after_success_dependency() {
        let trace = vec![
            effect_created(1, "upstream"),
            effect_created(2, "downstream"),
            TraceRecord {
                sequence: 3,
                event: TraceEvent::DependencyCreated(DependencyEdge {
                    upstream_effect_id: "upstream".to_owned(),
                    predicate: DependencyPredicate::Succeeds,
                    downstream_effect_id: "downstream".to_owned(),
                }),
            },
            claim(4, "upstream"),
            start(5, "upstream"),
            terminal(6, "upstream", EffectStatus::Completed),
            claim(7, "downstream"),
            start(8, "downstream"),
        ];

        assert_eq!(check_trace(&trace), Ok(()));
    }

    #[test]
    fn accepts_provider_diagnostic_before_terminal_event() {
        let trace = vec![
            effect_created(1, "a"),
            claim(2, "a"),
            start(3, "a"),
            diagnostic(4, "a", EffectStatus::Failed),
            terminal(5, "a", EffectStatus::Failed),
        ];

        assert_eq!(check_trace(&trace), Ok(()));
    }

    #[test]
    fn accepts_cancellation_request_before_terminal_event() {
        let trace = vec![
            effect_created(1, "a"),
            claim(2, "a"),
            start(3, "a"),
            cancellation_request(4, "a"),
            terminal(5, "a", EffectStatus::Completed),
        ];

        assert_eq!(check_trace(&trace), Ok(()));
    }

    #[test]
    fn accepts_monotonic_revision_activation() {
        let trace = vec![revision_activated(1, 0, 1), revision_activated(2, 1, 2)];

        assert_eq!(check_trace(&trace), Ok(()));
    }

    #[test]
    fn rejects_sequence_gap() {
        let trace = vec![effect_created(1, "a"), claim(3, "a")];

        let violation = check_trace(&trace).expect_err("sequence gap should fail");
        assert!(violation.message.contains("sequence gap"));
    }

    #[test]
    fn rejects_claim_before_dependency_satisfied() {
        let trace = vec![
            effect_created(1, "upstream"),
            effect_created(2, "downstream"),
            TraceRecord {
                sequence: 3,
                event: TraceEvent::DependencyCreated(DependencyEdge {
                    upstream_effect_id: "upstream".to_owned(),
                    predicate: DependencyPredicate::Succeeds,
                    downstream_effect_id: "downstream".to_owned(),
                }),
            },
            claim(4, "downstream"),
        ];

        let violation = check_trace(&trace).expect_err("unsatisfied dependency should fail");
        assert!(violation.message.contains("before dependency"));
    }

    #[test]
    fn accepts_dependency_block_for_unsatisfied_dependency() {
        let trace = vec![
            effect_created(1, "upstream"),
            effect_created(2, "downstream"),
            TraceRecord {
                sequence: 3,
                event: TraceEvent::DependencyCreated(DependencyEdge {
                    upstream_effect_id: "upstream".to_owned(),
                    predicate: DependencyPredicate::Succeeds,
                    downstream_effect_id: "downstream".to_owned(),
                }),
            },
            dependency_block(4, "downstream"),
        ];

        assert_eq!(check_trace(&trace), Ok(()));
    }

    #[test]
    fn rejects_dependency_block_without_unsatisfied_dependency() {
        let trace = vec![
            effect_created(1, "downstream"),
            dependency_block(2, "downstream"),
        ];

        let violation =
            check_trace(&trace).expect_err("dependency block without dependency should fail");
        assert!(violation
            .message
            .contains("without an unsatisfied dependency"));
    }

    #[test]
    fn rejects_dependency_block_for_satisfied_failure_dependency() {
        let trace = vec![
            effect_created(1, "upstream"),
            effect_created(2, "downstream"),
            TraceRecord {
                sequence: 3,
                event: TraceEvent::DependencyCreated(DependencyEdge {
                    upstream_effect_id: "upstream".to_owned(),
                    predicate: DependencyPredicate::Fails,
                    downstream_effect_id: "downstream".to_owned(),
                }),
            },
            claim(4, "upstream"),
            start(5, "upstream"),
            terminal(6, "upstream", EffectStatus::Failed),
            dependency_block(7, "downstream"),
        ];

        let violation =
            check_trace(&trace).expect_err("satisfied failure dependency block should fail");
        assert!(violation
            .message
            .contains("without an unsatisfied dependency"));
    }

    #[test]
    fn rejects_duplicate_terminal_completion() {
        let trace = vec![
            effect_created(1, "a"),
            claim(2, "a"),
            start(3, "a"),
            terminal(4, "a", EffectStatus::Completed),
            terminal(5, "a", EffectStatus::Failed),
        ];

        let violation = check_trace(&trace).expect_err("duplicate terminal should fail");
        assert!(violation.message.contains("duplicate terminal"));
    }

    #[test]
    fn rejects_stale_lease_completion() {
        let trace = vec![
            effect_created(1, "a"),
            claim(2, "a"),
            start(3, "a"),
            expire_lease(4, "a"),
            terminal(5, "a", EffectStatus::Completed),
        ];

        let violation = check_trace(&trace).expect_err("stale lease completion should fail");
        assert!(violation.message.contains("stale run"));
    }

    #[test]
    fn rejects_provider_diagnostic_after_terminal_event() {
        let trace = vec![
            effect_created(1, "a"),
            claim(2, "a"),
            start(3, "a"),
            terminal(4, "a", EffectStatus::Failed),
            diagnostic(5, "a", EffectStatus::Failed),
        ];

        let violation = check_trace(&trace).expect_err("late diagnostic should fail");
        assert!(violation.message.contains("non-live run"));
    }

    #[test]
    fn rejects_provider_diagnostic_with_invalid_json() {
        let trace = vec![
            effect_created(1, "a"),
            claim(2, "a"),
            start(3, "a"),
            TraceRecord {
                sequence: 4,
                event: TraceEvent::ProviderDiagnostic {
                    run_id: "run-a".to_owned(),
                    effect_id: "a".to_owned(),
                    provider: "test".to_owned(),
                    status: EffectStatus::Failed,
                    summary: "provider failed".to_owned(),
                    diagnostics_json: "not json".to_owned(),
                },
            },
        ];

        let violation = check_trace(&trace).expect_err("invalid diagnostic JSON should fail");
        assert!(violation.message.contains("valid JSON"));
    }

    #[test]
    fn rejects_cancellation_request_for_non_running_effect() {
        let trace = vec![effect_created(1, "a"), cancellation_request(2, "a")];

        let violation =
            check_trace(&trace).expect_err("cancellation request before running should fail");
        assert!(violation.message.contains("in status Queued"));
    }

    #[test]
    fn rejects_duplicate_cancellation_request() {
        let trace = vec![
            effect_created(1, "a"),
            claim(2, "a"),
            start(3, "a"),
            cancellation_request(4, "a"),
            cancellation_request(5, "a"),
        ];

        let violation = check_trace(&trace).expect_err("duplicate cancellation request fails");
        assert!(violation.message.contains("duplicate cancellation request"));
    }

    #[test]
    fn rejects_claim_after_cancellation_request_and_lease_expiry() {
        let trace = vec![
            effect_created(1, "a"),
            claim(2, "a"),
            start(3, "a"),
            cancellation_request(4, "a"),
            expire_lease(5, "a"),
            claim(6, "a"),
        ];

        let violation = check_trace(&trace).expect_err("cancel-requested effect claim fails");
        assert!(violation.message.contains("after cancellation request"));
    }

    #[test]
    fn rejects_revision_activation_with_stale_epoch() {
        let trace = vec![revision_activated(1, 1, 2)];

        let violation = check_trace(&trace).expect_err("stale revision epoch should fail");
        assert!(violation.message.contains("trace is at epoch 0"));
    }

    #[test]
    fn rejects_run_started_after_cancel() {
        let trace = vec![
            effect_created(1, "a"),
            claim(2, "a"),
            TraceRecord {
                sequence: 3,
                event: TraceEvent::InstanceCancelled,
            },
            start(4, "a"),
        ];

        let violation = check_trace(&trace).expect_err("start after cancel should fail");
        assert!(violation.message.contains("after instance cancellation"));
    }
}
