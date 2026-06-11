//! The builtin work-item tracker: the reference implementation of the
//! work-queue interface (`spec/work-queues.md`).
//!
//! Items live in a workspace-scoped SQLite file (default
//! `.whipplescript/items.sqlite`), deliberately separate from run stores:
//! run stores are disposable per experiment, the backlog is durable. The
//! builtin is just another tracker binding whose backend is a local file —
//! run stores never hold source-of-truth items, only projections.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::{StoreError, StoreResult};

/// Core status categories — the layer every surveyed tracker shares.
/// `ready` is a derived predicate (open and unclaimed), never a status.
pub const ITEM_STATUSES: &[&str] = &["open", "in_progress", "done", "cancelled"];

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

pub struct WorkItemStore {
    connection: Connection,
}

impl WorkItemStore {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS items (
                item_id TEXT PRIMARY KEY,
                queue TEXT NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'open',
                labels_json TEXT NOT NULL DEFAULT '[]',
                metadata_json TEXT NOT NULL DEFAULT '{}',
                claimed_by TEXT,
                claim_summary TEXT,
                filed_by TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS item_counter (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                next_id INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO item_counter (singleton, next_id) VALUES (1, 1);
            CREATE INDEX IF NOT EXISTS idx_items_queue_status ON items(queue, status);
            "#,
        )?;
        Ok(Self { connection })
    }

    /// Files an item, minting a sequential human-speakable id (`WS-1`,
    /// `WS-2`, ...). Sequential beats content hashes: "take WS-7" is
    /// speakable to an agent, and byte-identical items get distinct ids.
    pub fn file_item(
        &mut self,
        queue: &str,
        title: &str,
        body: &str,
        labels: &[String],
        metadata: &Value,
        filed_by: Option<&str>,
    ) -> StoreResult<WorkItem> {
        let tx = self.connection.transaction()?;
        let next: i64 = tx.query_row(
            "UPDATE item_counter SET next_id = next_id + 1 WHERE singleton = 1 RETURNING next_id - 1",
            [],
            |row| row.get(0),
        )?;
        let item_id = format!("WS-{next}");
        tx.execute(
            r#"
            INSERT INTO items (item_id, queue, title, body, labels_json, metadata_json, filed_by)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                item_id,
                queue,
                title,
                body,
                serde_json::to_string(labels)?,
                metadata.to_string(),
                filed_by,
            ],
        )?;
        tx.commit()?;
        self.get_item(&item_id)?
            .ok_or_else(|| StoreError::Conflict("filed item missing".to_owned()))
    }

    pub fn get_item(&self, item_id: &str) -> StoreResult<Option<WorkItem>> {
        self.connection
            .query_row(
                "SELECT item_id, queue, title, body, status, labels_json, metadata_json, claimed_by, filed_by, created_at, updated_at FROM items WHERE item_id = ?1",
                [item_id],
                row_to_item,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_items(
        &self,
        queue: Option<&str>,
        status: Option<&str>,
    ) -> StoreResult<Vec<WorkItem>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT item_id, queue, title, body, status, labels_json, metadata_json, claimed_by, filed_by, created_at, updated_at
            FROM items
            WHERE (?1 IS NULL OR queue = ?1)
              AND (?2 IS NULL OR status = ?2)
            ORDER BY created_at, item_id
            "#,
        )?;
        let rows = statement
            .query_map(params![queue, status], row_to_item)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Readiness is the tracker's promise: for the builtin, ready means
    /// `open` and unclaimed.
    pub fn ready_items(&self, queue: &str) -> StoreResult<Vec<WorkItem>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT item_id, queue, title, body, status, labels_json, metadata_json, claimed_by, filed_by, created_at, updated_at
            FROM items
            WHERE queue = ?1 AND status = 'open' AND claimed_by IS NULL
            ORDER BY created_at, item_id
            "#,
        )?;
        let rows = statement
            .query_map([queue], row_to_item)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Atomic claim: the tracker is the arbiter. "Already claimed" is a
    /// normal, branchable outcome, not an error.
    pub fn claim_item(&mut self, item_id: &str, claimed_by: &str) -> StoreResult<ClaimOutcome> {
        let tx = self.connection.transaction()?;
        let changed = tx.execute(
            r#"
            UPDATE items
            SET status = 'in_progress', claimed_by = ?2, updated_at = CURRENT_TIMESTAMP
            WHERE item_id = ?1 AND status = 'open' AND claimed_by IS NULL
            "#,
            params![item_id, claimed_by],
        )?;
        if changed == 1 {
            tx.commit()?;
            return Ok(ClaimOutcome::Claimed);
        }
        let holder: Option<Option<String>> = tx
            .query_row(
                "SELECT claimed_by FROM items WHERE item_id = ?1",
                [item_id],
                |row| row.get(0),
            )
            .optional()?;
        tx.commit()?;
        match holder {
            None => Ok(ClaimOutcome::NotFound),
            Some(holder) => Ok(ClaimOutcome::AlreadyClaimed {
                holder: holder.unwrap_or_default(),
            }),
        }
    }

    pub fn release_item(&mut self, item_id: &str) -> StoreResult<bool> {
        let changed = self.connection.execute(
            r#"
            UPDATE items
            SET status = 'open', claimed_by = NULL, updated_at = CURRENT_TIMESTAMP
            WHERE item_id = ?1 AND status = 'in_progress'
            "#,
            [item_id],
        )?;
        Ok(changed == 1)
    }

    /// Marks the item done. The optional summary is the agent-work audit
    /// trail landing in the tracker, where humans look.
    pub fn finish_item(&mut self, item_id: &str, summary: Option<&str>) -> StoreResult<bool> {
        let changed = self.connection.execute(
            r#"
            UPDATE items
            SET status = 'done', claim_summary = ?2, updated_at = CURRENT_TIMESTAMP
            WHERE item_id = ?1 AND status IN ('open', 'in_progress')
            "#,
            params![item_id, summary],
        )?;
        Ok(changed == 1)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimOutcome {
    Claimed,
    AlreadyClaimed { holder: String },
    NotFound,
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
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
        claimed_by: row.get(7)?,
        filed_by: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
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

    /// Claim atomicity (the TLA+ "two workers, one item, no double-claim"
    /// property, deterministic form): across many items and many contenders,
    /// every item is claimed by exactly one worker and the rest see
    /// `AlreadyClaimed`.
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
        // Every item is now in_progress; none remain ready.
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
        assert_eq!(item.status, "done");
        assert!(store.ready_items("backlog").expect("ready").is_empty());
    }
}
