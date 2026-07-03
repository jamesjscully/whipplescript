//! The durable-object runtime store: `RuntimeStore` implemented over the DO's
//! synchronous SQLite (`DoSql`), instead of native rusqlite. This is DR-0033
//! Phase 5's core store binding.
//!
//! STATUS (honest): the `DoSql` seam plus the store\'s hot-path core and the
//! straight-line registration / skill / inbox / fact-retirement / introspection
//! methods are ported and **verified against real SQLite** (the tests back
//! `DoSql` with rusqlite). The remaining methods — chiefly the multi-statement
//! transactional lifecycle (create/activate revisions, admit fact batches,
//! start/complete/retry effects, rebuild projections) and the large family of
//! `list_*`/`get_*` view queries — are `todo!()` placeholders. The DO runs the
//! *same* SQL the native `SqliteStore` does, so each is a port of that method\'s
//! SQL; the transactional ones additionally need a batch/transaction primitive
//! on `DoSql` (the DO\'s single-writer invocation gives implicit atomicity, but
//! faithful native testing wants explicit BEGIN/COMMIT). Final correctness is
//! against the *DO\'s* SQLite, which needs a live Durable Object (`worker` crate)
//! to build and verify. The tracker\'s Phase-5 store box stays open until that
//! port + live verification is done; this establishes the seam, proves the
//! pattern, and ports the mechanical majority-shape methods.

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

fn as_opt_text(value: &SqlValue) -> Option<String> {
    match value {
        SqlValue::Text(s) => Some(s.clone()),
        _ => None,
    }
}

fn as_opt_i64(value: &SqlValue) -> Option<i64> {
    match value {
        SqlValue::Int(n) => Some(*n),
        _ => None,
    }
}

fn int(n: i64) -> SqlValue {
    SqlValue::Int(n)
}

fn bool_int(value: bool) -> SqlValue {
    SqlValue::Int(if value { 1 } else { 0 })
}

