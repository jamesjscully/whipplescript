//! WhippleScript-native versioned workspace.
//!
//! A workspace is a content-addressed database projected into ordinary
//! directories at execution boundaries. Immutable manifest cuts form a DAG;
//! named lines point at cuts; materialized working sets are imported atomically
//! when a host declares a quiescence point. Git is neither an implementation
//! detail nor an interchange format.

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::result;

pub const EXPORT_FORMAT: &str = "whipplescript-workspace-v1";
const SCHEMA_VERSION: i64 = 1;
const EMPTY_PARENT: &str = "";

pub type Result<T> = result::Result<T, WorkspaceError>;
pub type Manifest = BTreeMap<String, String>;

#[derive(Debug)]
pub enum WorkspaceError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Invalid(String),
    Conflict(String),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "workspace I/O failed: {error}"),
            Self::Sqlite(error) => write!(f, "workspace store failed: {error}"),
            Self::Json(error) => write!(f, "workspace data is invalid: {error}"),
            Self::Invalid(message) => f.write_str(message),
            Self::Conflict(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for WorkspaceError {}
impl From<std::io::Error> for WorkspaceError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<rusqlite::Error> for WorkspaceError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}
impl From<serde_json::Error> for WorkspaceError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CutId(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineView {
    pub name: String,
    pub head: CutId,
    pub upstream: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeVerdict {
    Clean { manifest: Manifest },
    Conflict { conflicts: Vec<WorkspaceConflict> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceConflict {
    pub path: String,
    pub base: Option<String>,
    pub target: Option<String>,
    pub source: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorkspaceStore {
    root: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct ExportEnvelope {
    format: String,
    workspace_id: String,
    lines: Vec<ExportLine>,
    cuts: Vec<ExportCut>,
    blobs: BTreeMap<String, Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
struct ExportLine {
    name: String,
    head: String,
    upstream: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ExportCut {
    id: String,
    parent: Option<String>,
    merge_parent: Option<String>,
    change_id: String,
    message: String,
    manifest: Manifest,
}

impl WorkspaceStore {
    pub fn init(root: impl Into<PathBuf>) -> Result<Self> {
        let store = Self { root: root.into() };
        fs::create_dir_all(store.materializations_dir())?;
        let mut connection = store.connect()?;
        let tx = connection.transaction()?;
        let workspace_id =
            hash_bytes(format!("{}:{}", store.root.display(), unique_nonce()).as_bytes());
        tx.execute(
            "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES ('workspace_id', ?1)",
            [&workspace_id],
        )?;
        let empty = Manifest::new();
        let root_cut = insert_cut(&tx, None, None, "root", "initialize workspace", &empty)?;
        tx.execute(
            "INSERT OR IGNORE INTO workspace_lines (name, head_cut, upstream) VALUES ('main', ?1, NULL)",
            [&root_cut.0],
        )?;
        tx.commit()?;
        store.materialize("main", &store.materialization_path("main"))?;
        Ok(store)
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let store = Self { root: root.into() };
        if !store.db_path().is_file() {
            return Err(WorkspaceError::Invalid(format!(
                "no WhippleScript workspace at `{}`",
                store.root.display()
            )));
        }
        let connection = store.connect()?;
        let version: i64 = connection
            .query_row(
                "SELECT value FROM workspace_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )?
            .parse()
            .map_err(|_| WorkspaceError::Invalid("invalid workspace schema version".into()))?;
        if version != SCHEMA_VERSION {
            return Err(WorkspaceError::Invalid(format!(
                "unsupported workspace schema version {version}"
            )));
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn workspace_id(&self) -> Result<String> {
        Ok(self.connect()?.query_row(
            "SELECT value FROM workspace_meta WHERE key = 'workspace_id'",
            [],
            |row| row.get(0),
        )?)
    }
    pub fn materializations_dir(&self) -> PathBuf {
        self.root.join("materialized")
    }
    pub fn materialization_path(&self, line: &str) -> PathBuf {
        self.materializations_dir().join(encode_line(line))
    }

    pub fn line(&self, name: &str) -> Result<LineView> {
        validate_line(name)?;
        self.connect()?
            .query_row(
                "SELECT head_cut, upstream FROM workspace_lines WHERE name = ?1",
                [name],
                |row| {
                    Ok(LineView {
                        name: name.to_owned(),
                        head: CutId(row.get(0)?),
                        upstream: row.get(1)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| WorkspaceError::Invalid(format!("unknown workspace line `{name}`")))
    }

    pub fn lines(&self) -> Result<Vec<LineView>> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT name, head_cut, upstream FROM workspace_lines ORDER BY name")?;
        let rows = statement
            .query_map([], |row| {
                Ok(LineView {
                    name: row.get(0)?,
                    head: CutId(row.get(1)?),
                    upstream: row.get(2)?,
                })
            })?
            .collect::<result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn create_line(&self, name: &str, from: &str, upstream: Option<&str>) -> Result<()> {
        validate_line(name)?;
        let from_line = self.line(from)?;
        if let Some(parent) = upstream {
            self.line(parent)?;
        }
        self.connect()?.execute(
            "INSERT INTO workspace_lines (name, head_cut, upstream) VALUES (?1, ?2, ?3)",
            params![name, from_line.head.0, upstream],
        )?;
        self.materialize(name, &self.materialization_path(name))
    }

    /// Bind a named line to the real directory an embedding host uses as its
    /// execution working set, then project the current cut there. The binding
    /// is deployment-local and deliberately absent from exports.
    pub fn bind_materialization(&self, line: &str, path: &Path) -> Result<()> {
        self.line(line)?;
        self.connect()?.execute(
            "INSERT INTO workspace_materializations (line_name, path) VALUES (?1, ?2) \
             ON CONFLICT(line_name) DO UPDATE SET path = excluded.path",
            params![line, path.to_string_lossy()],
        )?;
        self.materialize(line, path)
    }

    pub fn bound_materialization(&self, line: &str) -> Result<PathBuf> {
        self.line(line)?;
        Ok(self
            .connect()?
            .query_row(
                "SELECT path FROM workspace_materializations WHERE line_name = ?1",
                [line],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(PathBuf::from)
            .unwrap_or_else(|| self.materialization_path(line)))
    }

    pub fn set_upstream(&self, name: &str, upstream: Option<&str>) -> Result<()> {
        self.line(name)?;
        if let Some(parent) = upstream {
            self.line(parent)?;
        }
        self.connect()?.execute(
            "UPDATE workspace_lines SET upstream = ?1 WHERE name = ?2",
            params![upstream, name],
        )?;
        Ok(())
    }

    pub fn remove_line(&self, name: &str) -> Result<()> {
        if name == "main" {
            return Err(WorkspaceError::Invalid("main cannot be removed".into()));
        }
        validate_line(name)?;
        let path = self
            .bound_materialization(name)
            .unwrap_or_else(|_| self.materialization_path(name));
        self.connect()?
            .execute("DELETE FROM workspace_lines WHERE name = ?1", [name])?;
        self.connect()?.execute(
            "DELETE FROM workspace_materializations WHERE line_name = ?1",
            [name],
        )?;
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
        Ok(())
    }

    pub fn cut(&self, line: &str, directory: &Path, message: &str) -> Result<Option<CutId>> {
        let current = self.line(line)?;
        let files = scan_directory(directory)?;
        let workspace_id = self.workspace_id()?;
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let manifest = store_files(&tx, &files)?;
        if manifest == load_manifest(&tx, &current.head.0)? {
            return Ok(None);
        }
        let change_id = hash_bytes(format!("{workspace_id}:{line}:{}", unique_nonce()).as_bytes());
        let cut = insert_cut(
            &tx,
            Some(&current.head.0),
            None,
            &change_id,
            message,
            &manifest,
        )?;
        tx.execute(
            "UPDATE workspace_lines SET head_cut = ?1 WHERE name = ?2",
            params![cut.0, line],
        )?;
        tx.commit()?;
        Ok(Some(cut))
    }

    pub fn materialize(&self, line: &str, destination: &Path) -> Result<()> {
        let line = self.line(line)?;
        let connection = self.connect()?;
        let manifest = load_manifest(&connection, &line.head.0)?;
        materialize_manifest(&connection, &manifest, destination)
    }

    pub fn restore_line(&self, line: &str, cut: &CutId) -> Result<()> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        load_manifest(&tx, &cut.0)?;
        tx.execute(
            "UPDATE workspace_lines SET head_cut = ?1 WHERE name = ?2",
            params![cut.0, line],
        )?;
        if tx.changes() == 0 {
            return Err(WorkspaceError::Invalid(format!(
                "unknown workspace line `{line}`"
            )));
        }
        tx.commit()?;
        self.materialize(line, &self.bound_materialization(line)?)
    }

    pub fn restore_from_upstream(&self, line: &str) -> Result<()> {
        let view = self.line(line)?;
        let upstream = view
            .upstream
            .ok_or_else(|| WorkspaceError::Invalid(format!("line `{line}` has no upstream")))?;
        let head = self.line(&upstream)?.head;
        self.restore_line(line, &head)
    }

    pub fn merge_probe(&self, source: &str, target: &str) -> Result<MergeVerdict> {
        let connection = self.connect()?;
        let source_head = self.line(source)?.head;
        let target_head = self.line(target)?.head;
        merge_verdict(&connection, &source_head.0, &target_head.0)
    }

    pub fn merge(&self, source: &str, target: &str, message: &str) -> Result<MergeVerdict> {
        let source_head = self.line(source)?.head;
        let target_head = self.line(target)?.head;
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let verdict = merge_verdict(&tx, &source_head.0, &target_head.0)?;
        let MergeVerdict::Clean { manifest } = &verdict else {
            return Ok(verdict);
        };
        if source_head == target_head {
            return Ok(verdict);
        }
        let change_id = hash_bytes(format!("merge:{}:{}", source_head.0, target_head.0).as_bytes());
        let cut = insert_cut(
            &tx,
            Some(&target_head.0),
            Some(&source_head.0),
            &change_id,
            message,
            manifest,
        )?;
        tx.execute(
            "UPDATE workspace_lines SET head_cut = ?1 WHERE name = ?2",
            params![cut.0, target],
        )?;
        tx.commit()?;
        self.materialize(target, &self.bound_materialization(target)?)?;
        Ok(verdict)
    }

    pub fn diff(
        &self,
        line: &str,
        target: &str,
        working_directory: Option<&Path>,
    ) -> Result<String> {
        let connection = self.connect()?;
        let before = load_manifest(&connection, &self.line(target)?.head.0)?;
        let after = if let Some(directory) = working_directory {
            let files = scan_directory(directory)?;
            files
                .iter()
                .map(|(path, body)| (path.clone(), hash_bytes(body)))
                .collect()
        } else {
            load_manifest(&connection, &self.line(line)?.head.0)?
        };
        render_diff(&connection, &before, &after, working_directory)
    }

    pub fn status_hash(&self, line: &str, working_directory: &Path) -> Result<(bool, CutId)> {
        let head = self.line(line)?.head;
        let connection = self.connect()?;
        let expected = load_manifest(&connection, &head.0)?;
        let actual: Manifest = scan_directory(working_directory)?
            .iter()
            .map(|(path, body)| (path.clone(), hash_bytes(body)))
            .collect();
        Ok((expected != actual, head))
    }

    pub fn export(&self) -> Result<Vec<u8>> {
        let connection = self.connect()?;
        let lines = self.lines()?;
        let cut_ids = reachable_cuts(&connection, lines.iter().map(|line| line.head.0.clone()))?;
        let mut cuts = Vec::new();
        let mut blob_ids = BTreeSet::new();
        for id in cut_ids {
            let cut = read_cut(&connection, &id)?;
            blob_ids.extend(cut.manifest.values().cloned());
            cuts.push(cut);
        }
        let mut blobs = BTreeMap::new();
        for id in blob_ids {
            let body: Option<Vec<u8>> = connection
                .query_row(
                    "SELECT body FROM workspace_blobs WHERE id = ?1",
                    [&id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            if let Some(body) = body {
                blobs.insert(id, body);
            }
        }
        let envelope = ExportEnvelope {
            format: EXPORT_FORMAT.into(),
            workspace_id: self.workspace_id()?,
            lines: lines
                .into_iter()
                .map(|line| ExportLine {
                    name: line.name,
                    head: line.head.0,
                    upstream: line.upstream,
                })
                .collect(),
            cuts,
            blobs,
        };
        Ok(serde_json::to_vec(&envelope)?)
    }

    pub fn import(root: impl Into<PathBuf>, bytes: &[u8]) -> Result<Self> {
        let envelope: ExportEnvelope = serde_json::from_slice(bytes)?;
        if envelope.format != EXPORT_FORMAT {
            return Err(WorkspaceError::Invalid(format!(
                "unsupported workspace export `{}`",
                envelope.format
            )));
        }
        let store = Self::init(root)?;
        let mut connection = store.connect()?;
        let tx = connection.transaction()?;
        tx.execute("DELETE FROM workspace_lines", [])?;
        tx.execute("DELETE FROM workspace_cuts", [])?;
        tx.execute("DELETE FROM workspace_blobs", [])?;
        import_objects(&tx, &envelope)?;
        for line in envelope.lines {
            tx.execute(
                "INSERT INTO workspace_lines (name, head_cut, upstream) VALUES (?1, ?2, ?3)",
                params![line.name, line.head, line.upstream],
            )?;
        }
        tx.execute(
            "UPDATE workspace_meta SET value = ?1 WHERE key = 'workspace_id'",
            [envelope.workspace_id],
        )?;
        tx.commit()?;
        for line in store.lines()? {
            store.materialize(&line.name, &store.materialization_path(&line.name))?;
        }
        Ok(store)
    }

    pub fn fork(root: impl Into<PathBuf>, source: &Self) -> Result<Self> {
        Self::import(root, &source.export()?)
    }

    pub fn pull_from(&self, source: &Self) -> Result<MergeVerdict> {
        let bytes = source.export()?;
        let envelope: ExportEnvelope = serde_json::from_slice(&bytes)?;
        let source_main = envelope
            .lines
            .iter()
            .find(|line| line.name == "main")
            .ok_or_else(|| WorkspaceError::Invalid("source workspace has no main line".into()))?
            .head
            .clone();
        let local_main = self.line("main")?.head;
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        import_objects(&tx, &envelope)?;
        let verdict = merge_verdict(&tx, &source_main, &local_main.0)?;
        let MergeVerdict::Clean { manifest } = &verdict else {
            return Ok(verdict);
        };
        if source_main != local_main.0 {
            let change_id = hash_bytes(format!("pull:{source_main}:{}", local_main.0).as_bytes());
            let cut = insert_cut(
                &tx,
                Some(&local_main.0),
                Some(&source_main),
                &change_id,
                "pull workspace lineage",
                manifest,
            )?;
            tx.execute(
                "UPDATE workspace_lines SET head_cut = ?1 WHERE name = 'main'",
                [&cut.0],
            )?;
        }
        tx.commit()?;
        self.materialize("main", &self.bound_materialization("main")?)?;
        Ok(verdict)
    }

    pub fn purge_unreachable(&self) -> Result<usize> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let heads = {
            let mut statement = tx.prepare("SELECT head_cut FROM workspace_lines")?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<result::Result<Vec<_>, _>>()?;
            rows
        };
        let reachable = reachable_cuts(&tx, heads)?;
        let mut live_blobs = BTreeSet::new();
        for cut in &reachable {
            live_blobs.extend(load_manifest(&tx, cut)?.values().cloned());
        }
        let all: Vec<String> = {
            let mut statement =
                tx.prepare("SELECT id FROM workspace_blobs WHERE body IS NOT NULL")?;
            let rows = statement
                .query_map([], |row| row.get(0))?
                .collect::<result::Result<Vec<_>, _>>()?;
            rows
        };
        let mut erased = 0;
        for id in all.into_iter().filter(|id| !live_blobs.contains(id)) {
            erased += tx.execute(
                "UPDATE workspace_blobs SET body = NULL, erased = 1 WHERE id = ?1",
                [&id],
            )?;
        }
        tx.commit()?;
        Ok(erased)
    }

    pub fn tree(&self, directory: &Path) -> Result<Vec<FileEntry>> {
        let mut entries = Vec::new();
        walk_entries(directory, directory, &mut entries)?;
        entries.sort_by(|a, b| a.path.cmp(&b.path).then(a.is_dir.cmp(&b.is_dir)));
        Ok(entries)
    }

    fn db_path(&self) -> PathBuf {
        self.root.join("workspace.sqlite3")
    }
    fn connect(&self) -> Result<Connection> {
        fs::create_dir_all(&self.root)?;
        let connection = Connection::open(self.db_path())?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )?;
        ensure_schema(&connection)?;
        Ok(connection)
    }
}

fn ensure_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(&format!(r#"
        CREATE TABLE IF NOT EXISTS workspace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        INSERT OR IGNORE INTO workspace_meta (key, value) VALUES ('schema_version', '{SCHEMA_VERSION}');
        CREATE TABLE IF NOT EXISTS workspace_blobs (
            id TEXT PRIMARY KEY, body BLOB, byte_len INTEGER NOT NULL, erased INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS workspace_cuts (
            id TEXT PRIMARY KEY, parent_cut TEXT, merge_parent_cut TEXT, change_id TEXT NOT NULL,
            message TEXT NOT NULL, manifest_json TEXT NOT NULL,
            FOREIGN KEY(parent_cut) REFERENCES workspace_cuts(id),
            FOREIGN KEY(merge_parent_cut) REFERENCES workspace_cuts(id)
        );
        CREATE TABLE IF NOT EXISTS workspace_lines (
            name TEXT PRIMARY KEY, head_cut TEXT NOT NULL, upstream TEXT,
            FOREIGN KEY(head_cut) REFERENCES workspace_cuts(id)
        );
        CREATE TABLE IF NOT EXISTS workspace_materializations (
            line_name TEXT PRIMARY KEY, path TEXT NOT NULL
        );
    "#))?;
    Ok(())
}

fn insert_cut(
    connection: &Transaction<'_>,
    parent: Option<&str>,
    merge_parent: Option<&str>,
    change_id: &str,
    message: &str,
    manifest: &Manifest,
) -> Result<CutId> {
    let json = serde_json::to_string(manifest)?;
    let id = hash_bytes(
        format!(
            "{}\0{}\0{}\0{}\0{}",
            parent.unwrap_or(EMPTY_PARENT),
            merge_parent.unwrap_or(EMPTY_PARENT),
            change_id,
            message,
            json
        )
        .as_bytes(),
    );
    connection.execute("INSERT OR IGNORE INTO workspace_cuts (id, parent_cut, merge_parent_cut, change_id, message, manifest_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![id, parent, merge_parent, change_id, message, json])?;
    Ok(CutId(id))
}

fn store_files(
    connection: &Transaction<'_>,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Manifest> {
    let mut manifest = Manifest::new();
    for (path, body) in files {
        let id = hash_bytes(body);
        connection.execute("INSERT OR IGNORE INTO workspace_blobs (id, body, byte_len, erased) VALUES (?1, ?2, ?3, 0)", params![id, body, body.len() as i64])?;
        manifest.insert(path.clone(), id);
    }
    Ok(manifest)
}

fn load_manifest(connection: &Connection, cut: &str) -> Result<Manifest> {
    let json: Option<String> = connection
        .query_row(
            "SELECT manifest_json FROM workspace_cuts WHERE id = ?1",
            [cut],
            |row| row.get(0),
        )
        .optional()?;
    let json =
        json.ok_or_else(|| WorkspaceError::Invalid(format!("unknown workspace cut `{cut}`")))?;
    Ok(serde_json::from_str(&json)?)
}

fn read_cut(connection: &Connection, id: &str) -> Result<ExportCut> {
    let row = connection.query_row("SELECT parent_cut, merge_parent_cut, change_id, message, manifest_json FROM workspace_cuts WHERE id = ?1", [id], |row| {
        let json: String = row.get(4)?;
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, json))
    }).optional()?;
    let Some((parent, merge_parent, change_id, message, json)) = row else {
        return Err(WorkspaceError::Invalid(format!(
            "unknown workspace cut `{id}`"
        )));
    };
    Ok(ExportCut {
        id: id.into(),
        parent,
        merge_parent,
        change_id,
        message,
        manifest: serde_json::from_str(&json)?,
    })
}

fn import_objects(connection: &Transaction<'_>, envelope: &ExportEnvelope) -> Result<()> {
    for (id, body) in &envelope.blobs {
        if hash_bytes(body) != *id {
            return Err(WorkspaceError::Invalid(format!(
                "workspace export blob `{id}` failed its content hash"
            )));
        }
        connection.execute("INSERT OR IGNORE INTO workspace_blobs (id, body, byte_len, erased) VALUES (?1, ?2, ?3, 0)", params![id, body, body.len() as i64])?;
    }
    let mut remaining: BTreeMap<&str, &ExportCut> = envelope
        .cuts
        .iter()
        .map(|cut| (cut.id.as_str(), cut))
        .collect();
    while !remaining.is_empty() {
        let before = remaining.len();
        let ready: Vec<&str> = remaining
            .iter()
            .filter(|(_, cut)| {
                [cut.parent.as_deref(), cut.merge_parent.as_deref()]
                    .into_iter()
                    .flatten()
                    .all(|parent| !remaining.contains_key(parent))
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ready {
            let cut = remaining
                .remove(id)
                .ok_or_else(|| WorkspaceError::Invalid("invalid cut import ordering".into()))?;
            let json = serde_json::to_string(&cut.manifest)?;
            connection.execute("INSERT OR IGNORE INTO workspace_cuts (id, parent_cut, merge_parent_cut, change_id, message, manifest_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![cut.id, cut.parent, cut.merge_parent, cut.change_id, cut.message, json])?;
        }
        if remaining.len() == before {
            return Err(WorkspaceError::Invalid(
                "workspace export has a cyclic or incomplete cut graph".into(),
            ));
        }
    }
    Ok(())
}

fn merge_verdict(connection: &Connection, source: &str, target: &str) -> Result<MergeVerdict> {
    let base = common_ancestor(connection, source, target)?;
    let base_manifest = load_manifest(connection, &base)?;
    let source_manifest = load_manifest(connection, source)?;
    let target_manifest = load_manifest(connection, target)?;
    let paths: BTreeSet<String> = base_manifest
        .keys()
        .chain(source_manifest.keys())
        .chain(target_manifest.keys())
        .cloned()
        .collect();
    let mut merged = Manifest::new();
    let mut conflicts = Vec::new();
    for path in paths {
        let base_value = base_manifest.get(&path).cloned();
        let source_value = source_manifest.get(&path).cloned();
        let target_value = target_manifest.get(&path).cloned();
        let selected = if source_value == target_value || target_value == base_value {
            source_value.clone()
        } else if source_value == base_value {
            target_value.clone()
        } else {
            conflicts.push(WorkspaceConflict {
                path: path.clone(),
                base: base_value,
                target: target_value,
                source: source_value,
            });
            continue;
        };
        if let Some(hash) = selected {
            merged.insert(path, hash);
        }
    }
    if conflicts.is_empty() {
        Ok(MergeVerdict::Clean { manifest: merged })
    } else {
        Ok(MergeVerdict::Conflict { conflicts })
    }
}

fn common_ancestor(connection: &Connection, a: &str, b: &str) -> Result<String> {
    let a_ancestors = ancestor_distances(connection, a)?;
    let b_ancestors = ancestor_distances(connection, b)?;
    a_ancestors
        .iter()
        .filter_map(|(id, ad)| b_ancestors.get(id).map(|bd| (id, ad + bd)))
        .min_by_key(|(_, distance)| *distance)
        .map(|(id, _)| id.clone())
        .ok_or_else(|| WorkspaceError::Conflict("workspace lines do not share lineage".into()))
}

fn ancestor_distances(connection: &Connection, head: &str) -> Result<BTreeMap<String, usize>> {
    let mut distances = BTreeMap::new();
    let mut queue = VecDeque::from([(head.to_owned(), 0usize)]);
    while let Some((id, distance)) = queue.pop_front() {
        if distances.get(&id).is_some_and(|old| *old <= distance) {
            continue;
        }
        distances.insert(id.clone(), distance);
        let parents: Option<(Option<String>, Option<String>)> = connection
            .query_row(
                "SELECT parent_cut, merge_parent_cut FROM workspace_cuts WHERE id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((parent, merge_parent)) = parents else {
            return Err(WorkspaceError::Invalid(format!(
                "unknown workspace cut `{id}`"
            )));
        };
        for parent in [parent, merge_parent].into_iter().flatten() {
            queue.push_back((parent, distance + 1));
        }
    }
    Ok(distances)
}

fn reachable_cuts(
    connection: &Connection,
    heads: impl IntoIterator<Item = String>,
) -> Result<BTreeSet<String>> {
    let mut reachable = BTreeSet::new();
    let mut queue: VecDeque<String> = heads.into_iter().collect();
    while let Some(id) = queue.pop_front() {
        if !reachable.insert(id.clone()) {
            continue;
        }
        let (parent, merge_parent): (Option<String>, Option<String>) = connection.query_row(
            "SELECT parent_cut, merge_parent_cut FROM workspace_cuts WHERE id = ?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        queue.extend([parent, merge_parent].into_iter().flatten());
    }
    Ok(reachable)
}

fn materialize_manifest(
    connection: &Connection,
    manifest: &Manifest,
    destination: &Path,
) -> Result<()> {
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    fs::create_dir_all(destination)?;
    for (relative, hash) in manifest {
        let path = safe_join(destination, relative)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body: Option<Vec<u8>> = connection
            .query_row(
                "SELECT body FROM workspace_blobs WHERE id = ?1",
                [hash],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let body = body.ok_or_else(|| {
            WorkspaceError::Invalid(format!(
                "content `{hash}` for `{relative}` was erased or is missing"
            ))
        })?;
        fs::write(path, body)?;
    }
    Ok(())
}

fn scan_directory(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    scan_directory_into(root, root, &mut files)?;
    Ok(files)
}

fn scan_directory_into(
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    if !current.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_symlink() {
            return Err(WorkspaceError::Invalid(format!(
                "workspace does not admit symbolic link `{}`",
                path.display()
            )));
        }
        if kind.is_dir() {
            scan_directory_into(root, &path, files)?;
        } else if kind.is_file() {
            let relative = path.strip_prefix(root).map_err(|_| {
                WorkspaceError::Invalid("workspace path escaped its materialization".into())
            })?;
            files.insert(path_string(relative)?, fs::read(path)?);
        }
    }
    Ok(())
}

fn walk_entries(root: &Path, current: &Path, entries: &mut Vec<FileEntry>) -> Result<()> {
    if !current.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|_| {
            WorkspaceError::Invalid("workspace path escaped its materialization".into())
        })?;
        entries.push(FileEntry {
            path: path_string(relative)?,
            is_dir: kind.is_dir(),
        });
        if kind.is_dir() {
            walk_entries(root, &path, entries)?;
        }
    }
    Ok(())
}

fn render_diff(
    connection: &Connection,
    before: &Manifest,
    after: &Manifest,
    working_directory: Option<&Path>,
) -> Result<String> {
    let paths: BTreeSet<String> = before.keys().chain(after.keys()).cloned().collect();
    let mut output = String::new();
    for path in paths {
        if before.get(&path) == after.get(&path) {
            continue;
        }
        let old = blob_or_empty(connection, before.get(&path))?;
        let new = if let Some(root) = working_directory {
            if after.contains_key(&path) {
                fs::read(safe_join(root, &path)?)?
            } else {
                Vec::new()
            }
        } else {
            blob_or_empty(connection, after.get(&path))?
        };
        output.push_str(&format!(
            "diff --git a/{path} b/{path}\n--- {}\n+++ {}\n",
            if before.contains_key(&path) {
                format!("a/{path}")
            } else {
                "/dev/null".into()
            },
            if after.contains_key(&path) {
                format!("b/{path}")
            } else {
                "/dev/null".into()
            }
        ));
        match (std::str::from_utf8(&old), std::str::from_utf8(&new)) {
            (Ok(old), Ok(new)) => output.push_str(&line_diff(old, new)),
            _ => output.push_str(&format!(
                "Binary files differ ({} -> {} bytes)\n",
                old.len(),
                new.len()
            )),
        }
    }
    Ok(output)
}

fn line_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut table = vec![vec![0usize; new_lines.len() + 1]; old_lines.len() + 1];
    for i in (0..old_lines.len()).rev() {
        for j in (0..new_lines.len()).rev() {
            table[i][j] = if old_lines[i] == new_lines[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut body = format!("@@ -1,{} +1,{} @@\n", old_lines.len(), new_lines.len());
    let (mut i, mut j) = (0, 0);
    while i < old_lines.len() || j < new_lines.len() {
        if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
            body.push_str(&format!(" {}\n", old_lines[i]));
            i += 1;
            j += 1;
        } else if j < new_lines.len()
            && (i == old_lines.len() || table[i][j + 1] >= table[i + 1][j])
        {
            body.push_str(&format!("+{}\n", new_lines[j]));
            j += 1;
        } else {
            body.push_str(&format!("-{}\n", old_lines[i]));
            i += 1;
        }
    }
    body
}

fn blob_or_empty(connection: &Connection, id: Option<&String>) -> Result<Vec<u8>> {
    let Some(id) = id else {
        return Ok(Vec::new());
    };
    connection
        .query_row(
            "SELECT body FROM workspace_blobs WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .flatten()
        .ok_or_else(|| {
            WorkspaceError::Invalid(format!("workspace content `{id}` was erased or is missing"))
        })
}

fn validate_line(line: &str) -> Result<()> {
    if line.is_empty()
        || line.starts_with('/')
        || line.ends_with('/')
        || line
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(WorkspaceError::Invalid(format!(
            "invalid workspace line `{line}`"
        )));
    }
    Ok(())
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(WorkspaceError::Invalid(format!(
            "workspace path `{relative}` is not relative and confined"
        )));
    }
    Ok(root.join(path))
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| {
            WorkspaceError::Invalid(format!("workspace path `{}` is not UTF-8", path.display()))
        })
}

fn encode_line(line: &str) -> String {
    line.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn unique_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}:{}:{sequence}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "whip-workspace-{name}-{}-{}",
            std::process::id(),
            unique_nonce()
        ))
    }

    #[test]
    fn branches_cut_merge_restore_and_export_without_git() {
        let root = temp("round-trip");
        let store = WorkspaceStore::init(&root).expect("init");
        let main = store.materialization_path("main");
        fs::write(main.join("shared.txt"), "base\n").expect("seed");
        let seeded = store
            .cut("main", &main, "seed")
            .expect("cut")
            .expect("changed");
        store
            .create_line("engagement/a", "main", Some("main"))
            .expect("branch");
        let branch = store.materialization_path("engagement/a");
        fs::write(branch.join("a.txt"), "from a\n").expect("write");
        store.cut("engagement/a", &branch, "turn").expect("cut");
        assert!(matches!(
            store.merge_probe("engagement/a", "main").expect("probe"),
            MergeVerdict::Clean { .. }
        ));
        store.merge("engagement/a", "main", "keep").expect("merge");
        assert_eq!(
            fs::read_to_string(main.join("a.txt")).expect("merged"),
            "from a\n"
        );
        store.restore_line("main", &seeded).expect("restore");
        assert!(!main.join("a.txt").exists());

        let export = store.export().expect("export");
        let imported_root = temp("imported");
        let imported = WorkspaceStore::import(&imported_root, &export).expect("import");
        assert_eq!(imported.line("main").expect("main").head, seeded);
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(imported_root);
    }

    #[test]
    fn both_modified_blob_conflicts_without_mutating_target() {
        let root = temp("conflict");
        let store = WorkspaceStore::init(&root).expect("init");
        let main = store.materialization_path("main");
        fs::write(main.join("same.txt"), "base").expect("seed");
        store.cut("main", &main, "seed").expect("cut");
        store.create_line("a", "main", Some("main")).expect("a");
        store.create_line("b", "main", Some("main")).expect("b");
        fs::write(store.materialization_path("a").join("same.txt"), "a").expect("write a");
        fs::write(store.materialization_path("b").join("same.txt"), "b").expect("write b");
        store
            .cut("a", &store.materialization_path("a"), "a")
            .expect("cut a");
        store
            .cut("b", &store.materialization_path("b"), "b")
            .expect("cut b");
        let head = store.line("b").expect("head").head;
        assert!(matches!(
            store.merge("a", "b", "merge").expect("merge"),
            MergeVerdict::Conflict { .. }
        ));
        assert_eq!(store.line("b").expect("after").head, head);
        assert_eq!(
            fs::read_to_string(store.materialization_path("b").join("same.txt")).expect("body"),
            "b"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn purge_erases_only_content_outside_live_lineage() {
        let root = temp("erase");
        let store = WorkspaceStore::init(&root).expect("init");
        store
            .create_line("doomed", "main", Some("main"))
            .expect("line");
        let path = store.materialization_path("doomed");
        fs::write(path.join("secret.txt"), "only here").expect("secret");
        store.cut("doomed", &path, "secret").expect("cut");
        store.remove_line("doomed").expect("remove");
        assert_eq!(store.purge_unreachable().expect("purge"), 1);
        let export = store.export().expect("export");
        assert!(!String::from_utf8_lossy(&export).contains("only here"));
        let _ = fs::remove_dir_all(root);
    }
}
