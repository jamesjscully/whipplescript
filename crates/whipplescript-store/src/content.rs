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

/// The content-addressed put/get seam, object-safe so the versioned
/// workspace (working sets, manifests) runs over any host's blob table:
/// natively `ContentStore`, on the durable object a thin impl over the
/// shared `DoSql` handle. Identical bytes dedupe to one id.
pub trait ContentBlobs {
    /// Store `body`, returning its content id (a stable hash of the
    /// bytes). Idempotent.
    fn put(&self, body: &str) -> crate::StoreResult<String>;
    /// Read the full stored bytes for a content id, or `None` if unknown.
    fn get(&self, id: &str) -> crate::StoreResult<Option<String>>;
}

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
        ContentBlobs::put(self, body)
    }

    /// Read the full stored bytes for a content id, or `None` if unknown.
    pub fn get(&self, id: &str) -> StoreResult<Option<String>> {
        ContentBlobs::get(self, id)
    }
}

#[cfg(feature = "native")]
impl ContentBlobs for ContentStore {
    fn put(&self, body: &str) -> StoreResult<String> {
        let id = crate::stable_hash_hex(body);
        self.connection.execute(
            "INSERT OR IGNORE INTO content_blobs (id, body, byte_len, created_at) \
             VALUES (?1, ?2, ?3, datetime('now'))",
            params![id, body, body.len() as i64],
        )?;
        Ok(id)
    }

    fn get(&self, id: &str) -> StoreResult<Option<String>> {
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
        CREATE TABLE IF NOT EXISTS content_chunk_roots (
            root_id    TEXT PRIMARY KEY,
            byte_len   INTEGER NOT NULL,
            erased_at  TEXT
        );
        CREATE TABLE IF NOT EXISTS content_chunk_refs (
            root_id  TEXT NOT NULL,
            seq      INTEGER NOT NULL,
            chunk_id TEXT NOT NULL,
            PRIMARY KEY (root_id, seq)
        );
        CREATE INDEX IF NOT EXISTS content_chunk_refs_chunk_idx
            ON content_chunk_refs(chunk_id);
        "#,
    )?;
    Ok(())
}

/// A chunked root's surviving identity after erasure: the retained
/// hashes and size — the honesty-downgrade handle (identity and pooling
/// verdicts survive; payload and replay do not).
#[cfg(feature = "native")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkRootInfo {
    pub root_id: String,
    pub byte_len: u64,
    pub chunk_ids: Vec<String>,
    pub erased: bool,
}

#[cfg(feature = "native")]
impl ContentStore {
    /// Store a body through the chunk tier (vw note §10.1): below the
    /// config threshold this IS `put` (plain content hash, nothing
    /// upstream re-keys); above it, UTF-8-snapped FastCDC chunks land as
    /// ordinary blobs deduped across roots, and the returned root id is
    /// the file's stable identity.
    pub fn put_chunked(
        &self,
        body: &str,
        config: &crate::chunking::ChunkingConfig,
    ) -> StoreResult<String> {
        let tree = crate::chunking::chunk_str(body, config);
        if tree.is_whole_blob() {
            return self.put(body);
        }
        for chunk in &tree.chunks {
            let piece = &body[chunk.offset..chunk.offset + chunk.len];
            let stored = self.put(piece)?;
            debug_assert_eq!(stored, chunk.hash);
        }
        self.connection.execute(
            "INSERT OR IGNORE INTO content_chunk_roots (root_id, byte_len) VALUES (?1, ?2)",
            params![tree.root_hash, body.len() as i64],
        )?;
        for (seq, chunk) in tree.chunks.iter().enumerate() {
            self.connection.execute(
                "INSERT OR IGNORE INTO content_chunk_refs (root_id, seq, chunk_id)                  VALUES (?1, ?2, ?3)",
                params![tree.root_hash, seq as i64, chunk.hash],
            )?;
        }
        Ok(tree.root_hash)
    }

    /// Read through the chunk tier: a plain blob id reads directly; a
    /// chunk root reassembles. `Ok(None)` = unknown id OR an erased root
    /// (consult `chunk_root_info` for the retained identity).
    pub fn get_chunked(&self, id: &str) -> StoreResult<Option<String>> {
        if let Some(body) = self.get(id)? {
            return Ok(Some(body));
        }
        let Some(info) = self.chunk_root_info(id)? else {
            return Ok(None);
        };
        if info.erased {
            return Ok(None);
        }
        let mut body = String::with_capacity(info.byte_len as usize);
        for chunk_id in &info.chunk_ids {
            let Some(piece) = self.get(chunk_id)? else {
                return Ok(None);
            };
            body.push_str(&piece);
        }
        Ok(Some(body))
    }

