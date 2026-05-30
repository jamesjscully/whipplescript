//! Deterministic runtime kernel scaffold.

pub mod coerce;
pub mod harness;
pub mod loft;
pub mod trace;

use coerce::{BamlClient, BamlCoerceRequest, BamlCoerceResult, BamlCoerceStatus};
use harness::{
    AgentHarness, AgentTurnRequest, ProviderFailure, ProviderRunResult, ProviderRunStatus,
};
use loft::{LoftAction, LoftClient, LoftEffectRequest, LoftEffectResult, LoftEffectStatus};
use serde_json::{json, Value};
use trace::{DependencyEdge, EffectStatus, TraceEvent, TraceRecord};
use whipplescript_parser::{
    DependencyPredicate, IrEffectKind, IrPrimitiveType, IrProgram, IrSchema, IrType,
    IrWorkflowContractKind, SourceSpan,
};
use whipplescript_store::{
    ArtifactRecord, ClaimableEffect, DerivedFact, EffectCancellation, EffectCompletion,
    EvidenceRecord, ExpiredLease, InstanceTransition, LeaseRenewal, NewEffectDependency, NewEvent,
    NewFact, NewInboxItem, NewInstance, NewProgramVersion, NewWorkflowInvocation,
    ProgramVersionRecord, RetryEffect, RevisionActivation, RuleCommit, RuleCommitRevisionGuard,
    RunStart, SkillEvidence, SqliteStore, StoreError, StoreResult, StoredEvent,
    TerminalDiagnosticRecord, WorkflowInvocationView, WorkflowRevisionView,
};

pub struct RuntimeKernel {
    store: SqliteStore,
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
pub struct BamlCoerceExecution<'a> {
    pub instance_id: &'a str,
    pub effect_id: &'a str,
    pub run_id: &'a str,
    pub provider: &'a str,
    pub worker_id: &'a str,
    pub lease_id: &'a str,
    pub lease_expires_at: &'a str,
    pub request: &'a BamlCoerceRequest,
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

impl RuntimeKernel {
    pub fn new(store: SqliteStore) -> Self {
        Self {
            store,
            trace: Vec::new(),
        }
    }

    pub fn into_store(self) -> SqliteStore {
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
        self.store
            .create_instance(NewInstance {
                program_id: &version.program_id,
                version_id: &version.version_id,
                input_json,
            })
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
        self.emit(TraceEvent::InstanceCancelled);
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
            let metadata_json = json!({
                "cancellation": "requested_before_provider_launch",
                "provider": execution.provider,
            })
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

        let result = harness.run(request);
        let evidence = self.record_provider_result(execution, &result)?;
        let metadata_json = provider_metadata(&result);
        self.emit_provider_diagnostic(
            execution.run_id,
            execution.effect_id,
            execution.provider,
            provider_effect_status(&result.status),
            &result.summary,
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
            summary: Some(&result.summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "terminal",
            ])),
        };
        let diagnostic = self.provider_terminal_diagnostic(
            execution.instance_id,
            execution.effect_id,
            execution.run_id,
            execution.provider,
            provider_effect_status(&result.status),
            &result.summary,
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

    pub fn run_baml_coerce(
        &mut self,
        execution: BamlCoerceExecution<'_>,
        client: &dyn BamlClient,
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
        let evidence = self.record_baml_result(execution, &result)?;
        let metadata_json = baml_metadata(&result);
        self.emit_provider_diagnostic(
            execution.run_id,
            execution.effect_id,
            execution.provider,
            baml_effect_status(&result.status),
            &result.summary,
            &metadata_json,
        );
        let completion = EffectCompletion {
            instance_id: execution.instance_id,
            effect_id: execution.effect_id,
            run_id: execution.run_id,
            provider: execution.provider,
            worker_id: execution.worker_id,
            status: baml_status(&result.status),
            exit_code: baml_exit_code(&result.status),
            summary: Some(&result.summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "baml-terminal",
            ])),
        };
        let diagnostic = self.provider_terminal_diagnostic(
            execution.instance_id,
            execution.effect_id,
            execution.run_id,
            execution.provider,
            baml_effect_status(&result.status),
            &result.summary,
            Some(match result.status {
                BamlCoerceStatus::Succeeded => "baml.coerce.completed",
                BamlCoerceStatus::Failed => "baml.coerce.failed",
                BamlCoerceStatus::TimedOut => "baml.coerce.timed_out",
            }),
            &metadata_json,
            &evidence,
        );

