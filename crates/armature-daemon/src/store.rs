use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use armature_core::{
    ArmatureError, ArmatureResult, EventId, EventRecord, LogRecord, ProcessState, RunId, RunOrigin,
    RunPaths, RunRecord, TriggerId, TriggerRecord, Workspace, WorkspaceRuntimePaths,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::protocol::ManualLockRecord;

const SCHEMA_VERSION: i32 = 2;
const LOCK_FILE_SUFFIX: &str = ".json";

#[derive(Debug)]
pub struct SqliteStore {
    paths: WorkspaceRuntimePaths,
    connection: Connection,
}

#[derive(Debug, Clone)]
pub struct ManualLockOwner {
    pub pid: u32,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct ManualLockStore {
    lock_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PreparedRun {
    pub record: RunRecord,
    pub logs: LogRecord,
    pub paths: RunPaths,
}

#[derive(Debug, Clone)]
pub struct NewRun {
    pub name: String,
    pub command: String,
    pub origin: RunOrigin,
    pub config_version: Option<String>,
    pub event_id: Option<EventId>,
    pub restart_of: Option<RunId>,
    pub attempt: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub event_type: Option<String>,
    pub source: Option<String>,
    pub correlation: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct RunFilter {
    pub name: Option<String>,
    pub origin: Option<String>,
    pub state: Option<String>,
    pub correlation: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct TriggerFilter {
    pub task: Option<String>,
    pub event_type: Option<String>,
    pub outcome: Option<String>,
    pub correlation: Option<String>,
    pub limit: Option<usize>,
}

impl ManualLockStore {
    pub fn new(lock_dir: impl Into<PathBuf>) -> Self {
        Self {
            lock_dir: lock_dir.into(),
        }
    }

    pub fn acquire(
        &self,
        name: String,
        owner: ManualLockOwner,
        reason: Option<String>,
        ttl: Duration,
    ) -> ArmatureResult<ManualLockRecord> {
        fs::create_dir_all(&self.lock_dir)?;
        let path = self.lock_file_path(&name);
        if let Some(existing) = self.read_lock_if_fresh(&path)? {
            return Err(ArmatureError::conflict(format!(
                "lock {:?} is already held by {}",
                existing.name, existing.owner_id
            )));
        }

        let acquired_at_ms = now_millis();
        let record = ManualLockRecord {
            name,
            owner_pid: owner.pid,
            owner_id: owner.id,
            reason,
            token: new_lock_token(),
            acquired_at_ms,
            renewed_at_ms: None,
            expires_at_ms: Some(acquired_at_ms + ttl.as_millis() as i64),
            manual: true,
        };
        self.write_new_lock(&path, &record)?;
        Ok(record)
    }

    pub fn renew(
        &self,
        name: &str,
        token: &str,
        ttl: Duration,
    ) -> ArmatureResult<ManualLockRecord> {
        let path = self.lock_file_path(name);
        let mut record = self.require_matching_lock(&path, name, token)?;
        let renewed_at_ms = now_millis();
        record.renewed_at_ms = Some(renewed_at_ms);
        record.expires_at_ms = Some(renewed_at_ms + ttl.as_millis() as i64);
        self.write_lock_replace(&path, &record)?;
        Ok(record)
    }

    pub fn release(&self, name: &str, token: &str) -> ArmatureResult<()> {
        let path = self.lock_file_path(name);
        self.require_matching_lock(&path, name, token)?;
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn list(&self) -> ArmatureResult<Vec<ManualLockRecord>> {
        let mut locks = Vec::new();
        if !self.lock_dir.exists() {
            return Ok(locks);
        }
        for entry in fs::read_dir(&self.lock_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if let Some(lock) = self.read_lock_if_fresh(&path)? {
                locks.push(lock);
            }
        }
        locks.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(locks)
    }

    fn require_matching_lock(
        &self,
        path: &Path,
        name: &str,
        token: &str,
    ) -> ArmatureResult<ManualLockRecord> {
        let Some(record) = self.read_lock_if_fresh(path)? else {
            return Err(ArmatureError::not_found(format!(
                "lock {:?} is not held",
                name
            )));
        };
        if record.token != token {
            return Err(ArmatureError::conflict(format!(
                "lock {:?} is held by a different token",
                name
            )));
        }
        Ok(record)
    }

    fn write_new_lock(&self, path: &Path, record: &ManualLockRecord) -> ArmatureResult<()> {
        let contents = serde_json::to_vec_pretty(record)
            .map_err(|error| ArmatureError::internal(error.to_string()))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::AlreadyExists => {
                    ArmatureError::conflict(format!("lock {:?} is already held", record.name))
                }
                _ => ArmatureError::internal(error.to_string()),
            })?;
        file.write_all(&contents)?;
        Ok(())
    }

    fn write_lock_replace(&self, path: &Path, record: &ManualLockRecord) -> ArmatureResult<()> {
        let contents = serde_json::to_vec_pretty(record)
            .map_err(|error| ArmatureError::internal(error.to_string()))?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, contents)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    fn read_lock_if_fresh(&self, path: &Path) -> ArmatureResult<Option<ManualLockRecord>> {
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read(path)?;
        let mut record: ManualLockRecord = serde_json::from_slice(&raw)
            .map_err(|error| ArmatureError::internal(format!("invalid lock record: {error}")))?;
        if record.owner_id.is_empty() {
            record.owner_id = format!("pid:{}", record.owner_pid);
        }
        if lock_is_stale(&record) {
            let _ = fs::remove_file(path);
            return Ok(None);
        }
        Ok(Some(record))
    }

    fn lock_file_path(&self, name: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(name.as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        self.lock_dir.join(format!("{digest}{LOCK_FILE_SUFFIX}"))
    }
}

impl SqliteStore {
    pub fn open(workspace: &Workspace) -> ArmatureResult<Self> {
        let paths = WorkspaceRuntimePaths::for_workspace(workspace)?;
        Self::open_with_paths(paths)
    }

    pub fn open_with_paths(paths: WorkspaceRuntimePaths) -> ArmatureResult<Self> {
        paths.ensure_state_root()?;
        let connection = Connection::open(paths.database_path()).map_err(map_sqlite_error)?;
        let store = Self { paths, connection };
        store.bootstrap()?;
        Ok(store)
    }

    pub fn paths(&self) -> &WorkspaceRuntimePaths {
        &self.paths
    }

    pub fn record_event(&self, event: &EventRecord) -> ArmatureResult<()> {
        let payload_json = serde_json::to_string(&event.payload)
            .map_err(|error| ArmatureError::internal(error.to_string()))?;

        self.connection
            .execute(
                "INSERT INTO events (
                    id, event_type, time, payload_json, routing, config_version, source,
                    source_run_id, parent_event_id, correlation_id, created_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    event.id.as_str(),
                    event.event_type,
                    event.time,
                    payload_json,
                    enum_to_sql(&event.routing)?,
                    event.config_version,
                    event.source,
                    event.source_run_id.as_ref().map(RunId::as_str),
                    event.parent_event_id.as_ref().map(EventId::as_str),
                    event.correlation_id,
                    event_time_to_millis(&event.time).unwrap_or_else(now_millis),
                ],
            )
            .map_err(map_sqlite_error)?;

        Ok(())
    }

    pub fn get_event(&self, event_id: &EventId) -> ArmatureResult<Option<EventRecord>> {
        self.connection
            .query_row(
                "SELECT id, event_type, time, payload_json, routing, config_version, source,
                        source_run_id, parent_event_id, correlation_id, created_at_ms
                 FROM events
                 WHERE id = ?1",
                params![event_id.as_str()],
                |row| {
                    let id = row.get::<_, String>(0)?;
                    let time = row.get::<_, String>(2)?;
                    let payload_json = row.get::<_, String>(3)?;
                    Ok(EventRecord {
                        id: EventId::parse(id).map_err(to_sqlite_user_error)?,
                        event_type: row.get(1)?,
                        time: if time.is_empty() {
                            millis_to_rfc3339(row.get(10)?)
                        } else {
                            time
                        },
                        payload: serde_json::from_str(&payload_json)
                            .map_err(to_sqlite_data_error)?,
                        routing: enum_from_sql(&row.get::<_, String>(4)?)
                            .map_err(to_sqlite_user_error)?,
                        config_version: row.get(5)?,
                        source: row.get(6)?,
                        source_run_id: row
                            .get::<_, Option<String>>(7)?
                            .map(RunId::parse)
                            .transpose()
                            .map_err(to_sqlite_user_error)?,
                        parent_event_id: row
                            .get::<_, Option<String>>(8)?
                            .map(EventId::parse)
                            .transpose()
                            .map_err(to_sqlite_user_error)?,
                        correlation_id: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(map_sqlite_error)
    }

    pub fn list_events(&self) -> ArmatureResult<Vec<EventRecord>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, event_type, time, payload_json, routing, config_version, source,
                        source_run_id, parent_event_id, correlation_id, created_at_ms
                 FROM events
                 ORDER BY created_at_ms DESC, id DESC",
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                let id = row.get::<_, String>(0)?;
                let time = row.get::<_, String>(2)?;
                let payload_json = row.get::<_, String>(3)?;
                Ok(EventRecord {
                    id: EventId::parse(id).map_err(to_sqlite_user_error)?,
                    event_type: row.get(1)?,
                    time: if time.is_empty() {
                        millis_to_rfc3339(row.get(10)?)
                    } else {
                        time
                    },
                    payload: serde_json::from_str(&payload_json).map_err(to_sqlite_data_error)?,
                    routing: enum_from_sql(&row.get::<_, String>(4)?)
                        .map_err(to_sqlite_user_error)?,
                    config_version: row.get(5)?,
                    source: row.get(6)?,
                    source_run_id: row
                        .get::<_, Option<String>>(7)?
                        .map(RunId::parse)
                        .transpose()
                        .map_err(to_sqlite_user_error)?,
                    parent_event_id: row
                        .get::<_, Option<String>>(8)?
                        .map(EventId::parse)
                        .transpose()
                        .map_err(to_sqlite_user_error)?,
                    correlation_id: row.get(9)?,
                })
            })
            .map_err(map_sqlite_error)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite_error)
    }

    pub fn list_events_filtered(&self, filter: &EventFilter) -> ArmatureResult<Vec<EventRecord>> {
        let events = self.list_events()?;
        Ok(apply_limit(
            events
                .into_iter()
                .filter(|event| {
                    filter
                        .event_type
                        .as_deref()
                        .map(|event_type| event.event_type == event_type)
                        .unwrap_or(true)
                        && filter
                            .source
                            .as_deref()
                            .map(|source| {
                                event
                                    .source
                                    .as_deref()
                                    .unwrap_or(armature_core::model::DEFAULT_EVENT_SOURCE)
                                    == source
                            })
                            .unwrap_or(true)
                        && filter
                            .correlation
                            .as_deref()
                            .map(|correlation| {
                                event.correlation_id.as_deref() == Some(correlation)
                                    || event_has_correlation(&event.payload, correlation)
                            })
                            .unwrap_or(true)
                })
                .collect(),
            filter.limit,
        ))
    }

    pub fn create_run(
        &self,
        name: impl Into<String>,
        command: impl Into<String>,
        origin: RunOrigin,
        config_version: Option<String>,
        event_id: Option<EventId>,
    ) -> ArmatureResult<PreparedRun> {
        self.create_run_record(NewRun {
            name: name.into(),
            command: command.into(),
            origin,
            config_version,
            event_id,
            restart_of: None,
            attempt: None,
        })
    }

    pub fn create_run_record(&self, new_run: NewRun) -> ArmatureResult<PreparedRun> {
        let run_id = RunId::new();
        let paths = self.paths.prepare_run_directory(&run_id)?;
        let run = RunRecord {
            id: run_id.clone(),
            name: new_run.name,
            command: new_run.command,
            origin: new_run.origin,
            state: ProcessState::Starting,
            start_time: now_rfc3339(),
            end_time: None,
            exit_code: None,
            signal: None,
            killed: false,
            config_version: new_run.config_version,
            event_id: new_run.event_id,
            restart_of: new_run.restart_of,
            attempt: new_run.attempt,
            run_directory: Some(paths.directory.display().to_string()),
            stdout_path: Some(paths.stdout.display().to_string()),
            stderr_path: Some(paths.stderr.display().to_string()),
        };
        let logs = LogRecord {
            run_id: run_id.clone(),
            stdout_path: paths.stdout.display().to_string(),
            stderr_path: paths.stderr.display().to_string(),
        };

        self.record_run(&run)?;
        self.record_logs(&logs)?;

        Ok(PreparedRun {
            record: run,
            logs,
            paths,
        })
    }

    pub fn record_run(&self, run: &RunRecord) -> ArmatureResult<()> {
        let now = now_millis();

        self.connection
            .execute(
                "INSERT INTO runs (
                    id, name, command, origin, state, start_time, end_time, exit_code, signal,
                    killed, config_version, event_id, restart_of, attempt, run_directory,
                    stdout_path, stderr_path, created_at_ms, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?18)",
                params![
                    run.id.as_str(),
                    run.name,
                    run.command,
                    enum_to_sql(&run.origin)?,
                    enum_to_sql(&run.state)?,
                    run.start_time,
                    run.end_time,
                    run.exit_code,
                    run.signal,
                    run.killed,
                    run.config_version,
                    run.event_id.as_ref().map(EventId::as_str),
                    run.restart_of.as_ref().map(RunId::as_str),
                    run.attempt,
                    run.run_directory,
                    run.stdout_path,
                    run.stderr_path,
                    now,
                ],
            )
            .map_err(map_sqlite_error)?;

        Ok(())
    }

    pub fn update_run_state(&self, run_id: &RunId, state: ProcessState) -> ArmatureResult<()> {
        let updated = self
            .connection
            .execute(
                "UPDATE runs
                 SET state = ?2, updated_at_ms = ?3
                 WHERE id = ?1",
                params![run_id.as_str(), enum_to_sql(&state)?, now_millis()],
            )
            .map_err(map_sqlite_error)?;

        if updated == 0 {
            return Err(ArmatureError::not_found(format!(
                "run {} was not found",
                run_id.as_str()
            )));
        }

        Ok(())
    }

    pub fn finish_run(
        &self,
        run_id: &RunId,
        state: ProcessState,
        exit_code: Option<i32>,
        signal: Option<i32>,
        killed: bool,
    ) -> ArmatureResult<()> {
        let updated = self
            .connection
            .execute(
                "UPDATE runs
                 SET state = ?2, end_time = ?3, exit_code = ?4, signal = ?5, killed = ?6,
                     updated_at_ms = ?7
                 WHERE id = ?1",
                params![
                    run_id.as_str(),
                    enum_to_sql(&state)?,
                    now_rfc3339(),
                    exit_code,
                    signal,
                    killed,
                    now_millis(),
                ],
            )
            .map_err(map_sqlite_error)?;

        if updated == 0 {
            return Err(ArmatureError::not_found(format!(
                "run {} was not found",
                run_id.as_str()
            )));
        }

        Ok(())
    }

    pub fn get_run(&self, run_id: &RunId) -> ArmatureResult<Option<RunRecord>> {
        self.connection
            .query_row(
                "SELECT id, name, command, origin, state, start_time, end_time, exit_code, signal,
                        killed, config_version, event_id, restart_of, attempt, run_directory,
                        stdout_path, stderr_path
                 FROM runs
                 WHERE id = ?1",
                params![run_id.as_str()],
                |row| {
                    let id = row.get::<_, String>(0)?;
                    let event_id = row.get::<_, Option<String>>(11)?;
                    let restart_of = row.get::<_, Option<String>>(12)?;
                    Ok(RunRecord {
                        id: RunId::parse(id).map_err(to_sqlite_user_error)?,
                        name: row.get(1)?,
                        command: row.get(2)?,
                        origin: enum_from_sql(&row.get::<_, String>(3)?)
                            .map_err(to_sqlite_user_error)?,
                        state: enum_from_sql(&row.get::<_, String>(4)?)
                            .map_err(to_sqlite_user_error)?,
                        start_time: row.get(5)?,
                        end_time: row.get(6)?,
                        exit_code: row.get(7)?,
                        signal: row.get(8)?,
                        killed: row.get(9)?,
                        config_version: row.get(10)?,
                        event_id: event_id
                            .map(EventId::parse)
                            .transpose()
                            .map_err(to_sqlite_user_error)?,
                        restart_of: restart_of
                            .map(RunId::parse)
                            .transpose()
                            .map_err(to_sqlite_user_error)?,
                        attempt: row.get(13)?,
                        run_directory: row.get(14)?,
                        stdout_path: row.get(15)?,
                        stderr_path: row.get(16)?,
                    })
                },
            )
            .optional()
            .map_err(map_sqlite_error)
    }

    pub fn list_runs(&self) -> ArmatureResult<Vec<RunRecord>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, name, command, origin, state, start_time, end_time, exit_code, signal,
                        killed, config_version, event_id, restart_of, attempt, run_directory,
                        stdout_path, stderr_path
                 FROM runs
                 ORDER BY created_at_ms DESC, id DESC",
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                let id = row.get::<_, String>(0)?;
                let event_id = row.get::<_, Option<String>>(11)?;
                let restart_of = row.get::<_, Option<String>>(12)?;
                Ok(RunRecord {
                    id: RunId::parse(id).map_err(to_sqlite_user_error)?,
                    name: row.get(1)?,
                    command: row.get(2)?,
                    origin: enum_from_sql(&row.get::<_, String>(3)?)
                        .map_err(to_sqlite_user_error)?,
                    state: enum_from_sql(&row.get::<_, String>(4)?)
                        .map_err(to_sqlite_user_error)?,
                    start_time: row.get(5)?,
                    end_time: row.get(6)?,
                    exit_code: row.get(7)?,
                    signal: row.get(8)?,
                    killed: row.get(9)?,
                    config_version: row.get(10)?,
                    event_id: event_id
                        .map(EventId::parse)
                        .transpose()
                        .map_err(to_sqlite_user_error)?,
                    restart_of: restart_of
                        .map(RunId::parse)
                        .transpose()
                        .map_err(to_sqlite_user_error)?,
                    attempt: row.get(13)?,
                    run_directory: row.get(14)?,
                    stdout_path: row.get(15)?,
                    stderr_path: row.get(16)?,
                })
            })
            .map_err(map_sqlite_error)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite_error)
    }

    pub fn list_runs_filtered(&self, filter: &RunFilter) -> ArmatureResult<Vec<RunRecord>> {
        let runs = self.list_runs()?;
        Ok(apply_limit(
            runs.into_iter()
                .filter(|run| {
                    filter
                        .name
                        .as_deref()
                        .map(|name| run.name == name)
                        .unwrap_or(true)
                        && filter
                            .origin
                            .as_deref()
                            .map(|origin| enum_matches_filter(&run.origin, origin))
                            .unwrap_or(true)
                        && filter
                            .state
                            .as_deref()
                            .map(|state| enum_matches_filter(&run.state, state))
                            .unwrap_or(true)
                        && filter
                            .correlation
                            .as_deref()
                            .map(|correlation| {
                                run.event_id
                                    .as_ref()
                                    .and_then(|event_id| self.get_event(event_id).ok().flatten())
                                    .map(|event| {
                                        event.correlation_id.as_deref() == Some(correlation)
                                            || event_has_correlation(&event.payload, correlation)
                                    })
                                    .unwrap_or(false)
                            })
                            .unwrap_or(true)
                })
                .collect(),
            filter.limit,
        ))
    }

    pub fn list_unfinished_runs(&self) -> ArmatureResult<Vec<RunRecord>> {
        Ok(self
            .list_runs()?
            .into_iter()
            .filter(|run| {
                matches!(
                    run.state,
                    ProcessState::Starting | ProcessState::Running | ProcessState::Stopping
                )
            })
            .collect())
    }

    pub fn record_logs(&self, logs: &LogRecord) -> ArmatureResult<()> {
        self.connection
            .execute(
                "INSERT INTO run_logs (run_id, stdout_path, stderr_path, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(run_id) DO UPDATE SET
                    stdout_path = excluded.stdout_path,
                    stderr_path = excluded.stderr_path,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    logs.run_id.as_str(),
                    logs.stdout_path,
                    logs.stderr_path,
                    now_millis(),
                ],
            )
            .map_err(map_sqlite_error)?;

        Ok(())
    }

    pub fn get_logs(&self, run_id: &RunId) -> ArmatureResult<Option<LogRecord>> {
        self.connection
            .query_row(
                "SELECT run_id, stdout_path, stderr_path
                 FROM run_logs
                 WHERE run_id = ?1",
                params![run_id.as_str()],
                |row| {
                    let run_id = row.get::<_, String>(0)?;
                    Ok(LogRecord {
                        run_id: RunId::parse(run_id).map_err(to_sqlite_user_error)?,
                        stdout_path: row.get(1)?,
                        stderr_path: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(map_sqlite_error)
    }

    pub fn record_trigger(&self, trigger: &TriggerRecord) -> ArmatureResult<()> {
        self.connection
            .execute(
                "INSERT INTO triggers (
                    id, task_name, event_id, event_type, routing, admission, outcome, run_id, detail,
                    created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    trigger.id.as_str(),
                    trigger.task_name,
                    trigger.event_id.as_ref().map(EventId::as_str),
                    trigger.event_type,
                    enum_to_sql(&trigger.routing)?,
                    enum_to_sql(&trigger.admission)?,
                    enum_to_sql(&trigger.outcome)?,
                    trigger.run_id.as_ref().map(RunId::as_str),
                    trigger.detail,
                    now_millis(),
                ],
            )
            .map_err(map_sqlite_error)?;

        Ok(())
    }

    pub fn list_triggers(&self) -> ArmatureResult<Vec<TriggerRecord>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, task_name, event_id, event_type, routing, admission, outcome, run_id, detail
                 FROM triggers
                 ORDER BY created_at_ms DESC, id DESC",
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                let id = row.get::<_, String>(0)?;
                let event_id = row.get::<_, Option<String>>(2)?;
                let run_id = row.get::<_, Option<String>>(7)?;
                Ok(TriggerRecord {
                    id: TriggerId::parse(id).map_err(to_sqlite_user_error)?,
                    task_name: row.get(1)?,
                    event_id: event_id
                        .map(EventId::parse)
                        .transpose()
                        .map_err(to_sqlite_user_error)?,
                    event_type: row.get(3)?,
                    routing: enum_from_sql(&row.get::<_, String>(4)?)
                        .map_err(to_sqlite_user_error)?,
                    admission: enum_from_sql(&row.get::<_, String>(5)?)
                        .map_err(to_sqlite_user_error)?,
                    outcome: enum_from_sql(&row.get::<_, String>(6)?)
                        .map_err(to_sqlite_user_error)?,
                    run_id: run_id
                        .map(RunId::parse)
                        .transpose()
                        .map_err(to_sqlite_user_error)?,
                    detail: row.get(8)?,
                })
            })
            .map_err(map_sqlite_error)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite_error)
    }

    pub fn list_triggers_filtered(
        &self,
        filter: &TriggerFilter,
    ) -> ArmatureResult<Vec<TriggerRecord>> {
        let triggers = self.list_triggers()?;
        Ok(apply_limit(
            triggers
                .into_iter()
                .filter(|trigger| {
                    filter
                        .task
                        .as_deref()
                        .map(|task| trigger.task_name == task)
                        .unwrap_or(true)
                        && filter
                            .event_type
                            .as_deref()
                            .map(|event_type| trigger.event_type == event_type)
                            .unwrap_or(true)
                        && filter
                            .outcome
                            .as_deref()
                            .map(|outcome| enum_matches_filter(&trigger.outcome, outcome))
                            .unwrap_or(true)
                        && filter
                            .correlation
                            .as_deref()
                            .map(|correlation| {
                                trigger
                                    .event_id
                                    .as_ref()
                                    .and_then(|event_id| self.get_event(event_id).ok().flatten())
                                    .map(|event| {
                                        event.correlation_id.as_deref() == Some(correlation)
                                            || event_has_correlation(&event.payload, correlation)
                                    })
                                    .unwrap_or(false)
                            })
                            .unwrap_or(true)
                })
                .collect(),
            filter.limit,
        ))
    }

    fn bootstrap(&self) -> ArmatureResult<()> {
        self.connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS events (
                    id TEXT PRIMARY KEY,
                    event_type TEXT NOT NULL,
                    time TEXT NOT NULL DEFAULT '',
                    payload_json TEXT NOT NULL,
                    routing TEXT NOT NULL,
                    config_version TEXT,
                    source TEXT,
                    source_run_id TEXT,
                    parent_event_id TEXT,
                    correlation_id TEXT,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS runs (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    command TEXT NOT NULL DEFAULT '',
                    origin TEXT NOT NULL,
                    state TEXT NOT NULL,
                    start_time TEXT NOT NULL DEFAULT '',
                    end_time TEXT,
                    exit_code INTEGER,
                    signal INTEGER,
                    killed INTEGER NOT NULL DEFAULT 0,
                    config_version TEXT,
                    event_id TEXT,
                    restart_of TEXT,
                    attempt INTEGER,
                    run_directory TEXT,
                    stdout_path TEXT,
                    stderr_path TEXT,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(event_id) REFERENCES events(id),
                    FOREIGN KEY(restart_of) REFERENCES runs(id)
                 );
                 CREATE TABLE IF NOT EXISTS run_logs (
                    run_id TEXT PRIMARY KEY,
                    stdout_path TEXT NOT NULL,
                    stderr_path TEXT NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(run_id) REFERENCES runs(id) ON DELETE CASCADE
                 );
                 CREATE TABLE IF NOT EXISTS triggers (
                    id TEXT PRIMARY KEY,
                    task_name TEXT NOT NULL,
                    event_id TEXT,
                    event_type TEXT NOT NULL,
                    routing TEXT NOT NULL,
                    admission TEXT NOT NULL,
                    outcome TEXT NOT NULL,
                    run_id TEXT,
                    detail TEXT,
                    created_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(event_id) REFERENCES events(id),
                    FOREIGN KEY(run_id) REFERENCES runs(id)
                 );",
            )
            .map_err(map_sqlite_error)?;

        let schema_version = self.schema_version()?;
        if schema_version > SCHEMA_VERSION {
            return Err(ArmatureError::internal(format!(
                "database schema version {schema_version} is newer than supported version {SCHEMA_VERSION}"
            )));
        }
        self.migrate_schema(schema_version)?;
        self.set_schema_version(SCHEMA_VERSION)?;

        Ok(())
    }

    fn schema_version(&self) -> ArmatureResult<i32> {
        self.connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(map_sqlite_error)
    }

    fn set_schema_version(&self, version: i32) -> ArmatureResult<()> {
        self.connection
            .execute_batch(&format!("PRAGMA user_version = {version}"))
            .map_err(map_sqlite_error)
    }

    fn migrate_schema(&self, from_version: i32) -> ArmatureResult<()> {
        if from_version < 1 {
            self.migrate_to_1()?;
        }
        if from_version < 2 {
            self.migrate_to_2()?;
        }

        Ok(())
    }

    fn migrate_to_1(&self) -> ArmatureResult<()> {
        self.ensure_events_column("time", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_runs_column("restart_of", "TEXT")?;
        self.ensure_runs_column("attempt", "INTEGER")?;
        self.ensure_runs_column("command", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_runs_column("start_time", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_runs_column("end_time", "TEXT")?;
        self.ensure_runs_column("exit_code", "INTEGER")?;
        self.ensure_runs_column("signal", "INTEGER")?;
        self.ensure_runs_column("killed", "INTEGER NOT NULL DEFAULT 0")?;

        Ok(())
    }

    fn migrate_to_2(&self) -> ArmatureResult<()> {
        self.ensure_events_column("source_run_id", "TEXT")?;
        self.ensure_events_column("parent_event_id", "TEXT")?;
        self.ensure_events_column("correlation_id", "TEXT")?;

        Ok(())
    }

    fn ensure_events_column(&self, name: &str, definition: &str) -> ArmatureResult<()> {
        self.ensure_column("events", name, definition)
    }

    fn ensure_runs_column(&self, name: &str, definition: &str) -> ArmatureResult<()> {
        self.ensure_column("runs", name, definition)
    }

    fn ensure_column(&self, table: &str, name: &str, definition: &str) -> ArmatureResult<()> {
        let mut statement = self
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(map_sqlite_error)?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(map_sqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite_error)?;

        if !columns.iter().any(|column| column == name) {
            self.connection
                .execute(
                    &format!("ALTER TABLE {table} ADD COLUMN {name} {definition}"),
                    [],
                )
                .map_err(map_sqlite_error)?;
        }

        Ok(())
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_millis() as i64
}

fn new_lock_token() -> String {
    format!("lock_{}", Ulid::new())
}

fn lock_is_stale(record: &ManualLockRecord) -> bool {
    record
        .expires_at_ms
        .map(|expires_at_ms| now_millis() >= expires_at_ms)
        .unwrap_or(false)
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn millis_to_rfc3339(timestamp_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|timestamp| timestamp.to_rfc3339())
        .unwrap_or_else(|| timestamp_ms.to_string())
}

fn event_time_to_millis(time: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(time)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn enum_to_sql<T>(value: &T) -> ArmatureResult<String>
where
    T: Serialize,
{
    let encoded =
        serde_json::to_string(value).map_err(|error| ArmatureError::internal(error.to_string()))?;
    encoded
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .map(ToOwned::to_owned)
        .ok_or_else(|| ArmatureError::internal("expected enum to serialize as a JSON string"))
}

fn enum_matches_filter<T>(value: &T, expected: &str) -> bool
where
    T: Serialize,
{
    enum_to_sql(value)
        .map(|actual| actual == expected)
        .unwrap_or(false)
}

fn event_has_correlation(payload: &Value, expected: &str) -> bool {
    payload
        .get("correlationId")
        .or_else(|| payload.get("correlation_id"))
        .and_then(Value::as_str)
        .map(|actual| actual == expected)
        .unwrap_or(false)
}

fn apply_limit<T>(items: Vec<T>, limit: Option<usize>) -> Vec<T> {
    match limit {
        Some(limit) => items.into_iter().take(limit).collect(),
        None => items,
    }
}

fn enum_from_sql<T>(value: &str) -> ArmatureResult<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(&format!("\"{value}\""))
        .map_err(|error| ArmatureError::internal(error.to_string()))
}

fn map_sqlite_error(error: rusqlite::Error) -> ArmatureError {
    ArmatureError::internal(error.to_string())
}

fn to_sqlite_user_error(error: ArmatureError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

fn to_sqlite_data_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::thread;
    use std::time::Duration;

    use armature_core::{discover_workspace, EventRouting, TriggerOutcome};
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        millis_to_rfc3339, now_rfc3339, EventFilter, ManualLockOwner, ManualLockStore, NewRun,
        RunFilter, SqliteStore, TriggerFilter, SCHEMA_VERSION,
    };
    use armature_core::{
        EventRecord, ProcessState, RunOrigin, TriggerId, TriggerRecord, WorkspaceRuntimePaths,
    };

    #[test]
    fn bootstraps_database_under_state_root() {
        let fixture = WorkspaceFixture::new();
        let workspace = discover_workspace(fixture.root()).unwrap();
        let runtime_paths = WorkspaceRuntimePaths::for_workspace_with_state_home(
            &workspace,
            fixture.state_home.path(),
        )
        .unwrap();

        let store = SqliteStore::open_with_paths(runtime_paths.clone()).unwrap();

        assert!(store.paths().database_path().is_file());
        assert!(store
            .paths()
            .state_root()
            .starts_with(fixture.state_home.path()));
        assert_eq!(
            store.paths().runs_root(),
            &fixture.root().join(".armature").join("runs")
        );
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn upgrades_legacy_schema_and_preserves_records() {
        let fixture = WorkspaceFixture::new();
        let workspace = discover_workspace(fixture.root()).unwrap();
        let runtime_paths = WorkspaceRuntimePaths::for_workspace_with_state_home(
            &workspace,
            fixture.state_home.path(),
        )
        .unwrap();
        runtime_paths.ensure_state_root().unwrap();

        let event_id = armature_core::EventId::new();
        let run_id = armature_core::RunId::new();
        let created_at_ms = 1_700_000_000_123_i64;
        {
            let connection = Connection::open(runtime_paths.database_path()).unwrap();
            connection
                .execute_batch(
                    "PRAGMA user_version = 0;
                     CREATE TABLE events (
                        id TEXT PRIMARY KEY,
                        event_type TEXT NOT NULL,
                        payload_json TEXT NOT NULL,
                        routing TEXT NOT NULL,
                        config_version TEXT,
                        source TEXT,
                        created_at_ms INTEGER NOT NULL
                     );
                     CREATE TABLE runs (
                        id TEXT PRIMARY KEY,
                        name TEXT NOT NULL,
                        origin TEXT NOT NULL,
                        state TEXT NOT NULL,
                        config_version TEXT,
                        event_id TEXT,
                        run_directory TEXT,
                        stdout_path TEXT,
                        stderr_path TEXT,
                        created_at_ms INTEGER NOT NULL,
                        updated_at_ms INTEGER NOT NULL,
                        FOREIGN KEY(event_id) REFERENCES events(id)
                     );
                     CREATE TABLE run_logs (
                        run_id TEXT PRIMARY KEY,
                        stdout_path TEXT NOT NULL,
                        stderr_path TEXT NOT NULL,
                        updated_at_ms INTEGER NOT NULL,
                        FOREIGN KEY(run_id) REFERENCES runs(id) ON DELETE CASCADE
                     );
                     CREATE TABLE triggers (
                        id TEXT PRIMARY KEY,
                        task_name TEXT NOT NULL,
                        event_id TEXT,
                        event_type TEXT NOT NULL,
                        routing TEXT NOT NULL,
                        admission TEXT NOT NULL,
                        outcome TEXT NOT NULL,
                        run_id TEXT,
                        detail TEXT,
                        created_at_ms INTEGER NOT NULL,
                        FOREIGN KEY(event_id) REFERENCES events(id),
                        FOREIGN KEY(run_id) REFERENCES runs(id)
                     );",
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO events (
                        id, event_type, payload_json, routing, config_version, source, created_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        event_id.as_str(),
                        "manual.run.requested",
                        r#"{"task":"build"}"#,
                        "manual",
                        "cfg_legacy",
                        "cli",
                        created_at_ms,
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO runs (
                        id, name, origin, state, config_version, event_id, run_directory,
                        stdout_path, stderr_path, created_at_ms, updated_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    rusqlite::params![
                        run_id.as_str(),
                        "build",
                        "task",
                        "running",
                        "cfg_legacy",
                        event_id.as_str(),
                        "/tmp/legacy-run",
                        "/tmp/legacy-run/stdout.log",
                        "/tmp/legacy-run/stderr.log",
                        created_at_ms,
                    ],
                )
                .unwrap();
        }

        let store = SqliteStore::open_with_paths(runtime_paths).unwrap();

        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);

        let event = store.get_event(&event_id).unwrap().unwrap();
        assert_eq!(event.time, millis_to_rfc3339(created_at_ms));
        assert_eq!(event.payload, json!({ "task": "build" }));
        assert_eq!(event.routing, EventRouting::Manual);

        let run = store.get_run(&run_id).unwrap().unwrap();
        assert_eq!(run.name, "build");
        assert_eq!(run.command, "");
        assert_eq!(run.start_time, "");
        assert_eq!(run.end_time, None);
        assert_eq!(run.exit_code, None);
        assert_eq!(run.signal, None);
        assert!(!run.killed);
        assert_eq!(run.restart_of, None);
        assert_eq!(run.attempt, None);

        let created = store
            .create_run("test", "cargo test", RunOrigin::Task, None, None)
            .unwrap();
        assert_eq!(created.record.command, "cargo test");
    }

    #[test]
    fn rejects_database_with_newer_schema_version() {
        let fixture = WorkspaceFixture::new();
        let workspace = discover_workspace(fixture.root()).unwrap();
        let runtime_paths = WorkspaceRuntimePaths::for_workspace_with_state_home(
            &workspace,
            fixture.state_home.path(),
        )
        .unwrap();
        runtime_paths.ensure_state_root().unwrap();
        let connection = Connection::open(runtime_paths.database_path()).unwrap();
        connection
            .execute_batch(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 1))
            .unwrap();
        drop(connection);

        let error = SqliteStore::open_with_paths(runtime_paths).unwrap_err();
        assert!(error.to_string().contains("newer than supported version"));
    }

    #[test]
    fn create_run_persists_isolated_paths_inside_workspace() {
        let fixture = WorkspaceFixture::new();
        let workspace = discover_workspace(fixture.root()).unwrap();
        let runtime_paths = WorkspaceRuntimePaths::for_workspace_with_state_home(
            &workspace,
            fixture.state_home.path(),
        )
        .unwrap();
        let store = SqliteStore::open_with_paths(runtime_paths).unwrap();

        let prepared = store
            .create_run(
                "build",
                "cargo build",
                RunOrigin::Task,
                Some("cfg_123".to_string()),
                None,
            )
            .unwrap();

        assert_eq!(prepared.record.command, "cargo build");
        assert!(!prepared.record.start_time.is_empty());
        assert!(prepared.paths.directory.is_dir());
        assert!(prepared.paths.tmp.is_dir());
        assert!(prepared
            .paths
            .directory
            .starts_with(fixture.root().join(".armature/runs")));
        assert_eq!(
            prepared.record.stdout_path.as_deref(),
            Some(prepared.paths.stdout.to_string_lossy().as_ref())
        );
        assert_eq!(
            store.get_logs(&prepared.record.id).unwrap().unwrap(),
            prepared.logs
        );
    }

    #[test]
    fn records_and_reads_back_events_and_runs() {
        let fixture = WorkspaceFixture::new();
        let workspace = discover_workspace(fixture.root()).unwrap();
        let runtime_paths = WorkspaceRuntimePaths::for_workspace_with_state_home(
            &workspace,
            fixture.state_home.path(),
        )
        .unwrap();
        let store = SqliteStore::open_with_paths(runtime_paths).unwrap();
        let event = EventRecord {
            id: armature_core::EventId::new(),
            event_type: "manual.run.requested".to_string(),
            time: now_rfc3339(),
            payload: json!({ "task": "build" }),
            routing: EventRouting::Manual,
            config_version: Some("cfg_123".to_string()),
            source: Some("cli".to_string()),
            source_run_id: None,
            parent_event_id: None,
            correlation_id: Some("corr-store".to_string()),
        };

        store.record_event(&event).unwrap();
        let other_event = EventRecord {
            id: armature_core::EventId::new(),
            event_type: "other.event".to_string(),
            time: now_rfc3339(),
            payload: json!({ "correlationId": "other-correlation" }),
            routing: EventRouting::Event,
            config_version: Some("cfg_123".to_string()),
            source: Some("sdk".to_string()),
            source_run_id: None,
            parent_event_id: None,
            correlation_id: Some("other-correlation".to_string()),
        };
        store.record_event(&other_event).unwrap();
        let prepared = store
            .create_run(
                "build",
                "cargo build",
                RunOrigin::Task,
                Some("cfg_123".to_string()),
                Some(event.id.clone()),
            )
            .unwrap();
        store
            .update_run_state(&prepared.record.id, ProcessState::Running)
            .unwrap();

        assert_eq!(store.get_event(&event.id).unwrap().unwrap(), event);
        assert_eq!(
            store
                .list_events_filtered(&EventFilter {
                    event_type: Some("manual.run.requested".to_string()),
                    source: Some("cli".to_string()),
                    correlation: None,
                    limit: Some(1),
                })
                .unwrap(),
            vec![event.clone()]
        );

        let run = store.get_run(&prepared.record.id).unwrap().unwrap();
        assert_eq!(run.event_id, Some(event.id.clone()));
        assert_eq!(run.state, ProcessState::Running);
        assert_eq!(run.command, "cargo build");
        assert_eq!(store.list_runs().unwrap(), vec![run.clone()]);
        assert_eq!(
            store
                .list_runs_filtered(&RunFilter {
                    name: Some("build".to_string()),
                    origin: Some("task".to_string()),
                    state: Some("running".to_string()),
                    correlation: Some("corr-store".to_string()),
                    limit: Some(1),
                })
                .unwrap(),
            vec![run]
        );
        assert_eq!(
            store
                .list_events_filtered(&EventFilter {
                    event_type: None,
                    source: None,
                    correlation: Some("corr-store".to_string()),
                    limit: None,
                })
                .unwrap(),
            vec![event.clone()]
        );
        assert_eq!(
            store
                .get_event(&event.id)
                .unwrap()
                .unwrap()
                .correlation_id
                .as_deref(),
            Some("corr-store")
        );
    }

    #[test]
    fn records_run_completion_details() {
        let fixture = WorkspaceFixture::new();
        let workspace = discover_workspace(fixture.root()).unwrap();
        let runtime_paths = WorkspaceRuntimePaths::for_workspace_with_state_home(
            &workspace,
            fixture.state_home.path(),
        )
        .unwrap();
        let store = SqliteStore::open_with_paths(runtime_paths).unwrap();

        let prepared = store
            .create_run(
                "build",
                "cargo build",
                RunOrigin::Task,
                Some("cfg_123".to_string()),
                None,
            )
            .unwrap();
        store
            .finish_run(
                &prepared.record.id,
                ProcessState::Failed,
                None,
                Some(9),
                true,
            )
            .unwrap();

        let run = store.get_run(&prepared.record.id).unwrap().unwrap();
        assert_eq!(run.state, ProcessState::Failed);
        assert!(run.end_time.is_some());
        assert_eq!(run.exit_code, None);
        assert_eq!(run.signal, Some(9));
        assert!(run.killed);
    }

    #[test]
    fn records_restart_lineage_on_runs() {
        let fixture = WorkspaceFixture::new();
        let workspace = discover_workspace(fixture.root()).unwrap();
        let runtime_paths = WorkspaceRuntimePaths::for_workspace_with_state_home(
            &workspace,
            fixture.state_home.path(),
        )
        .unwrap();
        let store = SqliteStore::open_with_paths(runtime_paths).unwrap();

        let original = store
            .create_run(
                "build",
                "server",
                RunOrigin::Service,
                Some("cfg_123".to_string()),
                None,
            )
            .unwrap();
        let restart = store
            .create_run_record(NewRun {
                name: "build".to_string(),
                command: "server".to_string(),
                origin: RunOrigin::Restart,
                config_version: Some("cfg_123".to_string()),
                event_id: None,
                restart_of: Some(original.record.id.clone()),
                attempt: Some(1),
            })
            .unwrap();

        let stored = store.get_run(&restart.record.id).unwrap().unwrap();
        assert_eq!(stored.restart_of, Some(original.record.id));
        assert_eq!(stored.attempt, Some(1));
        assert_eq!(stored.origin, RunOrigin::Restart);
    }

    #[test]
    fn records_trigger_outcomes_for_inspection() {
        let fixture = WorkspaceFixture::new();
        let workspace = discover_workspace(fixture.root()).unwrap();
        let runtime_paths = WorkspaceRuntimePaths::for_workspace_with_state_home(
            &workspace,
            fixture.state_home.path(),
        )
        .unwrap();
        let store = SqliteStore::open_with_paths(runtime_paths).unwrap();
        let event = EventRecord {
            id: armature_core::EventId::new(),
            event_type: "file.changed".to_string(),
            time: now_rfc3339(),
            payload: json!({ "paths": ["src/lib.rs"] }),
            routing: EventRouting::Watch,
            config_version: Some("cfg_123".to_string()),
            source: Some("watch:test".to_string()),
            source_run_id: None,
            parent_event_id: None,
            correlation_id: Some("corr-trigger".to_string()),
        };
        store.record_event(&event).unwrap();

        let trigger = TriggerRecord {
            id: TriggerId::new(),
            task_name: "test".to_string(),
            event_id: Some(event.id.clone()),
            event_type: event.event_type.clone(),
            routing: EventRouting::Watch,
            admission: armature_core::AdmissionPolicy::QueueOne,
            outcome: TriggerOutcome::Queued,
            run_id: None,
            detail: Some("waiting for active run".to_string()),
        };
        store.record_trigger(&trigger).unwrap();

        assert_eq!(store.list_events().unwrap(), vec![event]);
        assert_eq!(store.list_triggers().unwrap(), vec![trigger.clone()]);
        assert_eq!(
            store
                .list_triggers_filtered(&TriggerFilter {
                    task: Some("test".to_string()),
                    event_type: Some("file.changed".to_string()),
                    outcome: Some("queued".to_string()),
                    correlation: Some("corr-trigger".to_string()),
                    limit: Some(1),
                })
                .unwrap(),
            vec![trigger]
        );
    }

    #[test]
    fn manual_lock_store_requires_matching_tokens_for_renew_and_release() {
        let lock_dir = TempDir::new().unwrap();
        let store = ManualLockStore::new(lock_dir.path());
        let owner = ManualLockOwner {
            pid: 42,
            id: "pid:42".to_string(),
        };

        let first = store
            .acquire(
                "branch:main".to_string(),
                owner.clone(),
                Some("deploy".to_string()),
                Duration::from_millis(50),
            )
            .unwrap();
        assert_eq!(first.owner_id, "pid:42");
        assert_eq!(first.reason.as_deref(), Some("deploy"));
        assert!(first.token.starts_with("lock_"));

        let mismatch = store
            .renew("branch:main", "wrong-token", Duration::from_secs(5))
            .unwrap_err();
        assert_eq!(mismatch.kind.as_ref(), "conflict");

        let renewed = store
            .renew("branch:main", &first.token, Duration::from_secs(5))
            .unwrap();
        assert_eq!(renewed.token, first.token);
        assert!(renewed.renewed_at_ms.is_some());

        store.release("branch:main", &renewed.token).unwrap();
        assert!(store.list().unwrap().is_empty());

        let expired = store
            .acquire(
                "branch:main".to_string(),
                owner.clone(),
                None,
                Duration::from_millis(20),
            )
            .unwrap();
        thread::sleep(Duration::from_millis(40));
        let newer = store
            .acquire(
                "branch:main".to_string(),
                owner,
                None,
                Duration::from_secs(5),
            )
            .unwrap();
        assert_ne!(expired.token, newer.token);

        let stale_release = store.release("branch:main", &expired.token).unwrap_err();
        assert_eq!(stale_release.kind.as_ref(), "conflict");
        assert_eq!(store.list().unwrap()[0].token, newer.token);
    }

    struct WorkspaceFixture {
        root_dir: TempDir,
        state_home: TempDir,
    }

    impl WorkspaceFixture {
        fn new() -> Self {
            let root_dir = TempDir::new().unwrap();
            fs::create_dir_all(root_dir.path().join(".armature")).unwrap();
            fs::write(
                root_dir.path().join(".armature/armature.toml"),
                "[[task]]\nname = \"build\"\nrun = \"true\"\n",
            )
            .unwrap();

            Self {
                root_dir,
                state_home: TempDir::new().unwrap(),
            }
        }

        fn root(&self) -> &std::path::Path {
            self.root_dir.path()
        }
    }
}
