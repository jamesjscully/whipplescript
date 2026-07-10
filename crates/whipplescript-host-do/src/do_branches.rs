//! Durable-object branch tier: the `Branches` + `ContentBlobs` seams over
//! the DO's SQLite, so the versioned workspace (working sets, merge,
//! `WorkspaceVcs`) runs on the DO with the SAME store-crate logic as
//! native — parity by reuse, not reimplementation (untie-substrate
//! readiness tracker Phase 1; native counterpart
//! `whipplescript-store/src/branches.rs`).
//!
//! Schema and semantics mirror the native store exactly: O(1) pointer
//! creation, pinned branch points, optimistic head guards, fail-closed
//! terminal statuses, write-once instance binding. The DO is
//! single-writer, so the native store's transactions collapse to plain
//! statement sequences here (the same posture the coordination parity
//! impl takes). Content blobs share the DO's existing `content_blobs`
//! table (the one checkpoint manifests already live in), created
//! defensively for stores that predate it.

use std::collections::BTreeSet;

use whipplescript_store::branches::{
    AdvanceOutcome, BindOutcome, BranchRow, BranchStatus, Branches, CreateBranch,
    CreateBranchOutcome, StatusOutcome, MAINLINE_BRANCH_ID,
};
use whipplescript_store::content::ContentBlobs;
use whipplescript_store::{StoreError, StoreResult};

use crate::do_store::{
    as_opt_text, as_text, opt_text, sql_err, stable_hash_hex, text, DoSql, SqlValue,
};

pub struct DoBranches<S: DoSql> {
    sql: S,
}

impl<S: DoSql> DoBranches<S> {
    pub fn new(sql: S) -> StoreResult<Self> {
        let store = Self { sql };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&self) -> StoreResult<()> {
        for statement in [
            "CREATE TABLE IF NOT EXISTS branches (
                branch_id TEXT PRIMARY KEY,
                name TEXT,
                parent_branch_id TEXT,
                branch_point_cut_id TEXT,
                branch_point_manifest_hash TEXT,
                head_cut_id TEXT,
                head_manifest_hash TEXT,
                adopted_merge_cut_id TEXT,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                idempotency_key TEXT
            )",
            "CREATE UNIQUE INDEX IF NOT EXISTS branches_idempotency_idx
                ON branches(idempotency_key)
                WHERE idempotency_key IS NOT NULL",
            "CREATE UNIQUE INDEX IF NOT EXISTS branches_active_name_idx
                ON branches(name)
                WHERE name IS NOT NULL AND status = 'active'",
            "CREATE INDEX IF NOT EXISTS branches_parent_idx
                ON branches(parent_branch_id)",
            "CREATE TABLE IF NOT EXISTS branch_instances (
                instance_id TEXT PRIMARY KEY,
                branch_id TEXT NOT NULL,
                bound_at TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS branch_instances_branch_idx
                ON branch_instances(branch_id)",
        ] {
            self.sql.execute(statement, &[]).map_err(sql_err)?;
        }
        Ok(())
    }

    const ROW_COLUMNS: &'static str = "branch_id, name, parent_branch_id, \
        branch_point_cut_id, branch_point_manifest_hash, head_cut_id, \
        head_manifest_hash, adopted_merge_cut_id, status, created_at, \
        updated_at";

    fn decode_row(row: &[SqlValue]) -> BranchRow {
        BranchRow {
            branch_id: as_text(&row[0]),
            name: as_opt_text(&row[1]),
            parent_branch_id: as_opt_text(&row[2]),
            branch_point_cut_id: as_opt_text(&row[3]),
            branch_point_manifest_hash: as_opt_text(&row[4]),
            head_cut_id: as_opt_text(&row[5]),
            head_manifest_hash: as_opt_text(&row[6]),
            adopted_merge_cut_id: as_opt_text(&row[7]),
            status: BranchStatus::parse(&as_text(&row[8])).unwrap_or(BranchStatus::Active),
            created_at: as_text(&row[9]),
            updated_at: as_text(&row[10]),
        }
    }

    fn row_by_id(&self, branch_id: &str) -> StoreResult<Option<BranchRow>> {
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT {} FROM branches WHERE branch_id = ?1",
                    Self::ROW_COLUMNS
                ),
                &[text(branch_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.first().map(|row| Self::decode_row(row)))
    }
}

