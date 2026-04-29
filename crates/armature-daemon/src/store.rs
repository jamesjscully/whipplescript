use std::time::{SystemTime, UNIX_EPOCH};

use armature_core::{
    ArmatureError, ArmatureResult, EventId, EventRecord, LogRecord, ProcessState, RunId, RunOrigin,
    RunPaths, RunRecord, TriggerId, TriggerRecord, Workspace, WorkspaceRuntimePaths,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug)]
pub struct SqliteStore {
    paths: WorkspaceRuntimePaths,
    connection: Connection,
}

#[derive(Debug, Clone)]
pub struct PreparedRun {
    pub record: RunRecord,
    pub logs: LogRecord,
    pub paths: RunPaths,
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
                    id, event_type, payload_json, routing, config_version, source, created_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    event.id.as_str(),
                    event.event_type,
                    payload_json,
                    enum_to_sql(&event.routing)?,
                    event.config_version,
                    event.source,
                    now_millis(),
                ],
            )
            .map_err(map_sqlite_error)?;

        Ok(())
    }

    pub fn get_event(&self, event_id: &EventId) -> ArmatureResult<Option<EventRecord>> {
        self.connection
            .query_row(
                "SELECT id, event_type, payload_json, routing, config_version, source
                 FROM events
                 WHERE id = ?1",
                params![event_id.as_str()],
                |row| {
                    let id = row.get::<_, String>(0)?;
                    let payload_json = row.get::<_, String>(2)?;
                    Ok(EventRecord {
                        id: EventId::parse(id).map_err(to_sqlite_user_error)?,
                        event_type: row.get(1)?,
                        payload: serde_json::from_str(&payload_json)
                            .map_err(to_sqlite_data_error)?,
                        routing: enum_from_sql(&row.get::<_, String>(3)?)
                            .map_err(to_sqlite_user_error)?,
                        config_version: row.get(4)?,
                        source: row.get(5)?,
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
                "SELECT id, event_type, payload_json, routing, config_version, source
                 FROM events
                 ORDER BY created_at_ms DESC, id DESC",
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                let id = row.get::<_, String>(0)?;
                let payload_json = row.get::<_, String>(2)?;
                Ok(EventRecord {
                    id: EventId::parse(id).map_err(to_sqlite_user_error)?,
                    event_type: row.get(1)?,
                    payload: serde_json::from_str(&payload_json).map_err(to_sqlite_data_error)?,
                    routing: enum_from_sql(&row.get::<_, String>(3)?)
                        .map_err(to_sqlite_user_error)?,
                    config_version: row.get(4)?,
                    source: row.get(5)?,
                })
            })
            .map_err(map_sqlite_error)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite_error)
    }

    pub fn create_run(
        &self,
        name: impl Into<String>,
        origin: RunOrigin,
        config_version: Option<String>,
        event_id: Option<EventId>,
    ) -> ArmatureResult<PreparedRun> {
        let run_id = RunId::new();
        let paths = self.paths.prepare_run_directory(&run_id)?;
        let run = RunRecord {
            id: run_id.clone(),
            name: name.into(),
            origin,
            state: ProcessState::Starting,
            config_version,
            event_id,
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
                    id, name, origin, state, config_version, event_id, run_directory, stdout_path,
                    stderr_path, created_at_ms, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    run.id.as_str(),
                    run.name,
                    enum_to_sql(&run.origin)?,
                    enum_to_sql(&run.state)?,
                    run.config_version,
                    run.event_id.as_ref().map(EventId::as_str),
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

    pub fn get_run(&self, run_id: &RunId) -> ArmatureResult<Option<RunRecord>> {
        self.connection
            .query_row(
                "SELECT id, name, origin, state, config_version, event_id, run_directory, stdout_path, stderr_path
                 FROM runs
                 WHERE id = ?1",
                params![run_id.as_str()],
                |row| {
                    let id = row.get::<_, String>(0)?;
                    let event_id = row.get::<_, Option<String>>(5)?;
                    Ok(RunRecord {
                        id: RunId::parse(id).map_err(to_sqlite_user_error)?,
                        name: row.get(1)?,
                        origin: enum_from_sql(&row.get::<_, String>(2)?).map_err(to_sqlite_user_error)?,
                        state: enum_from_sql(&row.get::<_, String>(3)?).map_err(to_sqlite_user_error)?,
                        config_version: row.get(4)?,
                        event_id: event_id
                            .map(EventId::parse)
                            .transpose()
                            .map_err(to_sqlite_user_error)?,
                        run_directory: row.get(6)?,
                        stdout_path: row.get(7)?,
                        stderr_path: row.get(8)?,
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
                "SELECT id, name, origin, state, config_version, event_id, run_directory, stdout_path, stderr_path
                 FROM runs
                 ORDER BY created_at_ms DESC, id DESC",
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                let id = row.get::<_, String>(0)?;
                let event_id = row.get::<_, Option<String>>(5)?;
                Ok(RunRecord {
                    id: RunId::parse(id).map_err(to_sqlite_user_error)?,
                    name: row.get(1)?,
                    origin: enum_from_sql(&row.get::<_, String>(2)?)
                        .map_err(to_sqlite_user_error)?,
                    state: enum_from_sql(&row.get::<_, String>(3)?)
                        .map_err(to_sqlite_user_error)?,
                    config_version: row.get(4)?,
                    event_id: event_id
                        .map(EventId::parse)
                        .transpose()
                        .map_err(to_sqlite_user_error)?,
                    run_directory: row.get(6)?,
                    stdout_path: row.get(7)?,
                    stderr_path: row.get(8)?,
                })
            })
            .map_err(map_sqlite_error)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite_error)
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

    fn bootstrap(&self) -> ArmatureResult<()> {
        self.connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS events (
                    id TEXT PRIMARY KEY,
                    event_type TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    routing TEXT NOT NULL,
                    config_version TEXT,
                    source TEXT,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS runs (
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

        Ok(())
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_millis() as i64
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

    use armature_core::{discover_workspace, EventRouting, TriggerOutcome};
    use serde_json::json;
    use tempfile::TempDir;

    use super::SqliteStore;
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
            .create_run("build", RunOrigin::Task, Some("cfg_123".to_string()), None)
            .unwrap();

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
            payload: json!({ "task": "build" }),
            routing: EventRouting::Manual,
            config_version: Some("cfg_123".to_string()),
            source: Some("cli".to_string()),
        };

        store.record_event(&event).unwrap();
        let prepared = store
            .create_run(
                "build",
                RunOrigin::Task,
                Some("cfg_123".to_string()),
                Some(event.id.clone()),
            )
            .unwrap();
        store
            .update_run_state(&prepared.record.id, ProcessState::Running)
            .unwrap();

        assert_eq!(store.get_event(&event.id).unwrap().unwrap(), event);

        let run = store.get_run(&prepared.record.id).unwrap().unwrap();
        assert_eq!(run.event_id, Some(event.id));
        assert_eq!(run.state, ProcessState::Running);
        assert_eq!(store.list_runs().unwrap(), vec![run]);
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
            payload: json!({ "paths": ["src/lib.rs"] }),
            routing: EventRouting::Watch,
            config_version: Some("cfg_123".to_string()),
            source: Some("watch:test".to_string()),
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
        assert_eq!(store.list_triggers().unwrap(), vec![trigger]);
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