        let event = match result.status {
            BamlCoerceStatus::Succeeded => self.complete_run(completion)?,
            BamlCoerceStatus::Failed => self.fail_run_with_diagnostic(completion, diagnostic)?,
            BamlCoerceStatus::TimedOut => {
                self.timeout_run_with_diagnostic(completion, diagnostic)?
            }
        };
        self.append_baml_fact(execution, &result)?;
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
        self.emit_provider_diagnostic(
            execution.run_id,
            execution.effect_id,
            execution.provider,
            loft_effect_status(&result.status),
            &result.summary,
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
            summary: Some(&result.summary),
            metadata_json: &metadata_json,
            idempotency_key: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "loft-terminal",
            ])),
        };
        let diagnostic = self.provider_terminal_diagnostic(
            execution.instance_id,
            execution.effect_id,
            execution.run_id,
            execution.provider,
            loft_effect_status(&result.status),
            &result.summary,
            Some(match result.status {
                LoftEffectStatus::Succeeded => "loft.completed",
                LoftEffectStatus::Failed => "loft.failed",
                LoftEffectStatus::TimedOut => "loft.timed_out",
            }),
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
            "note": execution.request.note,
            "target_status": execution.request.target_status,
            "evidence": execution.request.evidence_json.as_deref().map(json_from_str),
            "evidence_kind": execution.request.evidence_kind,
            "evidence_artifact": execution.request.evidence_artifact,
            "evidence_data_path": execution.request.evidence_data_path,
            "resource_intent": execution.request.resource_intent_json.as_deref().map(json_from_str),
            "release_after_failure": execution.request.release_after_failure,
            "expect_heads": execution.request.expect_heads,
            "request_metadata": json_from_str(&execution.request.metadata_json),
            "value": result.value_json.as_deref().map(json_from_str),
            "error": result.error_json.as_deref().map(json_from_str),
            "transcript": result.transcript,
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
            summary: Some(&result.summary),
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
        let value = json!({
            "effect_id": execution.effect_id,
            "run_id": execution.run_id,
            "action": execution.request.action.effect_kind(),
            "issue_id": execution.request.issue_id,
            "lease_id": execution.request.lease_id,
            "command_id": execution.request.command_id,
            "status": status,
            "value": result.value_json.as_deref().map(json_from_str),
            "error": result.error_json.as_deref().map(json_from_str),
            "summary": result.summary,
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
            },
            source: "kernel",
            causation_id: Some(execution.run_id),
            idempotency_key: Some(&fact_key),
        })?;
        Ok(())
    }

    fn record_baml_result(
        &self,
        execution: BamlCoerceExecution<'_>,
        result: &BamlCoerceResult,
    ) -> StoreResult<ProviderEvidence> {
        let metadata = json!({
            "effect_id": execution.effect_id,
            "function_name": execution.request.function_name,
            "arguments": json_from_str(&execution.request.arguments_json),
            "output_type": execution.request.output_type,
            "generated_baml_source_hash": execution.request.generated_baml_source_hash,
            "input_schema_hash": execution.request.input_schema_hash,
            "output_schema_hash": execution.request.output_schema_hash,
            "value": result.value_json.as_deref().map(json_from_str),
            "error": result.error_json.as_deref().map(json_from_str),
            "transcript": result.transcript,
            "usage": json_from_str(&result.usage_json),
        })
        .to_string();
        let evidence_id = self.store.record_evidence(EvidenceRecord {
            instance_id: execution.instance_id,
            kind: "baml.coerce.provider",
            subject_type: "run",
            subject_id: execution.run_id,
            causation_id: Some(execution.effect_id),
            correlation_id: Some(&idempotency_key(&[
                execution.instance_id,
                execution.run_id,
                "baml-provider",
            ])),
            summary: Some(&result.summary),
            metadata_json: &metadata,
        })?;
        Ok(ProviderEvidence {
            evidence_id: Some(evidence_id),
            artifact_ids: Vec::new(),
        })
    }

    fn append_baml_fact(
        &mut self,
        execution: BamlCoerceExecution<'_>,
        result: &BamlCoerceResult,
    ) -> StoreResult<()> {
        let status = baml_status(&result.status);
        let fact_name = match result.status {
            BamlCoerceStatus::Succeeded => "baml.coerce.succeeded",
            BamlCoerceStatus::Failed => "baml.coerce.failed",
            BamlCoerceStatus::TimedOut => "baml.coerce.timed_out",
        };
        let value = json!({
            "effect_id": execution.effect_id,
            "run_id": execution.run_id,
            "function_name": execution.request.function_name,
            "status": status,
            "output_type": execution.request.output_type,
            "value": result.value_json.as_deref().map(json_from_str),
            "error": result.error_json.as_deref().map(json_from_str),
            "summary": result.summary,
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
                "baml-event",
            ])),
        })?;
        let fact_id = idempotency_key(&[execution.instance_id, "baml", execution.run_id]);
        let fact_key = idempotency_key(&[execution.instance_id, execution.run_id, "baml-fact"]);
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
            },
            source: "kernel",
            causation_id: Some(execution.run_id),
            idempotency_key: Some(&fact_key),
        })?;
        Ok(())
    }

    fn record_provider_result(
        &self,
        execution: AgentTurnExecution<'_>,
        result: &ProviderRunResult,
    ) -> StoreResult<ProviderEvidence> {
        let artifact_ids = result
            .artifacts
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
        let metadata = json!({
            "effect_id": execution.effect_id,
            "agent": execution.agent,
            "profile": execution.profile,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "transcript": result.transcript,
            "exit_code": result.exit_code,
            "usage": json_from_str(&result.usage_json),
            "artifact_ids": artifact_ids,
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
            summary: Some(&result.summary),
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
        let payload = json!({
            "effect_id": execution.effect_id,
            "run_id": execution.run_id,
            "agent": execution.agent,
            "provider": execution.provider,
            "status": status,
            "summary": result.summary,
            "exit_code": result.exit_code,
            "failure": result.failure.as_ref().map(provider_failure_json),
        })
        .to_string();
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
            },
            source: "kernel",
            causation_id: Some(execution.run_id),
            idempotency_key: Some(&fact_event_key),
        })?;
        Ok(())
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
            severity: "error".to_owned(),
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
        "blocked_by_dependency"
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
    let agents = program
        .agents
        .iter()
        .map(|agent| {
            json!({
                "name": agent.name,
                "profile": agent.profile,
                "capacity": agent.capacity,
                "skills": agent.skills,
                "capabilities": agent.capabilities,
            })
        })
        .collect::<Vec<_>>();
    json!({ "agents": agents }).to_string()
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

    json!({
        "workflow": program.workflow,
        "workflow_contracts": workflow_contracts,
        "include_closure": include_closure,
        "pattern_applications": pattern_applications,
        "generated_declarations": generated_declarations,
        "generated_declaration_hashes": generated_declaration_hashes,
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
                    "profile": agent.profile,
                    "capacity": agent.capacity,
                    "skills": agent.skills,
                    "capabilities": agent.capabilities,
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
        IrEffectKind::BamlCoerce => "baml.coerce",
        IrEffectKind::LoftClaim => "loft.claim",
        IrEffectKind::HumanAsk => "human.ask",
        IrEffectKind::CapabilityCall => "capability.call",
        IrEffectKind::EventEmit => "event.emit",
        IrEffectKind::WorkflowInvoke => "workflow.invoke",
    }
}

