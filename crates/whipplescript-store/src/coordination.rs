//! Coordination resources: lease, ledger, counter (spec/coordination.md).
//!
//! Workspace-scoped like the work-item tracker: run stores are disposable
//! per experiment, shared coordination state is durable. Every operation is
//! one atomic transaction with a branchable outcome — no read-then-act
//! surface exists, by construction (principle 2). Holder lifetime + TTL
//! bound every held lease (principle 3); the caller passes the current
//! time/period so the clock stays at the worker boundary.

use std::path::Path;

use rusqlite::{params, Connection};
use serde_json::Value;

use crate::StoreResult;

/// Outcome of one atomic lease-acquire attempt. `Contended` is a normal,
/// branchable outcome, not an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AcquireOutcome {
    Held,
    Contended { holders: Vec<String> },
}

/// Outcome of one atomic counter consume. `Over` is a normal, branchable
/// outcome — downgrade, not crash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsumeOutcome {
    Ok { remaining: i64 },
    Over { remaining: i64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaseRow {
    pub resource: String,
    pub key: String,
    pub holder: String,
    pub acquired_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerEntry {
    pub ledger: String,
    pub partition: String,
    pub seq: i64,
    pub payload_json: String,
    pub appended_by: String,
    pub appended_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CounterRow {
    pub counter: String,
    pub key: String,
    pub consumed: i64,
    pub period: String,
}

pub struct CoordinationStore {
    connection: Connection,
}

impl CoordinationStore {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 5000;
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS leases (
                resource TEXT NOT NULL,
                key TEXT NOT NULL,
                holder TEXT NOT NULL,
                acquired_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                expires_at TEXT NOT NULL,
                PRIMARY KEY (resource, key, holder)
            );
            CREATE TABLE IF NOT EXISTS ledger_entries (
                ledger TEXT NOT NULL,
                partition TEXT NOT NULL,
                seq INTEGER NOT NULL,
                payload_json TEXT NOT NULL,
                appended_by TEXT NOT NULL,
                appended_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (ledger, seq)
            );
            CREATE TABLE IF NOT EXISTS ledger_seq (
                ledger TEXT PRIMARY KEY,
                next_seq INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS counters (
                counter TEXT NOT NULL,
                key TEXT NOT NULL,
                consumed INTEGER NOT NULL DEFAULT 0,
                period TEXT NOT NULL,
                PRIMARY KEY (counter, key)
            );
            CREATE INDEX IF NOT EXISTS idx_ledger_partition ON ledger_entries(ledger, partition, seq);
            "#,
        )?;
        Ok(Self { connection })
    }

    /// One atomic attempt: expire stale holders (TTL crash net), then either
    /// take a free slot or report the current holders. Re-acquiring a lease
    /// already held by the same holder is `Held` (idempotent worker retry).
    pub fn try_acquire(
        &mut self,
        resource: &str,
        key: &str,
        slots: i64,
        ttl_seconds: i64,
        holder: &str,
    ) -> StoreResult<AcquireOutcome> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM leases WHERE resource = ?1 AND key = ?2 AND expires_at <= datetime('now')",
            params![resource, key],
        )?;
        let already_held: i64 = tx.query_row(
            "SELECT COUNT(*) FROM leases WHERE resource = ?1 AND key = ?2 AND holder = ?3",
            params![resource, key, holder],
            |row| row.get(0),
        )?;
        if already_held > 0 {
            tx.commit()?;
            return Ok(AcquireOutcome::Held);
        }
        let holders: i64 = tx.query_row(
            "SELECT COUNT(*) FROM leases WHERE resource = ?1 AND key = ?2",
            params![resource, key],
            |row| row.get(0),
        )?;
        if holders < slots {
            tx.execute(
                "INSERT INTO leases (resource, key, holder, expires_at) VALUES (?1, ?2, ?3, datetime('now', ?4))",
                params![resource, key, holder, format!("+{ttl_seconds} seconds")],
            )?;
            tx.commit()?;
            return Ok(AcquireOutcome::Held);
        }
        let mut statement = tx.prepare(
            "SELECT holder FROM leases WHERE resource = ?1 AND key = ?2 ORDER BY acquired_at",
        )?;
        let current = statement
            .query_map(params![resource, key], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        tx.commit()?;
        Ok(AcquireOutcome::Contended { holders: current })
    }

    pub fn release(&mut self, resource: &str, key: &str, holder: &str) -> StoreResult<bool> {
        let changed = self.connection.execute(
            "DELETE FROM leases WHERE resource = ?1 AND key = ?2 AND holder = ?3",
            params![resource, key, holder],
        )?;
        Ok(changed >= 1)
    }

    /// Instance-terminal release: a holder reaching a workflow terminal drops
    /// everything it held (principle 3).
    pub fn release_all_for_holder(&mut self, holder: &str) -> StoreResult<usize> {
        let changed = self
            .connection
            .execute("DELETE FROM leases WHERE holder = ?1", params![holder])?;
        Ok(changed)
    }

    /// Appends commute — there is no contention to resolve. `retain_seconds`
    /// prunes old entries in the same transaction (mandatory bounded growth).
    pub fn append(
        &mut self,
        ledger: &str,
        partition: &str,
        payload_json: &str,
        appended_by: &str,
        retain_seconds: i64,
    ) -> StoreResult<i64> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO ledger_seq (ledger, next_seq) VALUES (?1, 1)",
            params![ledger],
        )?;
        let seq: i64 = tx.query_row(
            "UPDATE ledger_seq SET next_seq = next_seq + 1 WHERE ledger = ?1 RETURNING next_seq - 1",
            params![ledger],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO ledger_entries (ledger, partition, seq, payload_json, appended_by) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ledger, partition, seq, payload_json, appended_by],
        )?;
        tx.execute(
            "DELETE FROM ledger_entries WHERE ledger = ?1 AND appended_at <= datetime('now', ?2)",
            params![ledger, format!("-{retain_seconds} seconds")],
        )?;
        tx.commit()?;
        Ok(seq)
    }

    /// One atomic consume with lazy reset (spec/coordination.md): the caller
    /// derives `period` from the worker's clock read; a rolled period zeroes
    /// the count in the same transaction as the consume. No scheduler.
    pub fn consume(
        &mut self,
        counter: &str,
        key: &str,
        amount: i64,
        cap: i64,
        period: &str,
    ) -> StoreResult<ConsumeOutcome> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO counters (counter, key, consumed, period) VALUES (?1, ?2, 0, ?3)",
            params![counter, key, period],
        )?;
        tx.execute(
            "UPDATE counters SET consumed = 0, period = ?3 WHERE counter = ?1 AND key = ?2 AND period != ?3",
            params![counter, key, period],
        )?;
        let consumed: i64 = tx.query_row(
            "SELECT consumed FROM counters WHERE counter = ?1 AND key = ?2",
            params![counter, key],
            |row| row.get(0),
        )?;
        if consumed + amount <= cap {
            tx.execute(
                "UPDATE counters SET consumed = consumed + ?3 WHERE counter = ?1 AND key = ?2",
                params![counter, key, amount],
            )?;
            tx.commit()?;
            return Ok(ConsumeOutcome::Ok {
                remaining: cap - consumed - amount,
            });
        }
        tx.commit()?;
        Ok(ConsumeOutcome::Over {
            remaining: cap - consumed,
        })
    }

    /// The current reset-period identifier, read from the store's clock at
    /// the worker boundary (the one place the clock is legal). The lazy
    /// counter reset compares it inside the consume transaction.
    pub fn current_period(&self, reset: &str) -> StoreResult<String> {
        let format = match reset {
            "hourly" => "%Y-%m-%dT%H",
            "weekly" => "%Y-W%W",
            "monthly" => "%Y-%m",
            _ => "%Y-%m-%d",
        };
        Ok(self
            .connection
            .query_row("SELECT strftime(?1, 'now')", [format], |row| row.get(0))?)
    }

    pub fn list_leases(&self, resource: Option<&str>) -> StoreResult<Vec<LeaseRow>> {
        let mut statement = self.connection.prepare(
            "SELECT resource, key, holder, acquired_at, expires_at FROM leases WHERE (?1 IS NULL OR resource = ?1) ORDER BY resource, key, acquired_at",
        )?;
        let rows = statement
            .query_map(params![resource], |row| {
                Ok(LeaseRow {
                    resource: row.get(0)?,
                    key: row.get(1)?,
                    holder: row.get(2)?,
                    acquired_at: row.get(3)?,
                    expires_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_entries(
        &self,
        ledger: Option<&str>,
        partition: Option<&str>,
    ) -> StoreResult<Vec<LedgerEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT ledger, partition, seq, payload_json, appended_by, appended_at FROM ledger_entries WHERE (?1 IS NULL OR ledger = ?1) AND (?2 IS NULL OR partition = ?2) ORDER BY ledger, seq",
        )?;
        let rows = statement
            .query_map(params![ledger, partition], |row| {
                Ok(LedgerEntry {
                    ledger: row.get(0)?,
                    partition: row.get(1)?,
                    seq: row.get(2)?,
                    payload_json: row.get(3)?,
                    appended_by: row.get(4)?,
                    appended_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_counters(&self, counter: Option<&str>) -> StoreResult<Vec<CounterRow>> {
        let mut statement = self.connection.prepare(
            "SELECT counter, key, consumed, period FROM counters WHERE (?1 IS NULL OR counter = ?1) ORDER BY counter, key",
        )?;
        let rows = statement
            .query_map(params![counter], |row| {
                Ok(CounterRow {
                    counter: row.get(0)?,
                    key: row.get(1)?,
                    consumed: row.get(2)?,
                    period: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// `payload_json` parsed leniently for projections.
pub fn entry_payload(entry: &LedgerEntry) -> Value {
    serde_json::from_str(&entry.payload_json).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> CoordinationStore {
        let path = std::env::temp_dir().join(format!(
            "whipplescript-coordination-{}-{:?}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
        ));
        CoordinationStore::open(path).expect("open store")
    }

    /// Mutual exclusion: at most `slots` holders per key, ever; contended
    /// attempts report the holders.
    #[test]
    fn lease_mutual_exclusion_and_release() {
        let mut store = store();
        assert_eq!(
            store
                .try_acquire("deploy", "prod", 1, 600, "ins_a")
                .expect("op"),
            AcquireOutcome::Held
        );
        // Same-holder re-acquire is idempotent.
        assert_eq!(
            store
                .try_acquire("deploy", "prod", 1, 600, "ins_a")
                .expect("op"),
            AcquireOutcome::Held
        );
        assert_eq!(
            store
                .try_acquire("deploy", "prod", 1, 600, "ins_b")
                .expect("op"),
            AcquireOutcome::Contended {
                holders: vec!["ins_a".to_owned()]
            }
        );
        // Distinct keys never contend (typed entity domains).
        assert_eq!(
            store
                .try_acquire("deploy", "staging", 1, 600, "ins_b")
                .expect("op"),
            AcquireOutcome::Held
        );
        assert!(store.release("deploy", "prod", "ins_a").expect("op"));
        assert_eq!(
            store
                .try_acquire("deploy", "prod", 1, 600, "ins_b")
                .expect("op"),
            AcquireOutcome::Held
        );
    }

    /// An expired TTL frees the slot without an explicit release (crash net).
    #[test]
    fn lease_ttl_expiry_frees_slot() {
        let mut store = store();
        assert_eq!(
            store
                .try_acquire("deploy", "prod", 1, 0, "ins_dead")
                .expect("op"),
            AcquireOutcome::Held
        );
        assert_eq!(
            store
                .try_acquire("deploy", "prod", 1, 600, "ins_b")
                .expect("op"),
            AcquireOutcome::Held
        );
    }

    /// N-slot semaphore: slots fill independently; terminal release drops
    /// every lease the holder had.
    #[test]
    fn lease_slots_and_holder_release() {
        let mut store = store();
        assert_eq!(
            store
                .try_acquire("pool", "gpu", 2, 600, "ins_a")
                .expect("op"),
            AcquireOutcome::Held
        );
        assert_eq!(
            store
                .try_acquire("pool", "gpu", 2, 600, "ins_b")
                .expect("op"),
            AcquireOutcome::Held
        );
        assert!(matches!(
            store
                .try_acquire("pool", "gpu", 2, 600, "ins_c")
                .expect("op"),
            AcquireOutcome::Contended { .. }
        ));
        assert_eq!(store.release_all_for_holder("ins_a").expect("op"), 1);
        assert_eq!(
            store
                .try_acquire("pool", "gpu", 2, 600, "ins_c")
                .expect("op"),
            AcquireOutcome::Held
        );
    }

    /// Appends are totally ordered per ledger and partition-filterable.
    #[test]
    fn ledger_append_order_and_partitions() {
        let mut store = store();
        let first = store
            .append("decisions", "api", r#"{"choice":"rest"}"#, "ins_a", 3600)
            .expect("op");
        let second = store
            .append("decisions", "ui", r#"{"choice":"react"}"#, "ins_b", 3600)
            .expect("op");
        assert!(second > first);
        let api = store
            .list_entries(Some("decisions"), Some("api"))
            .expect("op");
        assert_eq!(api.len(), 1);
        assert_eq!(api[0].appended_by, "ins_a");
        assert_eq!(
            store
                .list_entries(Some("decisions"), None)
                .expect("op")
                .len(),
            2
        );
    }

    /// Cap invariant with lazy reset: consumed never exceeds cap inside one
    /// period; a rolled period zeroes the count atomically with the consume.
    #[test]
    fn counter_cap_and_lazy_reset() {
        let mut store = store();
        assert_eq!(
            store
                .consume("budget", "cust-1", 600, 1000, "2026-06-10")
                .expect("op"),
            ConsumeOutcome::Ok { remaining: 400 }
        );
        assert_eq!(
            store
                .consume("budget", "cust-1", 600, 1000, "2026-06-10")
                .expect("op"),
            ConsumeOutcome::Over { remaining: 400 }
        );
        // The over-budget attempt consumed nothing.
        assert_eq!(
            store
                .consume("budget", "cust-1", 400, 1000, "2026-06-10")
                .expect("op"),
            ConsumeOutcome::Ok { remaining: 0 }
        );
        // Next period: lazy reset at the consume boundary.
        assert_eq!(
            store
                .consume("budget", "cust-1", 600, 1000, "2026-06-11")
                .expect("op"),
            ConsumeOutcome::Ok { remaining: 400 }
        );
    }
}
