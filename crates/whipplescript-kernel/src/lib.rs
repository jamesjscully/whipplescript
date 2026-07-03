//! Deterministic runtime kernel scaffold.

pub mod artifact_manifest;
#[cfg(feature = "claude")]
pub mod claude_agent_sdk;
#[cfg(feature = "codex")]
pub mod codex_app_server;
pub mod coerce;
pub mod coerce_native;
pub mod harness;
pub mod harness_loop;
pub mod harness_model;
pub mod loft;
pub mod lowering;
pub mod native_lifecycle;
pub mod pi_rpc;
pub mod provider;
pub mod rule_lowering;
pub mod sansio;
pub mod trace;

use artifact_manifest::{
    artifact_capture_failed_payload, provider_artifact_manifest, validate_artifact_manifest,
    ArtifactCaptureFailure,
};
use coerce::{CoerceClient, CoerceRequest, CoerceResult, CoerceStatus};
use harness::{
    AgentHarness, AgentTurnRequest, ProviderFailure, ProviderRunResult, ProviderRunStatus,
};
use loft::{LoftAction, LoftClient, LoftEffectRequest, LoftEffectResult, LoftEffectStatus};
use native_lifecycle::{AgentTurnLifecycleKind, NativeAgentTurnObservation};
use provider::{
    NativeProviderAdapter, NativeProviderArtifactRef, NativeProviderEvent, NativeProviderEventKind,
    NativeProviderTurnRequest,
};
use serde_json::{json, Value};
use trace::{DependencyEdge, EffectStatus, TraceEvent, TraceRecord};
use whipplescript_core::Severity;
use whipplescript_parser::{
    DependencyPredicate, IrEffectKind, IrPrimitiveType, IrProgram, IrSchema, IrType,
    IrWorkflowContractKind, SourceSpan,
};
use whipplescript_store::{
    ArtifactRecord, ClaimableEffect, DerivedFact, DiagnosticRecord, EffectCancellation,
    EffectCompletion, EvidenceRecord, ExpiredLease, FactBatch, FactBatchOutcome,
    InstanceTransition, LeaseRenewal, NewEffectDependency, NewEvent, NewFact, NewInboxItem,
    NewInstance, NewInstanceAuthority, NewProgramVersion, NewWorkflowInvocation,
    ProgramVersionRecord, RetryEffect, RevisionActivation, RuleCommit, RuleCommitRevisionGuard,
    RunStart, RuntimeStore, SkillEvidence, StoreError, StoreResult, StoredEvent,
    TerminalDiagnosticRecord, WorkflowInvocationView, WorkflowRevisionView,
};
// `SqliteStore` (the rusqlite store) is the default backend for
// `RuntimeKernel` under the `native` feature and is used by tests; the kernel's
// non-test code is otherwise generic over `RuntimeStore`, so it builds on wasm
// (no-native) without it.
#[cfg(feature = "native")]
use whipplescript_store::SqliteStore;

/// The runtime kernel over a [`RuntimeStore`] backend. Native defaults the
/// backend to `SqliteStore` (so existing `RuntimeKernel` type references keep
/// working); the durable-object host (Phase 5) uses `RuntimeKernel<DoSqliteStore>`.
/// The kernel calls only trait methods, so it is decoupled from the concrete
/// rusqlite store and builds on wasm without it (DR-0033 Phase 5) — where there
/// is no default backend, so `S` is always explicit.
#[cfg(feature = "native")]
pub struct RuntimeKernel<S: RuntimeStore = SqliteStore> {
    store: S,
    trace: Vec<TraceRecord>,
}

/// wasm / no-native form: no default backend (rusqlite `SqliteStore` is absent).
#[cfg(not(feature = "native"))]
pub struct RuntimeKernel<S: RuntimeStore> {
    store: S,
    trace: Vec<TraceRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgramVersionInput<'a> {
    pub program_name: &'a str,
    pub source_hash: &'a str,
    pub ir_hash: &'a str,
    pub compiler_version: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTurnExecution<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub run_id: &'a str,
    pub provider: &'a str,
    pub worker_id: &'a str,
    pub lease_id: &'a str,
    pub lease_expires_at: &'a str,
    pub agent: &'a str,
    pub profile: Option<&'a str>,
    pub input_json: &'a str,
    pub skill_names: &'a [&'a str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoerceExecution<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub run_id: &'a str,
    pub provider: &'a str,
    pub worker_id: &'a str,
    pub lease_id: &'a str,
    pub lease_expires_at: &'a str,
    pub request: &'a CoerceRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoftEffectExecution<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub run_id: &'a str,
    pub provider: &'a str,
    pub worker_id: &'a str,
    pub lease_id: &'a str,
    pub lease_expires_at: &'a str,
    pub request: &'a LoftEffectRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanAskExecution<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub run_id: &'a str,
    pub provider: &'a str,
    pub worker_id: &'a str,
    pub lease_id: &'a str,
    pub lease_expires_at: &'a str,
    pub inbox_item_id: &'a str,
    pub prompt: &'a str,
    pub choices_json: &'a str,
    pub freeform_allowed: bool,
    pub severity: &'a str,
    pub related_effects_json: &'a str,
    pub related_artifacts_json: &'a str,
}

#[derive(Debug, Default)]
struct ProviderEvidence {
    evidence_id: Option<String>,
    artifact_ids: Vec<String>,
}

pub fn idempotency_key(parts: &[&str]) -> String {
    let mut input = String::new();
    for part in parts {
        input.push_str(&part.len().to_string());
        input.push(':');
        input.push_str(part);
        input.push(';');
    }
    format!("key_{:016x}", stable_hash(&input))
}

/// Identifies the effect a brokered turn settles. The owned harness runner uses
/// it to open the run, attribute evidence, and key the terminal fact.
pub struct BrokeredTurnContext<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub agent: &'a str,
    pub profile: Option<&'a str>,
}

/// Evidence kind + shape-only metadata for one in-turn observation. Per DR-0024
/// these are evidence, never facts (I2): no tool args or results content is
/// recorded here, only the call shape (tool name, status, step).
fn brokered_observation_evidence(
    obs: &crate::harness_loop::LoopObservation,
) -> (&'static str, Value) {
    use crate::harness_loop::{LoopObservation, ToolStatus};
    match obs {
        LoopObservation::ModelRequest { step } => {
            ("agent.turn.brokered.model_request", json!({ "step": step }))
        }
        LoopObservation::ToolRequested { call_id, name } => (
            "agent.turn.tool_requested",
            json!({ "call_id": call_id, "tool": name }),
        ),
        LoopObservation::ToolResult {
            call_id,
            name,
            status,
        } => (
            "agent.turn.brokered.tool_result",
            json!({
                "call_id": call_id,
                "tool": name,
                "status": match status { ToolStatus::Ok => "ok", ToolStatus::Error => "error" },
            }),
        ),
    }
}

impl<S: RuntimeStore> RuntimeKernel<S> {
    pub fn new(store: S) -> Self {
        Self {
            store,
            trace: Vec::new(),
        }
    }

    /// Run one owned/brokered agent turn (DR-0024 slice 1).
    ///
    /// Opens the run, drives the pure brokered loop (the kernel executes every
    /// requested tool via `executor` — I1), records each in-turn observation as
    /// EVIDENCE only (I2: no rule-matchable fact), then settles the single
    /// terminal and derives exactly one `agent.turn.<status>` fact (layer 3) so
    /// `after <turn> succeeds` matches it just like the delegating harness.
    pub fn run_brokered_agent_turn<C, E>(
        &mut self,
        ctx: &BrokeredTurnContext<'_>,
        client: &C,
        executor: &E,
        input: &crate::harness_loop::BrokeredTurnInput,
    ) -> StoreResult<StoredEvent>
    where
        C: crate::harness_loop::HarnessModelClient + ?Sized,
        E: crate::harness_loop::ToolExecutor + ?Sized,
    {
        use crate::harness_loop::TurnStatus;

        let run_id = idempotency_key(&[ctx.instance_id, ctx.effect_id, "brokered-run"]);
        let lease_id = idempotency_key(&[ctx.instance_id, ctx.effect_id, "brokered-lease"]);
        self.start_run(RunStart {
            instance_id: ctx.instance_id,
            effect_id: ctx.effect_id,
            run_id: &run_id,
            provider: "owned-harness",
            worker_id: "whip-owned-harness",
            lease_id: &lease_id,
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: &json!({ "agent": ctx.agent }).to_string(),
        })?;

        // Resume-from-projection (slice 6): if a prior interrupted run left a
        // persisted transcript for this effect, continue from it; otherwise start
        // fresh. The loop persists the transcript after each step via `checkpoint`.
        let resume_from = self.load_brokered_transcript(ctx.instance_id, ctx.effect_id);
        let resume_input;
        let run_input: &crate::harness_loop::BrokeredTurnInput = if resume_from.is_empty() {
            input
        } else {
            resume_input = crate::harness_loop::BrokeredTurnInput {
                resume_from,
                ..input.clone()
            };
            &resume_input
        };
        let outcome = {
            let store = &self.store;
            let mut step: usize = 0;
            let mut checkpoint = |messages: &[crate::harness_loop::ChatMessage]| {
                let key = idempotency_key(&[
                    ctx.instance_id,
                    ctx.effect_id,
                    "transcript",
                    &step.to_string(),
                ]);
                step += 1;
                let payload = json!({
                    "effect_id": ctx.effect_id,
                    "messages": crate::harness_loop::chat_messages_to_json(messages),
                })
                .to_string();
                let _ = store.append_event(NewEvent {
                    instance_id: ctx.instance_id,
                    event_type: "agent.turn.brokered.transcript",
                    payload_json: &payload,
                    source: "kernel",
                    causation_id: None,
                    correlation_id: Some(ctx.effect_id),
                    idempotency_key: Some(&key),
                });
            };
            crate::harness_loop::run_brokered_loop(client, executor, run_input, &mut checkpoint)
        };

        // In-turn observations are evidence, never facts (leaf-ness, I2).
        for (index, obs) in outcome.observations.iter().enumerate() {
            let (kind, metadata) = brokered_observation_evidence(obs);
            self.store.record_evidence(EvidenceRecord {
                instance_id: ctx.instance_id,
                kind,
                subject_type: "run",
                subject_id: &run_id,
                causation_id: None,
                correlation_id: None,
                summary: None,
                metadata_json: &json!({ "index": index, "observation": metadata }).to_string(),
            })?;
        }

        let terminal_key = idempotency_key(&[ctx.instance_id, ctx.effect_id, "terminal"]);
        let metadata_json = json!({
            "steps": outcome.steps,
            "usage": outcome.usage,
            "agent": ctx.agent,
        })
        .to_string();

        let (status, fact_name) = match outcome.status {
            TurnStatus::Completed => ("completed", "agent.turn.completed"),
            TurnStatus::Failed => ("failed", "agent.turn.failed"),
            TurnStatus::TimedOut => ("timed_out", "agent.turn.timed_out"),
        };

        let completion = EffectCompletion {
            instance_id: ctx.instance_id,
            effect_id: ctx.effect_id,
            run_id: &run_id,
            provider: "owned-harness",
            worker_id: "whip-owned-harness",
            status,
            exit_code: Some(if matches!(outcome.status, TurnStatus::Completed) {
                0
            } else {
                1
            }),
            summary: Some(&outcome.summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&terminal_key),
        };
        let terminal = match outcome.status {
            TurnStatus::Completed => self.complete_run(completion)?,
            TurnStatus::Failed => self.fail_run(completion)?,
            TurnStatus::TimedOut => self.timeout_run(completion)?,
        };

        // The single terminal fact (layer 3) -- the only fact a brokered turn
        // derives. Byte-for-byte the same `agent.turn.<status>` convention the
        // delegating harness emits (fact keyed by run_id, correlated to the
        // effect, provenance `effect`) so `after <turn> succeeds` /
        // `when <agent> completed turn` resolve identically. Mirrors
        // append_agent_turn_event_and_fact.
        let fact_payload = json!({
            "effect_id": ctx.effect_id,
            "run_id": run_id,
            "agent": ctx.agent,
            "provider": "owned-harness",
            "status": status,
            "summary": outcome.summary,
        })
        .to_string();
        let fact_id = idempotency_key(&[ctx.instance_id, "agent-turn", &run_id]);
        let fact_event_key = idempotency_key(&[ctx.instance_id, &run_id, "agent-turn-fact"]);
        self.store.append_event(NewEvent {
            instance_id: ctx.instance_id,
            event_type: fact_name,
            payload_json: &fact_payload,
            source: "kernel",
            causation_id: Some(&run_id),
            correlation_id: Some(ctx.effect_id),
            idempotency_key: Some(&idempotency_key(&[
                ctx.instance_id,
                &run_id,
                "agent-turn-event",
            ])),
        })?;
        self.store.derive_fact(DerivedFact {
            instance_id: ctx.instance_id,
            fact: NewFact {
                fact_id: &fact_id,
                name: fact_name,
                key: &run_id,
                value_json: &fact_payload,
                schema_id: None,
                provenance_class: "effect",
                correlation_id: Some(ctx.effect_id),
                source_span_json: None,
            },
            source: "kernel",
            causation_id: Some(&run_id),
            idempotency_key: Some(&fact_event_key),
        })?;

        Ok(terminal)
    }

    /// Load the latest persisted brokered-turn transcript for an effect, for
    /// resume-from-projection. Empty when none was recorded (a fresh turn).
    fn load_brokered_transcript(
        &self,
        instance_id: &str,
        effect_id: &str,
    ) -> Vec<crate::harness_loop::ChatMessage> {
        let Ok(events) = self.store.list_events(instance_id) else {
            return Vec::new();
        };
        let latest = events.iter().rev().find(|event| {
            if event.event_type != "agent.turn.brokered.transcript" {
                return false;
            }
            serde_json::from_str::<Value>(&event.payload_json)
                .ok()
                .and_then(|payload| {
                    payload
                        .get("effect_id")
                        .and_then(Value::as_str)
                        .map(|id| id == effect_id)
                })
                .unwrap_or(false)
        });
        match latest {
            Some(event) => serde_json::from_str::<Value>(&event.payload_json)
                .ok()
                .map(|payload| {
                    crate::harness_loop::chat_messages_from_json(
                        payload.get("messages").unwrap_or(&Value::Null),
                    )
                })
                .unwrap_or_default(),
            None => Vec::new(),
        }
    }

    pub fn into_store(self) -> S {
        self.store
    }

    pub fn trace(&self) -> &[TraceRecord] {
        &self.trace
    }

    pub fn create_program_version(
        &mut self,
        input: ProgramVersionInput<'_>,
    ) -> StoreResult<ProgramVersionRecord> {
        self.store.create_program_version(NewProgramVersion {
            program_name: input.program_name,
            source_hash: input.source_hash,
            ir_hash: input.ir_hash,
            compiler_version: input.compiler_version,
            declared_capabilities_json: "[]",
            declared_profiles_json: "[]",
            declared_skills_json: "[]",
            declared_schemas_json: "[]",
            analysis_summary_json: "{}",
            generated_artifacts_json: "[]",
            artifact_root: None,
        })
    }

    pub fn create_program_version_for_program(
        &mut self,
        input: ProgramVersionInput<'_>,
        program: &IrProgram,
    ) -> StoreResult<ProgramVersionRecord> {
        let declared_profiles_json = declared_profiles_json(program);
        let declared_skills_json = declared_skills_json(program);
        let declared_schemas_json = declared_schemas_json(program);
        let analysis_summary_json = program_analysis_summary_json(program);
        self.store.create_program_version(NewProgramVersion {
            program_name: input.program_name,
            source_hash: input.source_hash,
            ir_hash: input.ir_hash,
            compiler_version: input.compiler_version,
            declared_capabilities_json: "[]",
            declared_profiles_json: &declared_profiles_json,
            declared_skills_json: &declared_skills_json,
            declared_schemas_json: &declared_schemas_json,
            analysis_summary_json: &analysis_summary_json,
            generated_artifacts_json: "[]",
            artifact_root: None,
        })
    }

    pub fn create_instance(
        &self,
        version: &ProgramVersionRecord,
        input_json: &str,
    ) -> StoreResult<String> {
        self.create_instance_with_authority(version, input_json, NewInstanceAuthority::empty())
    }

    pub fn create_instance_with_authority(
        &self,
        version: &ProgramVersionRecord,
        input_json: &str,
        authority: NewInstanceAuthority<'_>,
    ) -> StoreResult<String> {
        self.store
            .create_instance_with_authority(
                NewInstance {
                    program_id: &version.program_id,
                    version_id: &version.version_id,
                    input_json,
                },
                authority,
            )
            .map(|instance| instance.instance_id)
    }

    pub fn ingest_external_event(
        &self,
        instance_id: &str,
        event_type: &str,
        payload_json: &str,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        self.store.append_event(NewEvent {
            instance_id,
            event_type,
            payload_json,
            source: "external",
            causation_id: None,
            correlation_id: None,
            idempotency_key,
        })
    }

    pub fn derive_fact(
        &mut self,
        instance_id: &str,
        fact_name: &str,
        key: &str,
        value_json: &str,
        causation_id: Option<&str>,
        event_idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        let fact_id = idempotency_key(&[instance_id, "fact", fact_name, key]);
        self.store.derive_fact(DerivedFact {
            instance_id,
            fact: NewFact {
                fact_id: &fact_id,
                name: fact_name,
                key,
                value_json,
                schema_id: None,
                provenance_class: "external",
                correlation_id: None,
                source_span_json: None,
            },
            source: "kernel",
            causation_id,
            idempotency_key: event_idempotency_key,
        })
    }

    /// Admit a typed fact batch atomically from one validated effect outcome
    /// (spec/admission-and-idempotency.md). Delegates to the store primitive.
    pub fn admit_fact_batch(&mut self, batch: FactBatch<'_>) -> StoreResult<FactBatchOutcome> {
        self.store.admit_fact_batch(batch)
    }

    /// Records a typed fact ingested from an effect's output bytes
    /// (spec/json-ingestion.md): provenance `ingest`, stamped with the schema
    /// it validated against.
    pub fn ingest_fact(
        &mut self,
        instance_id: &str,
        fact_name: &str,
        key: &str,
        value_json: &str,
        causation_id: Option<&str>,
        event_idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        let fact_id = idempotency_key(&[instance_id, "fact", fact_name, key]);
        self.store.derive_fact(DerivedFact {
            instance_id,
            fact: NewFact {
                fact_id: &fact_id,
                name: fact_name,
                key,
                value_json,
                schema_id: Some(fact_name),
                provenance_class: "ingest",
                correlation_id: None,
                source_span_json: None,
            },
            source: "kernel",
            causation_id,
            idempotency_key: event_idempotency_key,
        })
    }

    pub fn evaluate_rules(
        &self,
        instance_id: &str,
        program: &IrProgram,
    ) -> StoreResult<Vec<String>> {
        let mut ready = Vec::new();
        for rule in &program.rules {
            let mut all_reads_available = true;
            for fact_read in &rule.metadata.fact_reads {
                if !self.store.fact_exists(instance_id, fact_read)? {
                    all_reads_available = false;
                    break;
                }
            }
            if all_reads_available {
                ready.push(rule.name.clone());
            }
        }
        Ok(ready)
    }

    pub fn commit_rule(&mut self, commit: RuleCommit<'_>) -> StoreResult<StoredEvent> {
        let event = self.store.commit_rule(commit)?;
        self.emit_rule_commit_trace(commit);
        Ok(event)
    }

    pub fn commit_rule_with_revision_guard(
        &mut self,
        commit: RuleCommit<'_>,
        guard: RuleCommitRevisionGuard<'_>,
    ) -> StoreResult<StoredEvent> {
        let event = self.store.commit_rule_with_revision_guard(commit, guard)?;
        self.emit_rule_commit_trace(commit);
        Ok(event)
    }

    fn emit_rule_commit_trace(&mut self, commit: RuleCommit<'_>) {
        for effect in commit.effects {
            self.emit(TraceEvent::EffectCreated {
                effect_id: effect.effect_id.to_owned(),
                status: effect_status(effect.status),
            });
        }
        for dependency in commit.dependencies {
            self.emit(TraceEvent::DependencyCreated(dependency_edge(dependency)));
        }
    }

    pub fn activate_revision(
        &mut self,
        activation: RevisionActivation<'_>,
    ) -> StoreResult<WorkflowRevisionView> {
        let revision = self.store.activate_revision(activation)?;
        self.emit_revision_activation_trace(&revision)?;
        Ok(revision)
    }

    fn emit_revision_activation_trace(
        &mut self,
        revision: &WorkflowRevisionView,
    ) -> StoreResult<()> {
        if self.trace.iter().any(|record| {
            matches!(
                &record.event,
                TraceEvent::RevisionActivated { revision_id, .. }
                    if revision_id == &revision.revision_id
            )
        }) {
            return Ok(());
        }
        let event = self
            .store
            .list_events(&revision.instance_id)?
            .into_iter()
            .find(|event| event.event_id == revision.activated_by_event_id)
            .ok_or_else(|| {
                StoreError::Conflict(format!(
                    "revision event {} was not found",
                    revision.activated_by_event_id
                ))
            })?;
        let payload: Value = serde_json::from_str(&event.payload_json)?;
        let terminal_cancel_effects =
            string_array_field(&payload, "terminal_cancel_effects").unwrap_or_default();
        let request_cancel_effects =
            string_array_field(&payload, "request_cancel_effects").unwrap_or_default();
        self.emit(TraceEvent::RevisionActivated {
            revision_id: revision.revision_id.clone(),
            from_version_id: revision.from_version_id.clone(),
            to_version_id: revision.to_version_id.clone(),
            from_epoch: payload
                .get("from_epoch")
                .and_then(Value::as_i64)
                .unwrap_or_else(|| revision.epoch.saturating_sub(1)),
            to_epoch: revision.epoch,
            cancellation_policy: payload
                .get("cancellation_policy")
                .and_then(Value::as_str)
                .unwrap_or(&revision.cancellation_policy)
                .to_owned(),
            terminal_cancel_effects: terminal_cancel_effects.clone(),
            request_cancel_effects: request_cancel_effects.clone(),
        });
        for effect_id in terminal_cancel_effects {
            self.emit(TraceEvent::EffectCancelled { effect_id });
        }
        for effect_id in request_cancel_effects {
            self.emit(TraceEvent::EffectCancellationRequested {
                effect_id,
                revision_id: Some(revision.revision_id.clone()),
                reason: Some("workflow revision".to_owned()),
                requested_by: "workflow.revision".to_owned(),
            });
        }
        Ok(())
    }

    pub fn record_workflow_invocation(
        &self,
        invocation: NewWorkflowInvocation<'_>,
    ) -> StoreResult<()> {
        self.store.record_workflow_invocation(invocation)
    }