fn dependency_predicate_name(predicate: &DependencyPredicate) -> &'static str {
    match predicate {
        DependencyPredicate::Succeeds => "succeeds",
        DependencyPredicate::Fails => "fails",
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

fn provider_status(status: &ProviderRunStatus) -> &'static str {
    match status {
        ProviderRunStatus::Completed => "completed",
        ProviderRunStatus::Failed => "failed",
        ProviderRunStatus::TimedOut => "timed_out",
    }
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
        "stdout": result.stdout,
        "stderr": result.stderr,
        "transcript": result.transcript,
        "usage": json_from_str(&result.usage_json),
        "failure": result.failure.as_ref().map(provider_failure_json),
    })
    .to_string()
}

fn provider_failure_json(failure: &ProviderFailure) -> serde_json::Value {
    json!({
        "phase": failure.phase,
        "error_kind": failure.error_kind,
        "message": failure.message,
        "recoverable": failure.recoverable,
        "retry_after": failure.retry_after,
        "provider_session_id": failure.provider_session_id,
        "provider_thread_id": failure.provider_thread_id,
        "raw": failure.raw_json.as_deref().map(json_from_str),
    })
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
    message
}

fn baml_status(status: &BamlCoerceStatus) -> &'static str {
    match status {
        BamlCoerceStatus::Succeeded => "completed",
        BamlCoerceStatus::Failed => "failed",
        BamlCoerceStatus::TimedOut => "timed_out",
    }
}

