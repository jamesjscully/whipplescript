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
        tx_append_event(
            &tx,
            Some(&item_id),
            "issue.created",
            &payload,
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
        // Ready items have no active lease by construction, so the overlay is a
        // no-op here; return them as the projection sees them (durable `open`,
        // unclaimed).
        Ok(rows)
    }

    /// Atomic claim (`tracker-lease.maude` I1, exclusivity): grants a lease ONLY
    /// when the issue carries no active lease. The `Immediate` transaction takes
    /// the write lock at `BEGIN`, so the "is there an active lease?" check and
    /// the lease insert are serialized against every concurrent claim — exactly
    /// one wins, the rest see `AlreadyClaimed`. "Already claimed" is a normal,
    /// branchable outcome, not an error.
    pub fn claim_item(&mut self, item_id: &str, claimed_by: &str) -> StoreResult<ClaimOutcome> {
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
        // status — readiness changes through the lease overlay.
        let n: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tracker_events WHERE issue_id = ?1 AND kind = 'claim.acquired'",
            [item_id],
            |row| row.get(0),
        )?;
        let lease_id = format!("L-{item_id}-{n}");
        let payload = json!({"lease_id": lease_id, "actor": claimed_by, "expires_at": Value::Null});
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
             VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            params![lease_id, item_id, claimed_by, now],
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
        let payload = json!({"from": from, "to": to, "kind": "blocks"});
        tx_append_event(&tx, Some(to), "relation.added", &payload, None, &now)?;
        tx.execute(
            "INSERT OR IGNORE INTO tracker_relations (from_issue, to_issue, kind, dep_kind) \
             VALUES (?1, ?2, 'blocks', NULL)",
            params![from, to],
        )?;
        tx.commit()?;
        Ok(())
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
        let events: Vec<(Option<String>, String, String, String)> = tx
            .prepare(
                "SELECT issue_id, kind, payload_json, created_at FROM tracker_events ORDER BY event_seq",
            )?
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (issue_id, kind, payload_json, created_at) in &events {
            let payload: Value = serde_json::from_str(payload_json).unwrap_or_else(|_| json!({}));
            fold_event(&tx, issue_id.as_deref(), kind, &payload, created_at)?;
        }
        tx.commit()?;
        Ok(())
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
/// `tracker_events` is INSERT-only — the source of truth; the rest are
/// disposable projections folded from it. `tracker_counter` mints the sequential
/// `WS-N` alias.
#[cfg(feature = "native")]
const TRACKER_SCHEMA_SQL: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS tracker_events (
    event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
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
CREATE INDEX IF NOT EXISTS idx_tracker_issues_queue ON tracker_issues(queue, status);
CREATE INDEX IF NOT EXISTS idx_tracker_leases_issue ON tracker_leases(issue_id, released_at);
CREATE INDEX IF NOT EXISTS idx_tracker_events_issue ON tracker_events(issue_id, kind);
"#;

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

/// Append one immutable event (INSERT only — never updated or deleted).
#[cfg(feature = "native")]
fn tx_append_event(
    tx: &Transaction<'_>,
    issue_id: Option<&str>,
    kind: &str,
    payload: &Value,
    actor: Option<&str>,
    now: &str,
) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO tracker_events (issue_id, kind, payload_json, actor, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![issue_id, kind, payload.to_string(), actor, now],
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

/// Fold one event into the projection tables — the shared step of both live
/// application and `rebuild_projection`, so a rebuild is bit-identical. `issue_id`
/// is the event row's subject column (the issue for issue/claim events, the
/// blocked issue for `relation.added`).
#[cfg(feature = "native")]
fn fold_event(
    tx: &Transaction<'_>,
    issue_id: Option<&str>,
    kind: &str,
    payload: &Value,
    created_at: &str,
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
            tx.execute(
                "INSERT OR IGNORE INTO tracker_relations (from_issue, to_issue, kind, dep_kind) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    str_of("from"),
                    str_of("to"),
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

    fn claim_item(&mut self, item_id: &str, claimed_by: &str) -> StoreResult<ClaimOutcome>;

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

    fn claim_item(&mut self, item_id: &str, claimed_by: &str) -> StoreResult<ClaimOutcome> {
        self.claim_item(item_id, claimed_by)
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
            items.claim_item(&filed.id, "worker-1").expect("claim"),
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
            store.claim_item(&item.id, "worker-1").expect("claims"),
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
            store.claim_item(&item.id, "worker-1").expect("claims"),
            ClaimOutcome::Claimed
        );
        assert_eq!(
            store.claim_item(&item.id, "worker-2").expect("claims"),
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
        store.claim_item(&item.id, "w").expect("claims");
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
                    .claim_item(id, &format!("worker-{worker}"))
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
            store.claim_item(&item.id, "w1").expect("claims"),
            ClaimOutcome::Claimed
        );
        assert!(store.release_item(&item.id).expect("releases"));
        let mut claimed = 0;
        for worker in 0..3 {
            if let ClaimOutcome::Claimed = store
                .claim_item(&item.id, &format!("w{worker}"))
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
        store.claim_item(&mine.id, "w1").expect("claims mine");
        store.claim_item(&theirs.id, "w2").expect("claims theirs");

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
            store.claim_item(&mine.id, "w3").expect("reclaims"),
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
        store.claim_item(&item.id, "w").expect("claims");
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
        store.claim_item(&item.id, "w1").expect("claims");

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

    /// An expired lease frees the issue for a fresh claim and lets it be ready
    /// again (`tracker-lease.maude`: expired leases do not block).
    #[test]
    fn expired_lease_frees_the_issue() {
        let mut store = open_memory();
        let item = store
            .file_item("backlog", "a", "", &[], &json!({}), None)
            .expect("files");
        store.claim_item(&item.id, "w1").expect("claims");
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
            store.claim_item(&item.id, "w2").expect("claims"),
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
        store.claim_item(&c.id, "w1").expect("claims c");
        store
            .renew_claim(&c.id, "w1", Some("2099-01-01 00:00:00"))
            .expect("renew");
        store.claim_item(&a.id, "w2").expect("claims a");
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