impl<S: DoSql> Branches for DoBranches<S> {
    fn ensure_mainline(&mut self, created_at: &str) -> StoreResult<BranchRow> {
        self.sql
            .execute(
                "INSERT OR IGNORE INTO branches \
                 (branch_id, name, parent_branch_id, status, created_at, updated_at) \
                 VALUES (?1, ?1, NULL, 'active', ?2, ?2)",
                &[text(MAINLINE_BRANCH_ID), text(created_at)],
            )
            .map_err(sql_err)?;
        self.row_by_id(MAINLINE_BRANCH_ID)?
            .ok_or_else(|| StoreError::Conflict("mainline row missing after insert".to_owned()))
    }

    fn create_branch(&mut self, request: CreateBranch<'_>) -> StoreResult<CreateBranchOutcome> {
        if let Some(existing) = self.row_by_id(request.branch_id)? {
            return Ok(CreateBranchOutcome::Existing(existing));
        }
        if let Some(key) = request.idempotency_key {
            let rows = self
                .sql
                .query(
                    "SELECT branch_id FROM branches WHERE idempotency_key = ?1",
                    &[text(key)],
                )
                .map_err(sql_err)?;
            if let Some(row) = rows.first() {
                let existing = self
                    .row_by_id(&as_text(&row[0]))?
                    .ok_or_else(|| StoreError::Conflict("row for key missing".to_owned()))?;
                return Ok(CreateBranchOutcome::Existing(existing));
            }
        }
        let Some(parent) = self.row_by_id(request.parent_branch_id)? else {
            return Ok(CreateBranchOutcome::ParentMissing);
        };
        if parent.status != BranchStatus::Active {
            return Ok(CreateBranchOutcome::ParentNotActive {
                status: parent.status,
            });
        }
        if let Some(name) = request.name {
            let rows = self
                .sql
                .query(
                    "SELECT branch_id FROM branches WHERE name = ?1 AND status = 'active'",
                    &[text(name)],
                )
                .map_err(sql_err)?;
            if let Some(row) = rows.first() {
                return Ok(CreateBranchOutcome::NameTaken {
                    holder_branch_id: as_text(&row[0]),
                });
            }
        }
        let (point_cut, point_manifest) = match request.at_cut {
            Some((cut, manifest)) => (Some(cut.to_owned()), Some(manifest.to_owned())),
            None => (
                parent.head_cut_id.clone(),
                parent.head_manifest_hash.clone(),
            ),
        };
        self.sql
            .execute(
                "INSERT INTO branches \
                 (branch_id, name, parent_branch_id, branch_point_cut_id, \
                  branch_point_manifest_hash, head_cut_id, head_manifest_hash, \
                  status, created_at, updated_at, idempotency_key) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?5, 'active', ?6, ?6, ?7)",
                &[
                    text(request.branch_id),
                    opt_text(request.name),
                    text(request.parent_branch_id),
                    opt_text(point_cut.as_deref()),
                    opt_text(point_manifest.as_deref()),
                    text(request.created_at),
                    opt_text(request.idempotency_key),
                ],
            )
            .map_err(sql_err)?;
        let row = self
            .row_by_id(request.branch_id)?
            .ok_or_else(|| StoreError::Conflict("created row missing".to_owned()))?;
        Ok(CreateBranchOutcome::Created(row))
    }

    fn get_branch(&self, branch_id: &str) -> StoreResult<Option<BranchRow>> {
        self.row_by_id(branch_id)
    }

    fn list_branches(&self, status: Option<BranchStatus>) -> StoreResult<Vec<BranchRow>> {
        let rows = match status {
            Some(status) => self
                .sql
                .query(
                    &format!(
                        "SELECT {} FROM branches WHERE status = ?1 ORDER BY branch_id",
                        Self::ROW_COLUMNS
                    ),
                    &[text(status.as_str())],
                )
                .map_err(sql_err)?,
            None => self
                .sql
                .query(
                    &format!(
                        "SELECT {} FROM branches ORDER BY branch_id",
                        Self::ROW_COLUMNS
                    ),
                    &[],
                )
                .map_err(sql_err)?,
        };
        Ok(rows.iter().map(|row| Self::decode_row(row)).collect())
    }