    /// The retained identity of a chunk root, before or after erasure.
    pub fn chunk_root_info(&self, root_id: &str) -> StoreResult<Option<ChunkRootInfo>> {
        let root: Option<(i64, Option<String>)> = self
            .connection
            .query_row(
                "SELECT byte_len, erased_at FROM content_chunk_roots WHERE root_id = ?1",
                params![root_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((byte_len, erased_at)) = root else {
            return Ok(None);
        };
        let mut stmt = self
            .connection
            .prepare("SELECT chunk_id FROM content_chunk_refs WHERE root_id = ?1 ORDER BY seq")?;
        let mapped = stmt.query_map(params![root_id], |row| row.get::<_, String>(0))?;
        let mut chunk_ids = Vec::new();
        for row in mapped {
            chunk_ids.push(row?);
        }
        Ok(Some(ChunkRootInfo {
            root_id: root_id.to_owned(),
            byte_len: byte_len as u64,
            chunk_ids,
            erased: erased_at.is_some(),
        }))
    }

    /// Chunk-level erasure with the retained root (vw note §10.1 item 1):
    /// drop this root's chunk BODIES — except chunks still referenced by
    /// another live root (dedup sharing survives, fail-closed) — and mark
    /// the root erased. Identity (root + chunk hashes + size) is kept;
    /// payload and replay are honestly gone. Returns how many chunk
    /// bodies were erased.
    pub fn erase_chunks(&self, root_id: &str, at: &str) -> StoreResult<usize> {
        let erased = self.connection.execute(
            "DELETE FROM content_blobs WHERE id IN (
                 SELECT refs.chunk_id FROM content_chunk_refs AS refs
                 WHERE refs.root_id = ?1
                   AND NOT EXISTS (
                     SELECT 1 FROM content_chunk_refs AS other
                     JOIN content_chunk_roots AS other_root
                       ON other_root.root_id = other.root_id
                     WHERE other.chunk_id = refs.chunk_id
                       AND other.root_id != ?1
                       AND other_root.erased_at IS NULL
                   )
             )",
            params![root_id],
        )?;
        self.connection.execute(
            "UPDATE content_chunk_roots SET erased_at = ?2              WHERE root_id = ?1 AND erased_at IS NULL",
            params![root_id, at],
        )?;
        Ok(erased)
    }
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

    /// The chunk tier: large bodies round-trip through UTF-8-snapped
    /// chunks deduped across roots; small bodies keep plain identity;
    /// erasure drops payload but keeps the root's identity, and a chunk
    /// shared with a live sibling root survives (fail-closed sharing).
    #[test]
    fn chunk_tier_roundtrip_dedup_and_erasure_with_retained_root() {
        use crate::chunking::ChunkingConfig;
        let dir = std::env::temp_dir().join(format!(
            "whip-chunk-tier-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
        ));
        let store = ContentStore::open(dir.join("content.db")).expect("open");
        let config = ChunkingConfig {
            whole_blob_threshold: 256,
            min_size: 64,
            avg_size: 256,
            max_size: 1024,
        };

        // Small bodies keep their plain content identity: put_chunked IS put.
        let small = store.put_chunked("tiny", &config).expect("put small");
        assert_eq!(small, store.put("tiny").expect("put"));
        assert_eq!(
            store.get_chunked(&small).expect("get").as_deref(),
            Some("tiny")
        );

        // A large body (with multi-byte chars to exercise the UTF-8
        // snap) round-trips through the chunk tier.
        let unit = "0123456789abcdefé漢字🚀-";
        let big: String = unit.repeat(600);
        let root = store.put_chunked(&big, &config).expect("put big");
        assert_ne!(
            root,
            store.put(&big).expect("plain put"),
            "chunked identity"
        );
        assert_eq!(
            store.get_chunked(&root).expect("get").as_deref(),
            Some(big.as_str())
        );
        let info = store.chunk_root_info(&root).expect("info").expect("root");
        assert!(info.chunk_ids.len() > 1);
        assert!(!info.erased);

        // A sibling body sharing a long prefix dedupes chunks.
        let sibling = format!("{big}TAIL");
        let sibling_root = store.put_chunked(&sibling, &config).expect("put sibling");
        let sibling_info = store
            .chunk_root_info(&sibling_root)
            .expect("info")
            .expect("root");
        let shared: std::collections::BTreeSet<_> = info
            .chunk_ids
            .iter()
            .filter(|id| sibling_info.chunk_ids.contains(id))
            .collect();
        assert!(!shared.is_empty(), "prefix chunks dedupe across roots");

        // Erase the first root: payload gone, identity retained, shared
        // chunks survive for the live sibling.
        let erased = store.erase_chunks(&root, "t1").expect("erase");
        assert!(erased >= 1, "unshared chunks were dropped");
        assert_eq!(store.get_chunked(&root).expect("get"), None);
        let info_after = store.chunk_root_info(&root).expect("info").expect("root");
        assert!(info_after.erased);
        assert_eq!(info_after.chunk_ids, info.chunk_ids, "hashes retained");
        assert_eq!(info_after.byte_len, big.len() as u64);
        assert_eq!(
            store.get_chunked(&sibling_root).expect("get").as_deref(),
            Some(sibling.as_str()),
            "the sibling sharing chunks still reads whole"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
