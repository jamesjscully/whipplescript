//! The durable-object runtime store: `RuntimeStore` implemented over the DO's
//! synchronous SQLite (`DoSql`), instead of native rusqlite. This is DR-0033
//! Phase 5's core store binding.
//!
//! STATUS (honest): the `DoSql` seam and the store\'s hot-path core
//! (`schema_version`, `fact_exists`, `append_event`) are ported and **verified
//! against real SQLite** (the tests back `DoSql` with rusqlite). The remaining
//! methods are `todo!()` placeholders — the DO runs the *same* SQL the native
//! `SqliteStore` does, so each is a mechanical port of that method\'s SQL, but the
//! full set is large and its final correctness is against the *DO\'s* SQLite,
//! which needs a live Durable Object to build (`worker` crate) and verify. The
//! tracker\'s Phase-5 store box stays open until that port + live verification is
//! done; this establishes the seam and proves the pattern end-to-end.

use serde_json::Value;
use whipplescript_store::{NewEvent, RuntimeStore, StoreError, StoreResult, StoredEvent};
// The remaining ported methods reference the full set of store data types.
#[allow(unused_imports)]
use whipplescript_store::*;

/// A SQL scalar crossing the `DoSql` boundary (the DO SQL API speaks JSON-ish
/// scalars; this is the Rust mirror).
#[derive(Clone, Debug, PartialEq)]
pub enum SqlValue {
    Null,
    Int(i64),
    Text(String),
}

/// The DO\'s synchronous SQLite, as the store needs it: run a statement, or run a
/// query and get back rows of scalars. The Worker shell implements this over
/// `state.storage.sql`; tests implement it over rusqlite so the ported SQL is
/// verified against a real engine.
pub trait DoSql {
    fn execute(&self, sql: &str, params: &[SqlValue]) -> Result<u64, String>;
    fn query(&self, sql: &str, params: &[SqlValue]) -> Result<Vec<Vec<SqlValue>>, String>;
}

/// `RuntimeStore` over a `DoSql` backend — the durable-object store impl.
pub struct DoSqliteStore<Sql: DoSql> {
    pub sql: Sql,
}

impl<Sql: DoSql> DoSqliteStore<Sql> {
    pub fn new(sql: Sql) -> Self {
        Self { sql }
    }
}

fn sql_err(message: String) -> StoreError {
    StoreError::Io(std::io::Error::other(message))
}

fn text(value: &str) -> SqlValue {
    SqlValue::Text(value.to_string())
}

fn opt_text(value: Option<&str>) -> SqlValue {
    match value {
        Some(v) => SqlValue::Text(v.to_string()),
        None => SqlValue::Null,
    }
}

fn as_i64(value: &SqlValue) -> i64 {
    match value {
        SqlValue::Int(n) => *n,
        _ => 0,
    }
}

fn as_text(value: &SqlValue) -> String {
    match value {
        SqlValue::Text(s) => s.clone(),
        _ => String::new(),
    }
}

#[allow(unused_variables, clippy::todo, clippy::too_many_arguments)]
impl<Sql: DoSql> RuntimeStore for DoSqliteStore<Sql> {
    fn schema_version(&self) -> StoreResult<i64> {
        let rows = self
            .sql
            .query(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                &[],
            )
            .map_err(sql_err)?;
        Ok(rows
            .first()
            .and_then(|r| r.first())
            .map(as_i64)
            .unwrap_or(0))
    }

    fn append_event(&self, event: NewEvent<'_>) -> StoreResult<StoredEvent> {
        let rows = self
            .sql
            .query(
                "INSERT INTO events (event_id, instance_id, sequence, event_type, \
                 payload_json, occurred_at, source, causation_id, correlation_id, \
                 idempotency_key) VALUES ('evt_' || lower(hex(randomblob(16))), ?1, \
                 (SELECT COALESCE(MAX(sequence), 0) + 1 FROM events WHERE instance_id = ?1), \
                 ?2, ?3, CURRENT_TIMESTAMP, ?4, ?5, ?6, ?7) RETURNING event_id, sequence",
                &[
                    text(event.instance_id),
                    text(event.event_type),
                    text(event.payload_json),
                    text(event.source),
                    opt_text(event.causation_id),
                    opt_text(event.correlation_id),
                    opt_text(event.idempotency_key),
                ],
            )
            .map_err(sql_err)?;
        let row = rows
            .first()
            .ok_or_else(|| sql_err("append_event returned no row".to_string()))?;
        Ok(StoredEvent {
            event_id: as_text(&row[0]),
            sequence: as_i64(&row[1]),
        })
    }

