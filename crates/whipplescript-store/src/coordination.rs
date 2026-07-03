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

pub const DEFAULT_COORDINATION_OWNER: &str = "shared";

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
    pub owner: String,
    pub resource: String,
    pub key: String,
    pub holder: String,
    pub acquired_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerEntry {
    pub owner: String,
    pub ledger: String,
    pub partition: String,
    pub seq: i64,
    pub payload_json: String,
    pub appended_by: String,
    pub appended_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CounterRow {
    pub owner: String,
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
            "#,
        )?;
        ensure_partitioned_schema(&connection)?;
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
        self.try_acquire_for_owner(
            DEFAULT_COORDINATION_OWNER,
            resource,
            key,
            slots,
            ttl_seconds,
            holder,
        )
    }

    pub fn try_acquire_for_owner(
        &mut self,
        owner: &str,
        resource: &str,
        key: &str,
        slots: i64,
        ttl_seconds: i64,
        holder: &str,
    ) -> StoreResult<AcquireOutcome> {
        let owner = normalized_owner(owner);
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM leases WHERE owner = ?1 AND resource = ?2 AND key = ?3 AND expires_at <= datetime('now')",
            params![owner, resource, key],
        )?;
        let already_held: i64 = tx.query_row(
            "SELECT COUNT(*) FROM leases WHERE owner = ?1 AND resource = ?2 AND key = ?3 AND holder = ?4",
            params![owner, resource, key, holder],
            |row| row.get(0),
        )?;
        if already_held > 0 {
            tx.commit()?;
            return Ok(AcquireOutcome::Held);
        }
        let holders: i64 = tx.query_row(
            "SELECT COUNT(*) FROM leases WHERE owner = ?1 AND resource = ?2 AND key = ?3",
            params![owner, resource, key],
            |row| row.get(0),
        )?;
        if holders < slots {
            tx.execute(
                "INSERT INTO leases (owner, resource, key, holder, expires_at) VALUES (?1, ?2, ?3, ?4, datetime('now', ?5))",
                params![owner, resource, key, holder, format!("+{ttl_seconds} seconds")],
            )?;
            tx.commit()?;
            return Ok(AcquireOutcome::Held);
        }
        let mut statement = tx.prepare(
            "SELECT holder FROM leases WHERE owner = ?1 AND resource = ?2 AND key = ?3 ORDER BY acquired_at",
        )?;
        let current = statement
            .query_map(params![owner, resource, key], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        tx.commit()?;
        Ok(AcquireOutcome::Contended { holders: current })
    }

    pub fn release(&mut self, resource: &str, key: &str, holder: &str) -> StoreResult<bool> {
        self.release_for_owner(DEFAULT_COORDINATION_OWNER, resource, key, holder)
    }

    pub fn release_for_owner(
        &mut self,
        owner: &str,
        resource: &str,
        key: &str,
        holder: &str,
    ) -> StoreResult<bool> {
        let owner = normalized_owner(owner);
        let changed = self.connection.execute(
            "DELETE FROM leases WHERE owner = ?1 AND resource = ?2 AND key = ?3 AND holder = ?4",
            params![owner, resource, key, holder],
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
        self.append_for_owner(
            DEFAULT_COORDINATION_OWNER,
            ledger,
            partition,
            payload_json,
            appended_by,
            retain_seconds,
        )
    }

    pub fn append_for_owner(
        &mut self,
        owner: &str,
        ledger: &str,
        partition: &str,
        payload_json: &str,
        appended_by: &str,
        retain_seconds: i64,
    ) -> StoreResult<i64> {
        let owner = normalized_owner(owner);
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO ledger_seq (owner, ledger, next_seq) VALUES (?1, ?2, 1)",
            params![owner, ledger],
        )?;
        let seq: i64 = tx.query_row(
            "UPDATE ledger_seq SET next_seq = next_seq + 1 WHERE owner = ?1 AND ledger = ?2 RETURNING next_seq - 1",
            params![owner, ledger],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO ledger_entries (owner, ledger, partition, seq, payload_json, appended_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![owner, ledger, partition, seq, payload_json, appended_by],
        )?;
        tx.execute(
            "DELETE FROM ledger_entries WHERE owner = ?1 AND ledger = ?2 AND appended_at <= datetime('now', ?3)",
            params![owner, ledger, format!("-{retain_seconds} seconds")],
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
        self.consume_for_owner(
            DEFAULT_COORDINATION_OWNER,
            counter,
            key,
            amount,
            cap,
            period,
        )
    }

    pub fn consume_for_owner(
        &mut self,
        owner: &str,
        counter: &str,
        key: &str,
        amount: i64,
        cap: i64,
        period: &str,
    ) -> StoreResult<ConsumeOutcome> {
        let owner = normalized_owner(owner);
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO counters (owner, counter, key, consumed, period) VALUES (?1, ?2, ?3, 0, ?4)",
            params![owner, counter, key, period],
        )?;
        tx.execute(
            "UPDATE counters SET consumed = 0, period = ?4 WHERE owner = ?1 AND counter = ?2 AND key = ?3 AND period != ?4",
            params![owner, counter, key, period],
        )?;
        let consumed: i64 = tx.query_row(
            "SELECT consumed FROM counters WHERE owner = ?1 AND counter = ?2 AND key = ?3",
            params![owner, counter, key],
            |row| row.get(0),
        )?;
        if consumed + amount <= cap {
            tx.execute(
                "UPDATE counters SET consumed = consumed + ?4 WHERE owner = ?1 AND counter = ?2 AND key = ?3",
                params![owner, counter, key, amount],
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
        self.list_leases_for_owner(None, resource)
    }

    pub fn list_leases_for_owner(
        &self,
        owner: Option<&str>,
        resource: Option<&str>,
    ) -> StoreResult<Vec<LeaseRow>> {
        let mut statement = self.connection.prepare(
            "SELECT owner, resource, key, holder, acquired_at, expires_at FROM leases WHERE (?1 IS NULL OR owner = ?1) AND (?2 IS NULL OR resource = ?2) ORDER BY owner, resource, key, acquired_at",
        )?;
        let rows = statement
            .query_map(params![owner.map(normalized_owner), resource], |row| {
                Ok(LeaseRow {
                    owner: row.get(0)?,
                    resource: row.get(1)?,
                    key: row.get(2)?,
                    holder: row.get(3)?,
                    acquired_at: row.get(4)?,
                    expires_at: row.get(5)?,
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
        self.list_entries_for_owner(None, ledger, partition)
    }

    pub fn list_entries_for_owner(
        &self,
        owner: Option<&str>,
        ledger: Option<&str>,
        partition: Option<&str>,
    ) -> StoreResult<Vec<LedgerEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT owner, ledger, partition, seq, payload_json, appended_by, appended_at FROM ledger_entries WHERE (?1 IS NULL OR owner = ?1) AND (?2 IS NULL OR ledger = ?2) AND (?3 IS NULL OR partition = ?3) ORDER BY owner, ledger, seq",
        )?;
        let rows = statement
            .query_map(
                params![owner.map(normalized_owner), ledger, partition],
                |row| {
                    Ok(LedgerEntry {
                        owner: row.get(0)?,
                        ledger: row.get(1)?,
                        partition: row.get(2)?,
                        seq: row.get(3)?,
                        payload_json: row.get(4)?,
                        appended_by: row.get(5)?,
                        appended_at: row.get(6)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_counters(&self, counter: Option<&str>) -> StoreResult<Vec<CounterRow>> {
        self.list_counters_for_owner(None, counter)
    }

    pub fn list_counters_for_owner(
        &self,
        owner: Option<&str>,
        counter: Option<&str>,
    ) -> StoreResult<Vec<CounterRow>> {
        let mut statement = self.connection.prepare(
            "SELECT owner, counter, key, consumed, period FROM counters WHERE (?1 IS NULL OR owner = ?1) AND (?2 IS NULL OR counter = ?2) ORDER BY owner, counter, key",
        )?;
        let rows = statement
            .query_map(params![owner.map(normalized_owner), counter], |row| {
                Ok(CounterRow {
                    owner: row.get(0)?,
                    counter: row.get(1)?,
                    key: row.get(2)?,
                    consumed: row.get(3)?,
                    period: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

fn normalized_owner(owner: &str) -> &str {
    if owner.trim().is_empty() {
        DEFAULT_COORDINATION_OWNER
    } else {
        owner
    }
}

fn ensure_partitioned_schema(connection: &Connection) -> StoreResult<()> {
    if !table_exists(connection, "leases")? {
        create_leases_table(connection)?;
    } else if !column_exists(connection, "leases", "owner")? {
        migrate_leases_table(connection)?;
    }

    if !table_exists(connection, "ledger_entries")? {
        create_ledger_entries_table(connection)?;
    } else if !column_exists(connection, "ledger_entries", "owner")? {
        migrate_ledger_entries_table(connection)?;
    }

    if !table_exists(connection, "ledger_seq")? {
        create_ledger_seq_table(connection)?;
    } else if !column_exists(connection, "ledger_seq", "owner")? {
        migrate_ledger_seq_table(connection)?;
    }

    if !table_exists(connection, "counters")? {
        create_counters_table(connection)?;
    } else if !column_exists(connection, "counters", "owner")? {
        migrate_counters_table(connection)?;
    }

    connection.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_leases_holder ON leases(holder);
        CREATE INDEX IF NOT EXISTS idx_ledger_partition ON ledger_entries(owner, ledger, partition, seq);
        "#,
    )?;
    Ok(())
}

fn create_leases_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE leases (
            owner TEXT NOT NULL,
            resource TEXT NOT NULL,
            key TEXT NOT NULL,
            holder TEXT NOT NULL,
            acquired_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            expires_at TEXT NOT NULL,
            PRIMARY KEY (owner, resource, key, holder)
        );
        "#,
    )?;
    Ok(())
}

fn create_ledger_entries_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE ledger_entries (
            owner TEXT NOT NULL,
            ledger TEXT NOT NULL,
            partition TEXT NOT NULL,
            seq INTEGER NOT NULL,
            payload_json TEXT NOT NULL,
            appended_by TEXT NOT NULL,
            appended_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (owner, ledger, seq)
        );
        "#,
    )?;
    Ok(())
}

fn create_ledger_seq_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE ledger_seq (
            owner TEXT NOT NULL,
            ledger TEXT NOT NULL,
            next_seq INTEGER NOT NULL,
            PRIMARY KEY (owner, ledger)
        );
        "#,
    )?;
    Ok(())
}

fn create_counters_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE counters (
            owner TEXT NOT NULL,
            counter TEXT NOT NULL,
            key TEXT NOT NULL,
            consumed INTEGER NOT NULL DEFAULT 0,
            period TEXT NOT NULL,
            PRIMARY KEY (owner, counter, key)
        );
        "#,
    )?;
    Ok(())
}

fn migrate_leases_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        DROP TABLE IF EXISTS leases_unpartitioned;
        ALTER TABLE leases RENAME TO leases_unpartitioned;
        "#,
    )?;
    create_leases_table(connection)?;
    connection.execute(
        r#"
        INSERT INTO leases (owner, resource, key, holder, acquired_at, expires_at)
        SELECT ?1, resource, key, holder, acquired_at, expires_at
        FROM leases_unpartitioned
        "#,
        params![DEFAULT_COORDINATION_OWNER],
    )?;
    connection.execute_batch("DROP TABLE leases_unpartitioned;")?;
    Ok(())
}

fn migrate_ledger_entries_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_ledger_partition;
        DROP TABLE IF EXISTS ledger_entries_unpartitioned;
        ALTER TABLE ledger_entries RENAME TO ledger_entries_unpartitioned;
        "#,
    )?;
    create_ledger_entries_table(connection)?;
    connection.execute(
        r#"
        INSERT INTO ledger_entries (
            owner,
            ledger,
            partition,
            seq,
            payload_json,
            appended_by,
            appended_at
        )
        SELECT ?1, ledger, partition, seq, payload_json, appended_by, appended_at
        FROM ledger_entries_unpartitioned
        "#,
        params![DEFAULT_COORDINATION_OWNER],
    )?;
    connection.execute_batch("DROP TABLE ledger_entries_unpartitioned;")?;
    Ok(())
}

fn migrate_ledger_seq_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        DROP TABLE IF EXISTS ledger_seq_unpartitioned;
        ALTER TABLE ledger_seq RENAME TO ledger_seq_unpartitioned;
        "#,
    )?;
    create_ledger_seq_table(connection)?;
    connection.execute(
        r#"
        INSERT INTO ledger_seq (owner, ledger, next_seq)
        SELECT ?1, ledger, next_seq
        FROM ledger_seq_unpartitioned
        "#,
        params![DEFAULT_COORDINATION_OWNER],
    )?;
    connection.execute_batch("DROP TABLE ledger_seq_unpartitioned;")?;
    Ok(())
}