    fn list_children(&self, parent_branch_id: &str) -> StoreResult<Vec<BranchRow>> {
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT {} FROM branches WHERE parent_branch_id = ?1 ORDER BY branch_id",
                    Self::ROW_COLUMNS
                ),
                &[text(parent_branch_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.iter().map(|row| Self::decode_row(row)).collect())
    }

    fn lineage(&self, branch_id: &str) -> StoreResult<Vec<BranchRow>> {
        let mut rows = Vec::new();
        let mut cursor = Some(branch_id.to_owned());
        let mut visited = BTreeSet::new();
        while let Some(current) = cursor {
            if !visited.insert(current.clone()) {
                break;
            }
            let Some(row) = self.row_by_id(&current)? else {
                break;
            };
            cursor = row.parent_branch_id.clone();
            rows.push(row);
        }
        Ok(rows)
    }

    fn advance_head(
        &mut self,
        branch_id: &str,
        expected_head_cut_id: Option<&str>,
        cut_id: &str,
        manifest_hash: &str,
        at: &str,
    ) -> StoreResult<AdvanceOutcome> {
        let Some(row) = self.row_by_id(branch_id)? else {
            return Ok(AdvanceOutcome::NotFound);
        };
        if row.status != BranchStatus::Active {
            return Ok(AdvanceOutcome::NotActive { status: row.status });
        }
        if row.head_cut_id.as_deref() != expected_head_cut_id {
            return Ok(AdvanceOutcome::Stale {
                current_head_cut_id: row.head_cut_id,
            });
        }
        self.sql
            .execute(
                "UPDATE branches SET head_cut_id = ?2, head_manifest_hash = ?3, \
                 updated_at = ?4 WHERE branch_id = ?1",
                &[text(branch_id), text(cut_id), text(manifest_hash), text(at)],
            )
            .map_err(sql_err)?;
        let row = self
            .row_by_id(branch_id)?
            .ok_or_else(|| StoreError::Conflict("advanced row missing".to_owned()))?;
        Ok(AdvanceOutcome::Advanced(Box::new(row)))
    }

    fn rebase_branch(
        &mut self,
        branch_id: &str,
        expected_head_cut_id: Option<&str>,
        point_cut_id: &str,
        point_manifest_hash: &str,
        head_cut_id: &str,
        head_manifest_hash: &str,
        at: &str,
    ) -> StoreResult<AdvanceOutcome> {
        let Some(row) = self.row_by_id(branch_id)? else {
            return Ok(AdvanceOutcome::NotFound);
        };
        if row.status != BranchStatus::Active {
            return Ok(AdvanceOutcome::NotActive { status: row.status });
        }
        if row.head_cut_id.as_deref() != expected_head_cut_id {
            return Ok(AdvanceOutcome::Stale {
                current_head_cut_id: row.head_cut_id,
            });
        }
        self.sql
            .execute(
                "UPDATE branches SET branch_point_cut_id = ?2, \
                 branch_point_manifest_hash = ?3, head_cut_id = ?4, \
                 head_manifest_hash = ?5, updated_at = ?6 WHERE branch_id = ?1",
                &[
                    text(branch_id),
                    text(point_cut_id),
                    text(point_manifest_hash),
                    text(head_cut_id),
                    text(head_manifest_hash),
                    text(at),
                ],
            )
            .map_err(sql_err)?;
        let row = self
            .row_by_id(branch_id)?
            .ok_or_else(|| StoreError::Conflict("rebased row missing".to_owned()))?;
        Ok(AdvanceOutcome::Advanced(Box::new(row)))
    }

    fn discard_branch(&mut self, branch_id: &str, at: &str) -> StoreResult<StatusOutcome> {
        let Some(row) = self.row_by_id(branch_id)? else {
            return Ok(StatusOutcome::NotFound);
        };
        if row.status != BranchStatus::Active {
            return Ok(StatusOutcome::InvalidTransition { from: row.status });
        }
        self.sql
            .execute(
                "UPDATE branches SET status = 'discarded', updated_at = ?2 \
                 WHERE branch_id = ?1",
                &[text(branch_id), text(at)],
            )
            .map_err(sql_err)?;
        let row = self
            .row_by_id(branch_id)?
            .ok_or_else(|| StoreError::Conflict("discarded row missing".to_owned()))?;
        Ok(StatusOutcome::Done(Box::new(row)))
    }