    fn create_program_version(
        &mut self,
        version: NewProgramVersion<'_>,
    ) -> StoreResult<ProgramVersionRecord> {
        todo!("Phase 5b: port `create_program_version` SQL to DoSql + verify against a live DO")
    }

    fn get_program_version(&self, version_id: &str) -> StoreResult<Option<ProgramVersionView>> {
        todo!("Phase 5b: port `get_program_version` SQL to DoSql + verify against a live DO")
    }

    fn create_instance(&self, instance: NewInstance<'_>) -> StoreResult<InstanceRecord> {
        todo!("Phase 5b: port `create_instance` SQL to DoSql + verify against a live DO")
    }

    fn create_instance_with_authority(
        &self,
        instance: NewInstance<'_>,
        authority: NewInstanceAuthority<'_>,
    ) -> StoreResult<InstanceRecord> {
        todo!("Phase 5b: port `create_instance_with_authority` SQL to DoSql + verify against a live DO")
    }

    fn list_instance_revisions(&self, instance_id: &str) -> StoreResult<Vec<WorkflowRevisionView>> {
        todo!("Phase 5b: port `list_instance_revisions` SQL to DoSql + verify against a live DO")
    }

    fn revision_cancellation_impact(
        &self,
        instance_id: &str,
        cancellation_policy: &str,
    ) -> StoreResult<RevisionCancellationImpact> {
        todo!(
            "Phase 5b: port `revision_cancellation_impact` SQL to DoSql + verify against a live DO"
        )
    }

    fn analyze_revision_compatibility(
        &self,
        instance_id: &str,
        candidate_version_id: &str,
    ) -> StoreResult<RevisionCompatibilityReport> {
        todo!("Phase 5b: port `analyze_revision_compatibility` SQL to DoSql + verify against a live DO")
    }

    fn analyze_revision_candidate(
        &self,
        instance_id: &str,
        candidate: RevisionCandidate<'_>,
    ) -> StoreResult<RevisionCompatibilityReport> {
        todo!("Phase 5b: port `analyze_revision_candidate` SQL to DoSql + verify against a live DO")
    }

    fn activate_revision(
        &mut self,
        activation: RevisionActivation<'_>,
    ) -> StoreResult<WorkflowRevisionView> {
        todo!("Phase 5b: port `activate_revision` SQL to DoSql + verify against a live DO")
    }

    fn request_effect_cancellation(
        &mut self,
        request: EffectCancellationRequest<'_>,
    ) -> StoreResult<EffectCancellationRequestView> {
        todo!(
            "Phase 5b: port `request_effect_cancellation` SQL to DoSql + verify against a live DO"
        )
    }

    fn effect_has_open_cancellation_request(
        &self,
        instance_id: &str,
        effect_id: &str,
    ) -> StoreResult<bool> {
        todo!("Phase 5b: port `effect_has_open_cancellation_request` SQL to DoSql + verify against a live DO")
    }

    fn list_effect_cancellation_requests(
        &self,
        instance_id: &str,
    ) -> StoreResult<Vec<EffectCancellationRequestView>> {
        todo!("Phase 5b: port `list_effect_cancellation_requests` SQL to DoSql + verify against a live DO")
    }

    fn record_workflow_invocation(&self, invocation: NewWorkflowInvocation<'_>) -> StoreResult<()> {
        todo!("Phase 5b: port `record_workflow_invocation` SQL to DoSql + verify against a live DO")
    }