fn migrate_counters_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        DROP TABLE IF EXISTS counters_unpartitioned;
        ALTER TABLE counters RENAME TO counters_unpartitioned;
        "#,
    )?;
    create_counters_table(connection)?;
    connection.execute(
        r#"
        INSERT INTO counters (owner, counter, key, consumed, period)
        SELECT ?1, counter, key, consumed, period
        FROM counters_unpartitioned
        "#,
        params![DEFAULT_COORDINATION_OWNER],
    )?;
    connection.execute_batch("DROP TABLE counters_unpartitioned;")?;
    Ok(())
}

fn table_exists(connection: &Connection, table: &str) -> StoreResult<bool> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> StoreResult<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `payload_json` parsed leniently for projections.
pub fn entry_payload(entry: &LedgerEntry) -> Value {
    serde_json::from_str(&entry.payload_json).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "whipplescript-coordination-{}-{}-{}.sqlite",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
        ))
    }

    fn store() -> CoordinationStore {
        let path = store_path("test");
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

    #[test]
    fn coordination_resources_are_partitioned_by_owner() {
        let mut store = store();
        assert_eq!(
            store
                .try_acquire_for_owner("local/Ship", "deploy", "prod", 1, 600, "ins_a")
                .expect("op"),
            AcquireOutcome::Held
        );
        assert_eq!(
            store
                .try_acquire_for_owner("local/Audit", "deploy", "prod", 1, 600, "ins_b")
                .expect("op"),
            AcquireOutcome::Held
        );
        assert_eq!(
            store
                .try_acquire_for_owner("local/Ship", "deploy", "prod", 1, 600, "ins_c")
                .expect("op"),
            AcquireOutcome::Contended {
                holders: vec!["ins_a".to_owned()]
            }
        );

        assert_eq!(
            store
                .consume_for_owner("local/Ship", "budget", "cust", 600, 1000, "2026-06-10")
                .expect("op"),
            ConsumeOutcome::Ok { remaining: 400 }
        );
        assert_eq!(
            store
                .consume_for_owner("local/Audit", "budget", "cust", 600, 1000, "2026-06-10")
                .expect("op"),
            ConsumeOutcome::Ok { remaining: 400 }
        );

        let ship_seq = store
            .append_for_owner("local/Ship", "decisions", "api", "{}", "ins_a", 3600)
            .expect("op");
        let audit_seq = store
            .append_for_owner("local/Audit", "decisions", "api", "{}", "ins_b", 3600)
            .expect("op");
        assert_eq!(ship_seq, 1);
        assert_eq!(audit_seq, 1);

        let ship_leases = store
            .list_leases_for_owner(Some("local/Ship"), Some("deploy"))
            .expect("list");
        assert_eq!(ship_leases.len(), 1);
        assert_eq!(ship_leases[0].owner, "local/Ship");
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

    #[test]
    fn open_migrates_unpartitioned_coordination_rows_to_shared_owner() {
        let path = store_path("migration");
        {
            let connection = Connection::open(&path).expect("open raw");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE leases (
                        resource TEXT NOT NULL,
                        key TEXT NOT NULL,
                        holder TEXT NOT NULL,
                        acquired_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                        expires_at TEXT NOT NULL,
                        PRIMARY KEY (resource, key, holder)
                    );
                    INSERT INTO leases (resource, key, holder, expires_at)
                    VALUES ('deploy', 'prod', 'ins_old', datetime('now', '+600 seconds'));

                    CREATE TABLE ledger_entries (
                        ledger TEXT NOT NULL,
                        partition TEXT NOT NULL,
                        seq INTEGER NOT NULL,
                        payload_json TEXT NOT NULL,
                        appended_by TEXT NOT NULL,
                        appended_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                        PRIMARY KEY (ledger, seq)
                    );
                    CREATE TABLE ledger_seq (
                        ledger TEXT PRIMARY KEY,
                        next_seq INTEGER NOT NULL
                    );
                    INSERT INTO ledger_seq (ledger, next_seq) VALUES ('decisions', 2);
                    INSERT INTO ledger_entries (ledger, partition, seq, payload_json, appended_by)
                    VALUES ('decisions', 'api', 1, '{"choice":"rest"}', 'ins_old');

                    CREATE TABLE counters (
                        counter TEXT NOT NULL,
                        key TEXT NOT NULL,
                        consumed INTEGER NOT NULL DEFAULT 0,
                        period TEXT NOT NULL,
                        PRIMARY KEY (counter, key)
                    );
                    INSERT INTO counters (counter, key, consumed, period)
                    VALUES ('budget', 'cust', 600, '2026-06-10');
                    "#,
                )
                .expect("seed legacy schema");
        }

        let store = CoordinationStore::open(&path).expect("open migrated");
        assert_eq!(
            store
                .list_leases_for_owner(Some(DEFAULT_COORDINATION_OWNER), Some("deploy"))
                .expect("leases")[0]
                .owner,
            DEFAULT_COORDINATION_OWNER
        );
        assert_eq!(
            store
                .list_entries_for_owner(Some(DEFAULT_COORDINATION_OWNER), Some("decisions"), None)
                .expect("entries")[0]
                .owner,
            DEFAULT_COORDINATION_OWNER
        );
        assert_eq!(
            store
                .list_counters_for_owner(Some(DEFAULT_COORDINATION_OWNER), Some("budget"))
                .expect("counters")[0]
                .owner,
            DEFAULT_COORDINATION_OWNER
        );
        let _ = std::fs::remove_file(path);
    }
}