/// FNV-1a, byte-identical to the native store's `stable_hash_hex` — the DO must
/// compute the same skill `content_hash` the native path does so a skill
/// registered under either backend has a stable identity.
fn stable_hash_hex(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Maps an 8-column skill row (`skill_id..required_capabilities`) to a `SkillView`.
fn skill_view_from_row(row: &[SqlValue]) -> SkillView {
    SkillView {
        skill_id: as_text(&row[0]),
        name: as_text(&row[1]),
        version: as_text(&row[2]),
        source: as_text(&row[3]),
        source_path: as_text(&row[4]),
        content_hash: as_text(&row[5]),
        description: as_text(&row[6]),
        required_capabilities_json: as_text(&row[7]),
    }
}

/// Maps a 14-column inbox row to an `InboxItemView` (nullable columns:
/// `effect_id`, `answer_json`, `answered_by`, `answered_at`).
fn inbox_item_view_from_row(row: &[SqlValue]) -> InboxItemView {
    InboxItemView {
        inbox_item_id: as_text(&row[0]),
        instance_id: as_text(&row[1]),
        effect_id: as_opt_text(&row[2]),
        status: as_text(&row[3]),
        prompt: as_text(&row[4]),
        choices_json: as_text(&row[5]),
        freeform_allowed: as_i64(&row[6]) != 0,
        severity: as_text(&row[7]),
        related_effects_json: as_text(&row[8]),
        related_artifacts_json: as_text(&row[9]),
        answer_json: as_opt_text(&row[10]),
        answered_by: as_opt_text(&row[11]),
        created_at: as_text(&row[12]),
        answered_at: as_opt_text(&row[13]),
    }
}

/// Maps a 10-column instance row to an `InstanceView`.
fn instance_view_from_row(row: &[SqlValue]) -> InstanceView {
    InstanceView {
        instance_id: as_text(&row[0]),
        program_id: as_text(&row[1]),
        version_id: as_text(&row[2]),
        revision_epoch: as_i64(&row[3]),
        workflow_principal: as_text(&row[4]),
        effective_authority_json: as_text(&row[5]),
        status: as_text(&row[6]),
        input_json: as_text(&row[7]),
        created_at: as_text(&row[8]),
        updated_at: as_text(&row[9]),
    }
}

/// Maps a 6-column event row to an `EventView`.
fn event_view_from_row(row: &[SqlValue]) -> EventView {
    EventView {
        event_id: as_text(&row[0]),
        sequence: as_i64(&row[1]),
        event_type: as_text(&row[2]),
        payload_json: as_text(&row[3]),
        source: as_text(&row[4]),
        occurred_at: as_text(&row[5]),
    }
}

/// Maps an 8-column fact row to a `FactView` (nullable: `program_version_id`,
/// `source_span_json`).
fn fact_view_from_row(row: &[SqlValue]) -> FactView {
    FactView {
        fact_id: as_text(&row[0]),
        program_version_id: as_opt_text(&row[1]),
        revision_epoch: as_i64(&row[2]),
        name: as_text(&row[3]),
        key: as_text(&row[4]),
        value_json: as_text(&row[5]),
        provenance_class: as_text(&row[6]),
        source_span_json: as_opt_text(&row[7]),
    }
}

/// Maps a 14-column effect row to an `EffectView` (last column is the
/// `EXISTS(...)` cancel-requested flag, 0/1).
fn effect_view_from_row(row: &[SqlValue]) -> EffectView {
    EffectView {
        effect_id: as_text(&row[0]),
        kind: as_text(&row[1]),
        target: as_opt_text(&row[2]),
        input_json: as_text(&row[3]),
        status: as_text(&row[4]),
        created_by_rule: as_text(&row[5]),
        program_version_id: as_opt_text(&row[6]),
        revision_epoch: as_i64(&row[7]),
        profile: as_opt_text(&row[8]),
        required_capabilities_json: as_text(&row[9]),
        policy_block_reason: as_opt_text(&row[10]),
        policy_block_category: as_opt_text(&row[11]),
        declared_profiles_json: as_text(&row[12]),
        cancel_requested: as_i64(&row[13]) != 0,
    }
}

/// Maps a 9-column run row to a `RunView` (last column is the `EXISTS(...)`
/// cancel-requested flag, 0/1).
fn run_view_from_row(row: &[SqlValue]) -> RunView {
    RunView {
        run_id: as_text(&row[0]),
        effect_id: as_text(&row[1]),
        provider: as_text(&row[2]),
        worker_id: as_text(&row[3]),
        status: as_text(&row[4]),
        started_at: as_text(&row[5]),
        completed_at: as_opt_text(&row[6]),
        metadata_json: as_text(&row[7]),
        cancel_requested: as_i64(&row[8]) != 0,
    }
}

/// Maps a 12-column instance-revision row to a `WorkflowRevisionView`.
fn workflow_revision_from_row(row: &[SqlValue]) -> WorkflowRevisionView {
    WorkflowRevisionView {
        revision_id: as_text(&row[0]),
        instance_id: as_text(&row[1]),
        epoch: as_i64(&row[2]),
        from_version_id: as_text(&row[3]),
        to_version_id: as_text(&row[4]),
        activated_by_event_id: as_text(&row[5]),
        activation_policy_json: as_text(&row[6]),
        cancellation_policy: as_text(&row[7]),
        status: as_text(&row[8]),
        idempotency_key: as_opt_text(&row[9]),
        created_at: as_text(&row[10]),
        activated_at: as_text(&row[11]),
    }
}

/// Maps a 12-column cancellation-request row to an `EffectCancellationRequestView`.
fn effect_cancellation_request_from_row(row: &[SqlValue]) -> EffectCancellationRequestView {
    EffectCancellationRequestView {
        request_id: as_text(&row[0]),
        instance_id: as_text(&row[1]),
        effect_id: as_text(&row[2]),
        revision_id: as_opt_text(&row[3]),
        reason: as_opt_text(&row[4]),
        requested_by: as_text(&row[5]),
        causation_event_id: as_opt_text(&row[6]),
        status: as_text(&row[7]),
        idempotency_key: as_opt_text(&row[8]),
        created_at: as_text(&row[9]),
        updated_at: as_text(&row[10]),
        resolved_by_event_id: as_opt_text(&row[11]),
    }
}

/// Maps a 19-column workflow-invocation row (with parent/child active-version
/// joins) to a `WorkflowInvocationView`.
fn workflow_invocation_from_row(row: &[SqlValue]) -> WorkflowInvocationView {
    WorkflowInvocationView {
        invocation_id: as_text(&row[0]),
        parent_instance_id: as_text(&row[1]),
        parent_effect_id: as_text(&row[2]),
        parent_program_version_id: as_opt_text(&row[3]),
        parent_revision_epoch: as_i64(&row[4]),
        parent_active_program_version_id: as_opt_text(&row[5]),
        parent_active_revision_epoch: as_opt_i64(&row[6]),
        child_instance_id: as_text(&row[7]),
        child_program_version_id: as_opt_text(&row[8]),
        child_revision_epoch: as_opt_i64(&row[9]),
        child_active_program_version_id: as_opt_text(&row[10]),
        child_active_revision_epoch: as_opt_i64(&row[11]),
        target_workflow: as_text(&row[12]),
        input_json: as_text(&row[13]),
        status: as_text(&row[14]),
        terminal_event_id: as_opt_text(&row[15]),
        source_span_json: as_opt_text(&row[16]),
        created_at: as_text(&row[17]),
        updated_at: as_text(&row[18]),
    }
}

/// The shared 19-column workflow-invocation projection (parent/child active
/// versions joined, status folded to the parent effect's terminal). Callers
/// append their own `WHERE ... ORDER BY ...` clause. Mirrors the native SQL.
const WORKFLOW_INVOCATION_SELECT: &str = "SELECT invocation_id, parent_instance_id, \
     parent_effect_id, parent_program_version_id, parent_revision_epoch, \
     parent_instance.version_id, parent_instance.revision_epoch, child_instance_id, \
     child_program_version_id, child_revision_epoch, child_instance.version_id, \
     child_instance.revision_epoch, workflow_invocations.target_workflow, \
     workflow_invocations.input_json, \
     CASE WHEN parent_effect.status IN ('completed', 'failed', 'timed_out', 'cancelled') \
     THEN parent_effect.status ELSE workflow_invocations.status END, \
     workflow_invocations.terminal_event_id, workflow_invocations.source_span_json, \
     workflow_invocations.created_at, \
     COALESCE(workflow_invocations.updated_at, workflow_invocations.created_at) \
     FROM workflow_invocations \
     LEFT JOIN instances AS parent_instance \
     ON parent_instance.instance_id = workflow_invocations.parent_instance_id \
     LEFT JOIN instances AS child_instance \
     ON child_instance.instance_id = workflow_invocations.child_instance_id \
     LEFT JOIN effects AS parent_effect \
     ON parent_effect.instance_id = workflow_invocations.parent_instance_id \
     AND parent_effect.effect_id = workflow_invocations.parent_effect_id ";

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
        let rows = self
            .sql
            .query(
                "SELECT revision_id, instance_id, epoch, from_version_id, to_version_id, \
                 activated_by_event_id, activation_policy_json, cancellation_policy, status, \
                 idempotency_key, created_at, activated_at FROM instance_revisions \
                 WHERE instance_id = ?1 ORDER BY epoch",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.iter().map(|r| workflow_revision_from_row(r)).collect())
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
        let rows = self
            .sql
            .query(
                "SELECT 1 FROM effect_cancellation_requests \
                 WHERE instance_id = ?1 AND effect_id = ?2 AND status = 'requested' LIMIT 1",
                &[text(instance_id), text(effect_id)],
            )
            .map_err(sql_err)?;
        Ok(!rows.is_empty())
    }

    fn list_effect_cancellation_requests(
        &self,
        instance_id: &str,
    ) -> StoreResult<Vec<EffectCancellationRequestView>> {
        let rows = self
            .sql
            .query(
                "SELECT request_id, instance_id, effect_id, revision_id, reason, requested_by, \
                 causation_event_id, status, idempotency_key, created_at, updated_at, \
                 resolved_by_event_id FROM effect_cancellation_requests \
                 WHERE instance_id = ?1 ORDER BY created_at, request_id",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        Ok(rows
            .iter()
            .map(|r| effect_cancellation_request_from_row(r))
            .collect())
    }

    fn record_workflow_invocation(&self, invocation: NewWorkflowInvocation<'_>) -> StoreResult<()> {
        serde_json::from_str::<Value>(invocation.input_json)?;
        let parent = self
            .sql
            .query(
                "SELECT program_version_id, revision_epoch FROM effects \
                 WHERE instance_id = ?1 AND effect_id = ?2",
                &[
                    text(invocation.parent_instance_id),
                    text(invocation.parent_effect_id),
                ],
            )
            .map_err(sql_err)?;
        let parent = parent.first().ok_or_else(|| {
            StoreError::Conflict("parent workflow invoke effect does not exist".to_owned())
        })?;
        let parent_program_version_id = as_opt_text(&parent[0]);
        let parent_revision_epoch = as_i64(&parent[1]);
        let child = self
            .sql
            .query(
                "SELECT version_id, revision_epoch FROM instances WHERE instance_id = ?1",
                &[text(invocation.child_instance_id)],
            )
            .map_err(sql_err)?;
        let child = child.first().ok_or_else(|| {
            StoreError::Conflict("child workflow instance does not exist".to_owned())
        })?;
        let child_program_version_id = as_text(&child[0]);
        let child_revision_epoch = as_i64(&child[1]);
        self.sql
            .execute(
                "INSERT INTO workflow_invocations (invocation_id, parent_instance_id, \
                 parent_effect_id, parent_program_version_id, parent_revision_epoch, \
                 child_instance_id, child_program_version_id, child_revision_epoch, \
                 target_workflow, input_json, source_span_json, idempotency_key) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT(idempotency_key) DO NOTHING",
                &[
                    text(invocation.invocation_id),
                    text(invocation.parent_instance_id),
                    text(invocation.parent_effect_id),
                    opt_text(parent_program_version_id.as_deref()),
                    int(parent_revision_epoch),
                    text(invocation.child_instance_id),
                    text(&child_program_version_id),
                    int(child_revision_epoch),
                    text(invocation.target_workflow),
                    text(invocation.input_json),
                    opt_text(invocation.source_span_json),
                    text(invocation.idempotency_key),
                ],
            )
            .map_err(sql_err)?;
        Ok(())
    }

    fn get_workflow_invocation(
        &self,
        parent_instance_id: &str,
        parent_effect_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>> {
        let sql = format!(
            "{WORKFLOW_INVOCATION_SELECT}WHERE workflow_invocations.parent_instance_id = ?1 \
             AND workflow_invocations.parent_effect_id = ?2 \
             ORDER BY workflow_invocations.created_at DESC, invocation_id DESC LIMIT 1"
        );
        let rows = self
            .sql
            .query(&sql, &[text(parent_instance_id), text(parent_effect_id)])
            .map_err(sql_err)?;
        Ok(rows.first().map(|r| workflow_invocation_from_row(r)))
    }

    fn list_child_workflow_invocations(
        &self,
        parent_instance_id: &str,
    ) -> StoreResult<Vec<WorkflowInvocationView>> {
        let sql = format!(
            "{WORKFLOW_INVOCATION_SELECT}WHERE workflow_invocations.parent_instance_id = ?1 \
             ORDER BY workflow_invocations.created_at, invocation_id"
        );
        let rows = self
            .sql
            .query(&sql, &[text(parent_instance_id)])
            .map_err(sql_err)?;
        Ok(rows
            .iter()
            .map(|r| workflow_invocation_from_row(r))
            .collect())
    }

    fn get_parent_workflow_invocation(
        &self,
        child_instance_id: &str,
    ) -> StoreResult<Option<WorkflowInvocationView>> {
        let sql = format!(
            "{WORKFLOW_INVOCATION_SELECT}WHERE workflow_invocations.child_instance_id = ?1 \
             ORDER BY workflow_invocations.created_at DESC, invocation_id DESC LIMIT 1"
        );
        let rows = self
            .sql
            .query(&sql, &[text(child_instance_id)])
            .map_err(sql_err)?;
        Ok(rows.first().map(|r| workflow_invocation_from_row(r)))
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
        serde_json::from_str::<Value>(skill.required_capabilities_json)?;
        serde_json::from_str::<Value>(skill.metadata_json)?;
        let content_hash = stable_hash_hex(&format!(
            "{}\n{}\n{}\n{}",
            skill.name, skill.version, skill.source_path, skill.source
        ));
        self.sql
            .execute(
                "INSERT INTO skills (skill_id, name, version, source, source_path, \
                 content_hash, description, required_capabilities, metadata_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(name) DO UPDATE SET \
                 version = excluded.version, source = excluded.source, \
                 source_path = excluded.source_path, content_hash = excluded.content_hash, \
                 description = excluded.description, \
                 required_capabilities = excluded.required_capabilities, \
                 metadata_json = excluded.metadata_json",
                &[
                    text(skill.skill_id),
                    text(skill.name),
                    text(skill.version),
                    text(skill.source),
                    text(skill.source_path),
                    text(&content_hash),
                    text(skill.description),
                    text(skill.required_capabilities_json),
                    text(skill.metadata_json),
                ],
            )
            .map_err(sql_err)?;
        Ok(())
    }

    fn attach_skill(&self, attachment: SkillAttachment<'_>) -> StoreResult<()> {
        let rows = self
            .sql
            .query(
                "SELECT skill_id FROM skills WHERE name = ?1",
                &[text(attachment.skill_name)],
            )
            .map_err(sql_err)?;
        let skill_id = rows
            .first()
            .map(|r| as_text(&r[0]))
            .ok_or_else(|| sql_err(format!("no skill named `{}`", attachment.skill_name)))?;
        self.sql
            .execute(
                "INSERT INTO skill_attachments (attachment_id, scope_type, scope_id, skill_id) \
                 VALUES (?1, ?2, ?3, ?4) ON CONFLICT(scope_type, scope_id, skill_id) DO NOTHING",
                &[
                    text(attachment.attachment_id),
                    text(attachment.scope_type),
                    text(attachment.scope_id),
                    text(&skill_id),
                ],
            )
            .map_err(sql_err)?;
        Ok(())
    }

    fn list_skills(&self) -> StoreResult<Vec<SkillView>> {
        let rows = self
            .sql
            .query(
                "SELECT skill_id, name, version, source, source_path, content_hash, \
                 description, required_capabilities FROM skills ORDER BY name",
                &[],
            )
            .map_err(sql_err)?;
        Ok(rows.iter().map(|r| skill_view_from_row(r)).collect())
    }

    fn list_skill_attachments(
        &self,
        scope_type: &str,
        scope_id: &str,
    ) -> StoreResult<Vec<SkillAttachmentView>> {
        let rows = self
            .sql
            .query(
                "SELECT attachment.attachment_id, attachment.scope_type, attachment.scope_id, \
                 skill.skill_id, skill.name, skill.version, skill.source, skill.source_path, \
                 skill.content_hash, skill.description, skill.required_capabilities \
                 FROM skill_attachments AS attachment \
                 JOIN skills AS skill ON skill.skill_id = attachment.skill_id \
                 WHERE attachment.scope_type = ?1 AND attachment.scope_id = ?2 \
                 ORDER BY skill.name",
                &[text(scope_type), text(scope_id)],
            )
            .map_err(sql_err)?;
        Ok(rows
            .iter()
            .map(|r| SkillAttachmentView {
                attachment_id: as_text(&r[0]),
                scope_type: as_text(&r[1]),
                scope_id: as_text(&r[2]),
                skill: skill_view_from_row(&r[3..11]),
            })
            .collect())
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
        serde_json::from_str::<Value>(item.choices_json)?;
        serde_json::from_str::<Value>(item.related_effects_json)?;
        serde_json::from_str::<Value>(item.related_artifacts_json)?;
        self.sql
            .execute(
                "INSERT INTO inbox_items (inbox_item_id, instance_id, effect_id, status, \
                 prompt, choices_json, freeform_allowed, severity, related_effects_json, \
                 related_artifacts_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                &[
                    text(item.inbox_item_id),
                    text(item.instance_id),
                    opt_text(item.effect_id),
                    text(item.status),
                    text(item.prompt),
                    text(item.choices_json),
                    bool_int(item.freeform_allowed),
                    text(item.severity),
                    text(item.related_effects_json),
                    text(item.related_artifacts_json),
                ],
            )
            .map_err(sql_err)?;
        Ok(())
    }

    fn list_inbox_items(&self, status: Option<&str>) -> StoreResult<Vec<InboxItemView>> {
        let mut sql = "SELECT inbox_item_id, instance_id, effect_id, status, prompt, \
             choices_json, freeform_allowed, severity, related_effects_json, \
             related_artifacts_json, answer_json, answered_by, created_at, answered_at \
             FROM inbox_items"
            .to_owned();
        if status.is_some() {
            sql.push_str(" WHERE status = ?1");
        }
        sql.push_str(" ORDER BY created_at, inbox_item_id");
        let params: Vec<SqlValue> = status.map(|s| vec![text(s)]).unwrap_or_default();
        let rows = self.sql.query(&sql, &params).map_err(sql_err)?;
        Ok(rows.iter().map(|r| inbox_item_view_from_row(r)).collect())
    }

    fn get_inbox_item(&self, inbox_item_id: &str) -> StoreResult<Option<InboxItemView>> {
        let rows = self
            .sql
            .query(
                "SELECT inbox_item_id, instance_id, effect_id, status, prompt, choices_json, \
                 freeform_allowed, severity, related_effects_json, related_artifacts_json, \
                 answer_json, answered_by, created_at, answered_at FROM inbox_items \
                 WHERE inbox_item_id = ?1",
                &[text(inbox_item_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.first().map(|r| inbox_item_view_from_row(r)))
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
        let rows = self
            .sql
            .query(
                "SELECT instance_id, program_id, version_id, revision_epoch, \
                 workflow_principal, effective_authority, status, input_json, created_at, \
                 updated_at FROM instances ORDER BY created_at, instance_id",
                &[],
            )
            .map_err(sql_err)?;
        Ok(rows.iter().map(|r| instance_view_from_row(r)).collect())
    }

    fn get_instance(&self, instance_id: &str) -> StoreResult<Option<InstanceView>> {
        let rows = self
            .sql
            .query(
                "SELECT instance_id, program_id, version_id, revision_epoch, \
                 workflow_principal, effective_authority, status, input_json, created_at, \
                 updated_at FROM instances WHERE instance_id = ?1",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.first().map(|r| instance_view_from_row(r)))
    }

    fn list_events(&self, instance_id: &str) -> StoreResult<Vec<EventView>> {
        let rows = self
            .sql
            .query(
                "SELECT event_id, sequence, event_type, payload_json, source, occurred_at \
                 FROM events WHERE instance_id = ?1 ORDER BY sequence",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.iter().map(|r| event_view_from_row(r)).collect())
    }

    fn list_facts(&self, instance_id: &str) -> StoreResult<Vec<FactView>> {
        let rows = self
            .sql
            .query(
                "SELECT fact_id, program_version_id, revision_epoch, name, key, value_json, \
                 provenance_class, source_span_json FROM facts \
                 WHERE instance_id = ?1 AND consumed_at IS NULL ORDER BY name, key",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.iter().map(|r| fact_view_from_row(r)).collect())
    }

    fn list_facts_including_consumed(&self, instance_id: &str) -> StoreResult<Vec<FactView>> {
        let rows = self
            .sql
            .query(
                "SELECT fact_id, program_version_id, revision_epoch, name, key, value_json, \
                 provenance_class, source_span_json FROM facts \
                 WHERE instance_id = ?1 ORDER BY name, key",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.iter().map(|r| fact_view_from_row(r)).collect())
    }

    fn list_effects(&self, instance_id: &str) -> StoreResult<Vec<EffectView>> {
        let rows = self
            .sql
            .query(
                "SELECT effects.effect_id, effects.kind, effects.target, effects.input_json, \
                 effects.status, effects.created_by_rule, effects.program_version_id, \
                 effects.revision_epoch, effects.profile, effects.required_capabilities, \
                 effects.policy_block_reason, effects.policy_block_category, \
                 COALESCE(effect_versions.declared_profiles, active_versions.declared_profiles, '[]'), \
                 EXISTS (SELECT 1 FROM effect_cancellation_requests AS request \
                 WHERE request.instance_id = effects.instance_id \
                 AND request.effect_id = effects.effect_id AND request.status = 'requested') \
                 FROM effects \
                 LEFT JOIN instances ON instances.instance_id = effects.instance_id \
                 LEFT JOIN program_versions AS active_versions \
                 ON active_versions.version_id = instances.version_id \
                 LEFT JOIN program_versions AS effect_versions \
                 ON effect_versions.version_id = effects.program_version_id \
                 WHERE effects.instance_id = ?1 ORDER BY effects.created_at, effects.effect_id",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.iter().map(|r| effect_view_from_row(r)).collect())
    }

    fn list_runs(&self, instance_id: &str) -> StoreResult<Vec<RunView>> {
        let rows = self
            .sql
            .query(
                "SELECT run_id, effect_id, provider, worker_id, status, started_at, \
                 completed_at, metadata_json, \
                 EXISTS (SELECT 1 FROM effect_cancellation_requests AS request \
                 WHERE request.instance_id = runs.instance_id \
                 AND request.effect_id = runs.effect_id AND request.status = 'requested') \
                 FROM runs WHERE runs.instance_id = ?1 ORDER BY started_at, run_id",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.iter().map(|r| run_view_from_row(r)).collect())
    }

    fn status(&self, instance_id: &str) -> StoreResult<Option<StatusView>> {
        let Some(instance) = self.get_instance(instance_id)? else {
            return Ok(None);
        };
        // COUNT(*) over `table` scoped to the instance, with an optional predicate —
        // mirrors the native `count_where` helper.
        let count_where = |table: &str, predicate: Option<&str>| -> StoreResult<i64> {
            let mut sql = format!("SELECT COUNT(*) FROM {table} WHERE instance_id = ?1");
            if let Some(predicate) = predicate {
                sql.push_str(" AND ");
                sql.push_str(predicate);
            }
            let rows = self
                .sql
                .query(&sql, &[text(instance_id)])
                .map_err(sql_err)?;
            Ok(rows.first().map(|r| as_i64(&r[0])).unwrap_or(0))
        };

        let fact_count = count_where("facts", None)?;
        let queued_effect_count = count_where(
            "effects",
            Some("status IN ('queued', 'blocked_by_dependency')"),
        )?;
        let blocked_effect_count = count_where(
            "effects",
            Some(
                "status IN ('blocked_by_capability', 'blocked_by_profile', 'blocked_by_capacity')",
            ),
        )?;
        let active_run_count = count_where("runs", Some("status = 'running'"))?;
        let failure_count = count_where("effects", Some("status IN ('failed', 'timed_out')"))?;
        let cancellation_request_count =
            count_where("effect_cancellation_requests", Some("status = 'requested'"))?;
        let mut recent_events = self.list_events(instance_id)?;
        if recent_events.len() > 5 {
            recent_events = recent_events.split_off(recent_events.len() - 5);
        }
        let revisions = self.list_instance_revisions(instance_id)?;
        let parent_invocation = self.get_parent_workflow_invocation(instance_id)?;
        let child_invocations = self.list_child_workflow_invocations(instance_id)?;

        Ok(Some(StatusView {
            instance,
            fact_count,
            queued_effect_count,
            blocked_effect_count,
            active_run_count,
            failure_count,
            cancellation_request_count,
            revisions,
            parent_invocation,
            child_invocations,
            recent_events,
        }))
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
        self.sql
            .execute(
                "UPDATE facts SET consumed_at = CURRENT_TIMESTAMP, \
                 updated_at = CURRENT_TIMESTAMP \
                 WHERE instance_id = ?1 AND fact_id = ?2 AND consumed_at IS NULL",
                &[text(instance_id), text(fact_id)],
            )
            .map_err(sql_err)?;
        Ok(())
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
        let rows = self
            .sql
            .query(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                &[text(table)],
            )
            .map_err(sql_err)?;
        Ok(!rows.is_empty())
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
            CREATE TABLE facts (
                fact_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, program_version_id TEXT,
                revision_epoch INTEGER NOT NULL DEFAULT 0, name TEXT NOT NULL,
                key TEXT NOT NULL DEFAULT '', value_json TEXT NOT NULL DEFAULT '{}',
                provenance_class TEXT NOT NULL DEFAULT 'derived', source_span_json TEXT,
                consumed_at TEXT, updated_at TEXT
            );
            CREATE TABLE instances (
                instance_id TEXT PRIMARY KEY, program_id TEXT NOT NULL, version_id TEXT NOT NULL,
                revision_epoch INTEGER NOT NULL DEFAULT 0, workflow_principal TEXT NOT NULL,
                effective_authority TEXT NOT NULL, status TEXT NOT NULL, input_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE program_versions (
                version_id TEXT PRIMARY KEY, declared_profiles TEXT NOT NULL DEFAULT '[]'
            );
            CREATE TABLE effects (
                effect_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, kind TEXT NOT NULL,
                target TEXT, input_json TEXT NOT NULL DEFAULT '{}', status TEXT NOT NULL,
                created_by_rule TEXT NOT NULL DEFAULT '', program_version_id TEXT,
                revision_epoch INTEGER NOT NULL DEFAULT 0, profile TEXT,
                required_capabilities TEXT NOT NULL DEFAULT '[]', policy_block_reason TEXT,
                policy_block_category TEXT, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE runs (
                run_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, effect_id TEXT NOT NULL,
                provider TEXT NOT NULL, worker_id TEXT NOT NULL, status TEXT NOT NULL,
                started_at TEXT NOT NULL, completed_at TEXT, metadata_json TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE effect_cancellation_requests (
                request_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, effect_id TEXT NOT NULL,
                revision_id TEXT, reason TEXT, requested_by TEXT NOT NULL DEFAULT 'kernel',
                causation_event_id TEXT, status TEXT NOT NULL, idempotency_key TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, resolved_by_event_id TEXT
            );
            CREATE TABLE instance_revisions (
                revision_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, epoch INTEGER NOT NULL,
                from_version_id TEXT NOT NULL, to_version_id TEXT NOT NULL,
                activated_by_event_id TEXT NOT NULL, activation_policy_json TEXT NOT NULL DEFAULT '{}',
                cancellation_policy TEXT NOT NULL, status TEXT NOT NULL, idempotency_key TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, activated_at TEXT NOT NULL
            );
            CREATE TABLE workflow_invocations (
                invocation_id TEXT PRIMARY KEY, parent_instance_id TEXT NOT NULL,
                parent_effect_id TEXT NOT NULL, parent_program_version_id TEXT,
                parent_revision_epoch INTEGER NOT NULL, child_instance_id TEXT NOT NULL,
                child_program_version_id TEXT, child_revision_epoch INTEGER,
                target_workflow TEXT NOT NULL, input_json TEXT NOT NULL DEFAULT '{}',
                source_span_json TEXT, idempotency_key TEXT UNIQUE, status TEXT NOT NULL DEFAULT 'running',
                terminal_event_id TEXT, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT
            );
            CREATE TABLE skills (
                skill_id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, version TEXT NOT NULL,
                source TEXT NOT NULL, source_path TEXT NOT NULL, content_hash TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '', required_capabilities TEXT NOT NULL DEFAULT '[]',
                metadata_json TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE skill_attachments (
                attachment_id TEXT PRIMARY KEY, scope_type TEXT NOT NULL, scope_id TEXT NOT NULL,
                skill_id TEXT NOT NULL, UNIQUE(scope_type, scope_id, skill_id)
            );
            CREATE TABLE inbox_items (
                inbox_item_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, effect_id TEXT,
                status TEXT NOT NULL, prompt TEXT NOT NULL, choices_json TEXT NOT NULL DEFAULT '[]',
                freeform_allowed INTEGER NOT NULL DEFAULT 1, severity TEXT NOT NULL DEFAULT 'normal',
                related_effects_json TEXT NOT NULL DEFAULT '[]',
                related_artifacts_json TEXT NOT NULL DEFAULT '[]', answer_json TEXT, answered_by TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, answered_at TEXT
            );
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

    /// Skills, inbox items, fact retirement, and table_exists run their real SQL
    /// and round-trip through the ported view mappers.
    #[test]
    fn do_store_skills_inbox_and_facts_run_real_sql() {
        let mut store = store();

        // register_skill validates JSON, computes content_hash, upserts by name;
        // attach_skill resolves skill_id and links a scope; the views round-trip.
        store
            .register_skill(SkillRegistration {
                skill_id: "skl_1",
                name: "triage",
                version: "1.0.0",
                source: "body",
                source_path: "skills/triage.md",
                description: "triage inbox",
                required_capabilities_json: "[]",
                metadata_json: "{}",
            })
            .expect("register_skill");
        assert_eq!(
            stable_hash_hex("triage\n1.0.0\nskills/triage.md\nbody"),
            store.list_skills().expect("list_skills")[0].content_hash,
        );

        store
            .attach_skill(SkillAttachment {
                attachment_id: "att_1",
                scope_type: "instance",
                scope_id: "i1",
                skill_name: "triage",
            })
            .expect("attach_skill");
        // Idempotent re-attach is a no-op (ON CONFLICT DO NOTHING).
        store
            .attach_skill(SkillAttachment {
                attachment_id: "att_2",
                scope_type: "instance",
                scope_id: "i1",
                skill_name: "triage",
            })
            .expect("attach_skill again");
        let attachments = store
            .list_skill_attachments("instance", "i1")
            .expect("list_skill_attachments");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].skill.name, "triage");

        // Attaching an unknown skill errors (native query_row would find no row).
        assert!(store
            .attach_skill(SkillAttachment {
                attachment_id: "att_3",
                scope_type: "instance",
                scope_id: "i1",
                skill_name: "missing",
            })
            .is_err());

        // create_inbox_item validates its 3 JSON fields and stores freeform_allowed
        // as an integer; the list/get views decode it back to a bool.
        store
            .create_inbox_item(NewInboxItem {
                inbox_item_id: "ibx_1",
                instance_id: "i1",
                effect_id: Some("eff_1"),
                status: "pending",
                prompt: "approve?",
                choices_json: "[\"yes\",\"no\"]",
                freeform_allowed: false,
                severity: "normal",
                related_effects_json: "[]",
                related_artifacts_json: "[]",
            })
            .expect("create_inbox_item");
        let pending = store
            .list_inbox_items(Some("pending"))
            .expect("list pending");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].effect_id.as_deref(), Some("eff_1"));
        assert!(!pending[0].freeform_allowed);
        assert!(store
            .list_inbox_items(Some("done"))
            .expect("list done")
            .is_empty());
        let got = store.get_inbox_item("ibx_1").expect("get").expect("some");
        assert_eq!(got.prompt, "approve?");
        assert!(store.get_inbox_item("nope").expect("get missing").is_none());

        // retire_fact marks an unconsumed fact consumed; a second call is a no-op.
        store
            .sql
            .execute(
                "INSERT INTO facts (fact_id, instance_id, name) VALUES (?1, ?2, ?3)",
                &[text("f1"), text("i1"), text("ready")],
            )
            .expect("insert fact");
        store.retire_fact("i1", "f1").expect("retire");
        let consumed = store
            .sql
            .query(
                "SELECT consumed_at FROM facts WHERE fact_id = ?1",
                &[text("f1")],
            )
            .expect("read fact");
        assert!(matches!(consumed[0][0], SqlValue::Text(_)));

        // table_exists reflects the schema.
        assert!(store.table_exists("inbox_items").expect("exists"));
        assert!(!store.table_exists("no_such_table").expect("absent"));
    }

    /// The instance/event/fact/effect/run read-query family runs its real SQL
    /// (including the effect/run join + EXISTS cancel-requested flag) and decodes
    /// rows through the ported view mappers.
    #[test]
    fn do_store_read_query_family_runs_real_sql() {
        let store = store();
        let e = |sql: &str, params: &[SqlValue]| store.sql.execute(sql, params).expect(sql);

        e(
            "INSERT INTO program_versions (version_id, declared_profiles) VALUES (?1, ?2)",
            &[text("ver_1"), text("[\"p\"]")],
        );
        e(
            "INSERT INTO instances (instance_id, program_id, version_id, revision_epoch, \
             workflow_principal, effective_authority, status, input_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            &[
                text("i1"),
                text("prog_1"),
                text("ver_1"),
                int(3),
                text("root"),
                text("{}"),
                text("running"),
                text("{}"),
            ],
        );
        e(
            "INSERT INTO events (event_id, instance_id, sequence, event_type, payload_json, \
             occurred_at, source) VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP, ?6)",
            &[
                text("evt_a"),
                text("i1"),
                int(1),
                text("started"),
                text("{}"),
                text("kernel"),
            ],
        );
        e(
            "INSERT INTO facts (fact_id, instance_id, program_version_id, revision_epoch, name, \
             key, value_json, provenance_class) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            &[
                text("f_live"),
                text("i1"),
                text("ver_1"),
                int(0),
                text("ready"),
                text(""),
                text("true"),
                text("derived"),
            ],
        );
        e(
            "INSERT INTO facts (fact_id, instance_id, name, provenance_class, consumed_at) \
             VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)",
            &[text("f_gone"), text("i1"), text("done"), text("derived")],
        );
        e(
            "INSERT INTO effects (effect_id, instance_id, kind, target, status, program_version_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            &[text("eff_1"), text("i1"), text("coerce"), SqlValue::Null, text("queued"), text("ver_1")],
        );
        e(
            "INSERT INTO runs (run_id, instance_id, effect_id, provider, worker_id, status, \
             started_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)",
            &[
                text("run_1"),
                text("i1"),
                text("eff_1"),
                text("anthropic"),
                text("w1"),
                text("running"),
            ],
        );
        e(
            "INSERT INTO effect_cancellation_requests (instance_id, effect_id, status) \
             VALUES (?1, ?2, ?3)",
            &[text("i1"), text("eff_1"), text("requested")],
        );

        let instances = store.list_instances().expect("list_instances");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].revision_epoch, 3);
        assert_eq!(instances[0].workflow_principal, "root");
        assert_eq!(
            store.get_instance("i1").expect("get").expect("some").status,
            "running"
        );
        assert!(store
            .get_instance("missing")
            .expect("get missing")
            .is_none());

        let events = store.list_events("i1").expect("list_events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 1);

        // list_facts hides the consumed fact; the "including" variant shows both.
        let live = store.list_facts("i1").expect("list_facts");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name, "ready");
        assert_eq!(live[0].program_version_id.as_deref(), Some("ver_1"));
        assert_eq!(
            store
                .list_facts_including_consumed("i1")
                .expect("incl")
                .len(),
            2
        );

        // list_effects joins declared_profiles and computes cancel_requested via EXISTS.
        let effects = store.list_effects("i1").expect("list_effects");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].target, None);
        assert_eq!(effects[0].declared_profiles_json, "[\"p\"]");
        assert!(effects[0].cancel_requested);

        let runs = store.list_runs("i1").expect("list_runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].provider, "anthropic");
        assert!(runs[0].cancel_requested);
    }

    /// Revisions, cancellation-request reads, workflow-invocation record/read (with
    /// the parent/child join), and the composite `status` view all run real SQL.
    #[test]
    fn do_store_revisions_invocations_and_status_run_real_sql() {
        let store = store();
        let e = |sql: &str, params: &[SqlValue]| store.sql.execute(sql, params).expect(sql);

        // Two instances (parent + child) and a version.
        e(
            "INSERT INTO program_versions (version_id, declared_profiles) VALUES (?1, ?2)",
            &[text("ver_1"), text("[]")],
        );
        for id in ["parent", "child"] {
            e(
                "INSERT INTO instances (instance_id, program_id, version_id, revision_epoch, \
                 workflow_principal, effective_authority, status, input_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                &[
                    text(id),
                    text("prog_1"),
                    text("ver_1"),
                    int(1),
                    text("root"),
                    text("{}"),
                    text("running"),
                    text("{}"),
                ],
            );
        }
        // A parent invoke-effect that the invocation points at.
        e(
            "INSERT INTO effects (effect_id, instance_id, kind, status, program_version_id, \
             revision_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            &[
                text("eff_inv"),
                text("parent"),
                text("workflow.invoke"),
                text("running"),
                text("ver_1"),
                int(1),
            ],
        );

        // A revision row round-trips.
        e(
            "INSERT INTO instance_revisions (revision_id, instance_id, epoch, from_version_id, \
             to_version_id, activated_by_event_id, cancellation_policy, status, activated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            &[
                text("rev_1"),
                text("parent"),
                int(1),
                text("ver_0"),
                text("ver_1"),
                text("evt_x"),
                text("keep"),
                text("activated"),
                text("2026-01-01T00:00:00Z"),
            ],
        );
        let revisions = store.list_instance_revisions("parent").expect("revisions");
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].to_version_id, "ver_1");

        // A cancellation request round-trips and the open-check sees it.
        e(
            "INSERT INTO effect_cancellation_requests (request_id, instance_id, effect_id, \
             requested_by, status) VALUES (?1, ?2, ?3, ?4, ?5)",
            &[
                text("req_1"),
                text("parent"),
                text("eff_inv"),
                text("kernel"),
                text("requested"),
            ],
        );
        assert!(store
            .effect_has_open_cancellation_request("parent", "eff_inv")
            .expect("open?"));
        assert!(!store
            .effect_has_open_cancellation_request("parent", "other")
            .expect("open?"));
        assert_eq!(
            store
                .list_effect_cancellation_requests("parent")
                .expect("list reqs")
                .len(),
            1
        );

        // record_workflow_invocation resolves parent effect + child instance versions.
        store
            .record_workflow_invocation(NewWorkflowInvocation {
                invocation_id: "inv_1",
                parent_instance_id: "parent",
                parent_effect_id: "eff_inv",
                child_instance_id: "child",
                target_workflow: "sub",
                input_json: "{}",
                source_span_json: None,
                idempotency_key: "idem_1",
            })
            .expect("record invocation");
        // Idempotent replay is a no-op (ON CONFLICT(idempotency_key) DO NOTHING).
        store
            .record_workflow_invocation(NewWorkflowInvocation {
                invocation_id: "inv_2",
                parent_instance_id: "parent",
                parent_effect_id: "eff_inv",
                child_instance_id: "child",
                target_workflow: "sub",
                input_json: "{}",
                source_span_json: None,
                idempotency_key: "idem_1",
            })
            .expect("replay invocation");

        let got = store
            .get_workflow_invocation("parent", "eff_inv")
            .expect("get inv")
            .expect("some");
        assert_eq!(got.invocation_id, "inv_1");
        assert_eq!(got.child_instance_id, "child");
        assert_eq!(
            got.child_active_program_version_id.as_deref(),
            Some("ver_1")
        );
        assert_eq!(got.parent_active_revision_epoch, Some(1));

        assert_eq!(
            store
                .list_child_workflow_invocations("parent")
                .expect("children")
                .len(),
            1
        );
        assert_eq!(
            store
                .get_parent_workflow_invocation("child")
                .expect("parent inv")
                .expect("some")
                .invocation_id,
            "inv_1"
        );

        // The composite status view assembles counts + reads.
        e(
            "INSERT INTO facts (fact_id, instance_id, name, provenance_class) \
             VALUES (?1, ?2, ?3, ?4)",
            &[text("f1"), text("parent"), text("ready"), text("derived")],
        );
        let status = store.status("parent").expect("status").expect("some");
        assert_eq!(status.instance.instance_id, "parent");
        assert_eq!(status.fact_count, 1);
        assert_eq!(status.cancellation_request_count, 1);
        assert_eq!(status.revisions.len(), 1);
        assert_eq!(status.child_invocations.len(), 1);
        assert!(store.status("ghost").expect("missing status").is_none());
    }
}