    fn get_workflow_invocation(
        &self,
        parent_instance_id: &str,
        parent_effect_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>> {
        todo!("Phase 5b: port `get_workflow_invocation` SQL to DoSql + verify against a live DO")
    }

    fn list_child_workflow_invocations(
        &self,
        parent_instance_id: &str,
    ) -> StoreResult<Vec<WorkflowInvocationView>> {
        todo!("Phase 5b: port `list_child_workflow_invocations` SQL to DoSql + verify against a live DO")
    }

    fn get_parent_workflow_invocation(
        &self,
        child_instance_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>> {
        todo!("Phase 5b: port `get_parent_workflow_invocation` SQL to DoSql + verify against a live DO")
    }

    fn commit_rule(&mut self, commit: RuleCommit<'_>) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `commit_rule` SQL to DoSql + verify against a live DO")
    }

    fn commit_rule_with_revision_guard(
        &mut self,
        commit: RuleCommit<'_>,
        guard: RuleCommitRevisionGuard<'_>,
    ) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `commit_rule_with_revision_guard` SQL to DoSql + verify against a live DO")
    }

    fn derive_fact(&mut self, derived: DerivedFact<'_>) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `derive_fact` SQL to DoSql + verify against a live DO")
    }

    fn admit_fact_batch(&mut self, batch: FactBatch<'_>) -> StoreResult<FactBatchOutcome> {
        todo!("Phase 5b: port `admit_fact_batch` SQL to DoSql + verify against a live DO")
    }

    fn complete_effect(&mut self, completion: EffectCompletion<'_>) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `complete_effect` SQL to DoSql + verify against a live DO")
    }

    fn complete_effect_with_terminal_diagnostic(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `complete_effect_with_terminal_diagnostic` SQL to DoSql + verify against a live DO")
    }

    fn resolve_effect_uncertain(
        &mut self,
        completion: EffectCompletion<'_>,
        diagnostic: Option<TerminalDiagnosticRecord>,
    ) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `resolve_effect_uncertain` SQL to DoSql + verify against a live DO")
    }

    fn claimable_effects(&self, instance_id: &str) -> StoreResult<Vec<ClaimableEffect>> {
        todo!("Phase 5b: port `claimable_effects` SQL to DoSql + verify against a live DO")
    }

    fn fact_exists(&self, instance_id: &str, fact_name: &str) -> StoreResult<bool> {
        let rows = self
            .sql
            .query(
                "SELECT 1 FROM facts WHERE instance_id = ?1 AND name = ?2 LIMIT 1",
                &[text(instance_id), text(fact_name)],
            )
            .map_err(sql_err)?;
        Ok(!rows.is_empty())
    }

    fn register_package(&self, package: PackageRegistration<'_>) -> StoreResult<()> {
        serde_json::from_str::<Value>(package.manifest_json)?;
        self.sql
            .execute(
                "INSERT INTO package_registrations (package_id, name, version, manifest_json) \
                 VALUES (?1, ?2, ?3, ?4) ON CONFLICT(package_id) DO UPDATE SET \
                 name = excluded.name, version = excluded.version, \
                 manifest_json = excluded.manifest_json",
                &[
                    text(package.package_id),
                    text(package.name),
                    text(package.version),
                    text(package.manifest_json),
                ],
            )
            .map_err(sql_err)?;
        Ok(())
    }

    fn register_package_manifest(&self, manifest_json: &str) -> StoreResult<String> {
        todo!("Phase 5b: port `register_package_manifest` SQL to DoSql + verify against a live DO")
    }

    fn register_capability_schema(
        &self,
        capability: CapabilitySchemaRegistration<'_>,
    ) -> StoreResult<()> {
        serde_json::from_str::<Value>(capability.schema_json)?;
        self.sql.execute(
            "INSERT INTO capability_schemas (capability, description, schema_json, registered_by_package_id) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(capability) DO UPDATE SET description = excluded.description, schema_json = excluded.schema_json, registered_by_package_id = excluded.registered_by_package_id",
            &[text(capability.capability), text(capability.description), text(capability.schema_json), opt_text(capability.registered_by_package_id)],
        ).map_err(sql_err)?;
        Ok(())
    }