fn baml_effect_status(status: &BamlCoerceStatus) -> EffectStatus {
    match status {
        BamlCoerceStatus::Succeeded => EffectStatus::Completed,
        BamlCoerceStatus::Failed => EffectStatus::Failed,
        BamlCoerceStatus::TimedOut => EffectStatus::TimedOut,
    }
}

fn baml_exit_code(status: &BamlCoerceStatus) -> Option<i64> {
    match status {
        BamlCoerceStatus::Succeeded => Some(0),
        BamlCoerceStatus::Failed => Some(1),
        BamlCoerceStatus::TimedOut => None,
    }
}

fn baml_metadata(result: &BamlCoerceResult) -> String {
    json!({
        "value": result.value_json.as_deref().map(json_from_str),
        "error": result.error_json.as_deref().map(json_from_str),
        "transcript": result.transcript,
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
        "value": result.value_json.as_deref().map(json_from_str),
        "error": result.error_json.as_deref().map(json_from_str),
        "transcript": result.transcript,
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
    use coerce::{BamlCoerceRequest, FakeBamlClient};
    use harness::{
        ClaudeCodeAgentHarness, CodexAgentHarness, CommandLaunchPlan, MockAgentHarness,
        PiStyleAgentHarness,
    };
    use loft::{FakeLoftClient, LoftAction, LoftEffectRequest};
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
            TraceEvent::EffectBlocked { effect_id, reason }
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
        }];
        let effects = [NewEffect {
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
    fn kernel_times_out_effect_run() {
        let store = SqliteStore::open_in_memory().expect("store opens");
        let mut kernel = RuntimeKernel::new(store);
        let effects = [NewEffect {
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
            TraceEvent::EffectBlocked { effect_id, reason }
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

        let terminal = kernel
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
                    profile: Some("repo-writer"),
                    input_json: r#"{"prompt":"go"}"#,
                    skill_names: &["loft-user"],
                },
                &MockAgentHarness::completed("done"),
            )
            .expect("mock turn runs");

        assert_eq!(terminal.sequence, 3);
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
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
        assert!(evidence
            .iter()
            .any(|evidence| evidence.kind == "agent.turn.provider"
                && evidence.metadata_json.contains("mock stdout")
                && evidence.metadata_json.contains("mock transcript")
                && evidence.metadata_json.contains("artifact_ids")));
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
        assert!(events
            .iter()
            .any(|event| event.event_type == "agent.turn.completed"));
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
    fn fake_baml_coerce_records_evidence_and_success_fact() {
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
            effect_id: "review",
            kind: "baml.coerce",
            target: None,
            input_json: r#"{"function_name":"reviewWork"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=review",
            required_capabilities_json: r#"["baml.coerce"]"#,
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

        let request = BamlCoerceRequest {
            function_name: "reviewWork".to_owned(),
            arguments_json: r#"{"summary":"done"}"#.to_owned(),
            output_type: "WorkReview".to_owned(),
            generated_baml_source_hash: "baml-source".to_owned(),
            input_schema_hash: "input-schema".to_owned(),
            output_schema_hash: "output-schema".to_owned(),
        };
        let terminal = kernel
            .run_baml_coerce(
                BamlCoerceExecution {
                    instance_id: &instance_id,
                    effect_id: "review",
                    run_id: "run-review",
                    provider: "fake-baml",
                    worker_id: "worker-1",
                    lease_id: "lease-review",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    request: &request,
                },
                &FakeBamlClient::succeeds(r#"{"status":"Accept","reason":"ok"}"#),
            )
            .expect("coerce runs");

        assert_eq!(terminal.sequence, 3);
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let evidence = store.list_evidence(&instance_id).expect("evidence lists");
        assert_eq!(evidence.len(), 2);
        assert!(evidence
            .iter()
            .any(|evidence| evidence.kind == "baml.coerce.provider"
                && evidence.metadata_json.contains("reviewWork")
                && evidence.metadata_json.contains("baml-source")));
        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(facts.iter().any(|fact| fact.name == "baml.coerce.succeeded"
            && fact.value_json.contains("\"status\":\"Accept\"")));
    }

    #[test]
    fn fake_baml_coerce_failure_records_failed_fact() {
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
            effect_id: "review",
            kind: "baml.coerce",
            target: None,
            input_json: r#"{"function_name":"reviewWork"}"#,
            status: "queued",
            idempotency_key: "rule=start;effect=review",
            required_capabilities_json: r#"["baml.coerce"]"#,
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
        let request = BamlCoerceRequest {
            function_name: "reviewWork".to_owned(),
            arguments_json: r#"{"summary":"done"}"#.to_owned(),
            output_type: "WorkReview".to_owned(),
            generated_baml_source_hash: "baml-source".to_owned(),
            input_schema_hash: "input-schema".to_owned(),
            output_schema_hash: "output-schema".to_owned(),
        };

        kernel
            .run_baml_coerce(
                BamlCoerceExecution {
                    instance_id: &instance_id,
                    effect_id: "review",
                    run_id: "run-review",
                    provider: "fake-baml",
                    worker_id: "worker-1",
                    lease_id: "lease-review",
                    lease_expires_at: "2030-01-01T00:00:00Z",
                    request: &request,
                },
                &FakeBamlClient::fails("invalid output"),
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
                && provider == "fake-baml"
                && *status == EffectStatus::Failed
                && diagnostics_json.contains("invalid output")
        )));
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let diagnostics = store
            .list_diagnostics(Some(&instance_id))
            .expect("diagnostics list");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code.as_deref(), Some("baml.coerce.failed"));
        assert_eq!(diagnostics[0].subject_id.as_deref(), Some("review"));
        assert_eq!(diagnostics[0].run_id.as_deref(), Some("run-review"));
        assert!(diagnostics[0].message.contains("invalid output"));

        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(facts
            .iter()
            .any(|fact| fact.name == "baml.coerce.failed"
                && fact.value_json.contains("invalid output")));
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
                && diagnostics_json.contains("issue already leased")
        )));
        check_trace(kernel.trace()).expect("kernel trace conforms");
        let store = kernel.into_store();
        let diagnostics = store
            .list_diagnostics(Some(&instance_id))
            .expect("diagnostics list");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code.as_deref(), Some("loft.failed"));
        assert_eq!(diagnostics[0].subject_id.as_deref(), Some("claim"));
        assert_eq!(diagnostics[0].run_id.as_deref(), Some("run-claim"));
        assert!(diagnostics[0].message.contains("issue already leased"));

        let facts = store.list_facts(&instance_id).expect("facts list");
        assert!(facts.iter().any(|fact| fact.name == "loft.claim.failed"
            && fact.value_json.contains("issue already leased")));
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
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "effect.terminal"
                    && event.payload_json.contains("\"status\":\"failed\"")
                    && event
                        .payload_json
                        .contains("\"phase\":\"provider.exit.failed\"")
                    && event
                        .payload_json
                        .contains("\"error_kind\":\"nonzero_exit\"")
                    && event.payload_json.contains(expected_stderr)),
            "missing failed terminal event for {provider}: {events:#?}"
        );
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
        assert!(evidence
            .iter()
            .any(|evidence| evidence.kind == "agent.turn.provider"
                && evidence
                    .metadata_json
                    .contains("\"phase\":\"provider.exit.failed\"")
                && evidence.metadata_json.contains(expected_stderr)));

        let diagnostics = store
            .list_diagnostics(Some(&instance_id))
            .expect("diagnostics list");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_deref() == Some("nonzero_exit")
                && diagnostic.message.contains(expected_stderr)
                && diagnostic.event_id.is_some()
                && diagnostic.run_id.as_deref() == Some("run-tell")
        }));
    }
}
