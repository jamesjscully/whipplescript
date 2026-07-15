//! The builtin work-item tracker: the reference implementation of the
//! work-queue interface (`spec/work-queues.md`), rebuilt as an event-sourced
//! provider (ADR-0002 v1, A+blockers scope).
//!
//! State model (the ADR cure for the old row-store): the source of truth is an
//! APPEND-ONLY transaction log (`tracker_events`) — every mutation is an
//! immutable event, never an in-place row update. Current issue state
//! (`tracker_issues`), blocker edges (`tracker_relations`), and runtime claim
//! leases (`tracker_leases`) are DISPOSABLE projections folded from that log; a
//! rebuild-from-events reproduces them exactly (`rebuild_projection`).
//!
//! Claims are LOCAL runtime leases, split from durable issue status (the
//! "combined claim/status write" cure): a plain `claim` appends only a
//! `claim.acquired` event and changes readiness through a lease OVERLAY, not a
//! durable `in_progress` write. Durable issue status is only `open` / `closed`
//! / `canceled`; `in_progress` is purely the active-lease overlay.
//!
//! The three invariant models under `models/maude/` are the spec this storage
//! realizes: exclusivity + expiry + holder-only renew + terminal-release
//! (`tracker-lease`), the readiness fold (`tracker-readiness`), and projection
//! determinism (`tracker-projection`).
//!
//! Items live in a workspace-scoped SQLite file (default
//! `.whipplescript/items.sqlite`), deliberately separate from run stores:
//! run stores are disposable per experiment, the backlog is durable.

#[cfg(feature = "native")]
use std::path::Path;

#[cfg(feature = "native")]
use rusqlite::{params, Connection, OptionalExtension, Transaction};
#[cfg(feature = "native")]
use serde_json::json;
use serde_json::Value;

#[cfg(feature = "native")]
use crate::StoreError;
use crate::StoreResult;

/// Core status categories. Durable status is one of `open` / `closed` /
/// `canceled`; `in_progress` is the active-lease overlay, never a durable
/// write, and `ready` is a derived predicate (open, unblocked, unleased).
pub const ITEM_STATUSES: &[&str] = &["open", "in_progress", "closed", "canceled", "archived"];

/// The active-lease predicate, shared by every readiness/overlay query: a lease
/// is active while it has not been released and has not expired. A NULL
/// `expires_at` models a lease with no TTL (the old builtin "no TTL backstop"
/// behavior) — it never auto-expires. `?N` binds the clock (`datetime('now')`
/// or a captured now-timestamp).
#[cfg(feature = "native")]
const ACTIVE_LEASE: &str = "released_at IS NULL AND (expires_at IS NULL OR expires_at > ?)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkItem {
    pub id: String,
    pub queue: String,
    pub title: String,
    pub body: String,
    pub status: String,
    pub labels: Vec<String>,
    pub metadata: Value,
    pub claimed_by: Option<String>,
    pub filed_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// One field whose value is disputed: its `bef`-maximal setters in the event
/// DAG disagree (ADR-0002 phase B1 slice ii; `tracker-merge.maude` `conflict`).
/// `values` are the distinct maximal-setter values, sorted for stable output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldConflict {
    pub field: String,
    pub values: Vec<String>,
}

/// The DAG view of one issue: its frontier (`heads`), a content-derived
/// `state_token` over that frontier (the optimistic-concurrency token, slice v),
/// and any per-field conflicts. An issue is `conflicted` iff `field_conflicts`
/// is non-empty — and a conflicted issue is not ready. Heads and token are
/// content-hashes (unit 1), so they are already merge-stable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueConflicts {
    pub heads: Vec<String>,
    pub state_token: String,
    pub field_conflicts: Vec<FieldConflict>,
}

impl IssueConflicts {
    #[must_use]
    pub fn conflicted(&self) -> bool {
        !self.field_conflicts.is_empty()
    }
}

/// The outcome of an optimistic field set (ADR-0002 phase B1 slice v): a set
/// guarded by the `state_token` the caller last observed. `StateChanged` means
/// the frontier moved under the caller — the set was NOT applied, and `actual`
/// is the current token to retry against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetFieldOutcome {
    Applied { state_token: String },
    NotFound,
    StateChanged { actual: String },
}

/// One tracker event in transport form (ADR-0002 phase B1 slice iii): the
/// content-addressed unit that crosses between clones. `issue_id` is the opaque
/// content_id (never a WS-N alias), so a set-union of two clones' events is
/// well-defined and deduped by `event_id`.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrackerEvent {
    pub event_id: String,
    #[serde(default)]
    pub parents: Vec<String>,
    pub issue_id: Option<String>,
    pub kind: String,
    pub payload_json: String,
    pub actor: Option<String>,
    pub created_at: String,
}

/// The result of importing another clone's events (ADR-0002 phase B1 slice iii).
/// `duplicate_submissions` names each byte-identical `issue.created` that
/// collapsed onto an existing one — surfaced as a WARNING, never silently
/// dropped, because a silent collapse could corrupt workflow integrity.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportReport {
    pub imported: usize,
    pub skipped: usize,
    pub new_issues: usize,
    pub duplicate_submissions: Vec<String>,
}

#[cfg(feature = "native")]
pub struct WorkItemStore {
    connection: Connection,
}