    fn register_effect_provider(
        &self,
        provider: EffectProviderRegistration<'_>,
    ) -> StoreResult<()> {
        serde_json::from_str::<Value>(provider.config_json)?;
        self.sql.execute(
            "INSERT INTO effect_providers (provider_id, effect_kind, provider, capability, config_json, registered_by_package_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(effect_kind, provider) DO UPDATE SET capability = excluded.capability, config_json = excluded.config_json, registered_by_package_id = excluded.registered_by_package_id",
            &[text(provider.provider_id), text(provider.effect_kind), text(provider.provider), text(provider.capability), text(provider.config_json), opt_text(provider.registered_by_package_id)],
        ).map_err(sql_err)?;
        Ok(())
    }

    fn register_profile(&self, profile: ProfileRegistration<'_>) -> StoreResult<()> {
        serde_json::from_str::<Value>(profile.allowed_capabilities_json)?;
        serde_json::from_str::<Value>(profile.config_json)?;
        self.sql.execute(
            "INSERT INTO profiles (profile_id, name, description, enforcement_mode, allowed_capabilities, config_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(name) DO UPDATE SET description = excluded.description, enforcement_mode = excluded.enforcement_mode, allowed_capabilities = excluded.allowed_capabilities, config_json = excluded.config_json",
            &[text(profile.profile_id), text(profile.name), text(profile.description), text(profile.enforcement_mode), text(profile.allowed_capabilities_json), text(profile.config_json)],
        ).map_err(sql_err)?;
        Ok(())
    }

    fn registered_profile_policy(
        &self,
        profile: &str,
    ) -> StoreResult<Option<RegisteredProfilePolicy>> {
        todo!("Phase 5b: port `registered_profile_policy` SQL to DoSql + verify against a live DO")
    }

    fn bind_capability(&self, binding: CapabilityBinding<'_>) -> StoreResult<()> {
        serde_json::from_str::<Value>(binding.config_json)?;
        self.sql.execute(
            "INSERT INTO capability_bindings (binding_id, program_id, capability, provider, config_json) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(binding_id) DO UPDATE SET program_id = excluded.program_id, capability = excluded.capability, provider = excluded.provider, config_json = excluded.config_json",
            &[text(binding.binding_id), opt_text(binding.program_id), text(binding.capability), text(binding.provider), text(binding.config_json)],
        ).map_err(sql_err)?;
        Ok(())
    }

    fn register_skill(&self, skill: SkillRegistration<'_>) -> StoreResult<()> {
        todo!("Phase 5b: port `register_skill` SQL to DoSql + verify against a live DO")
    }

    fn attach_skill(&self, attachment: SkillAttachment<'_>) -> StoreResult<()> {
        todo!("Phase 5b: port `attach_skill` SQL to DoSql + verify against a live DO")
    }

    fn list_skills(&self) -> StoreResult<Vec<SkillView>> {
        todo!("Phase 5b: port `list_skills` SQL to DoSql + verify against a live DO")
    }

    fn list_skill_attachments(
        &self,
        scope_type: &str,
        scope_id: &str,
    ) -> StoreResult<Vec<SkillAttachmentView>> {
        todo!("Phase 5b: port `list_skill_attachments` SQL to DoSql + verify against a live DO")
    }

    fn record_evidence(&self, evidence: EvidenceRecord<'_>) -> StoreResult<String> {
        todo!("Phase 5b: port `record_evidence` SQL to DoSql + verify against a live DO")
    }

    fn record_provider_validation_evidence(
        &self,
        evidence: ProviderValidationEvidence<'_>,
    ) -> StoreResult<String> {
        todo!("Phase 5b: port `record_provider_validation_evidence` SQL to DoSql + verify against a live DO")
    }

    fn record_codex_app_server_evidence(
        &self,
        evidence: CodexAppServerEvidence<'_>,
    ) -> StoreResult<String> {
        todo!("Phase 5b: port `record_codex_app_server_evidence` SQL to DoSql + verify against a live DO")
    }

    fn record_claude_agent_sdk_evidence(
        &self,
        evidence: ClaudeAgentSdkEvidence<'_>,
    ) -> StoreResult<String> {
        todo!("Phase 5b: port `record_claude_agent_sdk_evidence` SQL to DoSql + verify against a live DO")
    }

    fn record_pi_rpc_evidence(&self, evidence: PiRpcEvidence<'_>) -> StoreResult<String> {
        todo!("Phase 5b: port `record_pi_rpc_evidence` SQL to DoSql + verify against a live DO")
    }