    pub fn get_workflow_invocation(
        &self,
        parent_instance_id: &str,
        parent_effect_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>> {
        self.store
            .get_workflow_invocation(parent_instance_id, parent_effect_id)
    }

    pub fn claimable_effects(&self, instance_id: &str) -> StoreResult<Vec<ClaimableEffect>> {
        self.store.claimable_effects(instance_id)
    }

    pub fn satisfy_dependencies(&self, instance_id: &str) -> StoreResult<usize> {
        self.store.satisfy_dependencies(instance_id)
    }

    pub fn start_run(&mut self, run: RunStart<'_>) -> StoreResult<StoredEvent> {
        let event = match self.store.start_run(run) {
            Ok(event) => event,
            Err(StoreError::PolicyBlocked { effect_id, reason })
            | Err(StoreError::CapacityBlocked { effect_id, reason }) => {
                self.emit(TraceEvent::EffectBlocked {
                    effect_id: effect_id.clone(),
                    status: None,
                    reason: reason.clone(),
                });
                return Err(StoreError::PolicyBlocked { effect_id, reason });
            }
            Err(error) => return Err(error),
        };
        self.emit(TraceEvent::EffectClaimed {
            effect_id: run.effect_id.to_owned(),
        });
        self.emit(TraceEvent::RunStarted {
            run_id: run.run_id.to_owned(),
            effect_id: run.effect_id.to_owned(),
        });
        Ok(event)
    }

    /// Block an effect at provider-binding time (DR-0020): the worker found the
    /// provider unbindable before provider execution. Delegates to the store's
    /// idempotent binding-block and emits the blocked trace.
    pub fn block_effect_binding(
        &mut self,
        instance_id: &str,
        effect_id: &str,
        category: &str,
        detail: &str,
    ) -> StoreResult<StoredEvent> {
        let event = self
            .store
            .block_effect_binding(instance_id, effect_id, category, detail)?;
        self.emit(TraceEvent::EffectBlocked {
            effect_id: effect_id.to_owned(),
            status: Some("blocked".to_owned()),
            reason: format!("{category}: {detail}"),
        });
        Ok(event)
    }

    pub fn complete_run(&mut self, completion: EffectCompletion<'_>) -> StoreResult<StoredEvent> {
        let completion = EffectCompletion {
            status: "completed",
            ..completion
        };
        let event = self.store.complete_effect(completion)?;
        self.emit(TraceEvent::EffectTerminal {
            run_id: completion.run_id.to_owned(),
            effect_id: completion.effect_id.to_owned(),
            status: EffectStatus::Completed,
        });
        Ok(event)
    }

    pub fn fail_run(&mut self, completion: EffectCompletion<'_>) -> StoreResult<StoredEvent> {
        self.fail_run_with_diagnostic(completion, None)
    }

    fn fail_run_with_diagnostic(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent> {
        let completion = EffectCompletion {
            status: "failed",
            ..completion
        };
        let event = self
            .store
            .complete_effect_with_terminal_diagnostic(completion, diagnostic)?;
        self.emit(TraceEvent::EffectTerminal {
            run_id: completion.run_id.to_owned(),
            effect_id: completion.effect_id.to_owned(),
            status: EffectStatus::Failed,
        });
        Ok(event)
    }

    /// Recovery resolution terminal: the effect becomes `failed` (a Failed
    /// subkind) but the run records the distinct `uncertain` status, so an
    /// operator can tell "we don't know if the side effect happened" apart from
    /// an ordinary provider failure. See admission-and-idempotency.md.
    fn resolve_run_uncertain_with_diagnostic(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent> {
        let completion = EffectCompletion {
            status: "failed",
            ..completion
        };
        let event = self
            .store
            .resolve_effect_uncertain(completion, diagnostic)?;
        self.emit(TraceEvent::EffectTerminal {
            run_id: completion.run_id.to_owned(),
            effect_id: completion.effect_id.to_owned(),
            status: EffectStatus::Failed,
        });
        Ok(event)
    }

    pub fn timeout_run(&mut self, completion: EffectCompletion<'_>) -> StoreResult<StoredEvent> {
        self.timeout_run_with_diagnostic(completion, None)
    }

    pub fn cancel_run(&mut self, completion: EffectCompletion<'_>) -> StoreResult<StoredEvent> {
        let completion = EffectCompletion {
            status: "cancelled",
            ..completion
        };
        let event = self.store.complete_effect(completion)?;
        self.emit(TraceEvent::EffectTerminal {
            run_id: completion.run_id.to_owned(),
            effect_id: completion.effect_id.to_owned(),
            status: EffectStatus::Cancelled,
        });
        Ok(event)
    }

    fn timeout_run_with_diagnostic(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent> {
        let completion = EffectCompletion {
            status: "timed_out",
            ..completion
        };
        let event = self
            .store
            .complete_effect_with_terminal_diagnostic(completion, diagnostic)?;
        self.emit(TraceEvent::EffectTerminal {
            run_id: completion.run_id.to_owned(),
            effect_id: completion.effect_id.to_owned(),
            status: EffectStatus::TimedOut,
        });
        Ok(event)
    }

    pub fn cancel_effect(
        &mut self,
        cancellation: EffectCancellation<'_>,
    ) -> StoreResult<StoredEvent> {
        let event = self.store.cancel_effect(cancellation)?;
        self.emit(TraceEvent::EffectCancelled {
            effect_id: cancellation.effect_id.to_owned(),
        });
        Ok(event)
    }

    pub fn pause_instance(
        &mut self,
        instance_id: &str,
        reason: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        let event = self.store.transition_instance(InstanceTransition {
            instance_id,
            status: "paused",
            reason,
            idempotency_key,
        })?;
        self.emit(TraceEvent::InstancePaused);
        Ok(event)
    }

    pub fn resume_instance(
        &mut self,
        instance_id: &str,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        let event = self.store.transition_instance(InstanceTransition {
            instance_id,
            status: "running",
            reason: None,
            idempotency_key,
        })?;
        self.emit(TraceEvent::InstanceResumed);
        Ok(event)
    }

    pub fn cancel_instance(
        &mut self,
        instance_id: &str,
        reason: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        let event = self.store.transition_instance(InstanceTransition {
            instance_id,
            status: "cancelled",
            reason,
            idempotency_key,
        })?;
        // A cancelled instance has reached a terminal: its pending human asks
        // are moot, so retire them rather than leaving them answerable for a
        // dead instance (holder-lifetime cleanup, mirrors lease/claim release).
        self.store.cancel_pending_inbox_for_instance(instance_id)?;
        self.emit(TraceEvent::InstanceCancelled);
        Ok(event)
    }

    /// Generic internal workflow failure (spec/implementation-plan.md flow
    /// auto-fail): transitions the instance to `failed` with a plain reason and no
    /// typed `failure` payload — used when an unhandled effect failure in a
    /// self-terminating flow would otherwise stall the instance forever. Distinct
    /// from an author `fail error { … }` (which produces the declared failure
    /// type); this is a runtime/system failure the author did not handle. Mirrors
    /// `cancel_instance`'s terminal cleanup (pending human asks are retired).
    pub fn fail_instance_internal(
        &mut self,
        instance_id: &str,
        reason: &str,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        let event = self.store.transition_instance(InstanceTransition {
            instance_id,
            status: "failed",
            reason: Some(reason),
            idempotency_key,
        })?;
        // A failed instance has reached a terminal: retire its pending human asks
        // (holder-lifetime cleanup, mirrors cancel/lease/claim release).
        self.store.cancel_pending_inbox_for_instance(instance_id)?;
        self.emit(TraceEvent::InstanceFailed);
        Ok(event)
    }

    pub fn renew_lease(&mut self, renewal: LeaseRenewal<'_>) -> StoreResult<StoredEvent> {
        self.store.renew_lease(renewal)
    }

    pub fn expire_leases(
        &mut self,
        instance_id: &str,
        now: &str,
    ) -> StoreResult<Vec<ExpiredLease>> {
        let expired = self.store.expire_leases(instance_id, now)?;
        for lease in &expired {
            self.emit(TraceEvent::LeaseExpired {
                run_id: lease.run_id.clone(),
                effect_id: lease.effect_id.clone(),
            });
        }
        Ok(expired)
    }

    pub fn retry_effect(&mut self, retry: RetryEffect<'_>) -> StoreResult<StoredEvent> {
        self.store.retry_effect(retry)
    }

    pub fn run_agent_turn(
        &mut self,
        execution: AgentTurnExecution<'_>,
        harness: &dyn AgentHarness,
    ) -> StoreResult<StoredEvent> {
        self.run_agent_turn_with_metadata(execution, harness, "{}")
    }

    pub fn run_agent_turn_with_metadata(
        &mut self,
        execution: AgentTurnExecution<'_>,
        harness: &dyn AgentHarness,
        run_metadata_json: &str,
    ) -> StoreResult<StoredEvent> {
        self.start_run(RunStart {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            lease_id: execution.lease_id,
            lease_expires_at: execution.lease_expires_at,
            metadata_json: run_metadata_json,
        })?;
        self.store.record_skill_evidence(SkillEvidence {
            instance_id: execution.instance_id,
            run_id: execution.run_id,
            effect_id: execution.effect_id,
            skill_names: execution.skill_names,
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "skills",
            ])),
        })?;

        let request = AgentTurnRequest {
            instance_id: execution.instance_id.to_owned(),
            effect_id: execution.effect_id.to_owned(),
            run_id: execution.run_id.to_owned(),
            agent: execution.agent.to_owned(),
            profile: execution.profile.map(str::to_owned),
            input_json: execution.input_json.to_owned(),
            skill_names: execution
                .skill_names
                .iter()
                .map(|skill| (*skill).to_owned())
                .collect(),
        };
        harness.before_launch(&request);
        if self
            .store
            .effect_has_open_cancellation_request(execution.instance_id, execution.effect_id)?
        {
            let metadata_json = merge_provider_run_metadata(
                json!({
                    "cancellation": "requested_before_provider_launch",
                    "provider": execution.provider,
                }),
                run_metadata_json,
            )
            .to_string();
            return self.cancel_run(EffectCompletion {
                instance_id: execution.instance_id,
                effect_id: execution.effect_id,
                run_id: execution.run_id,
                provider: execution.provider,
                worker_id: execution.worker_id,
                status: "cancelled",
                exit_code: None,
                summary: Some("provider launch skipped because cancellation was requested"),
                metadata_json: &metadata_json,
                idempotency_key: Some(&idempotency_key(&[
                    execution.instance_id,
                    execution.run_id,
                    "cancel-before-launch",
                ])),
            });
        }

        let mut result = harness.run(request);
        enforce_required_artifact_capture_failure(&mut result);
        let evidence = self.record_provider_result(execution, &result)?;
        let (metadata_json, terminal_hash, provider_correlation_id) = provider_terminal_metadata(
            execution.instance_id,
            execution.run_id,
            &result,
            run_metadata_json,
        );
        let safe_summary = redacted_provider_summary(&result.summary);
        self.emit_provider_diagnostic(
            execution.run_id,
            execution.effect_id,
            execution.provider,
            provider_effect_status(&result.status),
            &safe_summary,
            &metadata_json,
        );
        let completion = EffectCompletion {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            status: provider_status(&result.status),
            exit_code: result.exit_code,
            summary: Some(&safe_summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&terminal_completion_idempotency_key(
                execution.instance_id,
                execution.run_id,
                &provider_correlation_id,
                &terminal_hash,
            )),
        };
        let diagnostic = self.provider_terminal_diagnostic(
            execution.instance_id,
            execution.effect_id,
            execution.run_id,
            execution.provider,
            provider_effect_status(&result.status),
            &safe_summary,
            provider_failure_code(result.failure.as_ref(), provider_status(&result.status)),
            &metadata_json,
            &evidence,
        );

        let event = match result.status {
            ProviderRunStatus::Completed => self.complete_run(completion)?,
            ProviderRunStatus::Failed => self.fail_run_with_diagnostic(completion, diagnostic)?,
            ProviderRunStatus::TimedOut => {
                self.timeout_run_with_diagnostic(completion, diagnostic)?
            }
        };
        self.append_agent_turn_event_and_fact(execution, &result)?;
        Ok(event)
    }

    pub fn run_native_agent_turn(
        &mut self,
        execution: AgentTurnExecution<'_>,
        request: NativeProviderTurnRequest,
        adapter: &mut dyn NativeProviderAdapter,
        max_events: usize,
    ) -> StoreResult<StoredEvent> {
        self.run_native_agent_turn_with_metadata(execution, request, adapter, max_events, "{}")
    }

    pub fn run_native_agent_turn_with_metadata(
        &mut self,
        execution: AgentTurnExecution<'_>,
        request: NativeProviderTurnRequest,
        adapter: &mut dyn NativeProviderAdapter,
        max_events: usize,
        run_metadata_json: &str,
    ) -> StoreResult<StoredEvent> {
        let run_metadata = merge_provider_run_metadata(
            json!({
                "native_provider": request.to_json_redacted(),
                "adapter_provider_id": adapter.provider_id(),
                "adapter_capability": {
                    "provider_kind": adapter.capability().provider_kind.as_str(),
                    "surface": adapter.capability().surface.as_str(),
                },
            }),
            run_metadata_json,
        )
        .to_string();
        self.start_run(RunStart {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            lease_id: execution.lease_id,
            lease_expires_at: execution.lease_expires_at,
            metadata_json: &run_metadata,
        })?;
        self.store.record_skill_evidence(SkillEvidence {
            instance_id: execution.instance_id,
            run_id: execution.run_id,
            effect_id: execution.effect_id,
            skill_names: execution.skill_names,
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "skills",
            ])),
        })?;

        let started = match adapter.start_turn(request) {
            Ok(event) => event,
            Err(error) => {
                let failed = native_boundary_error_event(execution, error);
                let evidence = self.record_native_provider_event(
                    execution,
                    &failed,
                    "native-provider-boundary-start",
                )?;
                return self.complete_native_agent_turn(
                    execution,
                    &failed,
                    &evidence,
                    run_metadata_json,
                );
            }
        };
        let mut latest_evidence =
            self.record_native_provider_event(execution, &started, "native-provider-started")?;
        if started.event_kind.is_terminal() {
            return self.complete_native_agent_turn(
                execution,
                &started,
                &latest_evidence,
                run_metadata_json,
            );
        }

        for index in 0..max_events {
            let event = match adapter.next_event(execution.run_id) {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(error) => {
                    let failed = native_boundary_error_event(execution, error);
                    latest_evidence = self.record_native_provider_event(
                        execution,
                        &failed,
                        &format!("native-provider-boundary-event-{index}"),
                    )?;
                    return self.complete_native_agent_turn(
                        execution,
                        &failed,
                        &latest_evidence,
                        run_metadata_json,
                    );
                }
            };
            latest_evidence = self.record_native_provider_event(
                execution,
                &event,
                &format!("native-provider-event-{index}"),
            )?;
            if event.event_kind.is_terminal() {
                return self.complete_native_agent_turn(
                    execution,
                    &event,
                    &latest_evidence,
                    run_metadata_json,
                );
            }
        }

        let timed_out = NativeProviderEvent {
            provider_id: execution.provider.to_owned(),
            run_id: execution.run_id.to_owned(),
            event_kind: NativeProviderEventKind::TimedOut,
            provider_event_type: "whip.native.timeout".to_owned(),
            provider_session_id: None,
            provider_turn_id: None,
            sequence: None,
            evidence: json!({"max_events": max_events}),
            artifacts: Vec::new(),
        };
        latest_evidence =
            self.record_native_provider_event(execution, &timed_out, "native-provider-timeout")?;
        self.complete_native_agent_turn(execution, &timed_out, &latest_evidence, run_metadata_json)
    }

    pub fn record_native_agent_turn_observation(
        &mut self,
        execution: AgentTurnExecution<'_>,
        observation: &NativeAgentTurnObservation,
        occurrence_key: &str,
    ) -> StoreResult<StoredEvent> {
        self.append_native_agent_turn_observation(execution, observation, occurrence_key)
    }

    pub fn record_artifact_capture_failure(
        &mut self,
        execution: AgentTurnExecution<'_>,
        failure: ArtifactCaptureFailure<'_>,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        let payload = artifact_capture_failed_payload(failure).map_err(StoreError::Conflict)?;
        let payload_json = payload.to_string();
        let event = self.store.append_event(NewEvent {
            instance_id: execution.instance_id,
            event_type: artifact_manifest::ARTIFACT_CAPTURE_FAILED_EVENT,
            payload_json: &payload_json,
            source: "kernel",
            causation_id: Some(execution.effect_id),
            correlation_id: Some(execution.run_id),
            idempotency_key,
        })?;
        let diagnostic_message = format!(
            "artifact capture failed for {}: {}",
            failure.artifact_ref, failure.error_kind
        );
        self.store.record_diagnostic(DiagnosticRecord {
            instance_id: Some(execution.instance_id),
            program_id: None,
            program_version_id: None,
            severity: Severity::Error,
            code: Some(artifact_manifest::ARTIFACT_CAPTURE_FAILED_EVENT),
            message: &diagnostic_message,
            source_span_json: None,
            subject_type: Some("effect"),
            subject_id: Some(execution.effect_id),
            event_id: Some(&event.event_id),
            effect_id: Some(execution.effect_id),
            run_id: Some(execution.run_id),
            assertion_id: None,
            evidence_ids_json: "[]",
            artifact_ids_json: "[]",
            causation_id: Some(execution.effect_id),
            correlation_id: Some(execution.run_id),
            idempotency_key,
        })?;
        Ok(event)
    }

    pub fn recover_provider_terminal_from_evidence(
        &mut self,
        execution: AgentTurnExecution<'_>,
    ) -> StoreResult<Option<StoredEvent>> {
        let Some(run) = self
            .store
            .list_runs(execution.instance_id)?
            .into_iter()
            .find(|run| run.run_id == execution.run_id)
        else {
            return Ok(None);
        };
        if run.status != "running" {
            return Ok(None);
        }

        let evidence = self.store.list_evidence(execution.instance_id)?;
        if let Some(native_evidence) = evidence.iter().rev().find(|evidence| {
            evidence.kind == "agent.turn.native_provider"
                && evidence.subject_type == "run"
                && evidence.subject_id == execution.run_id
                && evidence
                    .metadata_json
                    .parse::<Value>()
                    .ok()
                    .and_then(|metadata| metadata.get("native_event").cloned())
                    .and_then(|event| event.get("terminal").and_then(Value::as_bool))
                    == Some(true)
        }) {
            let metadata = serde_json::from_str::<Value>(&native_evidence.metadata_json)?;
            let native_event = recover_native_provider_event(&metadata, execution)?;
            let artifact_ids = metadata
                .get("artifact_ids")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let provider_evidence = ProviderEvidence {
                evidence_id: Some(native_evidence.evidence_id.clone()),
                artifact_ids,
            };
            return self
                .complete_native_agent_turn(
                    execution,
                    &native_event,
                    &provider_evidence,
                    &run.metadata_json,
                )
                .map(Some);
        }

        let Some(evidence) = evidence.into_iter().rev().find(|evidence| {
            evidence.kind == "agent.turn.provider"
                && evidence.subject_type == "run"
                && evidence.subject_id == execution.run_id
        }) else {
            return Ok(None);
        };

        let metadata = serde_json::from_str::<Value>(&evidence.metadata_json)?;
        let status = recover_provider_status(&metadata);
        let failure = metadata.get("failure").and_then(provider_failure_from_json);
        let result = ProviderRunResult {
            status,
            summary: evidence
                .summary
                .clone()
                .unwrap_or_else(|| "recovered provider terminal outcome".to_owned()),
            stdout: String::new(),
            stderr: String::new(),
            transcript: String::new(),
            exit_code: metadata.get("exit_code").and_then(Value::as_i64),
            usage_json: "{}".to_owned(),
            artifacts: Vec::new(),
            failure,
        };
        let artifact_ids = metadata
            .get("artifact_ids")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let provider_evidence = ProviderEvidence {
            evidence_id: Some(evidence.evidence_id.clone()),
            artifact_ids,
        };
        let provider_correlation_id = evidence.correlation_id.clone().unwrap_or_else(|| {
            provider_terminal_correlation_id(execution.instance_id, execution.run_id)
        });
        let terminal_metadata_base = json!({
            "recovery": "provider_evidence_terminal",
            "evidence_id": evidence.evidence_id,
            "provider_metadata": metadata,
            "provider_correlation_id": provider_correlation_id,
        });
        let terminal_hash = terminal_payload_hash(
            provider_status(&result.status),
            result.exit_code,
            Some(&result.summary),
            &terminal_metadata_base.to_string(),
        );
        let terminal_metadata = add_terminal_payload_hash(terminal_metadata_base, &terminal_hash);
        let safe_summary = redacted_provider_summary(&result.summary);
        let completion = EffectCompletion {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            status: provider_status(&result.status),
            exit_code: result.exit_code,
            summary: Some(&safe_summary),
            metadata_json: &terminal_metadata,
            idempotency_key: Some(&terminal_completion_idempotency_key(
                execution.instance_id,
                execution.run_id,
                &provider_correlation_id,
                &terminal_hash,
            )),
        };
        let diagnostic = self.provider_terminal_diagnostic(
            execution.instance_id,
            execution.effect_id,
            execution.run_id,
            execution.provider,
            provider_effect_status(&result.status),
            &safe_summary,
            provider_failure_code(result.failure.as_ref(), provider_status(&result.status)),
            &terminal_metadata,
            &provider_evidence,
        );
        let event = match result.status {
            ProviderRunStatus::Completed => self.complete_run(completion)?,
            ProviderRunStatus::Failed => self.fail_run_with_diagnostic(completion, diagnostic)?,
            ProviderRunStatus::TimedOut => {
                self.timeout_run_with_diagnostic(completion, diagnostic)?
            }
        };
        self.append_agent_turn_event_and_fact(execution, &result)?;
        Ok(Some(event))
    }

    pub fn recover_running_provider_runs(
        &mut self,
        instance_id: &str,
    ) -> StoreResult<Vec<StoredEvent>> {
        let runs = self.store.list_runs(instance_id)?;
        let effects = self.store.list_effects(instance_id)?;
        let mut recovered = Vec::new();
        for run in runs.into_iter().filter(|run| run.status == "running") {
            let Some(effect) = effects
                .iter()
                .find(|effect| effect.effect_id == run.effect_id)
            else {
                continue;
            };
            let execution = AgentTurnExecution {
                instance_id,
                effect_id: &run.effect_id,
                run_id: &run.run_id,
                provider: &run.provider,
                worker_id: &run.worker_id,
                lease_id: "",
                lease_expires_at: "",
                agent: effect.target.as_deref().unwrap_or("worker"),
                profile: effect.profile.as_deref(),
                input_json: &effect.input_json,
                skill_names: &[],
            };
            if let Some(event) = self.recover_provider_terminal_from_evidence(execution)? {
                recovered.push(event);
            } else {
                // No recoverable terminal evidence and no idempotent provider
                // re-query: resolve the started-without-terminal run to a single
                // `uncertain` terminal (a Failed subkind) rather than leaving it
                // stuck or silently re-executing the external side effect. This is
                // the admission-and-idempotency.md exactly-once recovery rule
                // (TLA+ ResolveUncertainRun).
                match self.resolve_uncertain_provider_run(execution) {
                    Ok(event) => recovered.push(event),
                    // A real terminal raced in between the run snapshot and this
                    // resolution; the store row guard already prevented a double
                    // terminal, so skip rather than aborting the whole sweep.
                    Err(StoreError::Conflict(_)) => {}
                    Err(err) => return Err(err),
                }
            }
        }
        Ok(recovered)
    }

    /// Recovery resolution for a run that started its external side effect but
    /// whose worker crashed before any terminal or evidence was recorded, where
    /// the provider offers no idempotent re-query. Resolves to a single
    /// `uncertain` terminal (a Failed subkind) under a deterministic idempotency
    /// key so re-running recovery cannot double-admit, and never re-executes the
    /// external side effect.
    fn resolve_uncertain_provider_run(
        &mut self,
        execution: AgentTurnExecution<'_>,
    ) -> StoreResult<StoredEvent> {
        let provider_correlation_id =
            provider_terminal_correlation_id(execution.instance_id, execution.run_id);
        let summary = "recovery could not determine the provider outcome; resolved as uncertain";
        let terminal_metadata_base = json!({
            "recovery": "uncertain",
            "provider_correlation_id": provider_correlation_id,
        });
        let terminal_hash = terminal_payload_hash(
            provider_status(&ProviderRunStatus::Failed),
            None,
            Some(summary),
            &terminal_metadata_base.to_string(),
        );
        let terminal_metadata = add_terminal_payload_hash(terminal_metadata_base, &terminal_hash);
        let completion = EffectCompletion {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            status: provider_status(&ProviderRunStatus::Failed),
            exit_code: None,
            summary: Some(summary),
            metadata_json: &terminal_metadata,
            idempotency_key: Some(&terminal_completion_idempotency_key(
                execution.instance_id,
                execution.run_id,
                &provider_correlation_id,
                &terminal_hash,
            )),
        };
        let provider_evidence = ProviderEvidence {
            evidence_id: None,
            artifact_ids: Vec::new(),
        };
        let diagnostic = self.provider_terminal_diagnostic(
            execution.instance_id,
            execution.effect_id,
            execution.run_id,
            execution.provider,
            provider_effect_status(&ProviderRunStatus::Failed),
            summary,
            Some("runtime.recovery_uncertain"),
            &terminal_metadata,
            &provider_evidence,
        );
        self.resolve_run_uncertain_with_diagnostic(completion, diagnostic)
    }

    pub fn run_coerce(
        &mut self,
        execution: CoerceExecution<'_>,
        client: &dyn CoerceClient,
    ) -> StoreResult<StoredEvent> {
        self.start_run(RunStart {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            lease_id: execution.lease_id,
            lease_expires_at: execution.lease_expires_at,
            metadata_json: "{}",
        })?;

        let result = client.coerce(execution.request);
        let evidence = self.record_coerce_result(execution, &result)?;
        let metadata_json = coerce_metadata(&result);
        let safe_summary = redacted_provider_summary(&result.summary);
        self.emit_provider_diagnostic(
            execution.run_id,
            execution.effect_id,
            execution.provider,
            coerce_effect_status(&result.status),
            &safe_summary,
            &metadata_json,
        );
        let completion = EffectCompletion {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            status: coerce_status(&result.status),
            exit_code: coerce_exit_code(&result.status),
            summary: Some(&safe_summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "coerce-terminal",
            ])),
        };
        let diagnostic = self.provider_terminal_diagnostic(
            execution.instance_id,
            execution.effect_id,
            execution.run_id,
            execution.provider,
            coerce_effect_status(&result.status),
            &safe_summary,
            Some(match result.status {
                CoerceStatus::Succeeded => "coerce.succeeded",
                CoerceStatus::Failed => "coerce.failed",
                CoerceStatus::TimedOut => "coerce.timed_out",
            }),
            &metadata_json,
            &evidence,
        );

        let event = match result.status {
            CoerceStatus::Succeeded => self.complete_run(completion)?,
            CoerceStatus::Failed => self.fail_run_with_diagnostic(completion, diagnostic)?,
            CoerceStatus::TimedOut => self.timeout_run_with_diagnostic(completion, diagnostic)?,
        };
        self.append_coerce_fact(execution, &result)?;
        Ok(event)
    }

    pub fn run_loft_effect(
        &mut self,
        execution: LoftEffectExecution<'_>,
        client: &dyn LoftClient,
    ) -> StoreResult<StoredEvent> {
        self.start_run(RunStart {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            lease_id: execution.lease_id,
            lease_expires_at: execution.lease_expires_at,
            metadata_json: "{}",
        })?;

        let result = client.execute(execution.request);
        let evidence = self.record_loft_result(execution, &result)?;
        let metadata_json = loft_metadata(&result);
        let safe_summary = redacted_provider_summary(&result.summary);
        self.emit_provider_diagnostic(
            execution.run_id,
            execution.effect_id,
            execution.provider,
            loft_effect_status(&result.status),
            &safe_summary,
            &metadata_json,
        );
        let completion = EffectCompletion {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            status: loft_status(&result.status),
            exit_code: loft_exit_code(&result.status),
            summary: Some(&safe_summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "loft-terminal",
            ])),
        };
        let diagnostic_code = loft_fact_name(execution.request.action, &result.status);
        let diagnostic = self.provider_terminal_diagnostic(
            execution.instance_id,
            execution.effect_id,
            execution.run_id,
            execution.provider,
            loft_effect_status(&result.status),
            &result.summary,
            Some(&diagnostic_code),
            &metadata_json,
            &evidence,
        );

        let event = match result.status {
            LoftEffectStatus::Succeeded => self.complete_run(completion)?,
            LoftEffectStatus::Failed => self.fail_run_with_diagnostic(completion, diagnostic)?,
            LoftEffectStatus::TimedOut => {
                self.timeout_run_with_diagnostic(completion, diagnostic)?
            }
        };
        self.append_loft_fact(execution, &result)?;
        Ok(event)
    }

    pub fn run_human_ask(&mut self, execution: HumanAskExecution<'_>) -> StoreResult<StoredEvent> {
        self.start_run(RunStart {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            lease_id: execution.lease_id,
            lease_expires_at: execution.lease_expires_at,
            metadata_json: "{}",
        })?;
        self.store.create_inbox_item(NewInboxItem {
            inbox_item_id: execution.inbox_item_id,
            instance_id: execution.instance_id,
            effect_id: Some(execution.effect_id),
            status: "pending",
            prompt: execution.prompt,
            choices_json: execution.choices_json,
            freeform_allowed: execution.freeform_allowed,
            severity: execution.severity,
            related_effects_json: execution.related_effects_json,
            related_artifacts_json: execution.related_artifacts_json,
        })?;
        self.record_human_ask(execution)?;
        let metadata_json = json!({
            "inbox_item_id": execution.inbox_item_id,
            "severity": execution.severity,
            "choices": json_from_str(execution.choices_json),
            "freeform_allowed": execution.freeform_allowed,
        })
        .to_string();
        let completion = EffectCompletion {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            status: "completed",
            exit_code: Some(0),
            summary: Some("human review requested"),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "human-ask-terminal",
            ])),
        };
        let event = self.complete_run(completion)?;
        self.append_human_ask_fact(execution)?;
        Ok(event)
    }

    fn record_human_ask(&self, execution: HumanAskExecution<'_>) -> StoreResult<()> {
        let metadata = json!({
            "effect_id": execution.effect_id,
            "inbox_item_id": execution.inbox_item_id,
            "prompt": execution.prompt,
            "choices": json_from_str(execution.choices_json),
            "freeform_allowed": execution.freeform_allowed,
            "severity": execution.severity,
            "related_effects": json_from_str(execution.related_effects_json),
            "related_artifacts": json_from_str(execution.related_artifacts_json),
        })
        .to_string();
        self.store.record_evidence(EvidenceRecord {
            instance_id: execution.instance_id,
            kind: "human.ask.provider",
            subject_type: "run",
            subject_id: execution.run_id,
            causation_id: Some(execution.effect_id),
            correlation_id: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "human-ask-provider",
            ])),
            summary: Some("human review requested"),
            metadata_json: &metadata,
        })?;
        Ok(())
    }

    fn append_human_ask_fact(&mut self, execution: HumanAskExecution<'_>) -> StoreResult<()> {
        let value = json!({
            "effect_id": execution.effect_id,
            "run_id": execution.run_id,
            "inbox_item_id": execution.inbox_item_id,
            "prompt": execution.prompt,
            "choices": json_from_str(execution.choices_json),
            "freeform_allowed": execution.freeform_allowed,
            "severity": execution.severity,
            "status": "pending",
        })
        .to_string();
        self.store.append_event(NewEvent {
            instance_id: execution.instance_id,
            event_type: "human.ask.created",
            payload_json: &value,
            source: "kernel",
            causation_id: Some(execution.run_id),
            correlation_id: Some(execution.effect_id),
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "human-ask-event",
            ])),
        })?;
        let fact_id = idempotency_key(&[execution.instance_id, "human.ask", execution.run_id]);
        let fact_key =
            idempotency_key(&[execution.instance_id, execution.run_id, "human-ask-fact"]);
        self.store.derive_fact(DerivedFact {
            instance_id: execution.instance_id,
            fact: NewFact {
                fact_id: &fact_id,
                name: "human.ask.created",
                key: execution.inbox_item_id,
                value_json: &value,
                schema_id: Some("HumanAsk"),
                provenance_class: "effect",
                correlation_id: Some(execution.effect_id),
                source_span_json: None,
            },
            source: "kernel",
            causation_id: Some(execution.run_id),
            idempotency_key: Some(&fact_key),
        })?;
        Ok(())
    }

    fn record_loft_result(
        &self,
        execution: LoftEffectExecution<'_>,
        result: &LoftEffectResult,
    ) -> StoreResult<ProviderEvidence> {
        let metadata = json!({
            "effect_id": execution.effect_id,
            "action": execution.request.action.effect_kind(),
            "issue_id": execution.request.issue_id,
            "lease_id": execution.request.lease_id,
            "claim_ready": execution.request.claim_ready,
            "issue_version": execution.request.issue_version,
            "actor": execution.request.actor,
            "lease_duration_seconds": execution.request.lease_duration_seconds,
            "command_id": execution.request.command_id,
            "note": execution.request.note.as_deref().map(redacted_text_metadata),
            "target_status": execution.request.target_status,
            "evidence": execution.request.evidence_json.as_deref().map(json_payload_summary),
            "evidence_kind": execution.request.evidence_kind,
            "evidence_artifact": execution.request.evidence_artifact,
            "evidence_data_path": execution.request.evidence_data_path,
            "resource_intent": execution.request.resource_intent_json.as_deref().map(json_payload_summary),
            "release_after_failure": execution.request.release_after_failure,
            "expect_heads": execution.request.expect_heads,
            "request_metadata": json_payload_summary(&execution.request.metadata_json),
            "value": result.value_json.as_deref().map(json_payload_summary),
            "error": result_error_payload(result.error_json.as_deref()),
            "transcript": redacted_text_metadata(&result.transcript),
        })
        .to_string();
        let evidence_id = self.store.record_evidence(EvidenceRecord {
            instance_id: execution.instance_id,
            kind: &format!("{}.provider", execution.request.action.effect_kind()),
            subject_type: "run",
            subject_id: execution.run_id,
            causation_id: Some(execution.effect_id),
            correlation_id: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "loft-provider",
            ])),
            summary: Some(&redacted_provider_summary(&result.summary)),
            metadata_json: &metadata,
        })?;
        Ok(ProviderEvidence {
            evidence_id: Some(evidence_id),
            artifact_ids: Vec::new(),
        })
    }

    fn append_loft_fact(
        &mut self,
        execution: LoftEffectExecution<'_>,
        result: &LoftEffectResult,
    ) -> StoreResult<()> {
        let status = loft_status(&result.status);
        let fact_name = loft_fact_name(execution.request.action, &result.status);
        let succeeded = matches!(result.status, LoftEffectStatus::Succeeded);
        let summary = redacted_provider_summary(&result.summary);
        // DR-0032: failure `value` is the EffectError base; success keeps the op
        // output.
        let value_field = if succeeded {
            result_value_payload(true, result.value_json.as_deref())
        } else {
            // DR-0032 + redaction (D4): loft is provider-backed, so the raw
            // `error_json` is confidential. The base `reason` is the already-redacted
            // summary, not the raw-error-derived text (which would leak into the fact).
            Some(effect_failure_base(
                execution.request.action.effect_kind(),
                &summary,
                &summary,
                execution.effect_id,
                execution.run_id,
            ))
        };
        let value = json!({
            "effect_id": execution.effect_id,
            "run_id": execution.run_id,
            "action": execution.request.action.effect_kind(),
            "issue_id": execution.request.issue_id,
            "lease_id": execution.request.lease_id,
            "command_id": execution.request.command_id,
            "status": status,
            "value": value_field,
            "error": result_error_payload(result.error_json.as_deref()),
            "summary": summary,
        })
        .to_string();
        self.store.append_event(NewEvent {
            instance_id: execution.instance_id,
            event_type: &fact_name,
            payload_json: &value,
            source: "kernel",
            causation_id: Some(execution.run_id),
            correlation_id: Some(execution.effect_id),
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "loft-event",
            ])),
        })?;
        let fact_id = idempotency_key(&[execution.instance_id, "loft", execution.run_id]);
        let fact_key = idempotency_key(&[execution.instance_id, execution.run_id, "loft-fact"]);
        self.store.derive_fact(DerivedFact {
            instance_id: execution.instance_id,
            fact: NewFact {
                fact_id: &fact_id,
                name: &fact_name,
                key: execution.run_id,
                value_json: &value,
                schema_id: loft_schema(execution.request.action, &result.status),
                provenance_class: "effect",
                correlation_id: Some(execution.effect_id),
                source_span_json: None,
            },
            source: "kernel",
            causation_id: Some(execution.run_id),
            idempotency_key: Some(&fact_key),
        })?;
        Ok(())
    }

    fn record_coerce_result(
        &self,
        execution: CoerceExecution<'_>,
        result: &CoerceResult,
    ) -> StoreResult<ProviderEvidence> {
        let metadata = json!({
            "effect_id": execution.effect_id,
            "function_name": execution.request.function_name,
            "arguments": json_payload_summary(&execution.request.arguments_json),
            "output_type": execution.request.output_type,
            "generated_coerce_source_hash": execution.request.generated_coerce_source_hash,
            "input_schema_hash": execution.request.input_schema_hash,
            "output_schema_hash": execution.request.output_schema_hash,
            "value": result.value_json.as_deref().map(json_payload_summary),
            "error": result_error_payload(result.error_json.as_deref()),
            "transcript": redacted_text_metadata(&result.transcript),
            "usage": json_from_str(&result.usage_json),
        })
        .to_string();
        let evidence_id = self.store.record_evidence(EvidenceRecord {
            instance_id: execution.instance_id,
            kind: "coerce.provider",
            subject_type: "run",
            subject_id: execution.run_id,
            causation_id: Some(execution.effect_id),
            correlation_id: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "coerce-provider",
            ])),
            summary: Some(&redacted_provider_summary(&result.summary)),
            metadata_json: &metadata,
        })?;
        Ok(ProviderEvidence {
            evidence_id: Some(evidence_id),
            artifact_ids: Vec::new(),
        })
    }

    fn append_coerce_fact(
        &mut self,
        execution: CoerceExecution<'_>,
        result: &CoerceResult,
    ) -> StoreResult<()> {
        let status = coerce_status(&result.status);
        let fact_name = match result.status {
            CoerceStatus::Succeeded => "coerce.succeeded",
            CoerceStatus::Failed => "coerce.failed",
            CoerceStatus::TimedOut => "coerce.timed_out",
        };
        let succeeded = matches!(result.status, CoerceStatus::Succeeded);
        let summary = redacted_provider_summary(&result.summary);
        // DR-0032: on failure, `value` is the EffectError base (what `after f fails
        // as e` binds); on success it is the coerced output. The prior `null` on
        // failure shadowed the error blob, so the bound value was unreadable.
        let value_field = if succeeded {
            result_value_payload(true, result.value_json.as_deref())
        } else {
            // DR-0032 + redaction (D4): coerce is provider-backed, so the raw
            // `error_json` is confidential model output. The base `reason` must be
            // the already-redacted summary, NOT the raw-error-derived text — echoing
            // the provider error here would leak it into the persisted fact value.
            Some(effect_failure_base(
                "coerce",
                &summary,
                &summary,
                execution.effect_id,
                execution.run_id,
            ))
        };
        let value = json!({
            "effect_id": execution.effect_id,
            "run_id": execution.run_id,
            "function_name": execution.request.function_name,
            "status": status,
            "output_type": execution.request.output_type,
            "value": value_field,
            "error": result_error_payload(result.error_json.as_deref()),
            "summary": summary,
        })
        .to_string();
        self.store.append_event(NewEvent {
            instance_id: execution.instance_id,
            event_type: fact_name,
            payload_json: &value,
            source: "kernel",
            causation_id: Some(execution.run_id),
            correlation_id: Some(execution.effect_id),
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "coerce-event",
            ])),
        })?;
        let fact_id = idempotency_key(&[execution.instance_id, "coerce", execution.run_id]);
        let fact_key = idempotency_key(&[execution.instance_id, execution.run_id, "coerce-fact"]);
        self.store.derive_fact(DerivedFact {
            instance_id: execution.instance_id,
            fact: NewFact {
                fact_id: &fact_id,
                name: fact_name,
                key: execution.run_id,
                value_json: &value,
                schema_id: Some(&execution.request.output_type),
                provenance_class: "effect",
                correlation_id: Some(execution.effect_id),
                source_span_json: None,
            },
            source: "kernel",
            causation_id: Some(execution.run_id),
            idempotency_key: Some(&fact_key),
        })?;
        Ok(())
    }

    fn record_native_provider_event(
        &mut self,
        execution: AgentTurnExecution<'_>,
        event: &NativeProviderEvent,
        occurrence_key: &str,
    ) -> StoreResult<ProviderEvidence> {
        let artifact_ids = event
            .artifacts
            .iter()
            .map(|artifact| {
                let artifact = redacted_native_artifact_ref(artifact);
                self.store.record_artifact(ArtifactRecord {
                    run_id: execution.run_id,
                    kind: &artifact.kind,
                    path: &artifact.uri,
                    content_hash: artifact.content_hash.as_deref(),
                    mime_type: artifact.mime_type.as_deref(),
                })
            })
            .collect::<StoreResult<Vec<_>>>()?;
        let metadata = json!({
            "effect_id": execution.effect_id,
            "agent": execution.agent,
            "profile": execution.profile,
            "provider": execution.provider,
            "native_event": event.to_json_redacted(),
            "artifact_ids": artifact_ids,
        })
        .to_string();
        let evidence_id = self.store.record_evidence(EvidenceRecord {
            instance_id: execution.instance_id,
            kind: "agent.turn.native_provider",
            subject_type: "run",
            subject_id: execution.run_id,
            causation_id: Some(execution.effect_id),
            correlation_id: Some(occurrence_key),
            summary: Some(&match event.redacted_provider_error() {
                Value::String(detail) => format!(
                    "{} native provider event from {}: {}",
                    event.event_kind.as_str(),
                    event.provider_event_type,
                    detail
                ),
                _ => format!(
                    "{} native provider event from {}",
                    event.event_kind.as_str(),
                    event.provider_event_type
                ),
            }),
            metadata_json: &metadata,
        })?;
        let observation = native_provider_event_observation(event);
        self.append_native_agent_turn_observation(execution, &observation, occurrence_key)?;
        Ok(ProviderEvidence {
            evidence_id: Some(evidence_id),
            artifact_ids,
        })
    }

    fn complete_native_agent_turn(
        &mut self,
        execution: AgentTurnExecution<'_>,
        event: &NativeProviderEvent,
        evidence: &ProviderEvidence,
        run_metadata_json: &str,
    ) -> StoreResult<StoredEvent> {
        let status = native_terminal_status(event.event_kind);
        let summary = match event.redacted_provider_error() {
            Value::String(detail) => format!(
                "{} native provider event from {}: {}",
                event.event_kind.as_str(),
                event.provider_event_type,
                detail
            ),
            _ => format!(
                "{} native provider event from {}",
                event.event_kind.as_str(),
                event.provider_event_type
            ),
        };
        let provider_correlation_id =
            provider_terminal_correlation_id(execution.instance_id, execution.run_id);
        let metadata_base = add_provider_correlation_id(
            merge_provider_run_metadata(
                json!({
                    "native_provider": event.to_json_redacted(),
                    "artifact_ids": evidence.artifact_ids,
                }),
                run_metadata_json,
            ),
            &provider_correlation_id,
        );
        let terminal_hash =
            terminal_payload_hash(status, Some(0), Some(&summary), &metadata_base.to_string());
        let metadata = add_terminal_payload_hash(metadata_base, &terminal_hash);
        let completion = EffectCompletion {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            status,
            exit_code: Some(0),
            summary: Some(&summary),
            metadata_json: &metadata,
            idempotency_key: Some(&terminal_completion_idempotency_key(
                execution.instance_id,
                execution.run_id,
                &provider_correlation_id,
                &terminal_hash,
            )),
        };
        match event.event_kind {
            NativeProviderEventKind::Completed => self.complete_run(completion),
            NativeProviderEventKind::Failed => {
                let diagnostic = self.provider_terminal_diagnostic(
                    execution.instance_id,
                    execution.effect_id,
                    execution.run_id,
                    execution.provider,
                    EffectStatus::Failed,
                    &summary,
                    Some("native_provider_failed"),
                    &metadata,
                    evidence,
                );
                self.fail_run_with_diagnostic(completion, diagnostic)
            }
            NativeProviderEventKind::TimedOut => {
                let diagnostic = self.provider_terminal_diagnostic(
                    execution.instance_id,
                    execution.effect_id,
                    execution.run_id,
                    execution.provider,
                    EffectStatus::TimedOut,
                    &summary,
                    Some("native_provider_timed_out"),
                    &metadata,
                    evidence,
                );
                self.timeout_run_with_diagnostic(completion, diagnostic)
            }
            NativeProviderEventKind::Cancelled => self.cancel_run(completion),
            _ => Err(StoreError::Conflict(format!(
                "native provider event `{}` is not terminal",
                event.event_kind.as_str()
            ))),
        }
    }

    fn record_provider_result(
        &self,
        execution: AgentTurnExecution<'_>,
        result: &ProviderRunResult,
    ) -> StoreResult<ProviderEvidence> {
        let artifacts = result
            .artifacts
            .iter()
            .map(sanitized_provider_artifact_metadata)
            .collect::<Vec<_>>();
        let artifact_ids = artifacts
            .iter()
            .map(|artifact| {
                self.store.record_artifact(ArtifactRecord {
                    run_id: execution.run_id,
                    kind: &artifact.kind,
                    path: &artifact.path,
                    content_hash: artifact.content_hash.as_deref(),
                    mime_type: artifact.mime_type.as_deref(),
                })
            })
            .collect::<StoreResult<Vec<_>>>()?;
        let artifact_manifest = provider_artifact_manifest(&artifact_ids, &artifacts);
        validate_artifact_manifest(&artifact_manifest).map_err(StoreError::Conflict)?;
        let metadata = json!({
            "effect_id": execution.effect_id,
            "agent": execution.agent,
            "profile": execution.profile,
            "status": provider_status(&result.status),
            "stdout": redacted_text_metadata(&result.stdout),
            "stderr": redacted_text_metadata(&result.stderr),
            "transcript": redacted_text_metadata(&result.transcript),
            "exit_code": result.exit_code,
            "usage": json_from_str(&result.usage_json),
            "artifact_ids": artifact_ids,
            "artifact_manifest": artifact_manifest,
            "failure": result.failure.as_ref().map(provider_failure_json),
        })
        .to_string();
        let evidence_id = self.store.record_evidence(EvidenceRecord {
            instance_id: execution.instance_id,
            kind: "agent.turn.provider",
            subject_type: "run",
            subject_id: execution.run_id,
            causation_id: Some(execution.effect_id),
            correlation_id: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "provider",
            ])),
            summary: Some(&redacted_provider_summary(&result.summary)),
            metadata_json: &metadata,
        })?;
        Ok(ProviderEvidence {
            evidence_id: Some(evidence_id),
            artifact_ids,
        })
    }

    fn append_agent_turn_event_and_fact(
        &mut self,
        execution: AgentTurnExecution<'_>,
        result: &ProviderRunResult,
    ) -> StoreResult<()> {
        let status = provider_status(&result.status);
        let fact_name = format!("agent.turn.{status}");
        let summary = redacted_provider_summary(&result.summary);
        let mut payload_value = json!({
            "effect_id": execution.effect_id,
            "run_id": execution.run_id,
            "agent": execution.agent,
            "provider": execution.provider,
            "status": status,
            "summary": summary,
            "exit_code": result.exit_code,
            "failure": result.failure.as_ref().map(provider_failure_json),
        });
        // DR-0032: a failed turn carries the EffectError base under `value` (what
        // `after turn fails as f` binds); the rich provider blob stays under
        // `failure`. Success has no `value` here (the turn output is read via the
        // whole payload), so we add `value` only on failure to avoid a null shadow.
        if let Some(failure) = result.failure.as_ref() {
            let reason = provider_failure_summary_message(&failure.message);
            if let Some(object) = payload_value.as_object_mut() {
                object.insert(
                    "value".to_owned(),
                    effect_failure_base(
                        "agent.tell",
                        &reason,
                        &summary,
                        execution.effect_id,
                        execution.run_id,
                    ),
                );
            }
        }
        let payload = payload_value.to_string();
        let fact_id = idempotency_key(&[execution.instance_id, "agent-turn", execution.run_id]);
        let fact_event_key =
            idempotency_key(&[execution.instance_id, execution.run_id, "agent-turn-fact"]);
        self.store.append_event(NewEvent {
            instance_id: execution.instance_id,
            event_type: &fact_name,
            payload_json: &payload,
            source: "kernel",
            causation_id: Some(execution.run_id),
            correlation_id: Some(execution.effect_id),
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "agent-turn-event",
            ])),
        })?;
        self.store.derive_fact(DerivedFact {
            instance_id: execution.instance_id,
            fact: NewFact {
                fact_id: &fact_id,
                name: &fact_name,
                key: execution.run_id,
                value_json: &payload,
                schema_id: None,
                provenance_class: "effect",
                correlation_id: Some(execution.effect_id),
                source_span_json: None,
            },
            source: "kernel",
            causation_id: Some(execution.run_id),
            idempotency_key: Some(&fact_event_key),
        })?;
        Ok(())
    }

    fn append_native_agent_turn_observation(
        &mut self,
        execution: AgentTurnExecution<'_>,
        observation: &NativeAgentTurnObservation,
        occurrence_key: &str,
    ) -> StoreResult<StoredEvent> {
        let event_type = observation.kind.event_type();
        let evidence_metadata = json!({
            "effect_id": execution.effect_id,
            "run_id": execution.run_id,
            "provider": execution.provider,
            "status": observation.kind.status(),
            "terminal": observation.terminal,
            "provider_event_type": observation.provider_event_type,
            "provider_session_id": observation.provider_session_id,
            "provider_turn_id": observation.provider_turn_id,
            "provider_payload_shape": observation.provider_payload_shape,
        })
        .to_string();
        let evidence_summary = format!(
            "{} observation from {}",
            observation.kind.status(),
            observation.provider_event_type
        );
        let evidence_id = self.store.record_evidence(EvidenceRecord {
            instance_id: execution.instance_id,
            kind: "agent.turn.native_event",
            subject_type: "run",
            subject_id: execution.run_id,
            causation_id: Some(execution.effect_id),
            correlation_id: Some(occurrence_key),
            summary: Some(&evidence_summary),
            metadata_json: &evidence_metadata,
        })?;
        let payload = json!({
            "effect_id": execution.effect_id,
            "run_id": execution.run_id,
            "agent": execution.agent,
            "provider": execution.provider,
            "status": observation.kind.status(),
            "terminal": observation.terminal,
            "provider_event_type": observation.provider_event_type,
            "provider_session_id": observation.provider_session_id,
            "provider_turn_id": observation.provider_turn_id,
            "provider_payload_shape": observation.provider_payload_shape,
            "evidence_id": evidence_id,
        })
        .to_string();
        let event = self.store.append_event(NewEvent {
            instance_id: execution.instance_id,
            event_type,
            payload_json: &payload,
            source: "kernel.native_provider",
            causation_id: Some(execution.run_id),
            correlation_id: Some(execution.effect_id),
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                occurrence_key,
                "native-lifecycle-event",
            ])),
        })?;
        // In-turn observations (streamed/tool_requested/artifact_captured) are
        // EVIDENCE only (recorded above) — never rule-matchable facts
        // (spec/agent-harness.md). Only the durable lifecycle kinds
        // (started/completed/failed/timed_out/cancelled) derive a fact; deriving
        // a fact for the evidence kinds would inflate the fact set with values no
        // rule may match (the compiler already forbids matching them).
        if observation.kind.derives_rule_matchable_fact() {
            let fact_id = idempotency_key(&[
                execution.instance_id,
                "native-agent-turn",
                execution.run_id,
                occurrence_key,
            ]);
            let fact_key = idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                occurrence_key,
                "native-lifecycle-fact",
            ]);
            self.store.derive_fact(DerivedFact {
                instance_id: execution.instance_id,
                fact: NewFact {
                    fact_id: &fact_id,
                    name: event_type,
                    key: &fact_key,
                    value_json: &payload,
                    schema_id: None,
                    provenance_class: "effect",
                    correlation_id: Some(execution.effect_id),
                    source_span_json: None,
                },
                source: "kernel.native_provider",
                causation_id: Some(execution.run_id),
                idempotency_key: Some(&fact_key),
            })?;
        }
        Ok(event)
    }

    fn emit(&mut self, event: TraceEvent) {
        self.trace.push(TraceRecord {
            sequence: self.trace.len() as u64 + 1,
            event,
        });
    }

    fn emit_provider_diagnostic(
        &mut self,
        run_id: &str,
        effect_id: &str,
        provider: &str,
        status: EffectStatus,
        summary: &str,
        diagnostics_json: &str,
    ) {
        if status == EffectStatus::Completed {
            return;
        }
        self.emit(TraceEvent::ProviderDiagnostic {
            run_id: run_id.to_owned(),
            effect_id: effect_id.to_owned(),
            provider: provider.to_owned(),
            status,
            summary: summary.to_owned(),
            diagnostics_json: diagnostics_json.to_owned(),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn provider_terminal_diagnostic(
        &self,
        instance_id: &str,
        effect_id: &str,
        run_id: &str,
        provider: &str,
        status: EffectStatus,
        summary: &str,
        code: Option<&str>,
        diagnostics_json: &str,
        evidence: &ProviderEvidence,
    ) -> Option<TerminalDiagnosticRecord> {
        if status == EffectStatus::Completed {
            return None;
        }
        let evidence_ids_json = evidence
            .evidence_id
            .as_ref()
            .map(|evidence_id| json!([evidence_id]).to_string())
            .unwrap_or_else(|| "[]".to_owned());
        let artifact_ids_json = json!(evidence.artifact_ids).to_string();
        let message = provider_diagnostic_message(provider, summary, diagnostics_json);
        let idempotency_key =
            idempotency_key(&[instance_id, run_id, "provider-diagnostic", diagnostics_json]);
        let source_span_json = self
            .store
            .effect_source_span_json(instance_id, effect_id)
            .ok()
            .flatten();
        Some(TerminalDiagnosticRecord {
            program_id: None,
            program_version_id: None,
            severity: Severity::Error,
            code: code.map(str::to_owned),
            message,
            source_span_json,
            subject_type: Some("effect".to_owned()),
            subject_id: Some(effect_id.to_owned()),
            assertion_id: None,
            evidence_ids_json,
            artifact_ids_json,
            causation_id: Some(run_id.to_owned()),
            correlation_id: Some(effect_id.to_owned()),
            idempotency_key: Some(idempotency_key),
        })
    }
}

fn effect_status(status: &str) -> EffectStatus {
    match status {
        "blocked"
        | "blocked_by_dependency"
        | "blocked_by_capability"
        | "blocked_by_profile"
        | "blocked_by_capacity" => EffectStatus::Blocked,
        "claimed" => EffectStatus::Claimed,
        "running" => EffectStatus::Running,
        "completed" => EffectStatus::Completed,
        "failed" => EffectStatus::Failed,
        "timed_out" => EffectStatus::TimedOut,
        "cancelled" => EffectStatus::Cancelled,
        _ => EffectStatus::Queued,
    }
}

fn declared_profiles_json(program: &IrProgram) -> String {
    let harnesses = program
        .harnesses
        .iter()
        .map(|harness| {
            json!({
                "name": harness.name,
                "kind": harness.kind,
            })
        })
        .collect::<Vec<_>>();
    let agents = program
        .agents
        .iter()
        .map(|agent| {
            json!({
                "name": agent.name,
                "harness": agent.harness,
                "provider": agent.provider,
                "profile": agent.profile,
                "capacity": agent.capacity,
                "skills": agent.skills,
                "capabilities": agent.capabilities,
            })
        })
        .collect::<Vec<_>>();
    json!({ "harnesses": harnesses, "agents": agents }).to_string()
}

fn declared_skills_json(program: &IrProgram) -> String {
    let mut skills = program
        .agents
        .iter()
        .flat_map(|agent| agent.skills.iter().cloned())
        .collect::<Vec<_>>();
    skills.sort();
    skills.dedup();
    json!(skills).to_string()
}

fn declared_schemas_json(program: &IrProgram) -> String {
    let schemas = program
        .schemas
        .iter()
        .map(|schema| {
            let (name, kind) = match schema {
                whipplescript_parser::IrSchema::Class(class) => (&class.name, "class"),
                whipplescript_parser::IrSchema::Enum(enum_decl) => (&enum_decl.name, "enum"),
            };
            json!({ "name": name, "kind": kind })
        })
        .collect::<Vec<_>>();
    json!(schemas).to_string()
}

pub fn program_analysis_summary_json(program: &IrProgram) -> String {
    let workflow_contracts = program
        .workflow_contracts
        .iter()
        .map(|contract| {
            json!({
                "kind": workflow_contract_kind_name(&contract.kind),
                "name": contract.name,
                "type": ir_type_signature(&contract.ty),
                "source_span": source_span_summary(contract.span, "workflow_contract"),
            })
        })
        .collect::<Vec<_>>();
    let include_closure = program
        .includes
        .iter()
        .map(|include| {
            json!({
                "path": include.path,
                "source_hash": include.source_hash,
            })
        })
        .collect::<Vec<_>>();
    // A single stable fingerprint of the whole include closure: the ordered
    // (path, source_hash) pairs. Changes iff any included file's path or content
    // changes, so it identifies the bundle as a unit (the per-file `source_hash`
    // values above identify each member individually). Deterministic and stable
    // for an include-free program (hashes the empty closure).
    let bundle_hash = {
        let canonical = program
            .includes
            .iter()
            .map(|include| {
                format!(
                    "{}\u{0}{}",
                    include.path,
                    include.source_hash.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        stable_hash_hex(&canonical)
    };
    let pattern_applications = program
        .pattern_applications
        .iter()
        .map(|application| {
            json!({
                "pattern": application.pattern,
                "alias": application.alias,
                "type_args": application
                    .type_args
                    .iter()
                    .map(ir_type_signature)
                    .collect::<Vec<_>>(),
                "value_args": application
                    .value_args
                    .iter()
                    .map(|argument| json!({
                        "name": argument.name,
                        "value": argument.value,
                    }))
                    .collect::<Vec<_>>(),
                "generated": application.generated,
            })
        })
        .collect::<Vec<_>>();
    let mut generated_declarations = program
        .pattern_applications
        .iter()
        .flat_map(|application| application.generated.iter().cloned())
        .collect::<Vec<_>>();
    generated_declarations.sort();
    generated_declarations.dedup();
    let generated_declaration_hashes = generated_declaration_hashes(program);
    let schemas = program
        .schemas
        .iter()
        .map(|schema| schema_summary(schema, true))
        .collect::<Vec<_>>();
    let harnesses = program
        .harnesses
        .iter()
        .map(|harness| {
            json!({
                "name": harness.name,
                "kind": harness.kind,
                "source_span": source_span_summary(harness.span, "harness"),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "workflow": program.workflow,
        "workflow_contracts": workflow_contracts,
        "include_closure": include_closure,
        "bundle_hash": bundle_hash,
        "pattern_applications": pattern_applications,
        "generated_declarations": generated_declarations,
        "generated_declaration_hashes": generated_declaration_hashes,
        "harnesses": harnesses,
        "schemas": schemas,
    })
    .to_string()
}

fn workflow_contract_kind_name(kind: &IrWorkflowContractKind) -> &'static str {
    match kind {
        IrWorkflowContractKind::Input => "input",
        IrWorkflowContractKind::Output => "output",
        IrWorkflowContractKind::Failure => "failure",
    }
}

fn source_span_summary(span: SourceSpan, construct: &str) -> Value {
    json!({
        "start": span.start,
        "end": span.end,
        "construct": construct,
    })
}

fn schema_summary(schema: &IrSchema, include_source_spans: bool) -> Value {
    match schema {
        IrSchema::Class(class) => {
            let mut value = json!({
            "kind": "class",
            "name": class.name,
            "fields": class
                .fields
                .iter()
                .map(|field| {
                    let mut value = json!({
                        "name": field.name,
                        "type": ir_type_signature(&field.ty),
                    });
                    if include_source_spans {
                        value["source_span"] = source_span_summary(field.span, "class_field");
                    }
                    value
                })
                .collect::<Vec<_>>(),
            });
            if include_source_spans {
                value["source_span"] = source_span_summary(class.span, "class");
            }
            value
        }
        IrSchema::Enum(enum_decl) => {
            let mut value = json!({
            "kind": "enum",
            "name": enum_decl.name,
            "variants": enum_decl.variants,
            });
            if include_source_spans {
                value["source_span"] = source_span_summary(enum_decl.span, "enum");
            }
            value
        }
    }
}

fn generated_declaration_hashes(program: &IrProgram) -> Vec<Value> {
    let mut generated_declarations = program
        .pattern_applications
        .iter()
        .flat_map(|application| application.generated.iter().cloned())
        .collect::<Vec<_>>();
    generated_declarations.sort();
    generated_declarations.dedup();

    generated_declarations
        .into_iter()
        .map(
            |declaration| match generated_declaration_payload(program, &declaration) {
                Some(payload) => json!({
                    "declaration": declaration,
                    "hash": stable_hash_hex(&payload.to_string()),
                }),
                None => json!({
                    "declaration": declaration,
                    "hash": stable_hash_hex("missing-generated-declaration"),
                    "missing": true,
                }),
            },
        )
        .collect()
}

fn generated_declaration_payload(program: &IrProgram, declaration: &str) -> Option<Value> {
    let (kind, name) = declaration.split_once(':')?;
    match kind {
        "agent" => program
            .agents
            .iter()
            .find(|agent| agent.name == name)
            .map(|agent| {
                json!({
                    "kind": "agent",
                    "name": agent.name,
                    "harness": agent.harness,
                    "profile": agent.profile,
                    "capacity": agent.capacity,
                    "skills": agent.skills,
                    "capabilities": agent.capabilities,
                })
            }),
        "harness" => program
            .harnesses
            .iter()
            .find(|harness| harness.name == name)
            .map(|harness| {
                json!({
                    "kind": "harness",
                    "name": harness.name,
                    "provider_kind": harness.kind,
                })
            }),
        "enum" => program.schemas.iter().find_map(|schema| match schema {
            IrSchema::Enum(enum_decl) if enum_decl.name == name => Some(json!({
                "kind": "enum",
                "name": enum_decl.name,
                "variants": enum_decl.variants,
            })),
            _ => None,
        }),
        "class" => program.schemas.iter().find_map(|schema| match schema {
            IrSchema::Class(class) if class.name == name => Some(schema_summary(schema, false)),
            _ => None,
        }),
        "coerce" => program
            .coerces
            .iter()
            .find(|coerce| coerce.name == name)
            .map(|coerce| {
                json!({
                    "kind": "coerce",
                    "name": coerce.name,
                    "params": coerce
                        .params
                        .iter()
                        .map(|param| json!({
                            "name": param.name,
                            "type": ir_type_signature(&param.ty),
                        }))
                        .collect::<Vec<_>>(),
                    "output": ir_type_signature(&coerce.output),
                    "body": coerce.body,
                })
            }),
        "rule" => program
            .rules
            .iter()
            .find(|rule| rule.name == name)
            .map(|rule| {
                json!({
                    "kind": "rule",
                    "name": rule.name,
                    "whens": rule
                        .whens
                        .iter()
                        .map(|when| json!({
                            "source": when.source,
                            "pattern": when.pattern,
                            "guard": when.guard.as_ref().map(|guard| guard.source.as_str()),
                        }))
                        .collect::<Vec<_>>(),
                    "body": rule.body,
                    "metadata": {
                        "fact_reads": rule.metadata.fact_reads,
                        "projection_reads": rule
                            .metadata
                            .projection_reads
                            .iter()
                            .map(|read| json!({
                                "kind": format!("{:?}", read.kind),
                                "head": read.head,
                                "guard": read.guard,
                            }))
                            .collect::<Vec<_>>(),
                        "fact_writes": rule.metadata.fact_writes,
                        "fact_consumes": rule.metadata.fact_consumes,
                        "effects": rule
                            .metadata
                            .effects
                            .iter()
                            .map(|effect| json!({
                                "id": effect.id,
                                "kind": effect_kind_name(&effect.kind),
                                "binding": effect.binding,
                                "required_capabilities": effect.required_capabilities,
                                "idempotency_key": effect.idempotency_key,
                            }))
                            .collect::<Vec<_>>(),
                        "dependencies": rule
                            .metadata
                            .dependencies
                            .iter()
                            .map(|dependency| json!({
                                "upstream": dependency.upstream,
                                "predicate": dependency_predicate_name(&dependency.predicate),
                                "downstream": dependency.downstream,
                            }))
                            .collect::<Vec<_>>(),
                        "terminal_outputs": rule
                            .metadata
                            .terminal_outputs
                            .iter()
                            .map(|terminal| json!({
                                "binding": terminal.binding,
                                "alternatives": terminal
                                    .alternatives
                                    .iter()
                                    .map(|alternative| json!({
                                        "tag": alternative.tag,
                                        "payload_type": ir_type_signature(&alternative.payload_type),
                                    }))
                                    .collect::<Vec<_>>(),
                            }))
                            .collect::<Vec<_>>(),
                    },
                })
            }),
        _ => None,
    }
}

fn ir_type_signature(ty: &IrType) -> String {
    match ty {
        IrType::Primitive(primitive) => primitive_type_name(primitive).to_owned(),
        IrType::LiteralString(value) => format!("literal<{value:?}>"),
        IrType::Ref(name) => format!("ref<{name}>"),
        IrType::AgentRef(agents) => format!("agentref<{}>", agents.join(" | ")),
        IrType::Object(fields) => {
            let fields = fields
                .iter()
                .map(|field| format!("{} {}", field.name, ir_type_signature(&field.ty)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("object<{{{fields}}}>")
        }
        IrType::Optional(inner) => format!("optional<{}>", ir_type_signature(inner)),
        IrType::Array(inner) => format!("array<{}>", ir_type_signature(inner)),
        IrType::Map(inner) => format!("map<{}>", ir_type_signature(inner)),
        IrType::Union(variants) => {
            let variants = variants
                .iter()
                .map(ir_type_signature)
                .collect::<Vec<_>>()
                .join(" | ");
            format!("union<{variants}>")
        }
    }
}

fn effect_kind_name(kind: &IrEffectKind) -> &'static str {
    match kind {
        IrEffectKind::AgentTell => "agent.tell",
        IrEffectKind::Coerce => "coerce",
        IrEffectKind::LoftClaim => "loft.claim",
        IrEffectKind::HumanAsk => "human.ask",
        IrEffectKind::CapabilityCall => "capability.call",
        IrEffectKind::EventEmit => "event.emit",
        IrEffectKind::WorkflowInvoke => "workflow.invoke",
        IrEffectKind::TimerWait => "timer.wait",
        IrEffectKind::ExecCommand => "exec.command",
        IrEffectKind::QueueFile => "queue.file",
        IrEffectKind::QueueClaim => "queue.claim",
        IrEffectKind::QueueRelease => "queue.release",
        IrEffectKind::QueueFinish => "queue.finish",
        IrEffectKind::LeaseAcquire => "lease.acquire",
        IrEffectKind::LedgerAppend => "ledger.append",
        IrEffectKind::CounterConsume => "counter.consume",
        IrEffectKind::EventNotify => "event.notify",
        IrEffectKind::FileRead => "file.read",
        IrEffectKind::FileWrite => "file.write",
        IrEffectKind::FileImport => "file.import",
        IrEffectKind::FileExport => "file.export",
    }
}

fn dependency_predicate_name(predicate: &DependencyPredicate) -> &'static str {
    match predicate {
        DependencyPredicate::Succeeds => "succeeds",
        DependencyPredicate::Fails => "fails",
        DependencyPredicate::TimedOut => "timed_out",
        DependencyPredicate::Cancelled => "cancelled",
        DependencyPredicate::Completes => "completes",
    }
}

fn primitive_type_name(primitive: &IrPrimitiveType) -> &'static str {
    match primitive {
        IrPrimitiveType::String => "string",
        IrPrimitiveType::Int => "int",
        IrPrimitiveType::Float => "float",
        IrPrimitiveType::Bool => "bool",
        IrPrimitiveType::Null => "null",
        IrPrimitiveType::Duration => "duration",
        IrPrimitiveType::Time => "time",
        IrPrimitiveType::Image => "image",
        IrPrimitiveType::Audio => "audio",
        IrPrimitiveType::Pdf => "pdf",
        IrPrimitiveType::Video => "video",
    }
}

fn dependency_edge(dependency: &NewEffectDependency<'_>) -> DependencyEdge {
    DependencyEdge {
        upstream_effect_id: dependency.upstream_effect_id.to_owned(),
        predicate: match dependency.predicate {
            "fails" => trace::DependencyPredicate::Fails,
            "completes" => trace::DependencyPredicate::Completes,
            _ => trace::DependencyPredicate::Succeeds,
        },
        downstream_effect_id: dependency.downstream_effect_id.to_owned(),
    }
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn stable_hash_hex(value: &str) -> String {
    format!("{:016x}", stable_hash(value))
}

fn redacted_text_metadata(value: &str) -> Value {
    json!({
        "redacted": true,
        "bytes": value.len(),
        "chars": value.chars().count(),
    })
}

fn json_payload_summary(source: &str) -> Value {
    json!({
        "redacted": true,
        "bytes": source.len(),
        "shape": json_shape(&json_from_str(source)),
    })
}

fn json_shape(value: &Value) -> Value {
    match value {
        Value::Null => json!({ "type": "null" }),
        Value::Bool(_) => json!({ "type": "bool" }),
        Value::Number(_) => json!({ "type": "number" }),
        Value::String(value) => json!({
            "type": "string",
            "chars": value.chars().count(),
        }),
        Value::Array(items) => json!({
            "type": "array",
            "items": items.len(),
        }),
        Value::Object(object) => json!({
            "type": "object",
            "keys": object.len(),
        }),
    }
}

fn result_value_payload(succeeded: bool, source: Option<&str>) -> Option<Value> {
    source.map(|source| {
        if succeeded {
            json_from_str(source)
        } else {
            json_payload_summary(source)
        }
    })
}

fn result_error_payload(source: Option<&str>) -> Option<Value> {
    source.map(json_payload_summary)
}

/// The `EffectError` base object (DR-0032) that `after <effect> fails as f` binds.
/// Every effect kind's `.failed` fact carries this under its `value` key so the
/// bound `f` is uniform: `reason`/`summary` are the human-facing failure text,
/// `effect_id`/`run_id` locate the run, `kind` names the effect. Per-kind extras are
/// kept elsewhere on the fact (raw, telemetry) and are not read by `f` until a
/// variant exposes them.
fn effect_failure_base(
    kind: &str,
    reason: &str,
    summary: &str,
    effect_id: &str,
    run_id: &str,
) -> Value {
    json!({
        "reason": reason,
        "summary": summary,
        "effect_id": effect_id,
        "run_id": run_id,
        "kind": kind,
    })
}

fn sanitized_provider_artifact_metadata(
    artifact: &harness::ProviderArtifact,
) -> harness::ProviderArtifact {
    harness::ProviderArtifact {
        kind: redact_log_text(&artifact.kind),
        path: redact_log_text(&artifact.path),
        content_hash: artifact
            .content_hash
            .as_ref()
            .map(|hash| redact_log_text(hash)),
        mime_type: artifact
            .mime_type
            .as_ref()
            .map(|mime_type| redact_log_text(mime_type)),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "api_key",
        "apikey",
        "credential",
        "password",
        "secret",
        "token",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn redact_log_text(value: &str) -> String {
    let mut tokens = Vec::new();
    let mut redact_next = false;
    for token in value.split_whitespace() {
        if redact_next {
            tokens.push("[REDACTED]".to_owned());
            redact_next = false;
            continue;
        }
        if token.eq_ignore_ascii_case("bearer") {
            tokens.push(token.to_owned());
            redact_next = true;
            continue;
        }
        if let Some((key, _)) = token.split_once('=') {
            if is_sensitive_key(key) {
                tokens.push(format!("{key}=[REDACTED]"));
                continue;
            }
        }
        if looks_like_secret_token(token) || contains_secret_token_pattern(token) {
            tokens.push("[REDACTED]".to_owned());
        } else {
            tokens.push(token.to_owned());
        }
    }
    let redacted = tokens.join(" ");
    const MAX_MESSAGE_CHARS: usize = 512;
    if redacted.chars().count() <= MAX_MESSAGE_CHARS {
        return redacted;
    }
    let mut truncated = redacted.chars().take(MAX_MESSAGE_CHARS).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn looks_like_secret_token(token: &str) -> bool {
    let token = token.trim_matches(|ch: char| {
        matches!(ch, '"' | '\'' | ',' | ';' | ':' | ')' | '(' | '[' | ']')
    });
    (token.starts_with("sk-") && token.len() >= 20)
        || (token.starts_with("ghp_") && token.len() >= 20)
        || token.starts_with("github_pat_")
        || (token.starts_with("AKIA") && token.len() >= 20)
}

fn contains_secret_token_pattern(token: &str) -> bool {
    let token = token.trim_matches(|ch: char| {
        matches!(ch, '"' | '\'' | ',' | ';' | ':' | ')' | '(' | '[' | ']')
    });
    token.contains("github_pat_")
        || token
            .find("sk-")
            .is_some_and(|start| token[start..].len() >= 20)
        || token
            .find("ghp_")
            .is_some_and(|start| token[start..].len() >= 20)
        || token
            .find("AKIA")
            .is_some_and(|start| token[start..].len() >= 20)
}

fn provider_status(status: &ProviderRunStatus) -> &'static str {
    match status {
        ProviderRunStatus::Completed => "completed",
        ProviderRunStatus::Failed => "failed",
        ProviderRunStatus::TimedOut => "timed_out",
    }
}

fn native_terminal_status(kind: NativeProviderEventKind) -> &'static str {
    match kind {
        NativeProviderEventKind::Completed => "completed",
        NativeProviderEventKind::Failed => "failed",
        NativeProviderEventKind::TimedOut => "timed_out",
        NativeProviderEventKind::Cancelled => "cancelled",
        _ => "running",
    }
}

fn native_event_kind_from_str(value: &str) -> Option<NativeProviderEventKind> {
    match value {
        "started" => Some(NativeProviderEventKind::Started),
        "streamed" => Some(NativeProviderEventKind::Streamed),
        "tool_requested" => Some(NativeProviderEventKind::ToolRequested),
        "artifact_captured" => Some(NativeProviderEventKind::ArtifactCaptured),
        "completed" => Some(NativeProviderEventKind::Completed),
        "failed" => Some(NativeProviderEventKind::Failed),
        "timed_out" => Some(NativeProviderEventKind::TimedOut),
        "cancelled" => Some(NativeProviderEventKind::Cancelled),
        "diagnostic" => Some(NativeProviderEventKind::Diagnostic),
        _ => None,
    }
}

fn recover_native_provider_event(
    metadata: &Value,
    execution: AgentTurnExecution<'_>,
) -> StoreResult<NativeProviderEvent> {
    let event = metadata.get("native_event").ok_or_else(|| {
        StoreError::Conflict("native provider evidence is missing event".to_owned())
    })?;
    let kind = event
        .get("event_kind")
        .and_then(Value::as_str)
        .and_then(native_event_kind_from_str)
        .ok_or_else(|| {
            StoreError::Conflict("native provider evidence has invalid event kind".to_owned())
        })?;
    if !kind.is_terminal() {
        return Err(StoreError::Conflict(
            "native provider recovery requires terminal evidence".to_owned(),
        ));
    }
    Ok(NativeProviderEvent {
        provider_id: event
            .get("provider_id")
            .and_then(Value::as_str)
            .unwrap_or(execution.provider)
            .to_owned(),
        run_id: event
            .get("run_id")
            .and_then(Value::as_str)
            .unwrap_or(execution.run_id)
            .to_owned(),
        event_kind: kind,
        provider_event_type: event
            .get("provider_event_type")
            .and_then(Value::as_str)
            .unwrap_or("whip.native.recovered")
            .to_owned(),
        provider_session_id: event
            .get("provider_session_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        provider_turn_id: event
            .get("provider_turn_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        sequence: event.get("sequence").and_then(Value::as_u64),
        evidence: json!({
            "recovery": "native_provider_terminal_evidence",
            "provider_payload_shape": event.get("evidence_shape").cloned().unwrap_or(Value::Null),
        }),
        artifacts: Vec::new(),
    })
}

fn native_boundary_error_event(
    execution: AgentTurnExecution<'_>,
    error: provider::NativeProviderBoundaryError,
) -> NativeProviderEvent {
    let redacted = error.to_json_redacted();
    NativeProviderEvent {
        provider_id: error.provider_id,
        run_id: execution.run_id.to_owned(),
        event_kind: NativeProviderEventKind::Failed,
        provider_event_type: format!("whip.native.boundary_error.{}", error.code),
        provider_session_id: None,
        provider_turn_id: None,
        sequence: None,
        evidence: json!({
            "boundary_error": redacted,
            "recoverable": error.recoverable,
        }),
        artifacts: Vec::new(),
    }
}

fn redacted_native_artifact_ref(artifact: &NativeProviderArtifactRef) -> NativeProviderArtifactRef {
    let redacted = artifact.to_json_redacted();
    NativeProviderArtifactRef {
        artifact_id: redacted
            .get("artifact_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        kind: redacted
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("artifact")
            .to_owned(),
        uri: redacted
            .get("uri")
            .and_then(Value::as_str)
            .unwrap_or("provider://redacted/artifact")
            .to_owned(),
        content_hash: redacted
            .get("content_hash")
            .and_then(Value::as_str)
            .map(str::to_owned),
        mime_type: redacted
            .get("mime_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        required: redacted
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn native_provider_event_observation(event: &NativeProviderEvent) -> NativeAgentTurnObservation {
    let kind = match event.event_kind {
        NativeProviderEventKind::Started => AgentTurnLifecycleKind::Started,
        NativeProviderEventKind::Streamed | NativeProviderEventKind::Diagnostic => {
            AgentTurnLifecycleKind::Streamed
        }
        NativeProviderEventKind::ToolRequested => AgentTurnLifecycleKind::ToolRequested,
        NativeProviderEventKind::ArtifactCaptured => AgentTurnLifecycleKind::ArtifactCaptured,
        NativeProviderEventKind::Completed => AgentTurnLifecycleKind::Completed,
        NativeProviderEventKind::Failed => AgentTurnLifecycleKind::Failed,
        NativeProviderEventKind::TimedOut => AgentTurnLifecycleKind::TimedOut,
        NativeProviderEventKind::Cancelled => AgentTurnLifecycleKind::Cancelled,
    };
    NativeAgentTurnObservation::fixture(
        kind,
        event.provider_event_type.clone(),
        event.provider_session_id.clone(),
        event.provider_turn_id.clone(),
        json!({
            "type": "object",
            "keys": event
                .evidence
                .as_object()
                .map(serde_json::Map::len)
                .unwrap_or(0),
        }),
    )
}

fn enforce_required_artifact_capture_failure(result: &mut ProviderRunResult) {
    let required_artifact_capture_failed = result
        .failure
        .as_ref()
        .is_some_and(|failure| failure.phase == artifact_manifest::ARTIFACT_CAPTURE_FAILED_EVENT);
    if result.status == ProviderRunStatus::Completed && required_artifact_capture_failed {
        result.status = ProviderRunStatus::Failed;
        if !result.summary.contains("artifact capture failed") {
            result.summary = format!("artifact capture failed: {}", result.summary);
        }
    }
}

fn recover_provider_status(metadata: &Value) -> ProviderRunStatus {
    match metadata.get("status").and_then(Value::as_str) {
        Some("completed") => ProviderRunStatus::Completed,
        Some("timed_out") => ProviderRunStatus::TimedOut,
        Some("failed") => ProviderRunStatus::Failed,
        _ => {
            let failure_phase = metadata
                .pointer("/failure/phase")
                .and_then(Value::as_str)
                .unwrap_or("");
            if failure_phase == "provider.timeout" {
                ProviderRunStatus::TimedOut
            } else if metadata
                .get("failure")
                .is_some_and(|failure| !failure.is_null())
            {
                ProviderRunStatus::Failed
            } else {
                ProviderRunStatus::Completed
            }
        }
    }
}

fn provider_failure_from_json(value: &Value) -> Option<ProviderFailure> {
    if value.is_null() {
        return None;
    }
    Some(ProviderFailure {
        provider: value
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        adapter: value
            .get("adapter")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        phase: value
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("provider.recovered.failed")
            .to_owned(),
        error_kind: value
            .get("error_kind")
            .and_then(Value::as_str)
            .unwrap_or("recovered_failure")
            .to_owned(),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("recovered provider failure")
            .to_owned(),
        recoverable: value
            .get("recoverable")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        retry_after: value
            .get("retry_after")
            .and_then(Value::as_str)
            .map(str::to_owned),
        workspace_id: value
            .get("workspace_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        provider_session_id: value
            .get("provider_session_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        provider_thread_id: value
            .get("provider_thread_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        missing_config_keys: value
            .get("missing_config_keys")
            .and_then(Value::as_array)
            .map(|keys| {
                keys.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        raw_json: None,
    })
}

fn provider_failure_code<'a>(
    failure: Option<&'a ProviderFailure>,
    fallback_status: &'static str,
) -> Option<&'a str> {
    failure
        .map(|failure| failure.error_kind.as_str())
        .or(Some(fallback_status))
}

fn provider_effect_status(status: &ProviderRunStatus) -> EffectStatus {
    match status {
        ProviderRunStatus::Completed => EffectStatus::Completed,
        ProviderRunStatus::Failed => EffectStatus::Failed,
        ProviderRunStatus::TimedOut => EffectStatus::TimedOut,
    }
}

fn provider_metadata(result: &ProviderRunResult) -> String {
    json!({
        "stdout": redacted_text_metadata(&result.stdout),
        "stderr": redacted_text_metadata(&result.stderr),
        "transcript": redacted_text_metadata(&result.transcript),
        "usage": json_from_str(&result.usage_json),
        "failure": result.failure.as_ref().map(provider_failure_json),
    })
    .to_string()
}

fn provider_terminal_metadata(
    instance_id: &str,
    run_id: &str,
    result: &ProviderRunResult,
    run_metadata_json: &str,
) -> (String, String, String) {
    let provider_correlation_id = provider_terminal_correlation_id(instance_id, run_id);
    let metadata = add_provider_correlation_id(
        merge_provider_run_metadata(
            serde_json::from_str::<Value>(&provider_metadata(result))
                .expect("provider metadata is generated from JSON values"),
            run_metadata_json,
        ),
        &provider_correlation_id,
    );
    let terminal_hash = terminal_payload_hash(
        provider_status(&result.status),
        result.exit_code,
        Some(&result.summary),
        &metadata.to_string(),
    );
    (
        add_terminal_payload_hash(metadata, &terminal_hash),
        terminal_hash,
        provider_correlation_id,
    )
}

fn merge_provider_run_metadata(mut metadata: Value, run_metadata_json: &str) -> Value {
    let run_metadata = json_from_str(run_metadata_json);
    let (Value::Object(metadata), Value::Object(run_metadata)) = (&mut metadata, run_metadata)
    else {
        return metadata;
    };
    for (key, value) in run_metadata {
        metadata.entry(key).or_insert(value);
    }
    Value::Object(metadata.clone())
}

fn provider_terminal_correlation_id(instance_id: &str, run_id: &str) -> String {
    idempotency_key(&[instance_id, run_id, "provider"])
}

fn terminal_completion_idempotency_key(
    instance_id: &str,
    run_id: &str,
    provider_correlation_id: &str,
    terminal_hash: &str,
) -> String {
    idempotency_key(&[
        instance_id,
        run_id,
        provider_correlation_id,
        terminal_hash,
        "terminal",
    ])
}

fn terminal_payload_hash(
    status: &str,
    exit_code: Option<i64>,
    summary: Option<&str>,
    metadata_json: &str,
) -> String {
    stable_hash_hex(
        &json!({
            "status": status,
            "exit_code": exit_code,
            "summary": summary,
            "metadata": json_from_str(metadata_json),
        })
        .to_string(),
    )
}

fn add_provider_correlation_id(mut metadata: Value, provider_correlation_id: &str) -> Value {
    if let Value::Object(ref mut object) = metadata {
        object.insert(
            "provider_correlation_id".to_owned(),
            Value::String(provider_correlation_id.to_owned()),
        );
    }
    metadata
}

fn add_terminal_payload_hash(mut metadata: Value, terminal_hash: &str) -> String {
    if let Value::Object(ref mut object) = metadata {
        object.insert(
            "terminal_payload_hash".to_owned(),
            Value::String(terminal_hash.to_owned()),
        );
    }
    metadata.to_string()
}

fn redacted_provider_summary(summary: &str) -> String {
    redact_log_text(summary)
}

fn provider_failure_json(failure: &ProviderFailure) -> serde_json::Value {
    json!({
        "provider": failure.provider,
        "adapter": failure.adapter,
        "phase": failure.phase,
        "error_kind": failure.error_kind,
        "message": provider_failure_summary_message(&failure.message),
        "recoverable": failure.recoverable,
        "retry_after": failure.retry_after,
        "workspace_id": failure.workspace_id,
        "provider_session_id": failure.provider_session_id,
        "provider_thread_id": failure.provider_thread_id,
        "missing_config_keys": failure.missing_config_keys,
        "raw": failure.raw_json.as_deref().map(json_payload_summary),
    })
}

fn provider_failure_summary_message(message: &str) -> String {
    let summary = message
        .split_once(':')
        .map_or(message, |(summary, _)| summary);
    redact_log_text(summary.trim())
}

fn provider_diagnostic_message(provider: &str, summary: &str, diagnostics_json: &str) -> String {
    let diagnostics = serde_json::from_str::<Value>(diagnostics_json).unwrap_or(Value::Null);
    let primary_detail = diagnostics
        .get("failure")
        .and_then(|failure| failure.get("message"))
        .and_then(Value::as_str)
        .or_else(|| {
            diagnostics
                .get("error")
                .and_then(|error| {
                    error
                        .get("message")
                        .or_else(|| error.get("reason"))
                        .or(Some(error))
                })
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|detail| !detail.is_empty());
    let stderr_detail = diagnostics
        .get("stderr")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|detail| !detail.is_empty());
    let mut message = format!("{provider} provider diagnostic: {summary}");
    if let Some(detail) = primary_detail {
        if !message.contains(detail) {
            message.push_str(": ");
            message.push_str(detail);
        }
    }
    if let Some(stderr) = stderr_detail {
        if !message.contains(stderr) {
            message.push_str(": ");
            message.push_str(stderr);
        }
    }
    redact_log_text(&message)
}

fn coerce_status(status: &CoerceStatus) -> &'static str {
    match status {
        CoerceStatus::Succeeded => "completed",
        CoerceStatus::Failed => "failed",
        CoerceStatus::TimedOut => "timed_out",
    }
}

fn coerce_effect_status(status: &CoerceStatus) -> EffectStatus {
    match status {
        CoerceStatus::Succeeded => EffectStatus::Completed,
        CoerceStatus::Failed => EffectStatus::Failed,
        CoerceStatus::TimedOut => EffectStatus::TimedOut,
    }
}

fn coerce_exit_code(status: &CoerceStatus) -> Option<i64> {
    match status {
        CoerceStatus::Succeeded => Some(0),
        CoerceStatus::Failed => Some(1),
        CoerceStatus::TimedOut => None,
    }
}

fn coerce_metadata(result: &CoerceResult) -> String {
    json!({
        "value": result.value_json.as_deref().map(json_payload_summary),
        "error": result_error_payload(result.error_json.as_deref()),
        "transcript": redacted_text_metadata(&result.transcript),
        "usage": json_from_str(&result.usage_json),
    })
    .to_string()
}

fn loft_status(status: &LoftEffectStatus) -> &'static str {
    match status {
        LoftEffectStatus::Succeeded => "completed",
        LoftEffectStatus::Failed => "failed",
        LoftEffectStatus::TimedOut => "timed_out",
    }
}

fn loft_effect_status(status: &LoftEffectStatus) -> EffectStatus {
    match status {
        LoftEffectStatus::Succeeded => EffectStatus::Completed,
        LoftEffectStatus::Failed => EffectStatus::Failed,
        LoftEffectStatus::TimedOut => EffectStatus::TimedOut,
    }
}

fn loft_exit_code(status: &LoftEffectStatus) -> Option<i64> {
    match status {
        LoftEffectStatus::Succeeded => Some(0),
        LoftEffectStatus::Failed => Some(1),
        LoftEffectStatus::TimedOut => None,
    }
}

fn loft_metadata(result: &LoftEffectResult) -> String {
    json!({
        "value": result.value_json.as_deref().map(json_payload_summary),
        "error": result_error_payload(result.error_json.as_deref()),
        "transcript": redacted_text_metadata(&result.transcript),
    })
    .to_string()
}

fn loft_fact_name(action: LoftAction, status: &LoftEffectStatus) -> String {
    let suffix = match status {
        LoftEffectStatus::Succeeded => "succeeded",
        LoftEffectStatus::Failed => "failed",
        LoftEffectStatus::TimedOut => "timed_out",
    };
    format!("{}.{}", action.effect_kind(), suffix)
}

fn loft_schema(action: LoftAction, status: &LoftEffectStatus) -> Option<&'static str> {
    match (action, status) {
        (LoftAction::Show, LoftEffectStatus::Succeeded) => Some("LoftShowSucceeded"),
        (LoftAction::Show, LoftEffectStatus::Failed) => Some("LoftShowFailed"),
        (LoftAction::Show, LoftEffectStatus::TimedOut) => Some("LoftShowTimedOut"),
        (LoftAction::Claim, LoftEffectStatus::Succeeded) => Some("LoftClaimSucceeded"),
        (LoftAction::Claim, LoftEffectStatus::Failed) => Some("LoftClaimFailed"),
        (LoftAction::Claim, LoftEffectStatus::TimedOut) => Some("LoftClaimTimedOut"),
        (LoftAction::Renew, LoftEffectStatus::Succeeded) => Some("LoftRenewSucceeded"),
        (LoftAction::Renew, LoftEffectStatus::Failed) => Some("LoftRenewFailed"),
        (LoftAction::Renew, LoftEffectStatus::TimedOut) => Some("LoftRenewTimedOut"),
        (LoftAction::Release, LoftEffectStatus::Succeeded) => Some("LoftReleaseSucceeded"),
        (LoftAction::Release, LoftEffectStatus::Failed) => Some("LoftReleaseFailed"),
        (LoftAction::Release, LoftEffectStatus::TimedOut) => Some("LoftReleaseTimedOut"),
        (LoftAction::Note, LoftEffectStatus::Succeeded) => Some("LoftNoteSucceeded"),
        (LoftAction::Note, LoftEffectStatus::Failed) => Some("LoftNoteFailed"),
        (LoftAction::Note, LoftEffectStatus::TimedOut) => Some("LoftNoteTimedOut"),
        (LoftAction::Transition, LoftEffectStatus::Succeeded) => Some("LoftTransitionSucceeded"),
        (LoftAction::Transition, LoftEffectStatus::Failed) => Some("LoftTransitionFailed"),
        (LoftAction::Transition, LoftEffectStatus::TimedOut) => Some("LoftTransitionTimedOut"),
        (LoftAction::Evidence, LoftEffectStatus::Succeeded) => Some("LoftEvidenceSucceeded"),
        (LoftAction::Evidence, LoftEffectStatus::Failed) => Some("LoftEvidenceFailed"),
        (LoftAction::Evidence, LoftEffectStatus::TimedOut) => Some("LoftEvidenceTimedOut"),
        (LoftAction::ResourceIntent, LoftEffectStatus::Succeeded) => {
            Some("LoftResourceIntentSucceeded")
        }
        (LoftAction::ResourceIntent, LoftEffectStatus::Failed) => Some("LoftResourceIntentFailed"),
        (LoftAction::ResourceIntent, LoftEffectStatus::TimedOut) => {
            Some("LoftResourceIntentTimedOut")
        }
        (LoftAction::Complete, LoftEffectStatus::Succeeded) => Some("LoftCompleteSucceeded"),
        (LoftAction::Complete, LoftEffectStatus::Failed) => Some("LoftCompleteFailed"),
        (LoftAction::Complete, LoftEffectStatus::TimedOut) => Some("LoftCompleteTimedOut"),
        (LoftAction::Fail, LoftEffectStatus::Succeeded) => Some("LoftFailSucceeded"),
        (LoftAction::Fail, LoftEffectStatus::Failed) => Some("LoftFailFailed"),
        (LoftAction::Fail, LoftEffectStatus::TimedOut) => Some("LoftFailTimedOut"),
    }
}

fn json_from_str(source: &str) -> Value {
    serde_json::from_str(source).unwrap_or_else(|_| Value::String(source.to_owned()))
}

fn string_array_field(value: &Value, field: &str) -> Option<Vec<String>> {
    value.get(field)?.as_array().map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    })
}

/// Placeholder kernel entry point.
///
/// The real kernel will own rule commits, effect graph enqueueing, dependency
/// release, leases, retries, and trace emission.
pub fn kernel_stage() -> &'static str {
    whipplescript_store::store_stage()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coerce::{CoerceRequest, FakeCoerceClient};
    use harness::{
        ClaudeCodeAgentHarness, CodexAgentHarness, CommandLaunchPlan, MockAgentHarness,
        PiStyleAgentHarness,
    };
    use loft::{FakeLoftClient, LoftAction, LoftEffectRequest};
    use native_lifecycle::{normalize_pi_rpc_event, AgentTurnLifecycleKind};
    use std::{fs, path::PathBuf, process::Command};
    use trace::check_trace;
    use whipplescript_parser::compile_program;
    use whipplescript_store::{
        EffectCancellation, EffectCancellationRequest, EffectCompletion, LeaseRenewal, NewEffect,
        NewFact, RetryEffect, RevisionActivation, RuleCommit, RunStart, SkillRegistration,
    };

    #[test]
    fn kernel_scaffold_links_to_store() {
        assert_eq!(kernel_stage(), whipplescript_core::IMPLEMENTATION_STAGE);
    }

    #[test]
    fn derives_deterministic_idempotency_keys() {
        let first = idempotency_key(&["instance-a", "rule", "start"]);
        let second = idempotency_key(&["instance-a", "rule", "start"]);
        let different = idempotency_key(&["instance-a", "rule", "again"]);

        assert_eq!(first, second);
        assert_ne!(first, different);
        assert!(first.starts_with("key_"));
    }

    #[test]
    fn analysis_summary_hashes_generated_declarations() {
        let compiled = compile_program(
            r#"
pattern Review<Input> {
  class Result {
    item Input
  }

  rule dispatch
    when Input as item
  => {
  }
}

workflow Root {
  class Task {
    title string
  }

  apply Review<Task> as taskReview {
  }
}
"#,
        );
        assert_eq!(compiled.diagnostics, Vec::new());
        let program = compiled.ir.expect("program compiles");

        let summary = serde_json::from_str::<Value>(&program_analysis_summary_json(&program))
            .expect("analysis summary is valid JSON");
        assert!(summary
            .get("schemas")
            .and_then(Value::as_array)
            .expect("schemas")
            .iter()
            .any(|schema| {
                schema.get("name").and_then(Value::as_str) == Some("Task")
                    && schema
                        .pointer("/source_span/construct")
                        .and_then(Value::as_str)
                        == Some("class")
            }));
        let hashes = summary
            .get("generated_declaration_hashes")
            .and_then(Value::as_array)
            .expect("generated declaration hashes are present");
        let class_hash = hashes
            .iter()
            .find(|entry| {
                entry.get("declaration").and_then(Value::as_str) == Some("class:taskReview_Result")
            })
            .expect("generated class hash is present");
        let rule_hash = hashes
            .iter()
            .find(|entry| {
                entry.get("declaration").and_then(Value::as_str) == Some("rule:taskReview_dispatch")
            })
            .expect("generated rule hash is present");

        for entry in [class_hash, rule_hash] {
            let hash = entry
                .get("hash")
                .and_then(Value::as_str)
                .expect("hash is a string");
            assert_eq!(hash.len(), 16);
            assert!(hash.chars().all(|ch| ch.is_ascii_hexdigit()));
            assert_ne!(entry.get("missing").and_then(Value::as_bool), Some(true));
        }
    }

    #[test]
    fn analysis_summary_reports_a_stable_whole_closure_bundle_hash() {
        // The compiled-IR report carries a single `bundle_hash` fingerprinting the
        // whole include closure (in addition to per-include `source_hash`). It is
        // deterministic, present for an include-free program, and changes iff the
        // closure changes (a member added, or a member's content changed).
        let compiled = compile_program(
            r#"
workflow Root {
  output result Done
  class Done { note string }

  rule finish
    when Done as d
  => {
    complete result { note d.note }
  }
}
"#,
        );
        assert_eq!(compiled.diagnostics, Vec::new());
        let base = compiled.ir.expect("program compiles");

        let bundle_hash = |program: &IrProgram| {
            serde_json::from_str::<Value>(&program_analysis_summary_json(program))
                .expect("valid JSON")
                .get("bundle_hash")
                .and_then(Value::as_str)
                .expect("bundle_hash present")
                .to_owned()
        };

        // Include-free closure: deterministic 16-hex fingerprint.
        let empty = bundle_hash(&base);
        assert_eq!(empty.len(), 16);
        assert!(empty.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(empty, bundle_hash(&base), "same closure -> same hash");

        // Adding a closure member changes the bundle hash.
        let mut with_one = base.clone();
        with_one.includes.push(whipplescript_parser::IrInclude {
            path: "lib.whip".to_owned(),
            source_hash: Some("aaaa".to_owned()),
        });
        let one = bundle_hash(&with_one);
        assert_ne!(one, empty, "adding an include changes the bundle hash");

        // Changing a member's content changes the bundle hash.
        let mut changed = base.clone();
        changed.includes.push(whipplescript_parser::IrInclude {
            path: "lib.whip".to_owned(),
            source_hash: Some("bbbb".to_owned()),
        });
        assert_ne!(
            bundle_hash(&changed),
            one,
            "changed include content changes the bundle hash"
        );
    }

    #[test]
    fn idempotent_revision_activation_emits_trace_once_from_stored_event() {
        let compiled = compile_program(
            r#"
workflow RevisionTrace

rule noop
  when started
=> {
}
"#,
        );
        assert_eq!(compiled.diagnostics, Vec::new());
        let program = compiled.ir.expect("program compiles");
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version1 = kernel
            .create_program_version_for_program(
                ProgramVersionInput {
                    program_name: &program.workflow,
                    source_hash: "source-1",
                    ir_hash: "ir-1",
                    compiler_version: "test",
                },
                &program,
            )
            .expect("first version creates");
        let version2 = kernel
            .create_program_version_for_program(
                ProgramVersionInput {
                    program_name: &program.workflow,
                    source_hash: "source-2",
                    ir_hash: "ir-2",
                    compiler_version: "test",
                },
                &program,
            )
            .expect("second version creates");
        let instance_id = kernel
            .create_instance(&version1, "{}")
            .expect("instance creates");

        let first = kernel
            .activate_revision(RevisionActivation {
                instance_id: &instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-once"),
            })
            .expect("revision activates");
        let retry = kernel
            .activate_revision(RevisionActivation {
                instance_id: &instance_id,
                from_version_id: &version1.version_id,
                to_version_id: &version2.version_id,
                activation_policy_json: "{}",
                cancellation_policy: "keep",
                idempotency_key: Some("revise-once"),
            })
            .expect("idempotent retry returns existing revision");

        assert_eq!(retry.revision_id, first.revision_id);
        let activation_records = kernel
            .trace()
            .iter()
            .filter(|record| matches!(record.event, TraceEvent::RevisionActivated { .. }))
            .count();
        assert_eq!(activation_records, 1);
        check_trace(kernel.trace()).expect("revision trace remains conformant");
    }

    #[test]
    fn program_agent_declarations_drive_capacity_blocks() {
        let compiled = compile_program(
            r#"
workflow CapacityFromSource

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell"]
}

rule start
  when started
=> {
  tell worker "one"
  tell worker "two"
}
"#,
        );
        let program = compiled.ir.expect("program compiles");
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version_for_program(
                ProgramVersionInput {
                    program_name: &program.workflow,
                    source_hash: "source",
                    ir_hash: "ir",
                    compiler_version: "test",
                },
                &program,
            )
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [
            NewEffect {
                timeout_seconds: None,
                effect_id: "tell-one",
                kind: "agent.tell",
                target: Some("worker"),
                input_json: r#"{"prompt":"one"}"#,
                status: "queued",
                idempotency_key: "rule=start;effect=tell-one",
                required_capabilities_json: "[]",
                profile: Some("repo-writer"),
                correlation_id: None,
                source_span_json: None,
            },
            NewEffect {
                timeout_seconds: None,
                effect_id: "tell-two",
                kind: "agent.tell",
                target: Some("worker"),
                input_json: r#"{"prompt":"two"}"#,
                status: "queued",
                idempotency_key: "rule=start;effect=tell-two",
                required_capabilities_json: "[]",
                profile: Some("repo-writer"),
                correlation_id: None,
                source_span_json: None,
            },
        ];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        kernel
            .start_run(RunStart {
                instance_id: &instance_id,
                effect_id: "tell-one",
                run_id: "run-one",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-one",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("first run starts");

        let blocked = kernel.start_run(RunStart {
            instance_id: &instance_id,
            effect_id: "tell-two",
            run_id: "run-two",
            provider: "test",
            worker_id: "worker-1",
            lease_id: "lease-two",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        });

        assert!(
            matches!(blocked, Err(StoreError::PolicyBlocked { reason, .. }) if reason.contains("capacity exhausted"))
        );
        assert!(kernel.trace().iter().any(|record| matches!(
            &record.event,
            TraceEvent::EffectBlocked {
                effect_id, reason, ..
            }
                if effect_id == "tell-two" && reason.contains("capacity exhausted")
        )));
        let store = kernel.into_store();
        let effects = store.list_effects(&instance_id).expect("effects load");
        let second = effects
            .iter()
            .find(|effect| effect.effect_id == "tell-two")
            .expect("second effect exists");
        assert_eq!(second.status, "blocked_by_capacity");
        assert!(second
            .policy_block_reason
            .as_deref()
            .expect("capacity reason")
            .contains("capacity exhausted"));
    }

    #[test]
    fn program_agent_capabilities_block_mismatched_effects() {
        let compiled = compile_program(
            r#"
workflow CapabilityFromSource

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell"]
}

rule start
  when started
=> {
  tell worker "write"
}
"#,
        );
        let program = compiled.ir.expect("program compiles");
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version_for_program(
                ProgramVersionInput {
                    program_name: &program.workflow,
                    source_hash: "source",
                    ir_hash: "ir",
                    compiler_version: "test",
                },
                &program,
            )
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell-write",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"write"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell-write",
            required_capabilities_json: r#"["repo.write"]"#,
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let blocked = kernel.start_run(RunStart {
            instance_id: &instance_id,
            effect_id: "tell-write",
            run_id: "run-write",
            provider: "test",
            worker_id: "worker-1",
            lease_id: "lease-write",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        });

        assert!(
            matches!(blocked, Err(StoreError::PolicyBlocked { reason, .. }) if reason.contains("does not declare required capability `repo.write`"))
        );
    }

    #[test]
    fn kernel_creates_program_instance_and_ingests_external_event() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "Ralph",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let event = kernel
            .ingest_external_event(&instance_id, "external.started", "{}", Some("start"))
            .expect("event ingests");

        assert_eq!(event.sequence, 1);
        assert!(instance_id.starts_with("ins_"));
    }

    #[test]
    fn kernel_derives_facts_and_evaluates_ready_rules() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let compiled = compile_program(
            r#"
workflow Ready

rule begin
  when started
=> {
}

rule wait
  when missing
=> {
}
"#,
        );
        let program = compiled.ir.expect("program compiles");

        let before = kernel
            .evaluate_rules("instance-a", &program)
            .expect("rules evaluate");
        assert!(before.is_empty());

        kernel
            .derive_fact(
                "instance-a",
                "pattern:started",
                "started",
                "{}",
                None,
                Some("derive-started"),
            )
            .expect("fact derives");
        let after = kernel
            .evaluate_rules("instance-a", &program)
            .expect("rules evaluate after fact");

        assert_eq!(after, vec!["begin".to_owned()]);
    }

    #[test]
    fn kernel_commits_rule_rewrite_with_effect_graph() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let facts = [NewFact {
            fact_id: "fact-1",
            name: "WorkItem",
            key: "issue-1",
            value_json: r#"{"title":"Implement"}"#,
            schema_id: Some("WorkItem"),
            provenance_class: "derived",
            correlation_id: None,
            source_span_json: None,
        }];
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"go"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell",
            required_capabilities_json: "[]",
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        let event = kernel
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &facts,
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        assert_eq!(event.sequence, 1);
    }

    #[test]
    fn kernel_claims_and_completes_effect_run() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"go"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell",
            required_capabilities_json: "[]",
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let claimable = kernel
            .claimable_effects("instance-a")
            .expect("claimable effects load");
        assert_eq!(claimable[0].effect_id, "tell");
        let started = kernel
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        let completed = kernel
            .complete_run(EffectCompletion {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                status: "ignored",
                exit_code: Some(0),
                summary: Some("done"),
                metadata_json: "{}",
                idempotency_key: Some("complete-tell"),
            })
            .expect("run completes");

        assert_eq!(started.sequence, 2);
        assert_eq!(completed.sequence, 3);
        check_trace(kernel.trace()).expect("kernel trace conforms");
    }

    #[test]
    fn run_native_agent_turn_records_lifecycle_artifacts_and_terminal() {
        struct FakeAdapter {
            capability: provider::ProviderCapability,
            events: Vec<NativeProviderEvent>,
        }

        impl NativeProviderAdapter for FakeAdapter {
            fn provider_id(&self) -> &str {
                "native-fixture"
            }

            fn capability(&self) -> &provider::ProviderCapability {
                &self.capability
            }

            fn start_turn(
                &mut self,
                request: NativeProviderTurnRequest,
            ) -> Result<NativeProviderEvent, provider::NativeProviderBoundaryError> {
                Ok(NativeProviderEvent {
                    provider_id: "native-fixture".to_owned(),
                    run_id: request.run_id,
                    event_kind: NativeProviderEventKind::Started,
                    provider_event_type: "fixture.turn.started".to_owned(),
                    provider_session_id: Some("session-1".to_owned()),
                    provider_turn_id: Some("turn-1".to_owned()),
                    sequence: Some(1),
                    evidence: json!({"started": true}),
                    artifacts: Vec::new(),
                })
            }

            fn next_event(
                &mut self,
                _run_id: &str,
            ) -> Result<Option<NativeProviderEvent>, provider::NativeProviderBoundaryError>
            {
                Ok((!self.events.is_empty()).then(|| self.events.remove(0)))
            }

            fn cancel_turn(
                &mut self,
                _cancellation: provider::NativeProviderCancellation,
            ) -> Result<NativeProviderEvent, provider::NativeProviderBoundaryError> {
                unreachable!("cancel not used in this regression")
            }
        }

        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"go"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell",
            required_capabilities_json: "[]",
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let capability = provider::ProviderCapability {
            provider_kind: provider::ProviderKind::Fixture,
            surface: provider::AdapterSurface::Fixture,
            protocol_version: Some("fixture.v1".to_owned()),
            session_identity_fields: vec!["session".to_owned()],
            stream_event_kinds: vec!["fixture.turn.started".to_owned()],
            tool_policy: "fixture".to_owned(),
            cancellation_depths: vec![provider::CancellationDepth::CooperativeRequest],
            artifact_manifest: true,
            health_checks: Vec::new(),
            auth_requirements: Vec::new(),
        };
        let mut adapter = FakeAdapter {
            capability,
            events: vec![
                NativeProviderEvent {
                    provider_id: "native-fixture".to_owned(),
                    run_id: "run-tell".to_owned(),
                    event_kind: NativeProviderEventKind::Streamed,
                    provider_event_type: "fixture.turn.delta".to_owned(),
                    provider_session_id: Some("session-1".to_owned()),
                    provider_turn_id: Some("turn-1".to_owned()),
                    sequence: Some(2),
                    evidence: json!({"delta": 1}),
                    artifacts: Vec::new(),
                },
                NativeProviderEvent {
                    provider_id: "native-fixture".to_owned(),
                    run_id: "run-tell".to_owned(),
                    event_kind: NativeProviderEventKind::Streamed,
                    provider_event_type: "fixture.turn.delta".to_owned(),
                    provider_session_id: Some("session-1".to_owned()),
                    provider_turn_id: Some("turn-1".to_owned()),
                    sequence: Some(3),
                    evidence: json!({"delta": 2}),
                    artifacts: Vec::new(),
                },
                NativeProviderEvent {
                    provider_id: "native-fixture".to_owned(),
                    run_id: "run-tell".to_owned(),
                    event_kind: NativeProviderEventKind::ArtifactCaptured,
                    provider_event_type: "fixture.artifact.captured".to_owned(),
                    provider_session_id: Some("session-1".to_owned()),
                    provider_turn_id: Some("turn-1".to_owned()),
                    sequence: Some(4),
                    evidence: json!({"artifact": true}),
                    artifacts: vec![NativeProviderArtifactRef {
                        artifact_id: Some("artifact-1".to_owned()),
                        kind: "diff".to_owned(),
                        uri: "provider://native-fixture/runs/run-tell/diff".to_owned(),
                        content_hash: Some("sha256:abc".to_owned()),
                        mime_type: Some("text/x-diff".to_owned()),
                        required: true,
                    }],
                },
                NativeProviderEvent {
                    provider_id: "native-fixture".to_owned(),
                    run_id: "run-tell".to_owned(),
                    event_kind: NativeProviderEventKind::Completed,
                    provider_event_type: "fixture.turn.completed".to_owned(),
                    provider_session_id: Some("session-1".to_owned()),
                    provider_turn_id: Some("turn-1".to_owned()),
                    sequence: Some(5),
                    evidence: json!({"completed": true}),
                    artifacts: Vec::new(),
                },
            ],
        };

        let request = NativeProviderTurnRequest {
            provider_id: "native-fixture".to_owned(),
            provider_kind: provider::ProviderKind::Fixture,
            surface: provider::AdapterSurface::Fixture,
            run_id: "run-tell".to_owned(),
            effect_id: "tell".to_owned(),
            agent: "worker".to_owned(),
            profile: Some("repo-writer".to_owned()),
            prompt_json: json!({"prompt": "go"}),
            workspace_policy: "isolated".to_owned(),
            required_capabilities: Vec::new(),
            cancellation_depth: provider::CancellationDepth::CooperativeRequest,
            artifact_policy: "metadata".to_owned(),
            credential_ref: None,
            provider_options: std::collections::BTreeMap::new(),
        };

        let terminal = kernel
            .run_native_agent_turn_with_metadata(
                AgentTurnExecution {
                    instance_id: "instance-a",
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider: "native-fixture",
                    worker_id: "worker-1",
                    lease_id: "lease-tell",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    agent: "worker",
                    profile: Some("repo-writer"),
                    input_json: r#"{"prompt":"go"}"#,
                    skill_names: &[],
                },
                request,
                &mut adapter,
                8,
                r#"{"provider_selection":{"provider_id":"native-fixture","provider_kind":"native-fixture","source_harness_id":"fixtureHarness","surface":"fixture"}}"#,
            )
            .expect("native turn completes");
        assert!(terminal.sequence > 1);

        let runs = kernel.store.list_runs("instance-a").expect("runs list");
        assert_eq!(runs[0].status, "completed");
        let run_metadata = json_from_str(&runs[0].metadata_json);
        assert_eq!(
            run_metadata
                .pointer("/provider_selection/source_harness_id")
                .and_then(Value::as_str),
            Some("fixtureHarness")
        );
        assert_eq!(
            run_metadata
                .pointer("/native_provider/provider_id")
                .and_then(Value::as_str),
            Some("native-fixture")
        );
        let artifacts = kernel
            .store
            .list_artifacts_for_run("run-tell")
            .expect("artifacts list");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].kind, "diff");
        let facts = kernel.store.list_facts("instance-a").expect("facts list");
        // Only the durable lifecycle kinds are rule-matchable facts
        // (spec/agent-harness.md): started + completed here.
        assert!(facts.iter().any(|fact| fact.name == "agent.turn.started"));
        assert!(facts.iter().any(|fact| fact.name == "agent.turn.completed"));
        // In-turn observations are EVIDENCE, never facts — they must not appear
        // in the fact set (where they would inflate `facts=N` with values no
        // rule can match).
        assert_eq!(
            facts
                .iter()
                .filter(|fact| fact.name == "agent.turn.streamed")
                .count(),
            0,
            "streamed observations must not be derived as facts"
        );
        assert!(
            !facts
                .iter()
                .any(|fact| fact.name == "agent.turn.artifact_captured"),
            "artifact_captured observations must not be derived as facts"
        );
        // They are recorded as evidence instead: every native lifecycle
        // observation (terminal and in-turn) is durable evidence.
        let evidence = kernel.store.list_evidence("instance-a").expect("evidence");
        let streamed_evidence = evidence
            .iter()
            .filter(|record| {
                record.kind == "agent.turn.native_event"
                    && json_from_str(&record.metadata_json)
                        .pointer("/status")
                        .and_then(Value::as_str)
                        == Some("streamed")
            })
            .count();
        assert_eq!(
            streamed_evidence, 2,
            "streamed observations must be recorded as evidence"
        );
        assert!(
            evidence.iter().any(|record| {
                record.kind == "agent.turn.native_event"
                    && json_from_str(&record.metadata_json)
                        .pointer("/status")
                        .and_then(Value::as_str)
                        == Some("artifact_captured")
            }),
            "artifact_captured observation must be recorded as evidence"
        );
    }

    #[test]
    fn kernel_times_out_effect_run() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"go"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell",
            required_capabilities_json: "[]",
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        kernel
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        let timed_out = kernel
            .timeout_run(EffectCompletion {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                status: "ignored",
                exit_code: None,
                summary: Some("timeout"),
                metadata_json: "{}",
                idempotency_key: Some("timeout-tell"),
            })
            .expect("run times out");

        assert_eq!(timed_out.sequence, 3);
        check_trace(kernel.trace()).expect("kernel trace conforms");
    }

    #[test]
    fn kernel_expires_leases_and_retries_failed_effects() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"go"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell",
            required_capabilities_json: "[]",
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: "instance-a",
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        kernel
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        kernel
            .renew_lease(LeaseRenewal {
                instance_id: "instance-a",
                lease_id: "lease-tell",
                run_id: "run-tell",
                new_expires_at: "2030-01-02T00:00:00Z",
                idempotency_key: Some("renew-tell"),
            })
            .expect("lease renews");
        let expired = kernel
            .expire_leases("instance-a", "2030-01-03T00:00:00Z")
            .expect("lease expires");
        assert_eq!(expired.len(), 1);
        let stale_completion = kernel.complete_run(EffectCompletion {
            instance_id: "instance-a",
            effect_id: "tell",
            run_id: "run-tell",
            provider: "test",
            worker_id: "worker-1",
            status: "ignored",
            exit_code: Some(0),
            summary: Some("late"),
            metadata_json: "{}",
            idempotency_key: Some("late-complete"),
        });
        assert!(stale_completion.is_err());

        kernel
            .start_run(RunStart {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell-2",
                provider: "test",
                worker_id: "worker-1",
                lease_id: "lease-tell-2",
                lease_expires_at: "2030-01-04T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("retry run starts after lease expiry");
        kernel
            .fail_run(EffectCompletion {
                instance_id: "instance-a",
                effect_id: "tell",
                run_id: "run-tell-2",
                provider: "test",
                worker_id: "worker-1",
                status: "ignored",
                exit_code: Some(1),
                summary: Some("failed"),
                metadata_json: "{}",
                idempotency_key: Some("fail-tell"),
            })
            .expect("run fails");
        kernel
            .retry_effect(RetryEffect {
                instance_id: "instance-a",
                effect_id: "tell",
                retry_after: Some("2030-01-05T00:00:00Z"),
                idempotency_key: Some("retry-tell"),
            })
            .expect("effect retries");

        check_trace(kernel.trace()).expect("kernel trace conforms");
    }

    #[test]
    fn kernel_cancels_effect_and_instance() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "Lifecycle",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"go"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell",
            required_capabilities_json: "[]",
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let cancelled_effect = kernel
            .cancel_effect(EffectCancellation {
                instance_id: &instance_id,
                effect_id: "tell",
                reason: Some("operator"),
                idempotency_key: Some("cancel-effect"),
            })
            .expect("effect cancels");
        let paused = kernel
            .pause_instance(&instance_id, Some("maintenance"), Some("pause"))
            .expect("instance pauses");
        let resumed = kernel
            .resume_instance(&instance_id, Some("resume"))
            .expect("instance resumes");
        let cancelled_instance = kernel
            .cancel_instance(&instance_id, Some("operator"), Some("cancel-instance"))
            .expect("instance cancels");

        assert_eq!(cancelled_effect.sequence, 2);
        assert_eq!(paused.sequence, 3);
        assert_eq!(resumed.sequence, 4);
        assert_eq!(cancelled_instance.sequence, 5);
        check_trace(kernel.trace()).expect("kernel trace conforms");
    }

    #[test]
    fn kernel_fail_instance_internal_marks_failed_terminal() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "AutoFail",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        // Generic internal failure (flow auto-fail): no typed `failure` payload.
        kernel
            .fail_instance_internal(
                &instance_id,
                "unhandled effect failure in flow review",
                Some("autofail-1"),
            )
            .expect("instance fails internally");
        let instance = kernel
            .store
            .get_instance(&instance_id)
            .expect("instance loads")
            .expect("instance exists");
        assert_eq!(instance.status, "failed");
        check_trace(kernel.trace()).expect("kernel trace conforms");
    }

    #[test]
    fn kernel_emits_trace_for_profile_policy_blocks() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "Policy",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "write",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"write"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=write",
            required_capabilities_json: r#"["agent.tell","repo.write"]"#,
            profile: Some("repo-reader"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let blocked = kernel.start_run(RunStart {
            instance_id: &instance_id,
            effect_id: "write",
            run_id: "run-write",
            provider: "test",
            worker_id: "worker-1",
            lease_id: "lease-write",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        });

        assert!(matches!(blocked, Err(StoreError::PolicyBlocked { .. })));
        assert!(kernel.trace().iter().any(|record| matches!(
            &record.event,
            TraceEvent::EffectBlocked {
                effect_id, reason, ..
            }
                if effect_id == "write" && reason.contains("repo.write")
        )));
        check_trace(kernel.trace()).expect("kernel trace conforms");
    }

    #[test]
    fn mock_agent_harness_records_artifacts_evidence_and_turn_fact() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        store
            .register_skill(SkillRegistration {
                skill_id: "skill-loft-user",
                name: "loft-user",
                version: "1.0.0",
                source: "# Loft User\n",
                source_path: "skills/loft-user/SKILL.md",
                description: "Loft instructions",
                required_capabilities_json: r#"["loft.claim"]"#,
                metadata_json: "{}",
            })
            .expect("skill registers");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "Harness",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"go"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell",
            required_capabilities_json: "[]",
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let run_metadata_json = json!({
            "provider_selection": {
                "provider_id": "mock",
                "provider_kind": "fixture",
                "source_harness_id": "mock",
                "surface": Value::Null,
            }
        })
        .to_string();
        let terminal = kernel
            .run_agent_turn_with_metadata(
                AgentTurnExecution {
                    instance_id: &instance_id,
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider: "mock",
                    worker_id: "worker-1",
                    lease_id: "lease-tell",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    agent: "worker",
                    profile: Some("repo-writer"),
                    input_json: r#"{"prompt":"go"}"#,
                    skill_names: &["loft-user"],
                },
                &MockAgentHarness::completed("done"),
                &run_metadata_json,
            )
            .expect("mock turn runs");

        assert_eq!(terminal.sequence, 3);
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let mut store = kernel.into_store();
        let artifacts = store
            .list_artifacts_for_run("run-tell")
            .expect("artifacts list");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].kind, "transcript");

        let evidence = store.list_evidence(&instance_id).expect("evidence lists");
        assert_eq!(evidence.len(), 3);
        assert!(evidence
            .iter()
            .any(|evidence| evidence.kind == "rule.committed"
                && evidence.metadata_json.contains("tell")));
        assert!(evidence
            .iter()
            .any(|evidence| evidence.kind == "skills.injected"
                && evidence.metadata_json.contains("loft-user")));
        let provider_evidence = evidence
            .iter()
            .find(|evidence| evidence.kind == "agent.turn.provider")
            .expect("provider evidence recorded");
        let provider_metadata = serde_json::from_str::<Value>(&provider_evidence.metadata_json)
            .expect("provider metadata json");
        assert_eq!(
            provider_metadata
                .pointer("/stdout/redacted")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            provider_metadata
                .pointer("/transcript/redacted")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(provider_metadata.get("artifact_ids").is_some());
        assert_eq!(
            provider_metadata
                .pointer("/artifact_manifest/schema_version")
                .and_then(Value::as_str),
            Some(artifact_manifest::ARTIFACT_MANIFEST_SCHEMA_VERSION)
        );
        artifact_manifest::validate_artifact_manifest(
            provider_metadata
                .get("artifact_manifest")
                .expect("artifact manifest"),
        )
        .expect("artifact manifest validates");
        assert_eq!(
            provider_metadata
                .pointer("/artifact_manifest/entries/0/artifact_id")
                .and_then(Value::as_str),
            Some(artifacts[0].artifact_id.as_str())
        );
        assert_eq!(
            provider_metadata
                .pointer("/artifact_manifest/entries/0/retention_policy")
                .and_then(Value::as_str),
            Some("provider_default")
        );
        assert!(!provider_evidence.metadata_json.contains("mock stdout"));
        assert!(!provider_evidence.metadata_json.contains("mock transcript"));
        let links = store
            .list_evidence_links(&instance_id)
            .expect("evidence links list");
        assert!(links
            .iter()
            .any(|link| link.target_type == "effect" && link.target_id == "tell"));

        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(facts.iter().any(|fact| fact.name == "agent.turn.completed"
            && fact.value_json.contains("\"provider\":\"mock\"")));
        let events = store.list_events(&instance_id).expect("events list");
        let terminal_event = events
            .iter()
            .find(|event| event.event_type == "effect.terminal")
            .expect("terminal event exists");
        let terminal_payload =
            serde_json::from_str::<Value>(&terminal_event.payload_json).expect("terminal payload");
        assert!(terminal_payload
            .pointer("/metadata/provider_correlation_id")
            .and_then(Value::as_str)
            .is_some());
        assert!(terminal_payload
            .pointer("/metadata/terminal_payload_hash")
            .and_then(Value::as_str)
            .is_some());
        assert_eq!(
            terminal_payload
                .pointer("/metadata/provider_selection/source_harness_id")
                .and_then(Value::as_str),
            Some("mock")
        );
        let runs = store.list_runs(&instance_id).expect("runs list");
        assert_eq!(
            serde_json::from_str::<Value>(&runs[0].metadata_json)
                .expect("run metadata json")
                .pointer("/provider_selection/provider_kind")
                .and_then(Value::as_str),
            Some("fixture")
        );
        assert!(events
            .iter()
            .any(|event| event.event_type == "agent.turn.completed"));

        store
            .rebuild_projections(&instance_id)
            .expect("projections rebuild");
        let replayed_artifacts = store
            .list_artifacts_for_run("run-tell")
            .expect("replayed artifacts list");
        assert_eq!(replayed_artifacts.len(), 1);
        assert_eq!(replayed_artifacts[0].artifact_id, artifacts[0].artifact_id);
    }

    #[test]
    fn native_provider_lifecycle_observation_records_event_and_fact() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "NativeLifecycle",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let observation = normalize_pi_rpc_event(&json!({
            "type": "turn_end",
            "message": {
                "role": "assistant",
                "stopReason": "aborted",
                "content": [{"type": "text", "text": "secret"}],
            },
        }))
        .expect("Pi event normalizes");
        assert_eq!(observation.kind, AgentTurnLifecycleKind::Cancelled);

        kernel
            .record_native_agent_turn_observation(
                AgentTurnExecution {
                    instance_id: &instance_id,
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider: "pi-main",
                    worker_id: "worker-1",
                    lease_id: "lease-tell",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    agent: "worker",
                    profile: Some("repo-reader"),
                    input_json: r#"{"prompt":"go"}"#,
                    skill_names: &[],
                },
                &observation,
                "pi-turn-end-1",
            )
            .expect("native lifecycle records");

        let store = kernel.into_store();
        let events = store.list_events(&instance_id).expect("events list");
        let event = events
            .iter()
            .find(|event| event.event_type == "agent.turn.cancelled")
            .expect("cancelled event recorded");
        let payload = serde_json::from_str::<Value>(&event.payload_json).expect("payload json");
        assert_eq!(
            payload.get("provider_event_type").and_then(Value::as_str),
            Some("turn_end")
        );
        assert_eq!(payload.get("terminal").and_then(Value::as_bool), Some(true));
        let evidence_id = payload
            .get("evidence_id")
            .and_then(Value::as_str)
            .expect("event links evidence");
        assert!(!event.payload_json.contains("secret"));

        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(facts.iter().any(|fact| {
            fact.name == "agent.turn.cancelled"
                && fact.key != "run-tell"
                && fact.value_json.contains(r#""run_id":"run-tell""#)
                && !fact.value_json.contains("secret")
        }));
        let evidence = store.list_evidence(&instance_id).expect("evidence lists");
        let native_evidence = evidence
            .iter()
            .find(|evidence| evidence.evidence_id == evidence_id)
            .expect("native lifecycle evidence recorded");
        assert_eq!(native_evidence.kind, "agent.turn.native_event");
        assert_eq!(native_evidence.subject_type, "run");
        assert_eq!(native_evidence.subject_id, "run-tell");
        assert!(!native_evidence.metadata_json.contains("secret"));
    }

    #[test]
    fn artifact_capture_failure_records_redacted_event_and_diagnostic() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "ArtifactFailure",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");

        let event = kernel
            .record_artifact_capture_failure(
                AgentTurnExecution {
                    instance_id: &instance_id,
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider: "codex",
                    worker_id: "worker-1",
                    lease_id: "lease-tell",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    agent: "worker",
                    profile: Some("repo-writer"),
                    input_json: "{}",
                    skill_names: &[],
                },
                ArtifactCaptureFailure {
                    provider: "codex",
                    adapter: "app_server",
                    run_id: "run-tell",
                    artifact_ref: "provider://codex/runs/run-tell/diff",
                    error_kind: "hash_mismatch",
                    recoverable: false,
                    message: "secret hash mismatch details",
                    transcript_ref: Some("provider://codex/runs/run-tell/transcript_ref"),
                    stderr_ref: None,
                },
                Some("artifact-capture-failed-run-tell"),
            )
            .expect("artifact capture failure records");

        let store = kernel.into_store();
        let events = store.list_events(&instance_id).expect("events list");
        let capture_event = events
            .iter()
            .find(|stored| stored.event_id == event.event_id)
            .expect("capture failure event exists");
        assert_eq!(
            capture_event.event_type,
            artifact_manifest::ARTIFACT_CAPTURE_FAILED_EVENT
        );
        assert!(capture_event
            .payload_json
            .contains("\"error_kind\":\"hash_mismatch\""));
        assert!(!capture_event
            .payload_json
            .contains("secret hash mismatch details"));
        let diagnostics = store
            .list_diagnostics(Some(&instance_id))
            .expect("diagnostics list");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].code.as_deref(),
            Some(artifact_manifest::ARTIFACT_CAPTURE_FAILED_EVENT)
        );
        assert_eq!(
            diagnostics[0].event_id.as_deref(),
            Some(event.event_id.as_str())
        );
        assert_eq!(diagnostics[0].effect_id.as_deref(), Some("tell"));
        assert_eq!(diagnostics[0].run_id.as_deref(), Some("run-tell"));
    }

    #[test]
    fn provider_secret_seed_never_reaches_durable_records() {
        struct SecretSeedHarness;

        impl AgentHarness for SecretSeedHarness {
            fn run(&self, _request: AgentTurnRequest) -> ProviderRunResult {
                let secret = "sk-test-secret-token-1234567890";
                ProviderRunResult {
                    status: ProviderRunStatus::Failed,
                    summary: format!("provider failed with token={secret}"),
                    stdout: format!("stdout {secret}\n"),
                    stderr: format!("stderr Bearer {secret}\n"),
                    transcript: format!("transcript api_key={secret}\n"),
                    exit_code: Some(1),
                    usage_json: r#"{"input_tokens":1,"output_tokens":0}"#.to_owned(),
                    artifacts: vec![harness::ProviderArtifact {
                        kind: "transcript".to_owned(),
                        path: format!("provider://codex/runs/run-tell/{secret}/transcript_ref"),
                        content_hash: Some(format!("sha256:{secret}")),
                        mime_type: Some("text/plain".to_owned()),
                    }],
                    failure: Some(
                        ProviderFailure::new(
                            "provider.fixture.failed",
                            "fixture_failure",
                            format!("failure token={secret}: stderr {secret}"),
                        )
                        .provider("codex")
                        .adapter("fixture")
                        .raw_json(format!(r#"{{"api_key":"{secret}","shape":"object"}}"#)),
                    ),
                }
            }
        }

        let secret = "sk-test-secret-token-1234567890";
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "SecretSeed",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: "{}",
            status: "queued",
            idempotency_key: "effect-tell",
            required_capabilities_json: "[]",
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start-secret-seed"),
            })
            .expect("rule commits");

        kernel
            .run_agent_turn(
                AgentTurnExecution {
                    instance_id: &instance_id,
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider: "codex",
                    worker_id: "worker-1",
                    lease_id: "lease-tell",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    agent: "worker",
                    profile: Some("repo-writer"),
                    input_json: r#"{"prompt":"go"}"#,
                    skill_names: &[],
                },
                &SecretSeedHarness,
            )
            .expect("secret-seeded turn runs");

        let store = kernel.into_store();
        let events = store.list_events(&instance_id).expect("events list");
        for event in &events {
            assert!(
                !event.payload_json.contains(secret),
                "event {} leaked secret: {}",
                event.event_type,
                event.payload_json
            );
        }
        let terminal_event = events
            .iter()
            .find(|event| event.event_type == "effect.terminal")
            .expect("terminal event exists");
        let terminal_payload =
            serde_json::from_str::<Value>(&terminal_event.payload_json).expect("terminal payload");
        assert_eq!(
            terminal_payload.pointer("/summary").and_then(Value::as_str),
            Some("provider failed with token=[REDACTED]")
        );
        assert_eq!(
            terminal_payload
                .pointer("/metadata/stderr/redacted")
                .and_then(Value::as_bool),
            Some(true)
        );

        for fact in store.list_facts(&instance_id).expect("facts list") {
            assert!(
                !fact.value_json.contains(secret),
                "fact {} leaked secret: {}",
                fact.name,
                fact.value_json
            );
        }
        for evidence in store.list_evidence(&instance_id).expect("evidence list") {
            assert!(
                !evidence
                    .summary
                    .as_deref()
                    .unwrap_or_default()
                    .contains(secret),
                "evidence {} summary leaked secret",
                evidence.kind
            );
            assert!(
                !evidence.metadata_json.contains(secret),
                "evidence {} metadata leaked secret: {}",
                evidence.kind,
                evidence.metadata_json
            );
        }
        for artifact in store
            .list_artifacts_for_run("run-tell")
            .expect("artifacts list")
        {
            assert!(
                !artifact.path.contains(secret),
                "artifact path leaked secret: {}",
                artifact.path
            );
            assert_eq!(artifact.path, "[REDACTED]");
            assert!(
                !artifact
                    .content_hash
                    .as_deref()
                    .unwrap_or_default()
                    .contains(secret),
                "artifact content hash leaked secret: {:?}",
                artifact.content_hash
            );
        }
        for diagnostic in store
            .list_diagnostics(Some(&instance_id))
            .expect("diagnostics list")
        {
            assert!(
                !diagnostic.message.contains(secret),
                "diagnostic leaked secret: {}",
                diagnostic.message
            );
        }
    }

    #[test]
    fn agent_harness_skips_provider_launch_when_cancel_requested_before_launch() {
        struct CancelBeforeLaunchHarness {
            store_path: PathBuf,
        }

        impl AgentHarness for CancelBeforeLaunchHarness {
            fn before_launch(&self, request: &AgentTurnRequest) {
                let mut store = SqliteStore::open(&self.store_path).expect("store reopens");
                store
                    .request_effect_cancellation(EffectCancellationRequest {
                        instance_id: &request.instance_id,
                        effect_id: &request.effect_id,
                        revision_id: None,
                        reason: Some("test pre-launch cancellation"),
                        requested_by: "test",
                        causation_event_id: None,
                        idempotency_key: Some("test-pre-launch-cancel"),
                    })
                    .expect("cancellation request records");
            }

            fn run(&self, _request: AgentTurnRequest) -> ProviderRunResult {
                panic!("provider should not launch after pre-launch cancellation request");
            }
        }

        let store_path = std::env::temp_dir().join(format!(
            "whip-kernel-pre-launch-cancel-{}.sqlite",
            idempotency_key(&["pre-launch-cancel", "store"])
        ));
        let _ = fs::remove_file(&store_path);
        let store = SqliteStore::open(&store_path).expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "HarnessPreLaunchCancel",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[NewEffect {
                    timeout_seconds: None,
                    effect_id: "tell",
                    kind: "agent.tell",
                    target: Some("worker"),
                    input_json: r#"{"prompt":"go"}"#,
                    status: "queued",
                    idempotency_key: "rule=start;effect=tell",
                    required_capabilities_json: "[]",
                    profile: Some("repo-reader"),
                    correlation_id: None,
                    source_span_json: None,
                }],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        kernel
            .run_agent_turn(
                AgentTurnExecution {
                    instance_id: &instance_id,
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider: "mock",
                    worker_id: "worker-1",
                    lease_id: "lease-tell",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    agent: "worker",
                    profile: Some("repo-reader"),
                    input_json: r#"{"prompt":"go"}"#,
                    skill_names: &[],
                },
                &CancelBeforeLaunchHarness {
                    store_path: store_path.clone(),
                },
            )
            .expect("turn cancels before provider launch");

        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let effect = store
            .list_effects(&instance_id)
            .expect("effects list")
            .pop()
            .expect("effect exists");
        assert_eq!(effect.status, "cancelled");
        let requests = store
            .list_effect_cancellation_requests(&instance_id)
            .expect("cancellation requests list");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].status, "terminal");
        let events = store.list_events(&instance_id).expect("events list");
        assert_eq!(
            events.last().map(|event| event.event_type.as_str()),
            Some("effect.terminal")
        );
        assert!(!events
            .iter()
            .any(|event| event.event_type == "agent.turn.completed"));

        let _ = fs::remove_file(store_path);
    }

    #[test]
    fn failed_agent_harness_records_structured_failure_event_and_evidence() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "HarnessFailure",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"go"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell",
            required_capabilities_json: "[]",
            profile: Some("repo-reader"),
            correlation_id: None,
            source_span_json: Some(
                r#"{"path":"workflow.whip","start":10,"end":42,"construct":"effect"}"#,
            ),
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        kernel
            .run_agent_turn(
                AgentTurnExecution {
                    instance_id: &instance_id,
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider: "mock",
                    worker_id: "worker-1",
                    lease_id: "lease-tell",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    agent: "worker",
                    profile: Some("repo-reader"),
                    input_json: r#"{"prompt":"go"}"#,
                    skill_names: &[],
                },
                &MockAgentHarness::failed("fixture exploded"),
            )
            .expect("mock turn records failure");

        assert!(kernel.trace().iter().any(|record| matches!(
            &record.event,
            TraceEvent::ProviderDiagnostic {
                run_id,
                effect_id,
                provider,
                status,
                diagnostics_json,
                ..
            } if run_id == "run-tell"
                && effect_id == "tell"
                && provider == "mock"
                && *status == EffectStatus::Failed
                && diagnostics_json.contains("\"phase\":\"provider.fixture.failed\"")
        )));
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let events = store.list_events(&instance_id).expect("events list");
        assert!(events
            .iter()
            .any(|event| event.event_type == "effect.terminal"
                && event.payload_json.contains("\"status\":\"failed\"")
                && event
                    .payload_json
                    .contains("\"phase\":\"provider.fixture.failed\"")));
        assert!(events
            .iter()
            .any(|event| event.event_type == "agent.turn.failed"
                && event
                    .payload_json
                    .contains("\"error_kind\":\"fixture_failure\"")));
        let failed_turn: Value = events
            .iter()
            .find(|event| event.event_type == "agent.turn.failed")
            .map(|event| serde_json::from_str(&event.payload_json).expect("payload parses"))
            .expect("failed turn event exists");
        assert_eq!(
            failed_turn
                .pointer("/failure/provider")
                .and_then(Value::as_str),
            Some("mock")
        );
        assert_eq!(
            failed_turn
                .pointer("/failure/adapter")
                .and_then(Value::as_str),
            Some("mock")
        );
        assert_eq!(
            failed_turn
                .pointer("/failure/missing_config_keys")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );

        let evidence = store.list_evidence(&instance_id).expect("evidence lists");
        assert!(evidence
            .iter()
            .any(|evidence| evidence.kind == "agent.turn.provider"
                && evidence
                    .metadata_json
                    .contains("\"phase\":\"provider.fixture.failed\"")));

        let diagnostics = store
            .list_diagnostics(Some(&instance_id))
            .expect("diagnostics list");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code.as_deref(), Some("fixture_failure"));
        assert_eq!(diagnostics[0].subject_type.as_deref(), Some("effect"));
        assert_eq!(diagnostics[0].subject_id.as_deref(), Some("tell"));
        assert_eq!(diagnostics[0].effect_id.as_deref(), Some("tell"));
        assert_eq!(diagnostics[0].run_id.as_deref(), Some("run-tell"));
        assert!(diagnostics[0].event_id.is_some());
        assert!(diagnostics[0].message.contains("fixture exploded"));
        assert_ne!(diagnostics[0].evidence_ids_json, "[]");
        assert_eq!(
            diagnostics[0].source_span_json.as_deref(),
            Some(r#"{"construct":"effect","end":42,"path":"workflow.whip","start":10}"#)
        );
        let replayed = store
            .list_diagnostics_from_events(&instance_id)
            .expect("event diagnostics replay");
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].code.as_deref(), Some("fixture_failure"));
        assert_eq!(
            replayed[0].source_span_json.as_deref(),
            diagnostics[0].source_span_json.as_deref()
        );

        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(facts.iter().any(|fact| fact.name == "agent.turn.failed"
            && fact.value_json.contains("\"recoverable\":false")));
    }

    #[test]
    fn required_artifact_capture_failure_prevents_successful_terminal_completion() {
        struct CompletedWithArtifactFailureHarness;

        impl AgentHarness for CompletedWithArtifactFailureHarness {
            fn run(&self, _request: AgentTurnRequest) -> ProviderRunResult {
                ProviderRunResult {
                    status: ProviderRunStatus::Completed,
                    summary: "provider completed before required artifact capture failed"
                        .to_owned(),
                    stdout: String::new(),
                    stderr: "artifact capture stderr".to_owned(),
                    transcript: "artifact capture transcript".to_owned(),
                    exit_code: Some(0),
                    usage_json: "{}".to_owned(),
                    artifacts: Vec::new(),
                    failure: Some(
                        ProviderFailure::new(
                            artifact_manifest::ARTIFACT_CAPTURE_FAILED_EVENT,
                            "missing",
                            "required artifact was missing",
                        )
                        .provider("codex")
                        .adapter("app_server"),
                    ),
                }
            }
        }

        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "RequiredArtifactFailure",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[NewEffect {
                    timeout_seconds: None,
                    effect_id: "tell",
                    kind: "agent.tell",
                    target: Some("worker"),
                    input_json: r#"{"prompt":"go"}"#,
                    status: "queued",
                    idempotency_key: "rule=start;effect=tell",
                    required_capabilities_json: "[]",
                    profile: Some("repo-writer"),
                    correlation_id: None,
                    source_span_json: None,
                }],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start-artifact-failure"),
            })
            .expect("rule commits");

        kernel
            .run_agent_turn(
                AgentTurnExecution {
                    instance_id: &instance_id,
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider: "codex",
                    worker_id: "worker-1",
                    lease_id: "lease-tell",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    agent: "worker",
                    profile: Some("repo-writer"),
                    input_json: r#"{"prompt":"go"}"#,
                    skill_names: &[],
                },
                &CompletedWithArtifactFailureHarness,
            )
            .expect("turn records artifact capture failure");

        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let effect = store
            .list_effects(&instance_id)
            .expect("effects list")
            .pop()
            .expect("effect exists");
        assert_eq!(effect.status, "failed");
        let events = store.list_events(&instance_id).expect("events list");
        assert!(events
            .iter()
            .any(|event| event.event_type == "effect.terminal"
                && event.payload_json.contains("\"status\":\"failed\"")
                && event
                    .payload_json
                    .contains("\"phase\":\"artifact.capture.failed\"")));
        assert!(events
            .iter()
            .any(|event| event.event_type == "agent.turn.failed"
                && event.payload_json.contains("\"error_kind\":\"missing\"")));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "agent.turn.completed"));
    }

    #[test]
    fn recovery_appends_terminal_from_provider_evidence_once_after_append_gap() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "ProviderTerminalRecovery",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[NewEffect {
                    timeout_seconds: None,
                    effect_id: "tell",
                    kind: "agent.tell",
                    target: Some("worker"),
                    input_json: r#"{"prompt":"go"}"#,
                    status: "queued",
                    idempotency_key: "rule=start;effect=tell",
                    required_capabilities_json: "[]",
                    profile: Some("repo-writer"),
                    correlation_id: None,
                    source_span_json: None,
                }],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start-recovery"),
            })
            .expect("rule commits");
        kernel
            .start_run(RunStart {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "codex",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        let provider_metadata = json!({
            "effect_id": "tell",
            "agent": "worker",
            "profile": "repo-writer",
            "status": "completed",
            "stdout": {"redacted": true, "bytes": 0, "chars": 0},
            "stderr": {"redacted": true, "bytes": 0, "chars": 0},
            "transcript": {"redacted": true, "bytes": 0, "chars": 0},
            "exit_code": 0,
            "usage": {},
            "artifact_ids": [],
            "failure": null,
        })
        .to_string();
        let evidence_id = kernel
            .store
            .record_evidence(EvidenceRecord {
                instance_id: &instance_id,
                kind: "agent.turn.provider",
                subject_type: "run",
                subject_id: "run-tell",
                causation_id: Some("tell"),
                correlation_id: Some("provider-terminal-gap"),
                summary: Some("provider completed before terminal append gap"),
                metadata_json: &provider_metadata,
            })
            .expect("provider evidence records");

        let recovered = kernel
            .recover_provider_terminal_from_evidence(AgentTurnExecution {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "codex",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                agent: "worker",
                profile: Some("repo-writer"),
                input_json: r#"{"prompt":"go"}"#,
                skill_names: &[],
            })
            .expect("terminal recovery succeeds");
        assert!(recovered.is_some());
        let duplicate = kernel
            .recover_provider_terminal_from_evidence(AgentTurnExecution {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "codex",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                agent: "worker",
                profile: Some("repo-writer"),
                input_json: r#"{"prompt":"go"}"#,
                skill_names: &[],
            })
            .expect("duplicate recovery is idempotent");
        assert!(duplicate.is_none());

        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let effects = store.list_effects(&instance_id).expect("effects list");
        assert_eq!(effects[0].status, "completed");
        let events = store.list_events(&instance_id).expect("events list");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "effect.terminal"
                    && event.payload_json.contains("\"run_id\":\"run-tell\""))
                .count(),
            1
        );
        let terminal = events
            .iter()
            .find(|event| event.event_type == "effect.terminal")
            .expect("terminal event exists");
        assert!(terminal
            .payload_json
            .contains("\"recovery\":\"provider_evidence_terminal\""));
        assert!(terminal.payload_json.contains(&evidence_id));
        let terminal_payload =
            serde_json::from_str::<Value>(&terminal.payload_json).expect("terminal payload");
        assert_eq!(
            terminal_payload
                .pointer("/metadata/provider_correlation_id")
                .and_then(Value::as_str),
            Some("provider-terminal-gap")
        );
        assert!(terminal_payload
            .pointer("/metadata/terminal_payload_hash")
            .and_then(Value::as_str)
            .is_some());
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "agent.turn.completed")
                .count(),
            1
        );
    }

    fn start_running_agent_turn_without_evidence() -> (RuntimeKernel, String) {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "UncertainRecovery",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[NewEffect {
                    timeout_seconds: None,
                    effect_id: "tell",
                    kind: "agent.tell",
                    target: Some("worker"),
                    input_json: r#"{"prompt":"go"}"#,
                    status: "queued",
                    idempotency_key: "rule=start;effect=tell",
                    required_capabilities_json: "[]",
                    profile: Some("repo-writer"),
                    correlation_id: None,
                    source_span_json: None,
                }],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start-uncertain"),
            })
            .expect("rule commits");
        kernel
            .start_run(RunStart {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "codex",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        (kernel, instance_id)
    }

    #[test]
    fn recovery_resolves_running_run_without_evidence_to_uncertain() {
        let (mut kernel, instance_id) = start_running_agent_turn_without_evidence();

        let recovered = kernel
            .recover_running_provider_runs(&instance_id)
            .expect("recovery sweep succeeds");
        assert_eq!(
            recovered.len(),
            1,
            "the started run resolves to one terminal"
        );

        check_trace(kernel.trace()).expect("kernel trace conforms");

        let store = kernel.into_store();
        // The effect is a Failed subkind...
        let effects = store.list_effects(&instance_id).expect("effects list");
        assert_eq!(effects[0].status, "failed");
        // ...but the run records the distinct `uncertain` status (TLA+ ResolveUncertainRun).
        let runs = store.list_runs(&instance_id).expect("runs list");
        let run = runs.iter().find(|r| r.run_id == "run-tell").expect("run");
        assert_eq!(run.status, "uncertain");
        // Exactly one terminal for the run, marked as an uncertain recovery.
        let events = store.list_events(&instance_id).expect("events list");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "effect.terminal"
                    && event.payload_json.contains("\"run_id\":\"run-tell\""))
                .count(),
            1
        );
        let terminal = events
            .iter()
            .find(|event| event.event_type == "effect.terminal")
            .expect("terminal event exists");
        assert!(terminal.payload_json.contains("\"recovery\":\"uncertain\""));
        // No success was fabricated.
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "agent.turn.completed")
                .count(),
            0
        );
    }

    #[test]
    fn recovery_resolving_uncertain_run_is_idempotent_on_rerun() {
        let (mut kernel, instance_id) = start_running_agent_turn_without_evidence();

        let first = kernel
            .recover_running_provider_runs(&instance_id)
            .expect("first recovery sweep succeeds");
        assert_eq!(first.len(), 1);
        // The run is now `uncertain` (terminal), so a re-run is a clean no-op,
        // not a swallowed unique-index error.
        let second = kernel
            .recover_running_provider_runs(&instance_id)
            .expect("rerun is idempotent");
        assert!(second.is_empty());

        let store = kernel.into_store();
        let events = store.list_events(&instance_id).expect("events list");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "effect.terminal"
                    && event.payload_json.contains("\"run_id\":\"run-tell\""))
                .count(),
            1,
            "rerun must not double-admit a terminal"
        );
    }

    #[test]
    fn uncertain_terminal_idempotency_key_is_deterministic() {
        let correlation_a = provider_terminal_correlation_id("inst", "run-tell");
        let correlation_b = provider_terminal_correlation_id("inst", "run-tell");
        assert_eq!(correlation_a, correlation_b);
        let hash_a = terminal_payload_hash("failed", None, Some("uncertain"), "{}");
        let hash_b = terminal_payload_hash("failed", None, Some("uncertain"), "{}");
        assert_eq!(hash_a, hash_b);
        let key_a =
            terminal_completion_idempotency_key("inst", "run-tell", &correlation_a, &hash_a);
        let key_b =
            terminal_completion_idempotency_key("inst", "run-tell", &correlation_b, &hash_b);
        assert_eq!(
            key_a, key_b,
            "uncertain terminal idempotency key must be deterministic"
        );
    }

    #[test]
    fn recovery_preserves_artifact_evidence_after_capture_before_terminal_gap() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "ArtifactRecovery",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[NewEffect {
                    timeout_seconds: None,
                    effect_id: "tell",
                    kind: "agent.tell",
                    target: Some("worker"),
                    input_json: r#"{"prompt":"go"}"#,
                    status: "queued",
                    idempotency_key: "rule=start;effect=tell",
                    required_capabilities_json: "[]",
                    profile: Some("repo-writer"),
                    correlation_id: None,
                    source_span_json: None,
                }],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start-artifact-recovery"),
            })
            .expect("rule commits");
        kernel
            .start_run(RunStart {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "codex",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        let artifact_id = kernel
            .store
            .record_artifact(ArtifactRecord {
                run_id: "run-tell",
                kind: "transcript_ref",
                path: "provider://codex/runs/run-tell/transcript_ref",
                content_hash: None,
                mime_type: Some("text/plain"),
            })
            .expect("artifact records before terminal gap");
        let provider_metadata = json!({
            "effect_id": "tell",
            "agent": "worker",
            "profile": "repo-writer",
            "status": "completed",
            "stdout": {"redacted": true, "bytes": 0, "chars": 0},
            "stderr": {"redacted": true, "bytes": 0, "chars": 0},
            "transcript": {"redacted": true, "bytes": 0, "chars": 0},
            "exit_code": 0,
            "usage": {},
            "artifact_ids": [artifact_id],
            "artifact_manifest": {
                "schema_version": artifact_manifest::ARTIFACT_MANIFEST_SCHEMA_VERSION,
                "entry_count": 1,
                "entries": [{
                    "artifact_id": artifact_id,
                    "kind": "transcript_ref",
                    "uri": {
                        "type": "ref",
                        "value": "provider://codex/runs/run-tell/transcript_ref"
                    },
                    "content_hash": null,
                    "mime_type": "text/plain",
                    "size_bytes": null,
                    "redaction_status": "unredacted_metadata_only",
                    "retention_policy": "provider_default",
                    "required": false,
                    "source_provider_event": null
                }]
            },
            "failure": null,
        })
        .to_string();
        kernel
            .store
            .record_evidence(EvidenceRecord {
                instance_id: &instance_id,
                kind: "agent.turn.provider",
                subject_type: "run",
                subject_id: "run-tell",
                causation_id: Some("tell"),
                correlation_id: Some("provider-artifact-terminal-gap"),
                summary: Some("provider completed with artifact before terminal append gap"),
                metadata_json: &provider_metadata,
            })
            .expect("provider evidence records");

        let recovered = kernel
            .recover_provider_terminal_from_evidence(AgentTurnExecution {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "codex",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                agent: "worker",
                profile: Some("repo-writer"),
                input_json: r#"{"prompt":"go"}"#,
                skill_names: &[],
            })
            .expect("terminal recovery succeeds");
        assert!(recovered.is_some());

        let store = kernel.into_store();
        let artifacts = store
            .list_artifacts_for_run("run-tell")
            .expect("artifacts list");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact_id, artifact_id);
        let terminal = store
            .list_events(&instance_id)
            .expect("events list")
            .into_iter()
            .find(|event| event.event_type == "effect.terminal")
            .expect("terminal exists");
        assert!(terminal.payload_json.contains(&artifact_id));
    }

    #[test]
    fn restart_reconciles_running_provider_run_with_persisted_terminal_evidence() {
        let store_path = std::env::temp_dir().join(format!(
            "whip-kernel-provider-recovery-{}.sqlite",
            idempotency_key(&["provider-recovery", "store"])
        ));
        let _ = fs::remove_file(&store_path);
        let store = SqliteStore::open(&store_path).expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "RestartProviderRecovery",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[NewEffect {
                    timeout_seconds: None,
                    effect_id: "tell",
                    kind: "agent.tell",
                    target: Some("worker"),
                    input_json: r#"{"prompt":"go"}"#,
                    status: "queued",
                    idempotency_key: "rule=start;effect=tell",
                    required_capabilities_json: "[]",
                    profile: Some("repo-writer"),
                    correlation_id: None,
                    source_span_json: None,
                }],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start-restart-provider-recovery"),
            })
            .expect("rule commits");
        kernel
            .start_run(RunStart {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "codex",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        let provider_metadata = json!({
            "effect_id": "tell",
            "agent": "worker",
            "profile": "repo-writer",
            "status": "completed",
            "stdout": {"redacted": true, "bytes": 0, "chars": 0},
            "stderr": {"redacted": true, "bytes": 0, "chars": 0},
            "transcript": {"redacted": true, "bytes": 0, "chars": 0},
            "exit_code": 0,
            "usage": {},
            "artifact_ids": [],
            "failure": null,
        })
        .to_string();
        kernel
            .store
            .record_evidence(EvidenceRecord {
                instance_id: &instance_id,
                kind: "agent.turn.provider",
                subject_type: "run",
                subject_id: "run-tell",
                causation_id: Some("tell"),
                correlation_id: Some("provider-restart-terminal-gap"),
                summary: Some("provider completed before worker restart"),
                metadata_json: &provider_metadata,
            })
            .expect("provider evidence records");
        drop(kernel);

        let store = SqliteStore::open(&store_path).expect("store reopens after restart");
        let mut restarted = RuntimeKernel::new(store);
        let recovered = restarted
            .recover_running_provider_runs(&instance_id)
            .expect("running provider runs recover");
        assert_eq!(recovered.len(), 1);
        let duplicate = restarted
            .recover_running_provider_runs(&instance_id)
            .expect("duplicate restart recovery is idempotent");
        assert!(duplicate.is_empty());

        let store = restarted.into_store();
        let effects = store.list_effects(&instance_id).expect("effects list");
        assert_eq!(effects[0].status, "completed");
        let events = store.list_events(&instance_id).expect("events list");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "effect.terminal"
                    && event.payload_json.contains("\"run_id\":\"run-tell\""))
                .count(),
            1
        );
        assert!(events
            .iter()
            .any(|event| event.event_type == "agent.turn.completed"));

        let _ = fs::remove_file(store_path);
    }

    #[test]
    fn restart_recovers_running_pi_native_run_from_terminal_evidence() {
        let store_path = std::env::temp_dir().join(format!(
            "whip-kernel-pi-native-recovery-{}.sqlite",
            idempotency_key(&["pi-native-provider-recovery", "store"])
        ));
        let _ = fs::remove_file(&store_path);
        let store = SqliteStore::open(&store_path).expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "RestartPiNativeRecovery",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &[NewEffect {
                    timeout_seconds: None,
                    effect_id: "tell",
                    kind: "agent.tell",
                    target: Some("pi"),
                    input_json: r#"{"prompt":"go"}"#,
                    status: "queued",
                    idempotency_key: "rule=start;effect=tell",
                    required_capabilities_json: "[]",
                    profile: Some("repo-reader"),
                    correlation_id: None,
                    source_span_json: None,
                }],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start-pi-native-recovery"),
            })
            .expect("rule commits");
        kernel
            .start_run(RunStart {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "pi",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("run starts");
        let native_metadata = json!({
            "effect_id": "tell",
            "agent": "pi",
            "profile": "repo-reader",
            "provider": "pi",
            "native_event": {
                "provider_id": "pi",
                "run_id": "run-tell",
                "event_kind": "completed",
                "terminal": true,
                "provider_event_type": "turn_end",
                "provider_session_id": "pi-session-1",
                "provider_turn_id": "pi-turn-1",
                "sequence": 4,
                "evidence_shape": {"type": "object", "keys": 2},
                "artifacts": []
            },
            "artifact_ids": []
        })
        .to_string();
        kernel
            .store
            .record_evidence(EvidenceRecord {
                instance_id: &instance_id,
                kind: "agent.turn.native_provider",
                subject_type: "run",
                subject_id: "run-tell",
                causation_id: Some("tell"),
                correlation_id: Some("native-pi-terminal-gap"),
                summary: Some("completed native provider event from turn_end"),
                metadata_json: &native_metadata,
            })
            .expect("native provider evidence records");
        drop(kernel);

        let store = SqliteStore::open(&store_path).expect("store reopens after restart");
        let mut restarted = RuntimeKernel::new(store);
        let recovered = restarted
            .recover_running_provider_runs(&instance_id)
            .expect("running native provider runs recover");
        assert_eq!(recovered.len(), 1);
        let duplicate = restarted
            .recover_running_provider_runs(&instance_id)
            .expect("duplicate native provider recovery is idempotent");
        assert!(duplicate.is_empty());

        let store = restarted.into_store();
        let effects = store.list_effects(&instance_id).expect("effects list");
        assert_eq!(effects[0].status, "completed");
        let events = store.list_events(&instance_id).expect("events list");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "effect.terminal"
                    && event.payload_json.contains("\"run_id\":\"run-tell\""))
                .count(),
            1
        );
        let terminal = events
            .iter()
            .find(|event| event.event_type == "effect.terminal")
            .expect("terminal event exists");
        assert!(terminal
            .payload_json
            .contains("\"provider_event_type\":\"turn_end\""));
        assert!(terminal.payload_json.contains("pi-session-1"));

        let _ = fs::remove_file(store_path);
    }

    #[test]
    fn real_codex_failure_reaches_event_stream() {
        if !command_exists("codex") {
            eprintln!("skipping real Codex failure smoke: codex not found on PATH");
            return;
        }
        let harness = CodexAgentHarness::new(
            CommandLaunchPlan::new("codex", "codex")
                .arg("app-server")
                .arg("--listen")
                .arg("not-a-url://"),
        );
        assert_real_provider_failure_reaches_event_stream(
            "RealCodexFailure",
            "codex",
            &harness,
            "unsupported --listen URL",
        );
    }

    #[test]
    fn real_claude_failure_reaches_event_stream() {
        if !command_exists("claude") {
            eprintln!("skipping real Claude failure smoke: claude not found on PATH");
            return;
        }
        let harness = ClaudeCodeAgentHarness::new(
            CommandLaunchPlan::new("claude", "claude").arg("--definitely-not-a-real-flag"),
        );
        assert_real_provider_failure_reaches_event_stream(
            "RealClaudeFailure",
            "claude",
            &harness,
            "unknown option",
        );
    }

    #[test]
    fn real_pi_failure_reaches_event_stream() {
        if !command_exists("pi") {
            eprintln!("skipping real Pi failure smoke: pi not found on PATH");
            return;
        }
        let harness = PiStyleAgentHarness::new(
            CommandLaunchPlan::new("pi", "pi").arg("--definitely-not-a-real-flag"),
        );
        assert_real_provider_failure_reaches_event_stream(
            "RealPiFailure",
            "pi",
            &harness,
            "Unknown option",
        );
    }

    #[test]
    fn fake_coerce_records_evidence_and_success_fact() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "Coerce",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "review",
            kind: "coerce",
            target: None,
            input_json: r#"{"function_name":"reviewWork"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=review",
            required_capabilities_json: r#"["coerce"]"#,
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let request = CoerceRequest {
            function_name: "reviewWork".to_owned(),
            arguments_json: r#"{"summary":"done"}"#.to_owned(),
            output_type: "WorkReview".to_owned(),
            generated_coerce_source_hash: "coerce-source".to_owned(),
            input_schema_hash: "input-schema".to_owned(),
            output_schema_hash: "output-schema".to_owned(),
        };
        let terminal = kernel
            .run_coerce(
                CoerceExecution {
                    instance_id: &instance_id,
                    effect_id: "review",
                    run_id: "run-review",
                    provider: "fake-coerce",
                    worker_id: "worker-1",
                    lease_id: "lease-review",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    request: &request,
                },
                &FakeCoerceClient::succeeds(r#"{"status":"Accept","reason":"ok"}"#),
            )
            .expect("coerce runs");

        assert_eq!(terminal.sequence, 3);
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let evidence = store.list_evidence(&instance_id).expect("evidence lists");
        assert_eq!(evidence.len(), 2);
        let coerce_evidence = evidence
            .iter()
            .find(|evidence| evidence.kind == "coerce.provider")
            .expect("coerce provider evidence recorded");
        assert!(coerce_evidence.metadata_json.contains("reviewWork"));
        assert!(coerce_evidence.metadata_json.contains("coerce-source"));
        let coerce_metadata = serde_json::from_str::<Value>(&coerce_evidence.metadata_json)
            .expect("coerce metadata json");
        assert_eq!(
            coerce_metadata
                .pointer("/arguments/redacted")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            coerce_metadata
                .pointer("/arguments/shape/type")
                .and_then(Value::as_str),
            Some("object")
        );
        assert!(!coerce_evidence
            .metadata_json
            .contains(r#""summary":"done""#));
        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(facts.iter().any(|fact| fact.name == "coerce.succeeded"
            && fact.value_json.contains("\"status\":\"Accept\"")));
    }

    #[test]
    fn fake_coerce_failure_records_failed_fact() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "CoerceFail",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "review",
            kind: "coerce",
            target: None,
            input_json: r#"{"function_name":"reviewWork"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=review",
            required_capabilities_json: r#"["coerce"]"#,
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");
        let request = CoerceRequest {
            function_name: "reviewWork".to_owned(),
            arguments_json: r#"{"summary":"done"}"#.to_owned(),
            output_type: "WorkReview".to_owned(),
            generated_coerce_source_hash: "coerce-source".to_owned(),
            input_schema_hash: "input-schema".to_owned(),
            output_schema_hash: "output-schema".to_owned(),
        };

        kernel
            .run_coerce(
                CoerceExecution {
                    instance_id: &instance_id,
                    effect_id: "review",
                    run_id: "run-review",
                    provider: "fake-coerce",
                    worker_id: "worker-1",
                    lease_id: "lease-review",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    request: &request,
                },
                &FakeCoerceClient::fails("invalid output"),
            )
            .expect("failed coerce records terminal event");

        assert!(kernel.trace().iter().any(|record| matches!(
            &record.event,
            TraceEvent::ProviderDiagnostic {
                run_id,
                effect_id,
                provider,
                status,
                diagnostics_json,
                ..
            } if run_id == "run-review"
                && effect_id == "review"
                && provider == "fake-coerce"
                && *status == EffectStatus::Failed
                && diagnostics_json.contains("\"redacted\":true")
                && !diagnostics_json.contains("invalid output")
        )));
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let diagnostics = store
            .list_diagnostics(Some(&instance_id))
            .expect("diagnostics list");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code.as_deref(), Some("coerce.failed"));
        assert_eq!(diagnostics[0].subject_id.as_deref(), Some("review"));
        assert_eq!(diagnostics[0].run_id.as_deref(), Some("run-review"));
        assert!(!diagnostics[0].message.contains("invalid output"));

        let facts = store.list_facts(&instance_id).expect("facts list");
        let failed_fact = facts
            .iter()
            .find(|fact| fact.name == "coerce.failed")
            .expect("failed coerce fact");
        assert!(failed_fact.value_json.contains("\"redacted\":true"));
        assert!(!failed_fact.value_json.contains("invalid output"));
    }

    #[test]
    fn fake_loft_claim_records_evidence_and_success_fact() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "LoftClaim",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "claim",
            kind: "loft.claim",
            target: None,
            input_json: r#"{"issue_id":"iss_abc"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=claim",
            required_capabilities_json: r#"["loft.claim"]"#,
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let request = LoftEffectRequest {
            action: LoftAction::Claim,
            issue_id: "iss_abc".to_owned(),
            lease_id: None,
            claim_ready: false,
            issue_version: None,
            actor: Some("agent-a".to_owned()),
            lease_duration_seconds: Some(1800),
            command_id: "cmd-claim".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: vec!["evt_head".to_owned()],
            metadata_json: "{}".to_owned(),
        };
        let terminal = kernel
            .run_loft_effect(
                LoftEffectExecution {
                    instance_id: &instance_id,
                    effect_id: "claim",
                    run_id: "run-claim",
                    provider: "fake-loft",
                    worker_id: "worker-1",
                    lease_id: "lease-whipplescript-claim",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    request: &request,
                },
                &FakeLoftClient::succeeds(
                    r#"{"lease_id":"lea_abc","issue":{"id":"iss_abc","state_token":"b3:ok"},"expires_at":"2030-01-01T00:00:00Z"}"#,
                ),
            )
            .expect("loft claim runs");

        assert_eq!(terminal.sequence, 3);
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let evidence = store.list_evidence(&instance_id).expect("evidence lists");
        assert_eq!(evidence.len(), 2);
        assert!(evidence
            .iter()
            .any(|evidence| evidence.kind == "loft.claim.provider"
                && evidence.metadata_json.contains("cmd-claim")
                && evidence.metadata_json.contains("evt_head")));
        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(
            facts
                .iter()
                .any(|fact| fact.name == "loft.claim.succeeded"
                    && fact.value_json.contains("lea_abc"))
        );
    }

    #[test]
    fn fake_loft_claim_failure_records_failed_fact() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "LoftClaimFail",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "claim",
            kind: "loft.claim",
            target: None,
            input_json: r#"{"issue_id":"iss_abc"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=claim",
            required_capabilities_json: r#"["loft.claim"]"#,
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let request = LoftEffectRequest {
            action: LoftAction::Claim,
            issue_id: "iss_abc".to_owned(),
            lease_id: None,
            claim_ready: false,
            issue_version: None,
            actor: Some("agent-a".to_owned()),
            lease_duration_seconds: None,
            command_id: "cmd-claim".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        kernel
            .run_loft_effect(
                LoftEffectExecution {
                    instance_id: &instance_id,
                    effect_id: "claim",
                    run_id: "run-claim",
                    provider: "fake-loft",
                    worker_id: "worker-1",
                    lease_id: "lease-whipplescript-claim",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    request: &request,
                },
                &FakeLoftClient::fails("issue already leased"),
            )
            .expect("failed claim records terminal event");

        assert!(kernel.trace().iter().any(|record| matches!(
            &record.event,
            TraceEvent::ProviderDiagnostic {
                run_id,
                effect_id,
                provider,
                status,
                diagnostics_json,
                ..
            } if run_id == "run-claim"
                && effect_id == "claim"
                && provider == "fake-loft"
                && *status == EffectStatus::Failed
                && diagnostics_json.contains("\"redacted\":true")
                && !diagnostics_json.contains("issue already leased")
        )));
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let diagnostics = store
            .list_diagnostics(Some(&instance_id))
            .expect("diagnostics list");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code.as_deref(), Some("loft.claim.failed"));
        assert_eq!(diagnostics[0].subject_id.as_deref(), Some("claim"));
        assert_eq!(diagnostics[0].run_id.as_deref(), Some("run-claim"));
        assert!(!diagnostics[0].message.contains("issue already leased"));

        let facts = store.list_facts(&instance_id).expect("facts list");
        let failed_fact = facts
            .iter()
            .find(|fact| fact.name == "loft.claim.failed")
            .expect("failed loft fact");
        assert!(failed_fact.value_json.contains("\"redacted\":true"));
        assert!(!failed_fact.value_json.contains("issue already leased"));
    }

    #[test]
    fn fake_loft_renew_records_evidence_and_success_fact() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "LoftRenew",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "renew",
            kind: "loft.renew",
            target: None,
            input_json: r#"{"lease_id":"lea_abc"}"#,
            status: "queued",
            idempotency_key: "rule=maintain;effect=renew",
            required_capabilities_json: r#"["loft.renew"]"#,
            profile: Some("repo-writer"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "maintain",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-maintain"),
            })
            .expect("rule commits");

        let request = LoftEffectRequest {
            action: LoftAction::Renew,
            issue_id: "iss_abc".to_owned(),
            lease_id: Some("lea_abc".to_owned()),
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: Some(1800),
            command_id: "cmd-renew".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        let terminal = kernel
            .run_loft_effect(
                LoftEffectExecution {
                    instance_id: &instance_id,
                    effect_id: "renew",
                    run_id: "run-renew",
                    provider: "fake-loft",
                    worker_id: "worker-1",
                    lease_id: "lease-whipplescript-renew",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    request: &request,
                },
                &FakeLoftClient::succeeds(
                    r#"{"lease_id":"lea_abc","expires_at":"2030-01-01T00:30:00Z"}"#,
                ),
            )
            .expect("loft renew runs");

        assert_eq!(terminal.sequence, 3);
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let evidence = store.list_evidence(&instance_id).expect("evidence lists");
        assert!(evidence
            .iter()
            .any(|evidence| evidence.kind == "loft.renew.provider"
                && evidence.metadata_json.contains("cmd-renew")));
        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(
            facts
                .iter()
                .any(|fact| fact.name == "loft.renew.succeeded"
                    && fact.value_json.contains("lea_abc"))
        );
    }

    #[test]
    fn human_ask_creates_inbox_item_and_pending_fact() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name: "HumanReview",
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "ask",
            kind: "human.ask",
            target: None,
            input_json: r#"{"prompt":"Approve?"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=ask",
            required_capabilities_json: r#"["human.ask"]"#,
            profile: Some("human-review"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        let terminal = kernel
            .run_human_ask(HumanAskExecution {
                instance_id: &instance_id,
                effect_id: "ask",
                run_id: "run-ask",
                provider: "builtin-human-review",
                worker_id: "worker-1",
                lease_id: "lease-ask",
                lease_expires_at: "2030-01-01T00:00:00Z",
                inbox_item_id: "inbox-ask",
                prompt: "Approve this change?",
                choices_json: r#"["approve","reject"]"#,
                freeform_allowed: true,
                severity: "normal",
                related_effects_json: r#"["ask"]"#,
                related_artifacts_json: "[]",
            })
            .expect("human ask runs");

        assert_eq!(terminal.sequence, 3);
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let inbox = store
            .list_inbox_items(Some("pending"))
            .expect("inbox lists");
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].inbox_item_id, "inbox-ask");
        assert_eq!(inbox[0].choices_json, r#"["approve","reject"]"#);
        let evidence = store.list_evidence(&instance_id).expect("evidence lists");
        assert!(evidence
            .iter()
            .any(|evidence| evidence.kind == "human.ask.provider"
                && evidence.metadata_json.contains("inbox-ask")));
        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(facts
            .iter()
            .any(|fact| fact.name == "human.ask.created" && fact.value_json.contains("inbox-ask")));
    }

    fn command_exists(binary: &str) -> bool {
        Command::new(binary).arg("--version").output().is_ok()
    }

    fn assert_real_provider_failure_reaches_event_stream(
        program_name: &str,
        provider: &str,
        harness: &dyn AgentHarness,
        expected_stderr: &str,
    ) {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version(ProgramVersionInput {
                program_name,
                source_hash: "source",
                ir_hash: "ir",
                compiler_version: "test",
            })
            .expect("program version creates");
        let instance_id = kernel
            .create_instance(&version, "{}")
            .expect("instance creates");
        let effects = [NewEffect {
            timeout_seconds: None,
            effect_id: "tell",
            kind: "agent.tell",
            target: Some("worker"),
            input_json: r#"{"prompt":"this should fail before a model turn"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=tell",
            required_capabilities_json: "[]",
            profile: Some("repo-reader"),
            correlation_id: None,
            source_span_json: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &[],
                terminal: None,
                idempotency_key: Some("commit-start"),
            })
            .expect("rule commits");

        kernel
            .run_agent_turn(
                AgentTurnExecution {
                    instance_id: &instance_id,
                    effect_id: "tell",
                    run_id: "run-tell",
                    provider,
                    worker_id: "worker-1",
                    lease_id: "lease-tell",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    agent: "worker",
                    profile: Some("repo-reader"),
                    input_json: r#"{"prompt":"this should fail before a model turn"}"#,
                    skill_names: &[],
                },
                harness,
            )
            .expect("real provider failure records terminal event");

        let store = kernel.into_store();
        let events = store.list_events(&instance_id).expect("events list");
        let terminal_event = events
            .iter()
            .find(|event| event.event_type == "effect.terminal")
            .expect("failed terminal event");
        let terminal_payload = serde_json::from_str::<Value>(&terminal_event.payload_json)
            .expect("terminal payload json");
        assert_eq!(
            terminal_payload.get("status").and_then(Value::as_str),
            Some("failed")
        );
        assert_eq!(
            terminal_payload
                .pointer("/metadata/failure/phase")
                .and_then(Value::as_str),
            Some("provider.exit.failed")
        );
        assert_eq!(
            terminal_payload
                .pointer("/metadata/failure/error_kind")
                .and_then(Value::as_str),
            Some("nonzero_exit")
        );
        assert_eq!(
            terminal_payload
                .pointer("/metadata/stderr/redacted")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(!terminal_event.payload_json.contains(expected_stderr));
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "agent.turn.failed"
                    && event
                        .payload_json
                        .contains("\"phase\":\"provider.exit.failed\"")
                    && event.payload_json.contains("\"recoverable\":true")),
            "missing failed agent turn event for {provider}: {events:#?}"
        );

        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(facts.iter().any(|fact| fact.name == "agent.turn.failed"
            && fact.value_json.contains("\"error_kind\":\"nonzero_exit\"")));

        let evidence = store.list_evidence(&instance_id).expect("evidence lists");
        let provider_evidence = evidence
            .iter()
            .find(|evidence| evidence.kind == "agent.turn.provider")
            .expect("provider evidence recorded");
        let provider_metadata = serde_json::from_str::<Value>(&provider_evidence.metadata_json)
            .expect("provider metadata json");
        assert_eq!(
            provider_metadata
                .pointer("/failure/phase")
                .and_then(Value::as_str),
            Some("provider.exit.failed")
        );
        assert_eq!(
            provider_metadata
                .pointer("/stderr/redacted")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(!provider_evidence.metadata_json.contains(expected_stderr));
        let diagnostics = store
            .list_diagnostics(Some(&instance_id))
            .expect("diagnostics list");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_deref() == Some("nonzero_exit")
                && diagnostic.message.contains(provider)
                && !diagnostic.message.contains(expected_stderr)
                && diagnostic.event_id.is_some()
                && diagnostic.run_id.as_deref() == Some("run-tell")
        }));
    }
}