    fn adopt_branch(
        &mut self,
        branch_id: &str,
        merge_cut_id: &str,
        at: &str,
    ) -> StoreResult<StatusOutcome> {
        let Some(row) = self.row_by_id(branch_id)? else {
            return Ok(StatusOutcome::NotFound);
        };
        if row.status != BranchStatus::Active {
            return Ok(StatusOutcome::InvalidTransition { from: row.status });
        }
        self.sql
            .execute(
                "UPDATE branches SET status = 'adopted', adopted_merge_cut_id = ?2, \
                 updated_at = ?3 WHERE branch_id = ?1",
                &[text(branch_id), text(merge_cut_id), text(at)],
            )
            .map_err(sql_err)?;
        let row = self
            .row_by_id(branch_id)?
            .ok_or_else(|| StoreError::Conflict("adopted row missing".to_owned()))?;
        Ok(StatusOutcome::Done(Box::new(row)))
    }

    fn bind_instance(
        &mut self,
        instance_id: &str,
        branch_id: &str,
        at: &str,
    ) -> StoreResult<BindOutcome> {
        let existing = self
            .sql
            .query(
                "SELECT branch_id FROM branch_instances WHERE instance_id = ?1",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        if let Some(row) = existing.first() {
            let existing_branch = as_text(&row[0]);
            return Ok(if existing_branch == branch_id {
                BindOutcome::Bound
            } else {
                BindOutcome::AlreadyBound {
                    branch_id: existing_branch,
                }
            });
        }
        let Some(row) = self.row_by_id(branch_id)? else {
            return Ok(BindOutcome::BranchMissing);
        };
        if row.status != BranchStatus::Active {
            return Ok(BindOutcome::BranchNotActive { status: row.status });
        }
        self.sql
            .execute(
                "INSERT INTO branch_instances (instance_id, branch_id, bound_at) \
                 VALUES (?1, ?2, ?3)",
                &[text(instance_id), text(branch_id), text(at)],
            )
            .map_err(sql_err)?;
        Ok(BindOutcome::Bound)
    }

    fn instance_branch(&self, instance_id: &str) -> StoreResult<Option<String>> {
        let rows = self
            .sql
            .query(
                "SELECT branch_id FROM branch_instances WHERE instance_id = ?1",
                &[text(instance_id)],
            )
            .map_err(sql_err)?;
        Ok(rows.first().map(|row| as_text(&row[0])))
    }
}

/// Content blobs over the DO's `content_blobs` table — the same table
/// checkpoint manifests live in, so branch manifests and cut manifests
/// share one blob space (exactly as native).
pub struct DoContentBlobs<S: DoSql> {
    sql: S,
}

impl<S: DoSql> DoContentBlobs<S> {
    pub fn new(sql: S) -> StoreResult<Self> {
        // Defensive for stores predating the base schema; matches the
        // do_store DDL (no created_at column on the DO).
        sql.execute(
            "CREATE TABLE IF NOT EXISTS content_blobs (
                id TEXT PRIMARY KEY,
                body TEXT NOT NULL,
                byte_len INTEGER NOT NULL
            )",
            &[],
        )
        .map_err(sql_err)?;
        Ok(Self { sql })
    }
}

impl<S: DoSql> ContentBlobs for DoContentBlobs<S> {
    fn put(&self, body: &str) -> StoreResult<String> {
        let id = stable_hash_hex(body);
        self.sql
            .execute(
                "INSERT OR IGNORE INTO content_blobs (id, body, byte_len) VALUES (?1, ?2, ?3)",
                &[text(&id), text(body), SqlValue::Int(body.len() as i64)],
            )
            .map_err(sql_err)?;
        Ok(id)
    }

    fn get(&self, id: &str) -> StoreResult<Option<String>> {
        let rows = self
            .sql
            .query("SELECT body FROM content_blobs WHERE id = ?1", &[text(id)])
            .map_err(sql_err)?;
        Ok(rows.first().map(|row| as_text(&row[0])))
    }
}
