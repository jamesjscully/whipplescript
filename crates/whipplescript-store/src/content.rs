//! Content-addressed blob store for large tool outputs (context-assembly Phase 5).
//!
//! When an owned-harness tool produces an output too large for the model context,
//! the executor middle-truncates what the model sees but stores the FULL bytes here,
//! keyed by a stable content hash, and hands the model a recall id. The `recall`
//! tool reads the full bytes back (paginated) — so truncation is lossless, not
//! lossy (the WhippleScript edge over Codex/pi). It is also what makes Layer A's
//! "full output kept as evidence" promise true, and what the `ToolResultCompactor`
//! elides old tool results down to.
//!
//! Workspace-scoped and disposable-friendly like the other auxiliary stores
//! (coordination, work items): one small table opened over a path. Identical bytes
//! dedupe to one id. The shape (put/get by hash) is durable-object portable; the
//! DO mirror lands with the DO agent-tool executor.

#[cfg(feature = "native")]
use std::path::Path;

#[cfg(feature = "native")]
use rusqlite::{params, Connection, OptionalExtension};

use crate::StoreResult;

#[cfg(feature = "native")]
pub struct ContentStore {
    connection: Connection,
}

#[cfg(feature = "native")]
impl ContentStore {
    /// Open (creating if needed) the content-addressed store at `path`.
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
            "#,
        )?;
        ensure_content_schema(&connection)?;
        Ok(Self { connection })
    }

    /// Store `body`, returning its content id (a stable hash of the bytes).
    /// Idempotent: identical bytes dedupe to the same id and one row.
    pub fn put(&self, body: &str) -> StoreResult<String> {
        let id = crate::stable_hash_hex(body);
        self.connection.execute(
            "INSERT OR IGNORE INTO content_blobs (id, body, byte_len, created_at) \
             VALUES (?1, ?2, ?3, datetime('now'))",
            params![id, body, body.len() as i64],
        )?;
        Ok(id)
    }

    /// Read the full stored bytes for a content id, or `None` if unknown.
    pub fn get(&self, id: &str) -> StoreResult<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT body FROM content_blobs WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }
}

#[cfg(feature = "native")]
fn ensure_content_schema(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS content_blobs (
            id         TEXT PRIMARY KEY,
            body       TEXT NOT NULL,
            byte_len   INTEGER NOT NULL,
            created_at TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use super::*;

    #[test]
    fn put_is_content_addressed_and_get_round_trips() {
        let dir = std::env::temp_dir().join(format!("whip-content-{}", std::process::id()));
        let path = dir.join("content.db");
        let _ = std::fs::remove_file(&path);
        let store = ContentStore::open(&path).expect("open");

        let id1 = store.put("hello world").expect("put");
        let id2 = store.put("hello world").expect("put again");
        assert_eq!(id1, id2, "identical bytes dedupe to one id");
        assert_eq!(
            store.get(&id1).expect("get").as_deref(),
            Some("hello world")
        );

        let other = store.put("different").expect("put other");
        assert_ne!(id1, other);
        assert_eq!(store.get("nonexistent").expect("get missing"), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