    fn link_evidence(&self, link: EvidenceLink<'_>) -> StoreResult<()> {
        todo!("Phase 5b: port `link_evidence` SQL to DoSql + verify against a live DO")
    }

    fn record_artifact(&self, artifact: ArtifactRecord<'_>) -> StoreResult<String> {
        todo!("Phase 5b: port `record_artifact` SQL to DoSql + verify against a live DO")
    }

    fn list_artifacts_for_run(&self, run_id: &str) -> StoreResult<Vec<ArtifactView>> {
        todo!("Phase 5b: port `list_artifacts_for_run` SQL to DoSql + verify against a live DO")
    }

    fn record_workspace(&self, workspace: WorkspaceRecord<'_>) -> StoreResult<String> {
        todo!("Phase 5b: port `record_workspace` SQL to DoSql + verify against a live DO")
    }

    fn get_workspace(&self, workspace_id: &str) -> StoreResult<Option<WorkspaceView>> {
        todo!("Phase 5b: port `get_workspace` SQL to DoSql + verify against a live DO")
    }

    fn list_workspaces_for_instance(&self, instance_id: &str) -> StoreResult<Vec<WorkspaceView>> {
        todo!(
            "Phase 5b: port `list_workspaces_for_instance` SQL to DoSql + verify against a live DO"
        )
    }

    fn record_diagnostic(&self, diagnostic: DiagnosticRecord<'_>) -> StoreResult<String> {
        todo!("Phase 5b: port `record_diagnostic` SQL to DoSql + verify against a live DO")
    }

    fn list_diagnostics(&self, instance_id: Option<&str>) -> StoreResult<Vec<DiagnosticView>> {
        todo!("Phase 5b: port `list_diagnostics` SQL to DoSql + verify against a live DO")
    }

    fn list_diagnostics_from_events(&self, instance_id: &str) -> StoreResult<Vec<DiagnosticView>> {
        todo!(
            "Phase 5b: port `list_diagnostics_from_events` SQL to DoSql + verify against a live DO"
        )
    }

    fn effect_source_span_json(
        &self,
        instance_id: &str,
        effect_id: &str,
    ) -> StoreResult<Option<String>> {
        todo!("Phase 5b: port `effect_source_span_json` SQL to DoSql + verify against a live DO")
    }

    fn create_inbox_item(&self, item: NewInboxItem<'_>) -> StoreResult<()> {
        todo!("Phase 5b: port `create_inbox_item` SQL to DoSql + verify against a live DO")
    }

    fn list_inbox_items(&self, status: Option<&str>) -> StoreResult<Vec<InboxItemView>> {
        todo!("Phase 5b: port `list_inbox_items` SQL to DoSql + verify against a live DO")
    }

    fn get_inbox_item(&self, inbox_item_id: &str) -> StoreResult<Option<InboxItemView>> {
        todo!("Phase 5b: port `get_inbox_item` SQL to DoSql + verify against a live DO")
    }

    fn answer_inbox_item(&mut self, answer: HumanAnswer<'_>) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `answer_inbox_item` SQL to DoSql + verify against a live DO")
    }

    fn cancel_pending_inbox_for_instance(&mut self, instance_id: &str) -> StoreResult<usize> {
        todo!("Phase 5b: port `cancel_pending_inbox_for_instance` SQL to DoSql + verify against a live DO")
    }

    fn record_skill_evidence(&self, evidence: SkillEvidence<'_>) -> StoreResult<String> {
        todo!("Phase 5b: port `record_skill_evidence` SQL to DoSql + verify against a live DO")
    }

    fn list_evidence(&self, instance_id: &str) -> StoreResult<Vec<EvidenceView>> {
        todo!("Phase 5b: port `list_evidence` SQL to DoSql + verify against a live DO")
    }

    fn list_evidence_for_subject(
        &self,
        subject_type: &str,
        subject_id: &str,
    ) -> StoreResult<Vec<EvidenceView>> {
        todo!("Phase 5b: port `list_evidence_for_subject` SQL to DoSql + verify against a live DO")
    }

    fn list_evidence_links(&self, instance_id: &str) -> StoreResult<Vec<EvidenceLinkView>> {
        todo!("Phase 5b: port `list_evidence_links` SQL to DoSql + verify against a live DO")
    }