#[cfg(feature = "native")]
impl WorkItemStore {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(TRACKER_SCHEMA_SQL)?;
        // Self-heal a pre-phase-B `tracker_events` (the ADR-0002 v1 linear log
        // had neither column): `CREATE TABLE IF NOT EXISTS` never alters an
        // existing table, so add the Merkle-DAG columns before the unique index
        // over `event_id` (which would otherwise fail on the old shape).
        tx_ensure_column(&connection, "tracker_events", "event_id", "TEXT")?;
        tx_ensure_column(
            &connection,
            "tracker_events",
            "parents_json",
            "TEXT NOT NULL DEFAULT '[]'",
        )?;
        connection.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_tracker_events_id ON tracker_events(event_id);",
        )?;
        Ok(Self { connection })
    }

    /// Files an item, minting a sequential human-speakable id (`WS-1`,
    /// `WS-2`, ...). Sequential beats content hashes: "take WS-7" is
    /// speakable to an agent, and byte-identical items get distinct ids.
    /// Appends `issue.created` and folds it into the issue projection in one
    /// transaction.
    pub fn file_item(
        &mut self,
        queue: &str,
        title: &str,
        body: &str,
        labels: &[String],
        metadata: &Value,
        filed_by: Option<&str>,
    ) -> StoreResult<WorkItem> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = tx_now(&tx)?;
        let next: i64 = tx.query_row(
            "UPDATE tracker_counter SET next_id = next_id + 1 WHERE singleton = 1 RETURNING next_id - 1",
            [],
            |row| row.get(0),
        )?;
        let item_id = format!("WS-{next}");
        let labels_json = serde_json::to_string(labels)?;
        let metadata_json = metadata.to_string();
        let payload = json!({
            "queue": queue,
            "title": title,
            "body": body,
            "labels": labels,
            "metadata": metadata,
            "filed_by": filed_by,
        });
        let payload_json = payload.to_string();
        // The issue's opaque MERGE identity = the content-hash of its creation
        // event (issue_id excluded — it derives FROM this). WS-N is only a local
        // alias for it; the event log is keyed by content_id.
        let content_id =
            event_content_id("issue.created", None, &payload_json, filed_by, &[], &now);
        tx.execute(
            "INSERT INTO tracker_aliases (content_id, alias) VALUES (?1, ?2)",
            params![content_id, item_id],
        )?;
        tx_append_raw(
            &tx,
            Some(&content_id),
            Some(&content_id),
            "issue.created",
            &payload_json,
            filed_by,
            &now,
        )?;
        tx.execute(
            "INSERT INTO tracker_issues \
             (issue_id, queue, title, body, status, labels_json, metadata_json, filed_by, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7, ?8, ?8)",
            params![item_id, queue, title, body, labels_json, metadata_json, filed_by, now],
        )?;
        tx.commit()?;
        self.get_item(&item_id)?
            .ok_or_else(|| StoreError::Conflict("filed item missing".to_owned()))
    }

    pub fn get_item(&self, item_id: &str) -> StoreResult<Option<WorkItem>> {
        let base = self
            .connection
            .query_row(
                &format!("SELECT {ISSUE_COLS} FROM tracker_issues WHERE issue_id = ?1"),
                [item_id],
                row_to_item,
            )
            .optional()?;
        match base {
            None => Ok(None),
            Some(item) => {
                let holder = self.active_holder(item_id)?;
                Ok(Some(apply_overlay(item, holder)))
            }
        }
    }

    pub fn list_items(
        &self,
        queue: Option<&str>,
        status: Option<&str>,
    ) -> StoreResult<Vec<WorkItem>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {ISSUE_COLS} FROM tracker_issues \
             WHERE (?1 IS NULL OR queue = ?1) ORDER BY created_at, issue_id"
        ))?;
        let bases = statement
            .query_map(params![queue], row_to_item)?
            .collect::<Result<Vec<_>, _>>()?;
        let holders = self.active_holders()?;
        let items = bases
            .into_iter()
            .map(|item| {
                let holder = holders.get(&item.id).cloned();
                apply_overlay(item, holder)
            })
            // The overlay can turn a durable-`open` issue into effective
            // `in_progress`, so the caller's status filter must run over the
            // OVERLAID status, not the durable column.
            .filter(|item| status.is_none_or(|want| item.status == want))
            .collect();
        Ok(items)
    }

    /// Readiness is the tracker's promise (`tracker-readiness.maude`): ready
    /// iff durable status is `open`, no ACTIVE blocker (`blocks(B, id)` with `B`
    /// still open), and no ACTIVE lease. Expired/released leases do not block.
    pub fn ready_items(&self, queue: &str) -> StoreResult<Vec<WorkItem>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {ISSUE_COLS} FROM tracker_issues i \
             WHERE i.queue = ?1 AND i.status = 'open' \
             AND NOT EXISTS ( \
               SELECT 1 FROM tracker_leases l \
               WHERE l.issue_id = i.issue_id \
                 AND l.released_at IS NULL \
                 AND (l.expires_at IS NULL OR l.expires_at > datetime('now'))) \
             AND NOT EXISTS ( \
               SELECT 1 FROM tracker_relations r JOIN tracker_issues b ON b.issue_id = r.from_issue \
               WHERE r.to_issue = i.issue_id AND r.kind = 'blocks' AND b.status = 'open') \
             ORDER BY i.created_at, i.issue_id"
        ))?;
        let rows = statement
            .query_map([queue], row_to_item)?
            .collect::<Result<Vec<_>, _>>()?;
        // A conflicted issue is not ready (ADR-0002 phase B1 slice ii): its
        // field values are in dispute, so handing it to a worker would race a
        // resolution. The DAG conflict test is not expressible in the SQL
        // predicate above, so filter it here (the candidate set is small).
        let mut ready = Vec::with_capacity(rows.len());
        for item in rows {
            let conflicted = match content_id_of(&self.connection, &item.id)? {
                Some(content_id) => {
                    analyze_issue_dag(&load_issue_events(&self.connection, &content_id)?)
                        .conflicted()
                }
                None => false,
            };
            if !conflicted {
                ready.push(item);
            }
        }
        // Ready items have no active lease by construction, so the overlay is a
        // no-op here; return them as the projection sees them (durable `open`,
        // unclaimed).
        Ok(ready)
    }

    /// Atomic claim (`tracker-lease.maude` I1, exclusivity): grants a lease ONLY
    /// when the issue carries no active lease. The `Immediate` transaction takes
    /// the write lock at `BEGIN`, so the "is there an active lease?" check and
    /// the lease insert are serialized against every concurrent claim — exactly
    /// one wins, the rest see `AlreadyClaimed`. "Already claimed" is a normal,
    /// branchable outcome, not an error.
    /// `expires` is an ABSOLUTE deadline (`None` = no TTL, the historical
    /// backstop behavior — the lease never auto-expires and terminal
    /// auto-release is the only recovery). A finite `expires` records a
    /// claim-TTL lease that `ready`/`claim` lazily reclaim once past-due
    /// (`tracker-lease.maude`: expired leases do not block). The T3 claim-TTL
    /// half of the renew mechanism.
    pub fn claim_item(
        &mut self,
        item_id: &str,
        claimed_by: &str,
        expires: Option<&str>,
    ) -> StoreResult<ClaimOutcome> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = tx_now(&tx)?;
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM tracker_issues WHERE issue_id = ?1",
                [item_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            tx.commit()?;
            return Ok(ClaimOutcome::NotFound);
        }
        tx_expire_stale_leases(&tx, item_id, &now)?;
        if let Some(holder) = tx_active_holder(&tx, item_id, &now)? {
            tx.commit()?;
            return Ok(ClaimOutcome::AlreadyClaimed { holder });
        }
        // No active lease: mint a stable, rebuild-deterministic lease id (the
        // k-th acquire on this issue) and grant. A plain claim writes NO durable
        // status — readiness changes through the lease overlay. The acquire count
        // is over the event log, which is keyed by the opaque content_id.
        let content_id = content_id_of(&tx, item_id)?
            .ok_or_else(|| StoreError::Conflict(format!("unknown issue alias {item_id}")))?;
        let n: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tracker_events WHERE issue_id = ?1 AND kind = 'claim.acquired'",
            [&content_id],
            |row| row.get(0),
        )?;
        let lease_id = format!("L-{item_id}-{n}");
        let payload = json!({"lease_id": lease_id, "actor": claimed_by, "expires_at": expires});
        tx_append_event(
            &tx,
            Some(item_id),
            "claim.acquired",
            &payload,
            Some(claimed_by),
            &now,
        )?;
        tx.execute(
            "INSERT INTO tracker_leases (lease_id, issue_id, actor, acquired_at, expires_at, released_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
            params![lease_id, item_id, claimed_by, now, expires],
        )?;
        tx.commit()?;
        Ok(ClaimOutcome::Claimed)
    }

    /// Extend/heartbeat a held lease (`tracker-lease.maude` I2, holder-only +
    /// monotonic). Only the actor that holds the active lease may renew; a
    /// finite `expires` may only move FORWARD (a non-monotonic request is
    /// rejected). `expires = None` re-affirms the lease without changing its
    /// deadline. The T3 sanctioned extension.
    pub fn renew_claim(
        &mut self,
        item_id: &str,
        actor: &str,
        expires: Option<&str>,
    ) -> StoreResult<RenewOutcome> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = tx_now(&tx)?;
        let lease: Option<(String, Option<String>)> = tx
            .query_row(
                &format!(
                    "SELECT lease_id, expires_at FROM tracker_leases \
                     WHERE issue_id = ?1 AND actor = ?2 AND {ACTIVE_LEASE}"
                ),
                params![item_id, actor, now],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((lease_id, current_expires)) = lease else {
            tx.commit()?;
            return Ok(RenewOutcome::NotHeld);
        };
        // Monotonicity: a finite deadline may not move backward. NULL (no TTL)
        // accepts a first finite deadline — the holder is voluntarily timing
        // its own lease, which the Maude model (Nat-only expiry) does not cover.
        if let (Some(want), Some(current)) = (expires, current_expires.as_deref()) {
            if want <= current {
                tx.commit()?;
                return Ok(RenewOutcome::NotMonotonic);
            }
        }
        let new_expires: Option<String> = match expires {
            Some(want) => Some(want.to_owned()),
            None => current_expires,
        };
        let payload = json!({"lease_id": lease_id, "actor": actor, "expires_at": new_expires});
        tx_append_event(
            &tx,
            Some(item_id),
            "claim.renewed",
            &payload,
            Some(actor),
            &now,
        )?;
        if expires.is_some() {
            tx.execute(
                "UPDATE tracker_leases SET expires_at = ?2 WHERE lease_id = ?1",
                params![lease_id, new_expires],
            )?;
        }
        tx.commit()?;
        Ok(RenewOutcome::Renewed {
            expires_at: new_expires,
        })
    }

    pub fn release_item(&mut self, item_id: &str) -> StoreResult<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = tx_now(&tx)?;
        let released = tx_release_active_lease(&tx, item_id, &now)?;
        tx.commit()?;
        Ok(released)
    }

    /// Terminal-releases-all (`tracker-lease.maude` I3, E7 non-opt-out): every
    /// active lease the actor holds across ALL issues is released in one
    /// transaction, so no intermediate state keeps a held lease.
    pub fn release_claims_for_holder(&mut self, holder: &str) -> StoreResult<usize> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = tx_now(&tx)?;
        let leases: Vec<(String, String)> = tx
            .prepare(&format!(
                "SELECT lease_id, issue_id FROM tracker_leases WHERE actor = ?1 AND {ACTIVE_LEASE}"
            ))?
            .query_map(params![holder, now], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        for (lease_id, issue_id) in &leases {
            tx_mark_lease_released(&tx, lease_id, issue_id, "claim.released", holder, &now)?;
        }
        tx.commit()?;
        Ok(leases.len())
    }

    /// Marks the item done (`issue.closed`), records the optional summary, and
    /// releases any active lease. Finishable only from durable `open`.
    pub fn finish_item(&mut self, item_id: &str, summary: Option<&str>) -> StoreResult<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = tx_now(&tx)?;
        let status: Option<String> = tx
            .query_row(
                "SELECT status FROM tracker_issues WHERE issue_id = ?1",
                [item_id],
                |row| row.get(0),
            )
            .optional()?;
        if status.as_deref() != Some("open") {
            tx.commit()?;
            return Ok(false);
        }
        let payload = json!({"status": "closed", "summary": summary});
        tx_append_event(&tx, Some(item_id), "issue.closed", &payload, None, &now)?;
        tx.execute(
            "UPDATE tracker_issues SET status = 'closed', claim_summary = ?2, updated_at = ?3 \
             WHERE issue_id = ?1",
            params![item_id, summary, now],
        )?;
        tx_release_active_lease(&tx, item_id, &now)?;
        tx.commit()?;
        Ok(true)
    }

    /// Records a `blocks(from -> to)` edge: `from` blocks `to`, so `to` is not
    /// ready until `from` closes (`tracker-readiness.maude`). Appends a
    /// `relation.added` event and folds it into `tracker_relations` in one
    /// transaction; idempotent via `INSERT OR IGNORE`. The `whip issue dep add`
    /// door (blocked depends-on blocker => `add_blocks(blocker, blocked)`).
    pub fn add_blocks(&mut self, from: &str, to: &str) -> StoreResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = tx_now(&tx)?;
        // The event payload references issues by opaque content_id (merge-stable
        // across clones); the projection keeps aliases (the readiness join is
        // alias-keyed and clone-local).
        let from_cid = content_id_of(&tx, from)?
            .ok_or_else(|| StoreError::Conflict(format!("unknown issue alias {from}")))?;
        let to_cid = content_id_of(&tx, to)?
            .ok_or_else(|| StoreError::Conflict(format!("unknown issue alias {to}")))?;
        let payload = json!({"from": from_cid, "to": to_cid, "kind": "blocks"});
        tx_append_event(&tx, Some(to), "relation.added", &payload, None, &now)?;
        tx.execute(
            "INSERT OR IGNORE INTO tracker_relations (from_issue, to_issue, kind, dep_kind) \
             VALUES (?1, ?2, 'blocks', NULL)",
            params![from, to],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Sets one field of an issue (`issue.field_set`) — the mutation whose
    /// events the conflict engine folds. Appends the event (chaining onto the
    /// issue's current heads, so two independent sets across a merge FORK the
    /// DAG) and updates the linear projection column for the known display
    /// fields. Returns `false` if the issue does not exist.
    pub fn set_field(&mut self, item_id: &str, field: &str, value: &str) -> StoreResult<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = tx_now(&tx)?;
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM tracker_issues WHERE issue_id = ?1",
                [item_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            tx.commit()?;
            return Ok(false);
        }
        tx_apply_field_set(&tx, item_id, field, value, &now)?;
        tx.commit()?;
        Ok(true)
    }

    /// Optimistic field set (ADR-0002 phase B1 slice v): apply only if the
    /// issue's current `state_token` still equals `expect_token`. The check and
    /// the append share one `Immediate` transaction, so a concurrent writer
    /// cannot slip a change in between — a stale token is refused with the
    /// current `actual`, never silently overwriting the other change.
    pub fn set_field_checked(
        &mut self,
        item_id: &str,
        field: &str,
        value: &str,
        expect_token: &str,
    ) -> StoreResult<SetFieldOutcome> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = tx_now(&tx)?;
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM tracker_issues WHERE issue_id = ?1",
                [item_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            tx.commit()?;
            return Ok(SetFieldOutcome::NotFound);
        }
        let content_id = content_id_of(&tx, item_id)?
            .ok_or_else(|| StoreError::Conflict(format!("unknown issue alias {item_id}")))?;
        let current = analyze_issue_dag(&load_issue_events(&tx, &content_id)?).state_token;
        if current != expect_token {
            tx.commit()?;
            return Ok(SetFieldOutcome::StateChanged { actual: current });
        }
        tx_apply_field_set(&tx, item_id, field, value, &now)?;
        let after = analyze_issue_dag(&load_issue_events(&tx, &content_id)?).state_token;
        tx.commit()?;
        Ok(SetFieldOutcome::Applied { state_token: after })
    }

    /// The DAG conflict view of one issue (ADR-0002 phase B1 slice ii): its
    /// `heads`, the `state_token` over that frontier, and any field whose
    /// `bef`-maximal setters disagree. `None` if the issue does not exist.
    pub fn issue_conflicts(&self, item_id: &str) -> StoreResult<Option<IssueConflicts>> {
        let exists: bool = self
            .connection
            .query_row(
                "SELECT 1 FROM tracker_issues WHERE issue_id = ?1",
                [item_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Ok(None);
        }
        let Some(content_id) = content_id_of(&self.connection, item_id)? else {
            return Ok(None);
        };
        let events = load_issue_events(&self.connection, &content_id)?;
        Ok(Some(analyze_issue_dag(&events)))
    }

    /// Rebuilds the disposable projections (`tracker_issues`,
    /// `tracker_relations`, `tracker_leases`) by folding the append-only event
    /// log from empty (`tracker-projection.maude` determinism). A rebuild
    /// reproduces the live projection exactly, because live writes and the fold
    /// derive every field — including timestamps — from the same event rows.
    pub fn rebuild_projection(&mut self) -> StoreResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "DELETE FROM tracker_issues; DELETE FROM tracker_relations; DELETE FROM tracker_leases;",
        )?;
        // The event log is keyed by opaque content_id; the projections are
        // alias-keyed. `tracker_aliases` (durable, NOT wiped) is the bridge.
        let alias_of: std::collections::HashMap<String, String> = tx
            .prepare("SELECT content_id, alias FROM tracker_aliases")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let events: Vec<(Option<String>, String, String, String)> = tx
            .prepare(
                "SELECT issue_id, kind, payload_json, created_at FROM tracker_events ORDER BY event_seq",
            )?
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (content_id, kind, payload_json, created_at) in &events {
            let payload: Value = serde_json::from_str(payload_json).unwrap_or_else(|_| json!({}));
            let issue_alias = content_id
                .as_deref()
                .and_then(|c| alias_of.get(c))
                .map(String::as_str);
            fold_event(&tx, issue_alias, kind, &payload, created_at, &alias_of)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Export every event in transport form (ADR-0002 phase B1 slice iii) — the
    /// content-addressed log another clone unions in. Ordered by append seq.
    pub fn export_events(&self) -> StoreResult<Vec<TrackerEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT event_id, parents_json, issue_id, kind, payload_json, actor, created_at \
             FROM tracker_events ORDER BY event_seq",
        )?;
        let rows = statement
            .query_map([], |row| {
                let event_id: Option<String> = row.get(0)?;
                let parents_json: String = row.get(1)?;
                Ok(TrackerEvent {
                    event_id: event_id.unwrap_or_default(),
                    parents: serde_json::from_str(&parents_json).unwrap_or_default(),
                    issue_id: row.get(2)?,
                    kind: row.get(3)?,
                    payload_json: row.get(4)?,
                    actor: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Merge another clone's events into this one (ADR-0002 phase B1 slice iii):
    /// a set-union of the content-addressed log, deduped by `event_id`. Two
    /// clones that edited the SAME issue (same content_id) from a shared parent
    /// FORK its DAG — the conflict engine then surfaces the disagreement. Newly
    /// seen issues are RE-ALIASED locally (this clone's WS-N, independent of the
    /// origin's). A byte-identical `issue.created` that collapses onto one we
    /// already hold is reported in `duplicate_submissions` — a warning, never a
    /// silent collapse. Projections are rebuilt from the unioned log.
    pub fn import_events(&mut self, events: &[TrackerEvent]) -> StoreResult<ImportReport> {
        let mut report = ImportReport::default();
        {
            let tx = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            for event in events {
                let parents_json = serde_json::to_string(&event.parents)?;
                let changes = tx.execute(
                    "INSERT OR IGNORE INTO tracker_events \
                     (event_id, parents_json, issue_id, kind, payload_json, actor, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        event.event_id,
                        parents_json,
                        event.issue_id,
                        event.kind,
                        event.payload_json,
                        event.actor,
                        event.created_at,
                    ],
                )?;
                if changes == 1 {
                    report.imported += 1;
                } else {
                    // Already present (deduped). A re-submitted creation is a
                    // duplicate submission — surface it, never collapse silently.
                    report.skipped += 1;
                    if event.kind == "issue.created" {
                        report.duplicate_submissions.push(
                            event
                                .issue_id
                                .clone()
                                .unwrap_or_else(|| event.event_id.clone()),
                        );
                    }
                }
            }
            // Re-alias every newly-seen issue (has a created event, no local
            // alias yet) in append order, minting this clone's own WS-N.
            let unaliased: Vec<String> = tx
                .prepare(
                    "SELECT issue_id FROM tracker_events \
                     WHERE kind = 'issue.created' AND issue_id IS NOT NULL \
                       AND issue_id NOT IN (SELECT content_id FROM tracker_aliases) \
                     GROUP BY issue_id ORDER BY MIN(event_seq)",
                )?
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for content_id in &unaliased {
                let next: i64 = tx.query_row(
                    "UPDATE tracker_counter SET next_id = next_id + 1 WHERE singleton = 1 RETURNING next_id - 1",
                    [],
                    |row| row.get(0),
                )?;
                tx.execute(
                    "INSERT INTO tracker_aliases (content_id, alias) VALUES (?1, ?2)",
                    params![content_id, format!("WS-{next}")],
                )?;
                report.new_issues += 1;
            }
            tx.commit()?;
        }
        // Materialize projections from the unioned log (folds forks in on read).
        self.rebuild_projection()?;
        Ok(report)
    }

    fn active_holder(&self, item_id: &str) -> StoreResult<Option<String>> {
        self.connection
            .query_row(
                &format!(
                    "SELECT actor FROM tracker_leases WHERE issue_id = ?1 AND {} \
                     ORDER BY acquired_at DESC LIMIT 1",
                    ACTIVE_LEASE.replace('?', "datetime('now')")
                ),
                [item_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    fn active_holders(&self) -> StoreResult<std::collections::HashMap<String, String>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT issue_id, actor FROM tracker_leases WHERE {}",
            ACTIVE_LEASE.replace('?', "datetime('now')")
        ))?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<std::collections::HashMap<String, String>, _>>()?;
        Ok(rows)
    }
}

/// The append-only tracker schema (native file and the DO share this shape).
/// `tracker_events` is INSERT-only — the source of truth; `tracker_issues` /
/// `tracker_relations` / `tracker_leases` are disposable projections folded from
/// it. `tracker_aliases` is NOT a projection: it is durable clone-local naming
/// state (content-hash issue id ↔ human `WS-N`), survives a rebuild, and is
/// re-assigned locally on merge-import — the WS-N of clone A is not the WS-N of
/// clone B. `tracker_counter` mints the sequential `WS-N`.
///
/// Identity (ADR-0002 phase B1 slice i unit 2): an issue's MERGE identity is the
/// content-hash of its `issue.created` event (`content_id`), carried in every
/// event's `issue_id` and in relation payloads, so two clones' logs union
/// correctly. `WS-N` is only a local alias for a human; the projection tables
/// stay keyed by it (clone-local), the event log by the opaque `content_id`.
#[cfg(feature = "native")]
const TRACKER_SCHEMA_SQL: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS tracker_events (
    event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT,
    parents_json TEXT NOT NULL DEFAULT '[]',
    issue_id TEXT,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS tracker_issues (
    issue_id TEXT PRIMARY KEY,
    queue TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open',
    labels_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}',
    claim_summary TEXT,
    filed_by TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS tracker_relations (
    from_issue TEXT NOT NULL,
    to_issue TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'blocks',
    dep_kind TEXT,
    PRIMARY KEY (from_issue, to_issue, kind)
);
CREATE TABLE IF NOT EXISTS tracker_leases (
    lease_id TEXT PRIMARY KEY,
    issue_id TEXT NOT NULL,
    actor TEXT NOT NULL,
    acquired_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TEXT,
    released_at TEXT
);
CREATE TABLE IF NOT EXISTS tracker_counter (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    next_id INTEGER NOT NULL
);
INSERT OR IGNORE INTO tracker_counter (singleton, next_id) VALUES (1, 1);
CREATE TABLE IF NOT EXISTS tracker_aliases (
    content_id TEXT PRIMARY KEY,
    alias TEXT NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS idx_tracker_issues_queue ON tracker_issues(queue, status);
CREATE INDEX IF NOT EXISTS idx_tracker_leases_issue ON tracker_leases(issue_id, released_at);
CREATE INDEX IF NOT EXISTS idx_tracker_events_issue ON tracker_events(issue_id, kind);
"#;

/// Add a column to a table if it is missing (`CREATE TABLE IF NOT EXISTS` never
/// alters an existing table). Idempotent — the schema self-heals across phase
/// upgrades without a migration file.
#[cfg(feature = "native")]
fn tx_ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> StoreResult<()> {
    let present = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == column);
    if !present {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

/// Projection columns in `WorkItem` order (see `row_to_item`).
#[cfg(feature = "native")]
const ISSUE_COLS: &str = "issue_id, queue, title, body, status, labels_json, metadata_json, \
     filed_by, created_at, updated_at";

/// Capture a single now-timestamp for one mutating op; every event + projection
/// field derived from this op uses it, so a rebuild reproduces the same values.
#[cfg(feature = "native")]
fn tx_now(tx: &Transaction<'_>) -> StoreResult<String> {
    tx.query_row("SELECT datetime('now')", [], |row| row.get(0))
        .map_err(Into::into)
}

/// The SHA-256 content id of an event (ADR-0002 phase B1: the tracker event
/// Merkle-DAG). The id commits to the event's whole content INCLUDING its
/// sorted parent ids, so the log is a hash-chain: altering any past event
/// changes its id and breaks every downstream `parents` link — a tampered
/// issue is DETECTABLE, the adversarial-integrity property FNV content-addressing
/// cannot give. SHA-256 (not FNV) precisely because the threat is a deliberate
/// forger, who could otherwise compute a colliding event. Two byte-identical
/// events (same kind/issue/payload/actor/parents/clock) share an id and dedup on
/// merge; the distinguishing `created_at` keeps genuine re-submissions distinct.
#[cfg(feature = "native")]
fn event_content_id(
    kind: &str,
    issue_id: Option<&str>,
    payload_json: &str,
    actor: Option<&str>,
    parents: &[String],
    created_at: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut sorted = parents.to_vec();
    sorted.sort();
    // A field-separated canonical form; the fields cannot themselves contain the
    // record separator (0x1e), so the encoding is injective.
    let material = [
        kind,
        issue_id.unwrap_or(""),
        payload_json,
        actor.unwrap_or(""),
        &sorted.join(","),
        created_at,
    ]
    .join("\u{1e}");
    format!("{:x}", Sha256::digest(material.as_bytes()))
}

/// The current head event id(s) of an issue — events with no child (nothing
/// lists them as a parent). A new event's parents. Under single-writer appends
/// (this store) there is exactly one head, the latest event; the DAG can only
/// FORK when a merge imports a divergent log (phase B1 slice iii), which is when
/// multiple heads arise. `None` (no prior event) roots the issue's history.
#[cfg(feature = "native")]
fn tx_issue_heads(tx: &Transaction<'_>, issue_id: &str) -> StoreResult<Vec<String>> {
    let heads: Vec<String> = tx
        .prepare(
            "SELECT e.event_id FROM tracker_events e \
             WHERE e.issue_id = ?1 AND e.event_id IS NOT NULL \
               AND NOT EXISTS ( \
                 SELECT 1 FROM tracker_events c \
                 WHERE c.issue_id = ?1 \
                   AND instr(c.parents_json, '\"' || e.event_id || '\"') > 0)",
        )?
        .query_map(params![issue_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(heads)
}

/// Resolve a human `WS-N` alias to its opaque `content_id` merge identity.
#[cfg(feature = "native")]
fn content_id_of(conn: &Connection, alias: &str) -> StoreResult<Option<String>> {
    conn.query_row(
        "SELECT content_id FROM tracker_aliases WHERE alias = ?1",
        [alias],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Append one immutable event keyed by the issue's opaque `content_id` (INSERT
/// only). Its parents are the issue's current heads; its `event_id` is the
/// content hash over the whole event, so the log is a Merkle-DAG. Pass
/// `event_id_override` only for the `issue.created` root, whose id IS the issue's
/// `content_id` (identity = the creation event). Deduped by `event_id` on merge.
#[cfg(feature = "native")]
fn tx_append_raw(
    tx: &Transaction<'_>,
    issue_content_id: Option<&str>,
    event_id_override: Option<&str>,
    kind: &str,
    payload_json: &str,
    actor: Option<&str>,
    now: &str,
) -> StoreResult<String> {
    let parents = match issue_content_id {
        Some(id) => tx_issue_heads(tx, id)?,
        None => Vec::new(),
    };
    let event_id = match event_id_override {
        Some(id) => id.to_owned(),
        None => event_content_id(kind, issue_content_id, payload_json, actor, &parents, now),
    };
    let parents_json = serde_json::to_string(&parents)?;
    tx.execute(
        "INSERT OR IGNORE INTO tracker_events \
         (event_id, parents_json, issue_id, kind, payload_json, actor, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event_id,
            parents_json,
            issue_content_id,
            kind,
            payload_json,
            actor,
            now
        ],
    )?;
    Ok(event_id)
}

/// Append one event for an issue given by its `WS-N` alias — the door for every
/// mutation the CLI drives (which speaks WS-N). Resolves the alias to the opaque
/// `content_id` the event log is keyed by, so the log stays merge-stable while
/// callers keep using the human handle.
#[cfg(feature = "native")]
fn tx_append_event(
    tx: &Transaction<'_>,
    alias: Option<&str>,
    kind: &str,
    payload: &Value,
    actor: Option<&str>,
    now: &str,
) -> StoreResult<()> {
    let content_id = match alias {
        Some(a) => Some(
            content_id_of(tx, a)?
                .ok_or_else(|| StoreError::Conflict(format!("unknown issue alias {a}")))?,
        ),
        None => None,
    };
    tx_append_raw(
        tx,
        content_id.as_deref(),
        None,
        kind,
        &payload.to_string(),
        actor,
        now,
    )?;
    Ok(())
}

/// The holder of the active lease on an issue, if any.
#[cfg(feature = "native")]
fn tx_active_holder(tx: &Transaction<'_>, item_id: &str, now: &str) -> StoreResult<Option<String>> {
    tx.query_row(
        &format!(
            "SELECT actor FROM tracker_leases WHERE issue_id = ?1 AND {ACTIVE_LEASE} \
             ORDER BY acquired_at DESC LIMIT 1"
        ),
        params![item_id, now],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Lazily expire past-due, still-held leases on an issue (append `claim.expired`,
/// mark released), so an expired lease frees the issue for a fresh claim.
#[cfg(feature = "native")]
fn tx_expire_stale_leases(tx: &Transaction<'_>, item_id: &str, now: &str) -> StoreResult<()> {
    let stale: Vec<String> = tx
        .prepare(
            "SELECT lease_id FROM tracker_leases \
             WHERE issue_id = ?1 AND released_at IS NULL AND expires_at IS NOT NULL \
               AND expires_at <= ?2",
        )?
        .query_map(params![item_id, now], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for lease_id in &stale {
        tx_mark_lease_released(tx, lease_id, item_id, "claim.expired", "system", now)?;
    }
    Ok(())
}

/// Release the (single) active lease on an issue, if present. Returns whether a
/// lease was released.
#[cfg(feature = "native")]
fn tx_release_active_lease(tx: &Transaction<'_>, item_id: &str, now: &str) -> StoreResult<bool> {
    let lease: Option<(String, String)> = tx
        .query_row(
            &format!(
                "SELECT lease_id, actor FROM tracker_leases WHERE issue_id = ?1 AND {ACTIVE_LEASE} \
                 ORDER BY acquired_at DESC LIMIT 1"
            ),
            params![item_id, now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match lease {
        None => Ok(false),
        Some((lease_id, actor)) => {
            tx_mark_lease_released(tx, &lease_id, item_id, "claim.released", &actor, now)?;
            Ok(true)
        }
    }
}

/// Append a lease-terminal event and fold it into the lease projection.
#[cfg(feature = "native")]
fn tx_mark_lease_released(
    tx: &Transaction<'_>,
    lease_id: &str,
    item_id: &str,
    kind: &str,
    actor: &str,
    now: &str,
) -> StoreResult<()> {
    let payload = json!({"lease_id": lease_id, "actor": actor, "released_at": now});
    tx_append_event(tx, Some(item_id), kind, &payload, Some(actor), now)?;
    tx.execute(
        "UPDATE tracker_leases SET released_at = ?2 WHERE lease_id = ?1",
        params![lease_id, now],
    )?;
    Ok(())
}

/// Append one `issue.field_set` (chaining onto the issue's heads) and fold it
/// into the linear display column. Shared by the plain and token-checked sets.
#[cfg(feature = "native")]
fn tx_apply_field_set(
    tx: &Transaction<'_>,
    item_id: &str,
    field: &str,
    value: &str,
    now: &str,
) -> StoreResult<()> {
    let payload = json!({"field": field, "value": value});
    tx_append_event(tx, Some(item_id), "issue.field_set", &payload, None, now)?;
    // The conflict view is computed on read from the DAG; the column is only
    // the last-writer display value.
    if let Some(column) = projection_column(field) {
        tx.execute(
            &format!(
                "UPDATE tracker_issues SET {column} = ?2, updated_at = ?3 WHERE issue_id = ?1"
            ),
            params![item_id, value, now],
        )?;
    }
    Ok(())
}

/// The `tracker_issues` display column an `issue.field_set` writes, if any.
/// Unknown fields still record an event (so the conflict view sees them) but
/// touch no column.
#[cfg(feature = "native")]
fn projection_column(field: &str) -> Option<&'static str> {
    match field {
        "title" => Some("title"),
        "body" => Some("body"),
        "status" => Some("status"),
        _ => None,
    }
}

/// SHA-256 hex of a string (the `state_token` hasher).
#[cfg(feature = "native")]
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

/// One event as the conflict engine reads it (id + DAG parents + kind/payload).
#[cfg(feature = "native")]
struct IssueEvent {
    event_id: String,
    parents: Vec<String>,
    kind: String,
    payload: Value,
}

/// Load one issue's events in append order for DAG analysis.
#[cfg(feature = "native")]
fn load_issue_events(conn: &Connection, issue_id: &str) -> StoreResult<Vec<IssueEvent>> {
    let mut statement = conn.prepare(
        "SELECT event_id, parents_json, kind, payload_json FROM tracker_events \
         WHERE issue_id = ?1 ORDER BY event_seq",
    )?;
    let rows = statement
        .query_map([issue_id], |row| {
            let event_id: Option<String> = row.get(0)?;
            let parents_json: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let payload_json: String = row.get(3)?;
            Ok((event_id, parents_json, kind, payload_json))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(event_id, parents_json, kind, payload_json)| IssueEvent {
            event_id: event_id.unwrap_or_default(),
            parents: serde_json::from_str(&parents_json).unwrap_or_default(),
            kind,
            payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
        })
        .collect())
}

/// The DAG conflict analysis (ADR-0002 phase B1 slice ii, realizing
/// `tracker-merge.maude`): compute the frontier (`heads`), a content `state_token`
/// over it, and any field whose `bef`-maximal `issue.field_set` setters disagree.
/// A setter is `bef`-maximal iff no OTHER setter of the same field has it as a
/// transitive ancestor — i.e. nothing supersedes it along the DAG. A field with
/// two or more distinct maximal values is conflicted; a linear history (one
/// maximal setter) never is, and agreeing forks converge.
#[cfg(feature = "native")]
fn analyze_issue_dag(events: &[IssueEvent]) -> IssueConflicts {
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

    // Frontier: an event that nothing else lists as a parent.
    let claimed: HashSet<&str> = events
        .iter()
        .flat_map(|e| e.parents.iter().map(String::as_str))
        .collect();
    let mut heads: Vec<String> = events
        .iter()
        .filter(|e| !e.event_id.is_empty() && !claimed.contains(e.event_id.as_str()))
        .map(|e| e.event_id.clone())
        .collect();
    heads.sort();
    heads.dedup();
    let state_token = sha256_hex(&heads.join("\n"));

    // Transitive-ancestor test over the parent map.
    let parents: HashMap<&str, &[String]> = events
        .iter()
        .map(|e| (e.event_id.as_str(), e.parents.as_slice()))
        .collect();
    let is_ancestor = |ancestor: &str, of: &str| -> bool {
        let mut stack: Vec<&str> = parents
            .get(of)
            .map(|ps| ps.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(x) = stack.pop() {
            if x == ancestor {
                return true;
            }
            if !seen.insert(x) {
                continue;
            }
            if let Some(ps) = parents.get(x) {
                stack.extend(ps.iter().map(String::as_str));
            }
        }
        false
    };

    // Group the field-setting events by field.
    let mut setters: BTreeMap<String, Vec<(&str, String)>> = BTreeMap::new();
    for e in events {
        if e.kind != "issue.field_set" || e.event_id.is_empty() {
            continue;
        }
        let (Some(field), Some(value)) = (
            e.payload.get("field").and_then(Value::as_str),
            e.payload.get("value").and_then(Value::as_str),
        ) else {
            continue;
        };
        setters
            .entry(field.to_owned())
            .or_default()
            .push((e.event_id.as_str(), value.to_owned()));
    }

    let mut field_conflicts = Vec::new();
    for (field, ss) in &setters {
        let values: BTreeSet<String> = ss
            .iter()
            .filter(|(id, _)| {
                !ss.iter()
                    .any(|(other, _)| other != id && is_ancestor(id, other))
            })
            .map(|(_, val)| val.clone())
            .collect();
        if values.len() > 1 {
            field_conflicts.push(FieldConflict {
                field: field.clone(),
                values: values.into_iter().collect(),
            });
        }
    }

    IssueConflicts {
        heads,
        state_token,
        field_conflicts,
    }
}

/// Fold one event into the projection tables — the shared step of both live
/// application and `rebuild_projection`, so a rebuild is bit-identical. `issue_id`
/// is the alias of the event's subject issue (already resolved from the event's
/// opaque `content_id`). `alias_of` maps content_id → alias so `relation.added`
/// payloads (which reference issues by opaque id) fold into the alias-keyed
/// projection.
#[cfg(feature = "native")]
fn fold_event(
    tx: &Transaction<'_>,
    issue_id: Option<&str>,
    kind: &str,
    payload: &Value,
    created_at: &str,
    alias_of: &std::collections::HashMap<String, String>,
) -> StoreResult<()> {
    let str_of = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
    match kind {
        "issue.created" => {
            let labels_json = payload
                .get("labels")
                .map_or_else(|| "[]".to_owned(), std::string::ToString::to_string);
            let metadata_json = payload
                .get("metadata")
                .map_or_else(|| "{}".to_owned(), std::string::ToString::to_string);
            tx.execute(
                "INSERT INTO tracker_issues \
                 (issue_id, queue, title, body, status, labels_json, metadata_json, filed_by, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7, ?8, ?8)",
                params![
                    issue_id,
                    payload.get("queue").and_then(Value::as_str).unwrap_or_default(),
                    payload.get("title").and_then(Value::as_str).unwrap_or_default(),
                    payload.get("body").and_then(Value::as_str).unwrap_or_default(),
                    labels_json,
                    metadata_json,
                    payload.get("filed_by").and_then(Value::as_str),
                    created_at,
                ],
            )?;
        }
        "issue.field_set" => {
            if let (Some(id), Some(field), Some(value)) =
                (issue_id, str_of("field"), str_of("value"))
            {
                // Only the columns v1 sets via field_set; unknown fields are ignored.
                let column = match field.as_str() {
                    "title" => "title",
                    "body" => "body",
                    "status" => "status",
                    _ => return Ok(()),
                };
                tx.execute(
                    &format!(
                        "UPDATE tracker_issues SET {column} = ?2, updated_at = ?3 WHERE issue_id = ?1"
                    ),
                    params![id, value, created_at],
                )?;
            }
        }
        "issue.closed" => fold_set_status(tx, issue_id, payload, "closed", created_at)?,
        "issue.canceled" => fold_set_status(tx, issue_id, payload, "canceled", created_at)?,
        "issue.reopened" => fold_set_status(tx, issue_id, payload, "open", created_at)?,
        "relation.added" => {
            // The payload references issues by opaque content_id; the projection
            // is alias-keyed. Skip the edge if either endpoint has no local alias
            // (an imported relation whose issue we do not yet hold).
            let (Some(from), Some(to)) = (
                str_of("from").and_then(|c| alias_of.get(&c).cloned()),
                str_of("to").and_then(|c| alias_of.get(&c).cloned()),
            ) else {
                return Ok(());
            };
            tx.execute(
                "INSERT OR IGNORE INTO tracker_relations (from_issue, to_issue, kind, dep_kind) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    from,
                    to,
                    str_of("kind").unwrap_or_else(|| "blocks".to_owned()),
                    str_of("dep_kind"),
                ],
            )?;
        }
        "claim.acquired" => {
            tx.execute(
                "INSERT OR IGNORE INTO tracker_leases (lease_id, issue_id, actor, acquired_at, expires_at, released_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                params![
                    str_of("lease_id"),
                    issue_id,
                    str_of("actor"),
                    created_at,
                    payload.get("expires_at").and_then(Value::as_str),
                ],
            )?;
        }
        "claim.renewed" => {
            tx.execute(
                "UPDATE tracker_leases SET expires_at = ?2 WHERE lease_id = ?1",
                params![
                    str_of("lease_id"),
                    payload.get("expires_at").and_then(Value::as_str)
                ],
            )?;
        }
        "claim.released" | "claim.expired" => {
            tx.execute(
                "UPDATE tracker_leases SET released_at = ?2 WHERE lease_id = ?1",
                params![
                    str_of("lease_id"),
                    str_of("released_at").unwrap_or_else(|| created_at.to_owned())
                ],
            )?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(feature = "native")]
fn fold_set_status(
    tx: &Transaction<'_>,
    issue_id: Option<&str>,
    payload: &Value,
    status: &str,
    created_at: &str,
) -> StoreResult<()> {
    if let Some(id) = issue_id {
        let summary = payload.get("summary").and_then(Value::as_str);
        tx.execute(
            "UPDATE tracker_issues SET status = ?2, claim_summary = COALESCE(?3, claim_summary), \
             updated_at = ?4 WHERE issue_id = ?1",
            params![id, status, summary, created_at],
        )?;
    }
    Ok(())
}

/// Apply the active-lease overlay to a projection row: an issue under an active
/// lease presents as `in_progress` claimed by the holder (readiness overlay),
/// never a durable status write. Only durable-`open` issues can be overlaid.
/// Shared by the native and durable-object backends so the overlay is identical.
pub fn apply_overlay(mut item: WorkItem, holder: Option<String>) -> WorkItem {
    if item.status == "open" {
        if let Some(holder) = holder {
            item.status = "in_progress".to_owned();
            item.claimed_by = Some(holder);
        }
    }
    item
}

/// The work-item tracker as a backend-agnostic trait — the sans-IO store seam
/// (DR-0033 Phase 3), so a durable-object SQLite backend can back the same queue
/// operations without the language changing (`spec/work-queues.md`). The native
/// `WorkItemStore` implements it by forwarding to its inherent methods, so
/// existing callers are unaffected.
pub trait WorkItems {
    /// The workspace-plane HIGH-WATER position of the tracker's monotone
    /// event log (max event_seq; 0 = empty). One half of the two-plane
    /// consistent cut (vw note §9.3).
    fn event_position(&self) -> StoreResult<i64>;
    fn file_item(
        &mut self,
        queue: &str,
        title: &str,
        body: &str,
        labels: &[String],
        metadata: &Value,
        filed_by: Option<&str>,
    ) -> StoreResult<WorkItem>;

    fn get_item(&self, item_id: &str) -> StoreResult<Option<WorkItem>>;

    fn list_items(&self, queue: Option<&str>, status: Option<&str>) -> StoreResult<Vec<WorkItem>>;

    fn ready_items(&self, queue: &str) -> StoreResult<Vec<WorkItem>>;

    /// Atomic claim with an optional absolute expiry (`None` = no TTL). The
    /// T3 claim-TTL half: the caller computes `now + ttl` and passes it here.
    fn claim_item(
        &mut self,
        item_id: &str,
        claimed_by: &str,
        expires: Option<&str>,
    ) -> StoreResult<ClaimOutcome>;

    /// Holder-only lease extension/heartbeat (the T3 sanctioned extension).
    fn renew_claim(
        &mut self,
        item_id: &str,
        actor: &str,
        expires: Option<&str>,
    ) -> StoreResult<RenewOutcome>;

    fn release_item(&mut self, item_id: &str) -> StoreResult<bool>;

    fn release_claims_for_holder(&mut self, holder: &str) -> StoreResult<usize>;

    fn finish_item(&mut self, item_id: &str, summary: Option<&str>) -> StoreResult<bool>;

    /// Records a `blocks(from -> to)` edge (`to` is gated until `from` closes).
    /// The relation source-verbs' A+blockers seam; `from` blocks `to`.
    fn add_blocks(&mut self, from: &str, to: &str) -> StoreResult<()>;
}

#[cfg(feature = "native")]
impl WorkItems for WorkItemStore {
    // Forwards to the inherent methods of the same name; inherent methods win
    // `self.method()` resolution, so this delegates rather than recurses.
    fn event_position(&self) -> StoreResult<i64> {
        let position: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(event_seq), 0) FROM tracker_events",
            [],
            |row| row.get(0),
        )?;
        Ok(position)
    }

    fn file_item(
        &mut self,
        queue: &str,
        title: &str,
        body: &str,
        labels: &[String],
        metadata: &Value,
        filed_by: Option<&str>,
    ) -> StoreResult<WorkItem> {
        self.file_item(queue, title, body, labels, metadata, filed_by)
    }

    fn get_item(&self, item_id: &str) -> StoreResult<Option<WorkItem>> {
        self.get_item(item_id)
    }

    fn list_items(&self, queue: Option<&str>, status: Option<&str>) -> StoreResult<Vec<WorkItem>> {
        self.list_items(queue, status)
    }

    fn ready_items(&self, queue: &str) -> StoreResult<Vec<WorkItem>> {
        self.ready_items(queue)
    }

    fn claim_item(
        &mut self,
        item_id: &str,
        claimed_by: &str,
        expires: Option<&str>,
    ) -> StoreResult<ClaimOutcome> {
        self.claim_item(item_id, claimed_by, expires)
    }

    fn renew_claim(
        &mut self,
        item_id: &str,
        actor: &str,
        expires: Option<&str>,
    ) -> StoreResult<RenewOutcome> {
        self.renew_claim(item_id, actor, expires)
    }

    fn release_item(&mut self, item_id: &str) -> StoreResult<bool> {
        self.release_item(item_id)
    }

    fn release_claims_for_holder(&mut self, holder: &str) -> StoreResult<usize> {
        self.release_claims_for_holder(holder)
    }

    fn finish_item(&mut self, item_id: &str, summary: Option<&str>) -> StoreResult<bool> {
        self.finish_item(item_id, summary)
    }

    fn add_blocks(&mut self, from: &str, to: &str) -> StoreResult<()> {
        self.add_blocks(from, to)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimOutcome {
    Claimed,
    AlreadyClaimed { holder: String },
    NotFound,
}

/// Outcome of `renew_claim` (`tracker-lease.maude` I2). `NotHeld` = the actor
/// does not hold an active lease on the issue; `NotMonotonic` = the requested
/// finite deadline would move the lease backward.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenewOutcome {
    Renewed { expires_at: Option<String> },
    NotHeld,
    NotMonotonic,
}

#[cfg(feature = "native")]
fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
    // Column order = ISSUE_COLS.
    let labels_json: String = row.get(5)?;
    let metadata_json: String = row.get(6)?;
    Ok(WorkItem {
        id: row.get(0)?,
        queue: row.get(1)?,
        title: row.get(2)?,
        body: row.get(3)?,
        status: row.get(4)?,
        labels: serde_json::from_str(&labels_json).unwrap_or_default(),
        metadata: serde_json::from_str(&metadata_json).unwrap_or_else(|_| json!({})),
        // Durable projection carries no holder; the overlay supplies it.
        claimed_by: None,
        filed_by: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_memory() -> WorkItemStore {
        WorkItemStore::open(":memory:").expect("opens")
    }

    #[test]
    fn files_items_with_sequential_speakable_ids() {
        let mut store = open_memory();
        let first = store
            .file_item("backlog", "Fix login", "repro...", &[], &json!({}), None)
            .expect("files");
        let second = store
            .file_item(
                "backlog",
                "Fix login",
                "repro...",
                &[],
                &json!({}),
                Some("turn-1"),
            )
            .expect("files");
        assert_eq!(first.id, "WS-1");
        assert_eq!(second.id, "WS-2");
        assert_eq!(second.filed_by.as_deref(), Some("turn-1"));
    }

    /// Reads the raw event log for one issue (given by alias) in append order.
    fn events_for(store: &WorkItemStore, alias: &str) -> Vec<(String, Vec<String>, String)> {
        let content_id = content_id_of(&store.connection, alias).unwrap().unwrap();
        store
            .connection
            .prepare(
                "SELECT event_id, parents_json, kind FROM tracker_events \
                 WHERE issue_id = ?1 ORDER BY event_seq",
            )
            .unwrap()
            .query_map([&content_id], |row| {
                let id: String = row.get(0)?;
                let parents_json: String = row.get(1)?;
                let kind: String = row.get(2)?;
                Ok((id, serde_json::from_str(&parents_json).unwrap(), kind))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    /// ADR-0002 phase B1 slice i: every tracker event carries a SHA-256
    /// content-hash id, and each new event's parents are the issue's prior
    /// heads — so the log is a hash-chained Merkle-DAG, not a flat list.
    #[test]
    fn events_form_a_content_hash_chain() {
        let mut store = open_memory();
        let filed = store
            .file_item("backlog", "Fix login", "repro", &[], &json!({}), None)
            .expect("files");
        // A second event on the same issue (claim), so we have a chain to check.
        assert_eq!(
            store
                .claim_item(&filed.id, "worker-1", None)
                .expect("claim"),
            ClaimOutcome::Claimed
        );

        let events = events_for(&store, &filed.id);
        assert_eq!(events.len(), 2, "created + claim");

        let (created_id, created_parents, created_kind) = &events[0];
        assert_eq!(created_kind, "issue.created");
        // Content-hash id: 64 lowercase hex chars (SHA-256), not "WS-N".
        assert_eq!(created_id.len(), 64);
        assert!(created_id.chars().all(|c| c.is_ascii_hexdigit()));
        // The issue's root event has no parents.
        assert!(created_parents.is_empty());

        let (claim_id, claim_parents, _) = &events[1];
        assert_eq!(claim_id.len(), 64);
        // The claim event chains onto the created event: it is a child.
        assert_eq!(claim_parents, &[created_id.clone()]);
        assert_ne!(claim_id, created_id);
    }

    /// ADR-0002 phase B1 slice i unit 2: the event log is keyed by the opaque
    /// content_id (merge identity), NOT the clone-local WS-N alias; the alias
    /// table bridges the two. Identity = the creation event's id.
    #[test]
    fn events_are_keyed_by_opaque_content_id_not_alias() {
        let mut store = open_memory();
        let filed = store
            .file_item("backlog", "Fix login", "repro", &[], &json!({}), None)
            .expect("files");
        assert_eq!(filed.id, "WS-1");

        let content_id = content_id_of(&store.connection, "WS-1")
            .unwrap()
            .expect("alias resolves");
        assert_eq!(content_id.len(), 64, "content_id is a SHA-256");

        // No event row carries the WS-N alias; every event's issue_id is the id.
        let alias_rows: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM tracker_events WHERE issue_id = 'WS-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(alias_rows, 0, "no event is keyed by the alias");
        let id_rows: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM tracker_events WHERE issue_id = ?1",
                [&content_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(id_rows, 1, "the created event is keyed by content_id");
        // Identity = the creation event: the created event's id IS the content_id.
        let (created_id, _, _) = &events_for(&store, "WS-1")[0];
        assert_eq!(created_id, &content_id);
    }

    /// A pre-phase-B `tracker_events` (the ADR-0002 v1 linear log, no `event_id`
    /// / `parents_json`) opens without crashing: the schema self-heals the
    /// columns before the unique index, so an old store upgrades in place.
    #[test]
    fn open_self_heals_a_pre_phase_b_event_table() {
        let path =
            std::env::temp_dir().join(format!("whip-tracker-heal-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE tracker_events ( \
                   event_seq INTEGER PRIMARY KEY AUTOINCREMENT, \
                   issue_id TEXT, kind TEXT NOT NULL, \
                   payload_json TEXT NOT NULL DEFAULT '{}', \
                   actor TEXT, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);",
            )
            .unwrap();
        }
        // Opening adds the Merkle-DAG columns + index rather than erroring.
        let mut store = WorkItemStore::open(&path).expect("open self-heals old schema");
        let filed = store
            .file_item("q", "t", "", &[], &json!({}), None)
            .expect("files on the healed store");
        assert_eq!(filed.id, "WS-1");
        let _ = std::fs::remove_file(&path);
    }

    /// A rebuild reconstructs the alias-keyed projection from the content_id-keyed
    /// event log via the durable `tracker_aliases` bridge — bit-identical to the
    /// live projection, across issues / field sets / dependencies / claims.
    #[test]
    fn rebuild_reproduces_projection_through_the_alias_bridge() {
        let mut store = open_memory();
        let a = store
            .file_item("q", "A title", "", &[], &json!({}), None)
            .expect("files");
        let b = store
            .file_item("q", "B title", "", &[], &json!({}), None)
            .expect("files");
        store.set_field(&a.id, "title", "A retitled").expect("set");
        store.add_blocks(&b.id, &a.id).expect("dep"); // b blocks a
        store.claim_item(&b.id, "worker-1", None).expect("claim");

        let before = store.get_item(&a.id).unwrap().unwrap();
        let ready_before = store.ready_items("q").unwrap();

        store.rebuild_projection().expect("rebuild");

        let after = store.get_item(&a.id).unwrap().unwrap();
        assert_eq!(after.id, "WS-1");
        assert_eq!(after.title, "A retitled", "field_set folded through");
        assert_eq!(after, before, "projection is bit-identical after rebuild");
        // a is still blocked by open b; b is claimed → neither ready, same as before.
        assert_eq!(
            store.ready_items("q").unwrap().len(),
            ready_before.len(),
            "readiness (deps + leases) reproduced"
        );
        // The dependency edge survived the content_id→alias round-trip.
        let edges: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM tracker_relations WHERE from_issue = ?1 AND to_issue = ?2",
                params![b.id, a.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(edges, 1, "relation folded back to aliases");
    }

    /// Inserts one event with EXPLICIT parents, bypassing the linear
    /// head-chaining of `tx_append_event`. This is how a test manufactures a
    /// DAG fork — two events sharing a parent — which single-writer appends
    /// never produce (only a merge does). Returns the event's content id.
    fn insert_event(
        store: &WorkItemStore,
        alias: &str,
        kind: &str,
        payload: &Value,
        parents: &[String],
        now: &str,
    ) -> String {
        let content_id = content_id_of(&store.connection, alias).unwrap().unwrap();
        let payload_json = payload.to_string();
        let event_id = event_content_id(kind, Some(&content_id), &payload_json, None, parents, now);
        let parents_json = serde_json::to_string(parents).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO tracker_events \
                 (event_id, parents_json, issue_id, kind, payload_json, actor, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
                params![event_id, parents_json, content_id, kind, payload_json, now],
            )
            .unwrap();
        event_id
    }

    fn heads_of(store: &WorkItemStore, id: &str) -> Vec<String> {
        store.issue_conflicts(id).unwrap().unwrap().heads
    }

    /// ADR-0002 phase B1 slice ii, realizing `tracker-merge.maude`. Single
    /// clone: a linear field history has one maximal setter, so it never
    /// conflicts and the issue stays ready.
    #[test]
    fn linear_field_history_never_conflicts() {
        let mut store = open_memory();
        let it = store
            .file_item("q", "t", "", &[], &json!({}), None)
            .expect("files");
        assert!(store.set_field(&it.id, "title", "A").expect("set"));
        assert!(store.set_field(&it.id, "title", "B").expect("set"));
        let c = store.issue_conflicts(&it.id).expect("q").expect("exists");
        assert!(!c.conflicted());
        assert_eq!(c.heads.len(), 1, "linear frontier is a single head");
        // The linear last-writer projection column tracks the latest set.
        assert_eq!(store.get_item(&it.id).unwrap().unwrap().title, "B");
        assert_eq!(store.ready_items("q").unwrap().len(), 1);
    }

    /// A fork whose two maximal setters DISAGREE conflicts, is reported per
    /// field with both values, and is no longer ready.
    #[test]
    fn disagreeing_fork_conflicts_and_is_not_ready() {
        let mut store = open_memory();
        let it = store
            .file_item("q", "t", "", &[], &json!({}), None)
            .expect("files");
        let base = heads_of(&store, &it.id);
        insert_event(
            &store,
            &it.id,
            "issue.field_set",
            &json!({"field": "title", "value": "A"}),
            &base,
            "2020-01-01 00:00:01",
        );
        insert_event(
            &store,
            &it.id,
            "issue.field_set",
            &json!({"field": "title", "value": "B"}),
            &base,
            "2020-01-01 00:00:02",
        );
        let c = store.issue_conflicts(&it.id).expect("q").expect("exists");
        assert!(c.conflicted());
        assert_eq!(c.field_conflicts.len(), 1);
        assert_eq!(c.field_conflicts[0].field, "title");
        assert_eq!(c.field_conflicts[0].values, vec!["A", "B"]);
        assert_eq!(c.heads.len(), 2, "the fork has two heads");
        assert!(
            store.ready_items("q").unwrap().is_empty(),
            "a conflicted issue is not ready"
        );
    }

    /// A fork whose two maximal setters AGREE converges: distinct events (they
    /// differ by clock) but one value, so no conflict — and still ready.
    #[test]
    fn agreeing_fork_converges() {
        let mut store = open_memory();
        let it = store
            .file_item("q", "t", "", &[], &json!({}), None)
            .expect("files");
        let base = heads_of(&store, &it.id);
        insert_event(
            &store,
            &it.id,
            "issue.field_set",
            &json!({"field": "title", "value": "A"}),
            &base,
            "2020-01-01 00:00:01",
        );
        insert_event(
            &store,
            &it.id,
            "issue.field_set",
            &json!({"field": "title", "value": "A"}),
            &base,
            "2020-01-01 00:00:02",
        );
        let c = store.issue_conflicts(&it.id).expect("q").expect("exists");
        assert!(!c.conflicted());
        assert_eq!(c.heads.len(), 2);
        assert_eq!(store.ready_items("q").unwrap().len(), 1);
    }

    /// A fork that sets DIFFERENT fields is not a conflict (soundness bite):
    /// each field has a single maximal setter.
    #[test]
    fn different_fields_fork_does_not_conflict() {
        let mut store = open_memory();
        let it = store
            .file_item("q", "t", "", &[], &json!({}), None)
            .expect("files");
        let base = heads_of(&store, &it.id);
        insert_event(
            &store,
            &it.id,
            "issue.field_set",
            &json!({"field": "title", "value": "A"}),
            &base,
            "2020-01-01 00:00:01",
        );
        insert_event(
            &store,
            &it.id,
            "issue.field_set",
            &json!({"field": "body", "value": "X"}),
            &base,
            "2020-01-01 00:00:02",
        );
        let c = store.issue_conflicts(&it.id).expect("q").expect("exists");
        assert!(!c.conflicted());
        assert_eq!(c.heads.len(), 2);
    }

    /// Resolution: an event parented on BOTH conflicting heads supersedes them,
    /// leaving one maximal setter — the conflict clears and the frontier
    /// collapses to the resolver. The `state_token` changes across the resolve.
    #[test]
    fn merge_resolution_clears_conflict() {
        let mut store = open_memory();
        let it = store
            .file_item("q", "t", "", &[], &json!({}), None)
            .expect("files");
        let base = heads_of(&store, &it.id);
        insert_event(
            &store,
            &it.id,
            "issue.field_set",
            &json!({"field": "title", "value": "A"}),
            &base,
            "2020-01-01 00:00:01",
        );
        insert_event(
            &store,
            &it.id,
            "issue.field_set",
            &json!({"field": "title", "value": "B"}),
            &base,
            "2020-01-01 00:00:02",
        );
        let conflicted = store.issue_conflicts(&it.id).unwrap().unwrap();
        assert!(conflicted.conflicted());
        let token_before = conflicted.state_token.clone();

        // Resolve: a single setter descending from both heads.
        let heads = heads_of(&store, &it.id);
        assert_eq!(heads.len(), 2);
        insert_event(
            &store,
            &it.id,
            "issue.field_set",
            &json!({"field": "title", "value": "C"}),
            &heads,
            "2020-01-01 00:00:03",
        );
        let resolved = store.issue_conflicts(&it.id).unwrap().unwrap();
        assert!(!resolved.conflicted(), "resolver supersedes both forks");
        assert_eq!(resolved.heads.len(), 1);
        assert_ne!(resolved.state_token, token_before, "the frontier changed");
    }

    /// ADR-0002 phase B1 slice iii — the real multi-writer scenario (two clones,
    /// one shared workbench). Clone B imports clone A's issue, both edit the SAME
    /// field independently, and a cross-import FORKS the field: the merged clone
    /// sees a genuine conflict with both values. This is what gaugedesk needs.
    #[test]
    fn two_clones_editing_one_issue_merge_to_a_conflict() {
        let mut a = open_memory();
        let mut b = open_memory();

        // A creates the issue; B imports it and re-aliases it locally.
        let issue = a
            .file_item("q", "Shared", "", &[], &json!({}), None)
            .expect("files");
        let report = b.import_events(&a.export_events().unwrap()).unwrap();
        assert_eq!(report.new_issues, 1, "B re-aliased A's issue");
        assert_eq!(report.imported, 1, "just the created event");
        // Both clones happen to name it WS-1 — independent, clone-local aliases.
        let b_alias = "WS-1";
        assert_eq!(b.get_item(b_alias).unwrap().unwrap().title, "Shared");

        // Divergent edits to the SAME field from a shared parent.
        a.set_field(&issue.id, "title", "from-A").expect("set A");
        b.set_field(b_alias, "title", "from-B").expect("set B");

        // Cross-import A's edit into B → the title forks.
        b.import_events(&a.export_events().unwrap()).unwrap();
        let conflicts = b.issue_conflicts(b_alias).unwrap().unwrap();
        assert!(conflicts.conflicted(), "the two writers disagree on title");
        assert_eq!(conflicts.field_conflicts.len(), 1);
        assert_eq!(conflicts.field_conflicts[0].field, "title");
        assert_eq!(
            conflicts.field_conflicts[0].values,
            vec!["from-A", "from-B"]
        );
        assert!(
            b.ready_items("q").unwrap().is_empty(),
            "a conflicted issue is not handed to a worker"
        );
    }

    /// Agreeing writers converge: two clones set the same value, merge is clean.
    #[test]
    fn two_clones_agreeing_merge_cleanly() {
        let mut a = open_memory();
        let mut b = open_memory();
        let issue = a
            .file_item("q", "Shared", "", &[], &json!({}), None)
            .expect("files");
        b.import_events(&a.export_events().unwrap()).unwrap();
        a.set_field(&issue.id, "status", "closed").expect("set A");
        b.set_field("WS-1", "status", "closed").expect("set B");
        b.import_events(&a.export_events().unwrap()).unwrap();
        let conflicts = b.issue_conflicts("WS-1").unwrap().unwrap();
        assert!(!conflicts.conflicted(), "same value → convergence");
    }

    /// Import is idempotent and content-dedups: re-importing the same log adds no
    /// events, and each re-submitted `issue.created` is reported as a
    /// duplicate_submission (warned, never silently collapsed).
    #[test]
    fn reimport_dedups_and_warns_on_duplicate_submission() {
        let mut a = open_memory();
        let mut b = open_memory();
        a.file_item("q", "One", "", &[], &json!({}), None)
            .expect("files");
        a.file_item("q", "Two", "", &[], &json!({}), None)
            .expect("files");
        let events = a.export_events().unwrap();

        let first = b.import_events(&events).unwrap();
        assert_eq!(first.imported, 2);
        assert_eq!(first.new_issues, 2);
        assert!(first.duplicate_submissions.is_empty());

        let second = b.import_events(&events).unwrap();
        assert_eq!(second.imported, 0, "nothing new on re-import");
        assert_eq!(second.new_issues, 0, "no re-aliasing");
        assert_eq!(
            second.duplicate_submissions.len(),
            2,
            "both re-submitted creations warned"
        );
        // Still exactly two issues — no silent collapse into duplicates.
        assert_eq!(b.list_items(Some("q"), None).unwrap().len(), 2);
    }

    /// ADR-0002 phase B1 slice v: an optimistic set applies against the current
    /// state token, moves the token, and refuses a stale token — reporting the
    /// actual one to retry against, without overwriting the intervening change.
    #[test]
    fn optimistic_set_guards_on_state_token() {
        let mut store = open_memory();
        let it = store
            .file_item("q", "t", "", &[], &json!({}), None)
            .expect("files");
        let token0 = store.issue_conflicts(&it.id).unwrap().unwrap().state_token;

        // A fresh token applies and returns the new frontier token.
        let token1 = match store
            .set_field_checked(&it.id, "title", "A", &token0)
            .expect("set")
        {
            SetFieldOutcome::Applied { state_token } => {
                assert_ne!(state_token, token0);
                state_token
            }
            other => panic!("expected Applied, got {other:?}"),
        };

        // Re-using the stale token0 is refused with the current actual token1.
        match store
            .set_field_checked(&it.id, "title", "B", &token0)
            .expect("set")
        {
            SetFieldOutcome::StateChanged { actual } => assert_eq!(actual, token1),
            other => panic!("expected StateChanged, got {other:?}"),
        }
        // The refused set did not apply: the linear column is still "A".
        assert_eq!(store.get_item(&it.id).unwrap().unwrap().title, "A");

        // A missing issue reports NotFound, not a false token mismatch.
        assert_eq!(
            store
                .set_field_checked("WS-999", "title", "Z", &token0)
                .unwrap(),
            SetFieldOutcome::NotFound
        );
    }

    /// Drive the store through the `WorkItems` trait as a `dyn` object: proves
    /// the seam is object-safe (a boxed durable-object backend is legal) and
    /// forwards faithfully to the inherent methods.
    #[test]
    fn work_items_trait_seam_is_faithful() {
        let mut store = open_memory();
        let items: &mut dyn WorkItems = &mut store;

        let filed = items
            .file_item(
                "backlog",
                "Fix login",
                "repro",
                &[],
                &json!({}),
                Some("turn-1"),
            )
            .expect("file");
        assert_eq!(filed.id, "WS-1");
        assert_eq!(items.ready_items("backlog").expect("ready").len(), 1);
        assert_eq!(
            items
                .claim_item(&filed.id, "worker-1", None)
                .expect("claim"),
            ClaimOutcome::Claimed
        );
        assert!(items.ready_items("backlog").expect("ready").is_empty());
        let fetched = items.get_item(&filed.id).expect("get").expect("present");
        // The lease overlay presents the claimed issue as in_progress by holder.
        assert_eq!(fetched.status, "in_progress");
        assert_eq!(fetched.claimed_by.as_deref(), Some("worker-1"));
        assert_eq!(
            items
                .release_claims_for_holder("worker-1")
                .expect("release"),
            1
        );
        assert!(items.finish_item(&filed.id, Some("done")).expect("finish"));
        assert_eq!(
            items
                .list_items(Some("backlog"), Some("closed"))
                .expect("list")
                .len(),
            1
        );
    }

    #[test]
    fn ready_means_open_and_unclaimed() {
        let mut store = open_memory();
        let item = store
            .file_item("backlog", "a", "", &[], &json!({}), None)
            .expect("files");
        assert_eq!(store.ready_items("backlog").expect("ready").len(), 1);
        assert_eq!(
            store
                .claim_item(&item.id, "worker-1", None)
                .expect("claims"),
            ClaimOutcome::Claimed
        );
        assert!(store.ready_items("backlog").expect("ready").is_empty());
    }

    #[test]
    fn double_claim_is_branchable_not_an_error() {
        let mut store = open_memory();
        let item = store
            .file_item("backlog", "a", "", &[], &json!({}), None)
            .expect("files");
        assert_eq!(
            store
                .claim_item(&item.id, "worker-1", None)
                .expect("claims"),
            ClaimOutcome::Claimed
        );
        assert_eq!(
            store
                .claim_item(&item.id, "worker-2", None)
                .expect("claims"),
            ClaimOutcome::AlreadyClaimed {
                holder: "worker-1".to_owned()
            }
        );
    }

    #[test]
    fn release_returns_item_to_ready() {
        let mut store = open_memory();
        let item = store
            .file_item("backlog", "a", "", &[], &json!({}), None)
            .expect("files");
        store.claim_item(&item.id, "w", None).expect("claims");
        assert!(store.release_item(&item.id).expect("releases"));
        assert_eq!(store.ready_items("backlog").expect("ready").len(), 1);
    }

    /// Claim atomicity (`tracker-lease.maude` I1, exclusivity, deterministic
    /// form): across many items and many contenders, every item is claimed by
    /// exactly one worker and the rest see `AlreadyClaimed`.
    #[test]
    fn claim_atomicity_no_double_claim() {
        let mut store = open_memory();
        let mut ids = Vec::new();
        for index in 0..20 {
            let item = store
                .file_item(
                    "backlog",
                    &format!("item-{index}"),
                    "",
                    &[],
                    &json!({}),
                    None,
                )
                .expect("files");
            ids.push(item.id);
        }
        for id in &ids {
            let mut claimed = 0;
            let mut already = 0;
            for worker in 0..5 {
                match store
                    .claim_item(id, &format!("worker-{worker}"), None)
                    .expect("claims")
                {
                    ClaimOutcome::Claimed => claimed += 1,
                    ClaimOutcome::AlreadyClaimed { .. } => already += 1,
                    ClaimOutcome::NotFound => panic!("item vanished"),
                }
            }
            assert_eq!(claimed, 1, "exactly one worker claims {id}");
            assert_eq!(already, 4, "the rest see already-claimed for {id}");
        }
        // Every item is now leased; none remain ready.
        assert!(store.ready_items("backlog").expect("ready").is_empty());
    }

    /// Release/claim cycles preserve the invariant: a released item is
    /// re-claimable exactly once again.
    #[test]
    fn release_then_reclaim_preserves_single_holder() {
        let mut store = open_memory();
        let item = store
            .file_item("backlog", "a", "", &[], &json!({}), None)
            .expect("files");
        assert_eq!(
            store.claim_item(&item.id, "w1", None).expect("claims"),
            ClaimOutcome::Claimed
        );
        assert!(store.release_item(&item.id).expect("releases"));
        let mut claimed = 0;
        for worker in 0..3 {
            if let ClaimOutcome::Claimed = store
                .claim_item(&item.id, &format!("w{worker}"), None)
                .expect("claims")
            {
                claimed += 1;
            }
        }
        assert_eq!(claimed, 1);
    }

    /// Terminal-releases-all (`tracker-lease.maude` I3): a terminal holder drops
    /// only its OWN active leases (issue returns to ready), leaving other
    /// holders' leases untouched.
    #[test]
    fn release_claims_for_holder_frees_only_that_holders_in_progress_items() {
        let mut store = open_memory();
        let mine = store
            .file_item("backlog", "mine", "", &[], &json!({}), None)
            .expect("files");
        let theirs = store
            .file_item("backlog", "theirs", "", &[], &json!({}), None)
            .expect("files");
        store.claim_item(&mine.id, "w1", None).expect("claims mine");
        store
            .claim_item(&theirs.id, "w2", None)
            .expect("claims theirs");

        assert_eq!(
            store.release_claims_for_holder("w1").expect("releases"),
            1,
            "exactly w1's one active lease is released"
        );

        let mine = store.get_item(&mine.id).expect("gets").expect("exists");
        assert_eq!(mine.status, "open");
        assert!(mine.claimed_by.is_none());
        let theirs = store.get_item(&theirs.id).expect("gets").expect("exists");
        assert_eq!(theirs.status, "in_progress", "w2's lease is untouched");
        assert_eq!(theirs.claimed_by.as_deref(), Some("w2"));

        // The released item is claimable again; a holder with nothing held is a
        // no-op (e.g. an instance that already `finish`ed everything).
        assert_eq!(
            store.claim_item(&mine.id, "w3", None).expect("reclaims"),
            ClaimOutcome::Claimed
        );
        assert_eq!(store.release_claims_for_holder("w1").expect("noop"), 0);
    }

    #[test]
    fn finish_records_summary_and_leaves_done() {
        let mut store = open_memory();
        let item = store
            .file_item("backlog", "a", "", &[], &json!({}), None)
            .expect("files");
        store.claim_item(&item.id, "w", None).expect("claims");
        assert!(store
            .finish_item(&item.id, Some("done by agent"))
            .expect("finishes"));
        let item = store.get_item(&item.id).expect("gets").expect("exists");
        assert_eq!(item.status, "closed");
        assert!(store.ready_items("backlog").expect("ready").is_empty());
    }

    /// Holder-only + monotonic renew (`tracker-lease.maude` I2). A non-holder
    /// cannot renew; a finite deadline may only move forward.
    #[test]
    fn renew_is_holder_only_and_monotonic() {
        let mut store = open_memory();
        let item = store
            .file_item("backlog", "a", "", &[], &json!({}), None)
            .expect("files");
        store.claim_item(&item.id, "w1", None).expect("claims");

        // Non-holder cannot renew.
        assert_eq!(
            store.renew_claim(&item.id, "w2", None).expect("renew"),
            RenewOutcome::NotHeld
        );
        // Holder sets a first finite deadline (NULL -> finite is allowed).
        assert!(matches!(
            store
                .renew_claim(&item.id, "w1", Some("2099-01-01 00:00:00"))
                .expect("renew"),
            RenewOutcome::Renewed { .. }
        ));
        // Forward move is accepted.
        assert!(matches!(
            store
                .renew_claim(&item.id, "w1", Some("2099-06-01 00:00:00"))
                .expect("renew"),
            RenewOutcome::Renewed { .. }
        ));
        // Backward move is rejected (non-monotonic).
        assert_eq!(
            store
                .renew_claim(&item.id, "w1", Some("2099-03-01 00:00:00"))
                .expect("renew"),
            RenewOutcome::NotMonotonic
        );
    }

    /// Claim TTL (T3): a finite absolute `expires` records a timed lease that
    /// blocks readiness while in the future and stops blocking once past-due —
    /// so a claim in the past is expired-on-arrival and never blocks.
    #[test]
    fn claim_with_ttl_records_a_finite_expiry() {
        let mut store = open_memory();
        let item = store
            .file_item("backlog", "a", "", &[], &json!({}), None)
            .expect("files");
        // A far-future TTL: the claim is held, so the issue is not ready.
        assert_eq!(
            store
                .claim_item(&item.id, "w1", Some("2099-01-01 00:00:00"))
                .expect("claims"),
            ClaimOutcome::Claimed
        );
        assert!(store.ready_items("backlog").expect("ready").is_empty());
        let held = store.get_item(&item.id).expect("gets").expect("exists");
        assert_eq!(held.status, "in_progress");
        assert_eq!(held.claimed_by.as_deref(), Some("w1"));
        // A different worker cannot claim the still-live timed lease.
        assert!(matches!(
            store
                .claim_item(&item.id, "w2", None)
                .expect("contended claim"),
            ClaimOutcome::AlreadyClaimed { .. }
        ));
        // A past TTL is expired-on-arrival: it never blocks readiness, and the
        // lazy expiry sweep lets a fresh claim win.
        let stale = store
            .file_item("backlog", "b", "", &[], &json!({}), None)
            .expect("files");
        assert_eq!(
            store
                .claim_item(&stale.id, "w1", Some("2000-01-01 00:00:00"))
                .expect("claims"),
            ClaimOutcome::Claimed
        );
        let ready: Vec<String> = store
            .ready_items("backlog")
            .expect("ready")
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert!(
            ready.contains(&stale.id),
            "expired claim does not block: {ready:?}"
        );
        assert_eq!(
            store.claim_item(&stale.id, "w2", None).expect("reclaims"),
            ClaimOutcome::Claimed
        );
    }

    /// An expired lease frees the issue for a fresh claim and lets it be ready
    /// again (`tracker-lease.maude`: expired leases do not block).
    #[test]
    fn expired_lease_frees_the_issue() {
        let mut store = open_memory();
        let item = store
            .file_item("backlog", "a", "", &[], &json!({}), None)
            .expect("files");
        store.claim_item(&item.id, "w1", None).expect("claims");
        // Set the holder's own lease to a past deadline (NULL -> finite is
        // allowed for the holder), so it is now expired.
        assert!(matches!(
            store
                .renew_claim(&item.id, "w1", Some("2000-01-01 00:00:00"))
                .expect("renew"),
            RenewOutcome::Renewed { .. }
        ));
        // Expired lease no longer blocks readiness.
        assert_eq!(store.ready_items("backlog").expect("ready").len(), 1);
        // And a different worker can claim it (the lazy expiry sweep frees it).
        assert_eq!(
            store.claim_item(&item.id, "w2", None).expect("claims"),
            ClaimOutcome::Claimed
        );
    }

    /// Blocker readiness (`tracker-readiness.maude`): an issue with an active
    /// `blocks` edge from an open issue is not ready; closing the blocker frees
    /// it.
    #[test]
    fn active_blocker_gates_readiness() {
        let mut store = open_memory();
        let blocker = store
            .file_item("backlog", "blocker", "", &[], &json!({}), None)
            .expect("files");
        let blocked = store
            .file_item("backlog", "blocked", "", &[], &json!({}), None)
            .expect("files");
        store
            .add_blocks(&blocker.id, &blocked.id)
            .expect("add blocks");
        let ready: Vec<String> = store
            .ready_items("backlog")
            .expect("ready")
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(ready, vec![blocker.id.clone()], "blocked issue is gated");
        // Closing the blocker frees the blocked issue.
        assert!(store.finish_item(&blocker.id, None).expect("finish"));
        let ready: Vec<String> = store
            .ready_items("backlog")
            .expect("ready")
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(ready, vec![blocked.id]);
    }

    /// Projection determinism (`tracker-projection.maude`): a rebuild-from-events
    /// reproduces the live projection exactly.
    #[test]
    fn rebuild_from_events_equals_live_projection() {
        let mut store = open_memory();
        let a = store
            .file_item(
                "backlog",
                "a",
                "body-a",
                &["x".to_owned()],
                &json!({"k": 1}),
                Some("f"),
            )
            .expect("files a");
        let b = store
            .file_item("backlog", "b", "", &[], &json!({}), None)
            .expect("files b");
        let c = store
            .file_item("backlog", "c", "", &[], &json!({}), None)
            .expect("files c");
        store.add_blocks(&a.id, &b.id).expect("blocks");
        store.claim_item(&c.id, "w1", None).expect("claims c");
        store
            .renew_claim(&c.id, "w1", Some("2099-01-01 00:00:00"))
            .expect("renew");
        store.claim_item(&a.id, "w2", None).expect("claims a");
        store.release_item(&a.id).expect("release a");
        store.finish_item(&b.id, Some("done")).expect("finish b");

        let before = store.dump_projection().expect("dump before");
        store.rebuild_projection().expect("rebuild");
        let after = store.dump_projection().expect("dump after");
        assert_eq!(before, after, "rebuild reproduces the live projection");
    }

    // -- test-only projection helpers -------------------------------------

    impl WorkItemStore {
        /// A stable string snapshot of the three projection tables, for the
        /// rebuild-determinism assertion.
        fn dump_projection(&self) -> StoreResult<String> {
            let mut out = String::new();
            let mut issues = self.connection.prepare(
                "SELECT issue_id, queue, title, body, status, labels_json, metadata_json, \
                 claim_summary, filed_by, created_at, updated_at FROM tracker_issues \
                 ORDER BY issue_id",
            )?;
            let rows = issues.query_map([], |row| {
                Ok(format!(
                    "I {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })?;
            for row in rows {
                out.push_str(&row?);
                out.push('\n');
            }
            let mut rels = self.connection.prepare(
                "SELECT from_issue, to_issue, kind, dep_kind FROM tracker_relations \
                 ORDER BY from_issue, to_issue, kind",
            )?;
            let rows = rels.query_map([], |row| {
                Ok(format!(
                    "R {:?} {:?} {:?} {:?}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            for row in rows {
                out.push_str(&row?);
                out.push('\n');
            }
            let mut leases = self.connection.prepare(
                "SELECT lease_id, issue_id, actor, acquired_at, expires_at, released_at \
                 FROM tracker_leases ORDER BY lease_id",
            )?;
            let rows = leases.query_map([], |row| {
                Ok(format!(
                    "L {:?} {:?} {:?} {:?} {:?} {:?}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?;
            for row in rows {
                out.push_str(&row?);
                out.push('\n');
            }
            Ok(out)
        }
    }
}