    fn list_instances(&self) -> StoreResult<Vec<InstanceView>> {
        todo!("Phase 5b: port `list_instances` SQL to DoSql + verify against a live DO")
    }

    fn get_instance(&self, instance_id: &str) -> StoreResult<Option<InstanceView>> {
        todo!("Phase 5b: port `get_instance` SQL to DoSql + verify against a live DO")
    }

    fn list_events(&self, instance_id: &str) -> StoreResult<Vec<EventView>> {
        todo!("Phase 5b: port `list_events` SQL to DoSql + verify against a live DO")
    }

    fn list_facts(&self, instance_id: &str) -> StoreResult<Vec<FactView>> {
        todo!("Phase 5b: port `list_facts` SQL to DoSql + verify against a live DO")
    }

    fn list_facts_including_consumed(&self, instance_id: &str) -> StoreResult<Vec<FactView>> {
        todo!("Phase 5b: port `list_facts_including_consumed` SQL to DoSql + verify against a live DO")
    }

    fn list_effects(&self, instance_id: &str) -> StoreResult<Vec<EffectView>> {
        todo!("Phase 5b: port `list_effects` SQL to DoSql + verify against a live DO")
    }

    fn list_runs(&self, instance_id: &str) -> StoreResult<Vec<RunView>> {
        todo!("Phase 5b: port `list_runs` SQL to DoSql + verify against a live DO")
    }

    fn status(&self, instance_id: &str) -> StoreResult<Option<StatusView>> {
        todo!("Phase 5b: port `status` SQL to DoSql + verify against a live DO")
    }

    fn satisfy_dependencies(&self, instance_id: &str) -> StoreResult<usize> {
        todo!("Phase 5b: port `satisfy_dependencies` SQL to DoSql + verify against a live DO")
    }

    fn start_run(&mut self, run: RunStart<'_>) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `start_run` SQL to DoSql + verify against a live DO")
    }

    fn block_effect_binding(
        &mut self,
        instance_id: &str,
        effect_id: &str,
        category: &str,
        detail: &str,
    ) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `block_effect_binding` SQL to DoSql + verify against a live DO")
    }

    fn transition_instance(
        &mut self,
        transition: InstanceTransition<'_>,
    ) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `transition_instance` SQL to DoSql + verify against a live DO")
    }

    fn due_time_effects(&self, instance_id: &str, now: &str) -> StoreResult<Vec<DueTimeEffect>> {
        todo!("Phase 5b: port `due_time_effects` SQL to DoSql + verify against a live DO")
    }

    fn due_interval_occurrences(
        &self,
        after_scheduled: &str,
        interval_seconds: i64,
        now: &str,
    ) -> StoreResult<Vec<String>> {
        todo!("Phase 5b: port `due_interval_occurrences` SQL to DoSql + verify against a live DO")
    }

    fn resolve_clock(&self, now: &str) -> StoreResult<String> {
        todo!("Phase 5b: port `resolve_clock` SQL to DoSql + verify against a live DO")
    }

    fn last_clock_occurrence(
        &self,
        instance_id: &str,
        signal: &str,
    ) -> StoreResult<Option<String>> {
        todo!("Phase 5b: port `last_clock_occurrence` SQL to DoSql + verify against a live DO")
    }

    fn pending_time_effects(&self, instance_id: &str) -> StoreResult<Vec<DueTimeEffect>> {
        todo!("Phase 5b: port `pending_time_effects` SQL to DoSql + verify against a live DO")
    }

    fn expire_effect(
        &mut self,
        instance_id: &str,
        effect_id: &str,
        idempotency_key: Option<&str>,
    ) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `expire_effect` SQL to DoSql + verify against a live DO")
    }

    fn retire_fact(&mut self, instance_id: &str, fact_id: &str) -> StoreResult<()> {
        todo!("Phase 5b: port `retire_fact` SQL to DoSql + verify against a live DO")
    }

    fn cancel_effect(&mut self, cancellation: EffectCancellation<'_>) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `cancel_effect` SQL to DoSql + verify against a live DO")
    }

    fn renew_lease(&mut self, renewal: LeaseRenewal<'_>) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `renew_lease` SQL to DoSql + verify against a live DO")
    }

    fn expire_leases(&mut self, instance_id: &str, now: &str) -> StoreResult<Vec<ExpiredLease>> {
        todo!("Phase 5b: port `expire_leases` SQL to DoSql + verify against a live DO")
    }

    fn retry_effect(&mut self, retry: RetryEffect<'_>) -> StoreResult<StoredEvent> {
        todo!("Phase 5b: port `retry_effect` SQL to DoSql + verify against a live DO")
    }

    fn rebuild_projections(&mut self, instance_id: &str) -> StoreResult<()> {
        todo!("Phase 5b: port `rebuild_projections` SQL to DoSql + verify against a live DO")
    }

    fn table_exists(&self, table: &str) -> StoreResult<bool> {
        todo!("Phase 5b: port `table_exists` SQL to DoSql + verify against a live DO")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::types::{Value, ValueRef};
    use rusqlite::Connection;

    /// Backs `DoSql` with real in-memory SQLite, so the ported store SQL is
    /// checked against an actual engine.
    struct RusqliteDoSql {
        conn: Connection,
    }

    fn to_value(v: &SqlValue) -> Value {
        match v {
            SqlValue::Null => Value::Null,
            SqlValue::Int(n) => Value::Integer(*n),
            SqlValue::Text(s) => Value::Text(s.clone()),
        }
    }

    fn from_ref(r: ValueRef<'_>) -> SqlValue {
        match r {
            ValueRef::Null => SqlValue::Null,
            ValueRef::Integer(n) => SqlValue::Int(n),
            ValueRef::Real(f) => SqlValue::Int(f as i64),
            ValueRef::Text(t) => SqlValue::Text(String::from_utf8_lossy(t).into_owned()),
            ValueRef::Blob(_) => SqlValue::Null,
        }
    }

    impl DoSql for RusqliteDoSql {
        fn execute(&self, sql: &str, params: &[SqlValue]) -> Result<u64, String> {
            self.conn
                .execute(sql, rusqlite::params_from_iter(params.iter().map(to_value)))
                .map(|n| n as u64)
                .map_err(|e| e.to_string())
        }

        fn query(&self, sql: &str, params: &[SqlValue]) -> Result<Vec<Vec<SqlValue>>, String> {
            let mut stmt = self.conn.prepare(sql).map_err(|e| e.to_string())?;
            let cols = stmt.column_count();
            let rows = stmt
                .query_map(
                    rusqlite::params_from_iter(params.iter().map(to_value)),
                    |row| {
                        let mut out = Vec::with_capacity(cols);
                        for i in 0..cols {
                            out.push(from_ref(row.get_ref(i)?));
                        }
                        Ok(out)
                    },
                )
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        }
    }

    fn store() -> DoSqliteStore<RusqliteDoSql> {
        let conn = Connection::open_in_memory().expect("sqlite");
        conn.execute_batch(
            r#"
            CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, name TEXT);
            INSERT INTO schema_migrations (version, name) VALUES (1, 'init');
            CREATE TABLE events (
                event_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, sequence INTEGER NOT NULL,
                event_type TEXT NOT NULL, payload_json TEXT NOT NULL, occurred_at TEXT NOT NULL,
                source TEXT NOT NULL, causation_id TEXT, correlation_id TEXT, idempotency_key TEXT
            );
            CREATE TABLE facts (instance_id TEXT NOT NULL, name TEXT NOT NULL);
            CREATE TABLE package_registrations (
                package_id TEXT PRIMARY KEY, name TEXT NOT NULL, version TEXT NOT NULL,
                manifest_json TEXT NOT NULL
            );
            CREATE TABLE capability_schemas (
                capability TEXT PRIMARY KEY, description TEXT NOT NULL, schema_json TEXT NOT NULL,
                registered_by_package_id TEXT
            );
            CREATE TABLE effect_providers (
                provider_id TEXT NOT NULL, effect_kind TEXT NOT NULL, provider TEXT NOT NULL,
                capability TEXT NOT NULL, config_json TEXT NOT NULL, registered_by_package_id TEXT,
                UNIQUE(effect_kind, provider)
            );
            CREATE TABLE profiles (
                profile_id TEXT NOT NULL, name TEXT PRIMARY KEY, description TEXT NOT NULL,
                enforcement_mode TEXT NOT NULL, allowed_capabilities TEXT NOT NULL,
                config_json TEXT NOT NULL
            );
            CREATE TABLE capability_bindings (
                binding_id TEXT PRIMARY KEY, program_id TEXT, capability TEXT NOT NULL,
                provider TEXT NOT NULL, config_json TEXT NOT NULL
            );
            "#,
        )
        .expect("schema");
        DoSqliteStore::new(RusqliteDoSql { conn })
    }

    /// The ported core methods run their real SQL against a real engine.
    #[test]
    fn do_store_core_methods_run_real_sql() {
        let store = store();

        assert_eq!(store.schema_version().expect("version"), 1);
        assert!(!store.fact_exists("i1", "ready").expect("fact"));

        let event = store
            .append_event(NewEvent {
                instance_id: "i1",
                event_type: "workflow.started",
                payload_json: "{}",
                source: "test",
                causation_id: None,
                correlation_id: None,
                idempotency_key: Some("k1"),
            })
            .expect("append");
        assert!(event.event_id.starts_with("evt_"));
        assert_eq!(event.sequence, 1);

        // A second event on the same instance advances the per-instance sequence.
        let event2 = store
            .append_event(NewEvent {
                instance_id: "i1",
                event_type: "effect.claimed",
                payload_json: "{}",
                source: "test",
                causation_id: None,
                correlation_id: None,
                idempotency_key: None,
            })
            .expect("append 2");
        assert_eq!(event2.sequence, 2);

        // fact_exists reflects a fact row.
        store
            .sql
            .execute(
                "INSERT INTO facts (instance_id, name) VALUES (?1, ?2)",
                &[text("i1"), text("ready")],
            )
            .expect("insert fact");
        assert!(store.fact_exists("i1", "ready").expect("fact"));

        // register_package runs its real INSERT ... ON CONFLICT and validates JSON.
        store
            .register_package(PackageRegistration {
                package_id: "pkg_1",
                name: "std",
                version: "1.0.0",
                manifest_json: "{}",
            })
            .expect("register");
        let rows = store
            .sql
            .query(
                "SELECT name FROM package_registrations WHERE package_id = ?1",
                &[text("pkg_1")],
            )
            .expect("read package");
        assert_eq!(as_text(&rows[0][0]), "std");
    }

    /// The ported registration methods run their real INSERT...ON CONFLICT SQL.
    #[test]
    fn do_store_registration_methods_run_real_sql() {
        let store = store();

        store
            .register_capability_schema(CapabilitySchemaRegistration {
                capability: "std.files",
                description: "file access",
                schema_json: "{}",
                registered_by_package_id: Some("pkg_1"),
            })
            .expect("cap schema");
        store
            .register_effect_provider(EffectProviderRegistration {
                provider_id: "prov_1",
                effect_kind: "coerce",
                provider: "anthropic",
                capability: "std.model",
                config_json: "{}",
                registered_by_package_id: None,
            })
            .expect("provider");
        store
            .register_profile(ProfileRegistration {
                profile_id: "prof_1",
                name: "default",
                description: "d",
                enforcement_mode: "enforce",
                allowed_capabilities_json: "[]",
                config_json: "{}",
            })
            .expect("profile");
        store
            .bind_capability(CapabilityBinding {
                binding_id: "bind_1",
                program_id: Some("prg_1"),
                capability: "std.files",
                provider: "local",
                config_json: "{}",
            })
            .expect("binding");

        for (table, key_col, key) in [
            ("capability_schemas", "capability", "std.files"),
            ("effect_providers", "provider_id", "prov_1"),
            ("profiles", "profile_id", "prof_1"),
            ("capability_bindings", "binding_id", "bind_1"),
        ] {
            let rows = store
                .sql
                .query(
                    &format!("SELECT 1 FROM {table} WHERE {key_col} = ?1"),
                    &[text(key)],
                )
                .expect("read");
            assert_eq!(rows.len(), 1, "{table} row present");
        }
    }
}
