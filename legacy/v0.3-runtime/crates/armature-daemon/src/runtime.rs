use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::str::FromStr;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use armature_core::{
    load_workspace_config, AdmissionConfig, AdmissionPolicy, ArmatureConfig, ArmatureError,
    ArmatureResult, EventId, EventRecord, EventRouting, ProcessState, ResourcePolicy, RestartMode,
    RunId, RunOrigin, RunRecord, ServiceConfig, SupervisionPolicyConfig, TaskConfig, TriggerConfig,
    TriggerId, TriggerOutcome, TriggerRecord, Workspace, WorkspaceRuntimePaths,
};
use chrono::{DateTime, Local, Utc};
use cron::Schedule;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::duration::parse_duration;
use crate::process::{signal_process_group, spawn_command, spawn_shell_command};
use crate::protocol::{
    DaemonRequest, DaemonResponse, InspectResponse, ManualLockRecord, ResponsePayload,
    RuntimeHealthStatus, RuntimeServiceStatus, RuntimeTaskStatus,
};
use crate::store::{ManualLockOwner, ManualLockStore, NewRun, SqliteStore};

const LOOP_SLEEP: Duration = Duration::from_millis(25);
const TERMINATION_GRACE: Duration = Duration::from_secs(2);
const DEFAULT_WATCH_SETTLE: Duration = Duration::from_millis(300);
const LOCKS_DIR_NAME: &str = "locks";

#[derive(Debug, Clone)]
pub struct DaemonOptions {
    pub poll_interval: Duration,
    pub termination_grace: Duration,
}

impl Default for DaemonOptions {
    fn default() -> Self {
        Self {
            poll_interval: LOOP_SLEEP,
            termination_grace: TERMINATION_GRACE,
        }
    }
}

pub struct DaemonServer;

pub struct DaemonHandle {
    socket_path: PathBuf,
    join_handle: Option<JoinHandle<ArmatureResult<()>>>,
}

#[derive(Clone)]
pub struct DaemonClient {
    socket_path: PathBuf,
}

impl DaemonServer {
    pub fn start(workspace: Workspace) -> ArmatureResult<DaemonHandle> {
        Self::start_with_options(workspace, DaemonOptions::default())
    }

    pub fn start_with_options(
        workspace: Workspace,
        options: DaemonOptions,
    ) -> ArmatureResult<DaemonHandle> {
        let runtime_paths = WorkspaceRuntimePaths::for_workspace(&workspace)?;
        runtime_paths.ensure_state_root()?;

        let socket_path = runtime_paths.socket_path();
        let pid_path = runtime_paths.pid_path();
        let lock_path = runtime_paths.workspace_lock_path();
        let pid = std::process::id();

        acquire_lock(&lock_path, pid)?;
        let startup = (|| -> ArmatureResult<()> {
            if socket_path.exists() {
                fs::remove_file(&socket_path)?;
            }
            fs::write(&pid_path, format!("{pid}\n"))?;
            Ok(())
        })();

        if let Err(error) = startup {
            let _ = fs::remove_file(&lock_path);
            return Err(error);
        }

        let handle_socket_path = socket_path.clone();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let join_handle = thread::spawn(move || {
            let result = match Runtime::new(workspace, runtime_paths, options) {
                Ok(mut runtime) => {
                    let _ = startup_tx.send(Ok(()));
                    runtime.run()
                }
                Err(error) => {
                    let _ = startup_tx.send(Err(error.clone()));
                    Err(error)
                }
            };
            let _ = fs::remove_file(socket_path);
            let _ = fs::remove_file(pid_path);
            let _ = fs::remove_file(lock_path);
            result
        });

        wait_for_startup(
            &startup_rx,
            &join_handle,
            &handle_socket_path,
            Duration::from_secs(1),
        )?;

        Ok(DaemonHandle {
            socket_path: handle_socket_path,
            join_handle: Some(join_handle),
        })
    }
}

impl DaemonHandle {
    pub fn client(&self) -> DaemonClient {
        DaemonClient {
            socket_path: self.socket_path.clone(),
        }
    }

    pub fn join(mut self) -> ArmatureResult<()> {
        if let Some(handle) = self.join_handle.take() {
            return handle
                .join()
                .map_err(|_| ArmatureError::internal("daemon thread panicked"))?;
        }
        Ok(())
    }
}

impl DaemonClient {
    pub fn from_socket_path(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    pub fn inspect(&self) -> ArmatureResult<InspectResponse> {
        match self.send(DaemonRequest::Inspect)? {
            ResponsePayload::Inspect(response) => Ok(response),
            _ => Err(ArmatureError::internal("unexpected inspect response")),
        }
    }

    pub fn events(&self) -> ArmatureResult<Vec<EventRecord>> {
        match self.send(DaemonRequest::Events)? {
            ResponsePayload::Events { events } => Ok(events),
            _ => Err(ArmatureError::internal("unexpected events response")),
        }
    }

    pub fn triggers(&self) -> ArmatureResult<Vec<TriggerRecord>> {
        match self.send(DaemonRequest::Triggers)? {
            ResponsePayload::Triggers { triggers } => Ok(triggers),
            _ => Err(ArmatureError::internal("unexpected triggers response")),
        }
    }

    pub fn runs(&self) -> ArmatureResult<Vec<RunRecord>> {
        match self.send(DaemonRequest::Runs)? {
            ResponsePayload::Runs { runs } => Ok(runs),
            _ => Err(ArmatureError::internal("unexpected runs response")),
        }
    }

    pub fn start_task(&self, name: impl Into<String>) -> ArmatureResult<RunId> {
        self.start_task_with_provenance(name, None, None, None)
    }

    pub fn start_task_with_provenance(
        &self,
        name: impl Into<String>,
        source_run_id: Option<RunId>,
        parent_event_id: Option<EventId>,
        correlation_id: Option<String>,
    ) -> ArmatureResult<RunId> {
        match self.send(DaemonRequest::StartTask {
            name: name.into(),
            source_run_id,
            parent_event_id,
            correlation_id,
        })? {
            ResponsePayload::StartedRun { run_id } => Ok(run_id),
            _ => Err(ArmatureError::internal("unexpected task start response")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_adhoc(
        &self,
        name: impl Into<String>,
        command: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        timeout: Option<Duration>,
        payload: Value,
        source_run_id: Option<RunId>,
        parent_event_id: Option<EventId>,
        correlation_id: Option<String>,
    ) -> ArmatureResult<RunId> {
        match self.send(DaemonRequest::StartAdhoc {
            name: name.into(),
            command,
            cwd,
            env,
            timeout_ms: timeout.map(|duration| duration.as_millis() as u64),
            payload,
            source_run_id,
            parent_event_id,
            correlation_id,
        })? {
            ResponsePayload::StartedRun { run_id } => Ok(run_id),
            _ => Err(ArmatureError::internal("unexpected ad hoc start response")),
        }
    }

    pub fn emit_event(
        &self,
        event_type: impl Into<String>,
        payload: Value,
        source: Option<String>,
    ) -> ArmatureResult<()> {
        self.emit_event_with_provenance(event_type, payload, source, None, None, None)
    }

    pub fn emit_event_with_provenance(
        &self,
        event_type: impl Into<String>,
        payload: Value,
        source: Option<String>,
        source_run_id: Option<RunId>,
        parent_event_id: Option<EventId>,
        correlation_id: Option<String>,
    ) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::EmitEvent {
            event_type: event_type.into(),
            payload,
            source,
            source_run_id,
            parent_event_id,
            correlation_id,
        })
    }

    pub fn cancel_run(&self, run_id: impl Into<String>) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::CancelRun {
            run_id: run_id.into(),
        })
    }

    pub fn acquire_lock(
        &self,
        name: impl Into<String>,
        ttl: Duration,
        reason: Option<String>,
    ) -> ArmatureResult<ManualLockRecord> {
        let owner_pid = std::process::id();
        match self.send(DaemonRequest::LockAcquire {
            name: name.into(),
            ttl_ms: ttl.as_millis() as u64,
            owner_pid,
            owner_id: lock_owner_id(owner_pid),
            reason,
        })? {
            ResponsePayload::LockAcquired { lock } => Ok(lock),
            _ => Err(ArmatureError::internal("unexpected lock acquire response")),
        }
    }

    pub fn renew_lock(
        &self,
        name: impl Into<String>,
        token: impl Into<String>,
        ttl: Duration,
    ) -> ArmatureResult<ManualLockRecord> {
        match self.send(DaemonRequest::LockRenew {
            name: name.into(),
            token: token.into(),
            ttl_ms: ttl.as_millis() as u64,
        })? {
            ResponsePayload::LockRenewed { lock } => Ok(lock),
            _ => Err(ArmatureError::internal("unexpected lock renew response")),
        }
    }

    pub fn release_lock(
        &self,
        name: impl Into<String>,
        token: impl Into<String>,
    ) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::LockRelease {
            name: name.into(),
            token: token.into(),
        })
    }

    pub fn force_release_lock(
        &self,
        name: impl Into<String>,
        reason: impl Into<String>,
    ) -> ArmatureResult<ManualLockRecord> {
        match self.send(DaemonRequest::LockForceRelease {
            name: name.into(),
            reason: reason.into(),
            requested_by_pid: std::process::id(),
        })? {
            ResponsePayload::LockForceReleased { lock } => Ok(lock),
            _ => Err(ArmatureError::internal(
                "unexpected lock force-release response",
            )),
        }
    }

    pub fn locks(&self) -> ArmatureResult<Vec<ManualLockRecord>> {
        match self.send(DaemonRequest::LockStatus)? {
            ResponsePayload::Locks { locks } => Ok(locks),
            _ => Err(ArmatureError::internal("unexpected lock status response")),
        }
    }

    pub fn service_stop(&self, name: impl Into<String>) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::ServiceStop { name: name.into() })
    }

    pub fn service_start(&self, name: impl Into<String>) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::ServiceStart { name: name.into() })
    }

    pub fn service_restart(&self, name: impl Into<String>) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::ServiceRestart { name: name.into() })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn service_add(
        &self,
        name: impl Into<String>,
        command: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        restart: RestartMode,
        reason: Option<String>,
        source_run_id: Option<RunId>,
        parent_event_id: Option<EventId>,
        correlation_id: Option<String>,
    ) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::ServiceAdd {
            name: name.into(),
            command,
            cwd,
            env,
            restart,
            reason,
            source_run_id,
            parent_event_id,
            correlation_id,
        })
    }

    pub fn service_remove(&self, name: impl Into<String>) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::ServiceRemove { name: name.into() })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn task_add(
        &self,
        name: impl Into<String>,
        command: Vec<String>,
        on: Option<String>,
        watch: Vec<String>,
        schedule: Option<String>,
        settle: Option<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        source_run_id: Option<RunId>,
        parent_event_id: Option<EventId>,
        correlation_id: Option<String>,
    ) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::TaskAdd {
            name: name.into(),
            command,
            on,
            watch,
            schedule,
            settle,
            cwd,
            env,
            source_run_id,
            parent_event_id,
            correlation_id,
        })
    }

    pub fn task_remove(&self, name: impl Into<String>) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::TaskRemove { name: name.into() })
    }

    pub fn reload_config(&self) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::ReloadConfig)
    }

    pub fn shutdown(&self) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::Shutdown)
    }

    fn expect_empty(&self, request: DaemonRequest) -> ArmatureResult<()> {
        match self.send(request)? {
            ResponsePayload::Empty => Ok(()),
            _ => Err(ArmatureError::internal("unexpected daemon response")),
        }
    }

    fn send(&self, request: DaemonRequest) -> ArmatureResult<ResponsePayload> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|error| {
            ArmatureError::unavailable(format!(
                "failed to connect to daemon at {}: {}",
                self.socket_path.display(),
                error
            ))
        })?;
        write_json_line(&mut stream, &request)?;
        let response: DaemonResponse = read_json_line(&mut stream)?;
        match response {
            DaemonResponse::Ok { payload } => Ok(payload),
            DaemonResponse::Error { kind, message } => Err(ArmatureError {
                kind: kind.into(),
                message: message.into(),
            }),
        }
    }
}

struct Runtime {
    workspace: Workspace,
    runtime_paths: WorkspaceRuntimePaths,
    listener: UnixListener,
    store: SqliteStore,
    config: ArmatureConfig,
    services: HashMap<String, ManagedService>,
    tasks: HashMap<String, ManagedTask>,
    schedules: HashMap<String, ScheduleState>,
    watches: HashMap<String, WatchState>,
    active_runs: HashMap<String, ManagedRun>,
    shutdown_requested: bool,
    options: DaemonOptions,
}

struct ManagedService {
    config: ServiceConfig,
    dynamic: bool,
    command_argv: Option<Vec<String>>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    created_by_run_id: Option<RunId>,
    parent_event_id: Option<EventId>,
    correlation_id: Option<String>,
    reason: Option<String>,
    stop_override: bool,
    active_run_id: Option<String>,
    health: Option<ManagedHealth>,
    restart_attempts: VecDeque<Instant>,
    pending_restart: Option<RestartLineage>,
    next_start_at: Option<Instant>,
    exhausted: bool,
    last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RestartLineage {
    restart_of: RunId,
    attempt: u32,
}

#[derive(Clone)]
struct ManagedHealth {
    state: HealthState,
    active_run_id: Option<String>,
    last_run_id: Option<String>,
    last_error: Option<String>,
    next_check_at: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HealthState {
    Unknown,
    Checking,
    Healthy,
    Unhealthy,
}

struct ManagedTask {
    config: TaskConfig,
    dynamic: bool,
    command_argv: Option<Vec<String>>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    created_by_run_id: Option<RunId>,
    parent_event_id: Option<EventId>,
    correlation_id: Option<String>,
    active_run_ids: Vec<String>,
    pending: VecDeque<PendingTrigger>,
}

#[derive(Clone)]
struct PendingTrigger {
    event: EventRecord,
}

struct ManagedRun {
    record: RunRecord,
    child: Child,
    kind: RunKind,
    task_name: Option<String>,
    kill_after: Option<Duration>,
    started_at: Instant,
    stop_started_at: Option<Instant>,
    stop_reason: Option<StopReason>,
}

struct AdhocRunRequest {
    name: String,
    command: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    timeout: Option<Duration>,
    payload: Value,
    source_run_id: Option<RunId>,
    parent_event_id: Option<EventId>,
    correlation_id: Option<String>,
}

struct DynamicServiceRequest {
    name: String,
    command: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    restart: RestartMode,
    reason: Option<String>,
    source_run_id: Option<RunId>,
    parent_event_id: Option<EventId>,
    correlation_id: Option<String>,
}

struct DynamicTaskRequest {
    name: String,
    command: Vec<String>,
    on: Option<String>,
    watch: Vec<String>,
    schedule: Option<String>,
    settle: Option<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    source_run_id: Option<RunId>,
    parent_event_id: Option<EventId>,
    correlation_id: Option<String>,
}

#[derive(Clone)]
struct ScheduleState {
    schedule: Schedule,
    next_fire_at: Option<DateTime<Utc>>,
}

struct WatchState {
    settle_for: Duration,
    known_files: HashMap<PathBuf, FileFingerprint>,
    pending_paths: HashMap<PathBuf, Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileFingerprint {
    modified_at: Option<SystemTime>,
    len: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RunKind {
    Task,
    Service,
    HealthCheck,
    Adhoc,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StopReason {
    Cancelled,
    TimedOut,
    Shutdown,
    ServiceUpdate,
    AdmissionRestart,
}

enum RouteTarget {
    AllMatching,
    Task(String),
}

impl Runtime {
    fn new(
        workspace: Workspace,
        runtime_paths: WorkspaceRuntimePaths,
        options: DaemonOptions,
    ) -> ArmatureResult<Self> {
        let listener = bind_listener(runtime_paths.socket_path())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| ArmatureError::internal(error.to_string()))?;
        let store = SqliteStore::open_with_paths(runtime_paths.clone())?;
        recover_unfinished_runs(&store)?;
        let config = load_workspace_config(&workspace)?;
        let mut runtime = Self {
            workspace,
            runtime_paths,
            listener,
            store,
            config,
            services: HashMap::new(),
            tasks: HashMap::new(),
            schedules: HashMap::new(),
            watches: HashMap::new(),
            active_runs: HashMap::new(),
            shutdown_requested: false,
            options,
        };
        runtime.rebuild_runtime_state()?;
        runtime.reconcile_services()?;
        Ok(runtime)
    }

    fn run(&mut self) -> ArmatureResult<()> {
        while !self.shutdown_requested || !self.active_runs.is_empty() {
            self.accept_requests()?;
            if !self.shutdown_requested {
                self.poll_schedules()?;
                self.poll_watches()?;
            }
            self.poll_runs()?;
            if !self.shutdown_requested {
                self.reconcile_services()?;
                self.poll_health_checks()?;
            } else {
                self.stop_all_runs(StopReason::Shutdown)?;
            }
            thread::sleep(self.options.poll_interval);
        }
        Ok(())
    }

    fn accept_requests(&mut self) -> ArmatureResult<()> {
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let response = match read_json_line::<DaemonRequest>(&mut stream) {
                        Ok(request) => self.handle_request(request),
                        Err(error) => DaemonResponse::Error {
                            kind: error.kind.to_string(),
                            message: error.message.to_string(),
                        },
                    };
                    write_json_line(&mut stream, &response)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(ArmatureError::internal(error.to_string())),
            }
        }
    }

    fn handle_request(&mut self, request: DaemonRequest) -> DaemonResponse {
        match self.handle_request_inner(request) {
            Ok(payload) => DaemonResponse::Ok { payload },
            Err(error) => DaemonResponse::Error {
                kind: error.kind.to_string(),
                message: error.message.to_string(),
            },
        }
    }

    fn handle_request_inner(&mut self, request: DaemonRequest) -> ArmatureResult<ResponsePayload> {
        if self.shutdown_requested && !request_allowed_during_shutdown(&request) {
            return Err(ArmatureError::invalid_state("daemon is shutting down"));
        }

        match request {
            DaemonRequest::Inspect => Ok(ResponsePayload::Inspect(self.inspect())),
            DaemonRequest::Events => Ok(ResponsePayload::Events {
                events: self.store.list_events()?,
            }),
            DaemonRequest::Triggers => Ok(ResponsePayload::Triggers {
                triggers: self.store.list_triggers()?,
            }),
            DaemonRequest::Runs => Ok(ResponsePayload::Runs {
                runs: self.store.list_runs()?,
            }),
            DaemonRequest::StartTask {
                name,
                source_run_id,
                parent_event_id,
                correlation_id,
            } => {
                let run_id =
                    self.start_task(&name, source_run_id, parent_event_id, correlation_id)?;
                Ok(ResponsePayload::StartedRun { run_id })
            }
            DaemonRequest::StartAdhoc {
                name,
                command,
                cwd,
                env,
                timeout_ms,
                payload,
                source_run_id,
                parent_event_id,
                correlation_id,
            } => {
                let run_id = self.start_adhoc(AdhocRunRequest {
                    name,
                    command,
                    cwd,
                    env,
                    timeout: timeout_ms.map(Duration::from_millis),
                    payload,
                    source_run_id,
                    parent_event_id,
                    correlation_id,
                })?;
                Ok(ResponsePayload::StartedRun { run_id })
            }
            DaemonRequest::EmitEvent {
                event_type,
                payload,
                source,
                source_run_id,
                parent_event_id,
                correlation_id,
            } => {
                self.emit_user_event(
                    event_type,
                    payload,
                    source,
                    source_run_id,
                    parent_event_id,
                    correlation_id,
                )?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::CancelRun { run_id } => {
                self.cancel_run(&run_id, StopReason::Cancelled)?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::LockAcquire {
                name,
                ttl_ms,
                owner_pid,
                owner_id,
                reason,
            } => {
                let lock =
                    ManualLockStore::new(self.runtime_paths.state_root().join(LOCKS_DIR_NAME))
                        .acquire(
                            name,
                            ManualLockOwner {
                                pid: owner_pid,
                                id: owner_id,
                            },
                            reason,
                            Duration::from_millis(ttl_ms),
                        )?;
                Ok(ResponsePayload::LockAcquired { lock })
            }
            DaemonRequest::LockRenew {
                name,
                token,
                ttl_ms,
            } => {
                let lock =
                    ManualLockStore::new(self.runtime_paths.state_root().join(LOCKS_DIR_NAME))
                        .renew(&name, &token, Duration::from_millis(ttl_ms))?;
                Ok(ResponsePayload::LockRenewed { lock })
            }
            DaemonRequest::LockRelease { name, token } => {
                ManualLockStore::new(self.runtime_paths.state_root().join(LOCKS_DIR_NAME))
                    .release(&name, &token)?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::LockForceRelease {
                name,
                reason,
                requested_by_pid,
            } => {
                let lock =
                    ManualLockStore::new(self.runtime_paths.state_root().join(LOCKS_DIR_NAME))
                        .force_release(&name)?;
                self.record_lock_audit_event(
                    "lock.force_released",
                    &lock,
                    json!({
                        "name": lock.name,
                        "token": lock.token,
                        "owner_id": lock.owner_id,
                        "owner_pid": lock.owner_pid,
                        "reason": reason,
                        "requested_by_pid": requested_by_pid,
                    }),
                )?;
                Ok(ResponsePayload::LockForceReleased { lock })
            }
            DaemonRequest::LockStatus => {
                let locks =
                    ManualLockStore::new(self.runtime_paths.state_root().join(LOCKS_DIR_NAME))
                        .list()?;
                Ok(ResponsePayload::Locks { locks })
            }
            DaemonRequest::ServiceStart { name } => {
                let service = self.service_mut(&name)?;
                service.stop_override = false;
                service.exhausted = false;
                service.next_start_at = None;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::ServiceStop { name } => {
                let run_ids = {
                    let service = self.service_mut(&name)?;
                    service.stop_override = true;
                    let mut run_ids = Vec::new();
                    if let Some(run_id) = service.active_run_id.clone() {
                        run_ids.push(run_id);
                    }
                    if let Some(run_id) = service
                        .health
                        .as_ref()
                        .and_then(|health| health.active_run_id.clone())
                    {
                        run_ids.push(run_id);
                    }
                    run_ids
                };
                for run_id in run_ids {
                    self.cancel_run(&run_id, StopReason::ServiceUpdate)?;
                }
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::ServiceRestart { name } => {
                let active_run_id = {
                    let service = self.service_mut(&name)?;
                    service.stop_override = false;
                    service.exhausted = false;
                    service.next_start_at = None;
                    service.active_run_id.clone()
                };
                if let Some(run_id) = active_run_id {
                    self.cancel_run(&run_id, StopReason::ServiceUpdate)?;
                }
                let health_run_id = self
                    .services
                    .get(&name)
                    .and_then(|service| service.health.as_ref())
                    .and_then(|health| health.active_run_id.clone());
                if let Some(run_id) = health_run_id {
                    self.cancel_run(&run_id, StopReason::ServiceUpdate)?;
                }
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::ServiceAdd {
                name,
                command,
                cwd,
                env,
                restart,
                reason,
                source_run_id,
                parent_event_id,
                correlation_id,
            } => {
                self.add_dynamic_service(DynamicServiceRequest {
                    name,
                    command,
                    cwd,
                    env,
                    restart,
                    reason,
                    source_run_id,
                    parent_event_id,
                    correlation_id,
                })?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::ServiceRemove { name } => {
                self.remove_dynamic_service(&name)?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::TaskAdd {
                name,
                command,
                on,
                watch,
                schedule,
                settle,
                cwd,
                env,
                source_run_id,
                parent_event_id,
                correlation_id,
            } => {
                self.add_dynamic_task(DynamicTaskRequest {
                    name,
                    command,
                    on,
                    watch,
                    schedule,
                    settle,
                    cwd,
                    env,
                    source_run_id,
                    parent_event_id,
                    correlation_id,
                })?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::TaskRemove { name } => {
                self.remove_dynamic_task(&name)?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::ReloadConfig => {
                self.reload_config()?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::Shutdown => {
                self.shutdown_requested = true;
                Ok(ResponsePayload::Empty)
            }
        }
    }

    fn inspect(&self) -> InspectResponse {
        let mut services = self
            .services
            .values()
            .map(|service| RuntimeServiceStatus {
                name: service.config.name.clone(),
                command: service.config.run.clone(),
                configured_enabled: service.config.enabled,
                dynamic: service.dynamic,
                cwd: service.cwd.clone(),
                env: service.env.clone(),
                created_by_run_id: service.created_by_run_id.clone(),
                parent_event_id: service.parent_event_id.clone(),
                correlation_id: service.correlation_id.clone(),
                reason: service.reason.clone(),
                restart: format!("{:?}", service.config.supervision.restart).to_lowercase(),
                stop_override: service.stop_override,
                state: service_state(service),
                supervision_state: supervision_state(service).to_string(),
                health: service.health.as_ref().map(runtime_health_status),
                active_run_id: service
                    .active_run_id
                    .as_deref()
                    .and_then(|value| RunId::parse(value).ok()),
                last_error: service.last_error.clone(),
            })
            .collect::<Vec<_>>();
        services.sort_by(|left, right| left.name.cmp(&right.name));

        let mut tasks = self
            .tasks
            .values()
            .map(|task| RuntimeTaskStatus {
                name: task.config.name.clone(),
                command: task.config.run.clone(),
                dynamic: task.dynamic,
                cwd: task.cwd.clone(),
                env: task.env.clone(),
                created_by_run_id: task.created_by_run_id.clone(),
                parent_event_id: task.parent_event_id.clone(),
                correlation_id: task.correlation_id.clone(),
                admission: admission_label(task.config.admission.when_busy.clone()).to_string(),
                active_run_ids: task
                    .active_run_ids
                    .iter()
                    .filter_map(|run_id| RunId::parse(run_id).ok())
                    .collect(),
                queued_triggers: task.pending.len(),
                schedule_active: task.config.trigger.schedule.is_some(),
                watch_active: !task.config.trigger.watch.is_empty(),
                event_trigger: task.config.trigger.on.clone(),
                schedule: task.config.trigger.schedule.clone(),
                watch: task.config.trigger.watch.clone(),
                settle: task.config.trigger.settle.clone(),
            })
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| left.name.cmp(&right.name));

        let mut active_runs = self
            .active_runs
            .values()
            .map(|managed| managed.record.clone())
            .collect::<Vec<_>>();
        active_runs.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));

        InspectResponse {
            config_version: self.config.version.clone(),
            socket_path: self.runtime_paths.socket_path().display().to_string(),
            pid_path: self.runtime_paths.pid_path().display().to_string(),
            services,
            tasks,
            active_runs,
        }
    }

    fn start_task(
        &mut self,
        name: &str,
        source_run_id: Option<RunId>,
        parent_event_id: Option<EventId>,
        correlation_id: Option<String>,
    ) -> ArmatureResult<RunId> {
        if !self.tasks.contains_key(name) {
            return Err(ArmatureError::not_found(format!(
                "task {name:?} was not found"
            )));
        }

        let event = EventRecord {
            id: EventId::new(),
            event_type: "manual.run.requested".to_string(),
            time: Utc::now().to_rfc3339(),
            payload: json!({ "task": name }),
            routing: EventRouting::Manual,
            config_version: Some(self.config.version.clone()),
            source: Some("manual".to_string()),
            source_run_id,
            parent_event_id,
            correlation_id,
        };
        self.store.record_event(&event)?;

        let route_result = self.route_event(event, RouteTarget::Task(name.to_string()))?;
        route_result.started_run_id.ok_or_else(|| {
            ArmatureError::conflict(format!(
                "task {name:?} did not start immediately because admission policy queued or rejected it"
            ))
        })
    }

    fn start_adhoc(&mut self, request: AdhocRunRequest) -> ArmatureResult<RunId> {
        if request.name.trim().is_empty() {
            return Err(ArmatureError::invalid_input(
                "ad hoc run name cannot be empty",
            ));
        }
        if request.command.is_empty() {
            return Err(ArmatureError::invalid_input(
                "ad hoc command cannot be empty",
            ));
        }
        let event = EventRecord {
            id: EventId::new(),
            event_type: "adhoc.run.requested".to_string(),
            time: Utc::now().to_rfc3339(),
            payload: request.payload,
            routing: EventRouting::Manual,
            config_version: Some(self.config.version.clone()),
            source: Some("adhoc".to_string()),
            source_run_id: request.source_run_id,
            parent_event_id: request.parent_event_id,
            correlation_id: request.correlation_id,
        };
        self.store.record_event(&event)?;
        self.spawn_adhoc(
            request.name,
            request.command,
            request.cwd,
            request.env,
            request.timeout,
            event,
        )
    }

    fn emit_user_event(
        &mut self,
        event_type: String,
        payload: Value,
        source: Option<String>,
        source_run_id: Option<RunId>,
        parent_event_id: Option<EventId>,
        correlation_id: Option<String>,
    ) -> ArmatureResult<()> {
        let event = EventRecord {
            id: EventId::new(),
            event_type,
            time: Utc::now().to_rfc3339(),
            payload,
            routing: EventRouting::Event,
            config_version: Some(self.config.version.clone()),
            source: source.or_else(|| Some("user".to_string())),
            source_run_id,
            parent_event_id,
            correlation_id,
        };
        self.store.record_event(&event)?;
        let _ = self.route_event(event, RouteTarget::AllMatching)?;
        Ok(())
    }

    fn record_lock_audit_event(
        &self,
        event_type: &str,
        lock: &ManualLockRecord,
        payload: Value,
    ) -> ArmatureResult<()> {
        let event = EventRecord {
            id: EventId::new(),
            event_type: event_type.to_string(),
            time: Utc::now().to_rfc3339(),
            payload,
            routing: EventRouting::Manual,
            config_version: Some(self.config.version.clone()),
            source: Some("lock".to_string()),
            source_run_id: None,
            parent_event_id: None,
            correlation_id: Some(lock.name.clone()),
        };
        self.store.record_event(&event)
    }

    fn reload_config(&mut self) -> ArmatureResult<()> {
        let next = load_workspace_config(&self.workspace)?;
        let previous = self.config.clone();
        self.config = next;
        self.apply_service_config_changes(&previous)?;
        self.rebuild_runtime_state()?;
        Ok(())
    }

    fn apply_service_config_changes(&mut self, previous: &ArmatureConfig) -> ArmatureResult<()> {
        let old_services = previous
            .services
            .iter()
            .map(|service| (service.name.clone(), service.clone()))
            .collect::<HashMap<_, _>>();
        let new_services = self
            .config
            .services
            .iter()
            .map(|service| (service.name.clone(), service.clone()))
            .collect::<HashMap<_, _>>();

        let running_services = self
            .services
            .iter()
            .flat_map(|(name, service)| {
                let mut run_ids = Vec::new();
                if let Some(run_id) = &service.active_run_id {
                    run_ids.push(run_id.clone());
                }
                if let Some(run_id) = service
                    .health
                    .as_ref()
                    .and_then(|health| health.active_run_id.clone())
                {
                    run_ids.push(run_id);
                }
                run_ids.into_iter().map(|run_id| (name.clone(), run_id))
            })
            .collect::<Vec<_>>();

        for (name, active_run_id) in running_services {
            let restart_required = match (old_services.get(&name), new_services.get(&name)) {
                (Some(old), Some(new)) => old != new,
                (None, Some(_)) => true,
                (_, None) => true,
            };
            if restart_required {
                self.cancel_run(&active_run_id, StopReason::ServiceUpdate)?;
            }
        }

        Ok(())
    }

    fn rebuild_runtime_state(&mut self) -> ArmatureResult<()> {
        self.rebuild_services();
        self.rebuild_tasks();
        self.rebuild_schedules()?;
        self.rebuild_watches()?;
        Ok(())
    }

    fn rebuild_services(&mut self) {
        let mut next = HashMap::new();
        for config in &self.config.services {
            let existing = self.services.remove(&config.name);
            let stop_override = existing
                .as_ref()
                .map(|entry| entry.stop_override)
                .unwrap_or(false);
            let active_run_id = existing
                .as_ref()
                .and_then(|entry| entry.active_run_id.clone());
            let pending_restart = existing
                .as_ref()
                .and_then(|entry| entry.pending_restart.clone());
            let existing_health = existing.as_ref().and_then(|entry| entry.health.clone());
            let health = config.health.as_ref().map(|_| {
                existing_health.unwrap_or(ManagedHealth {
                    state: HealthState::Unknown,
                    active_run_id: None,
                    last_run_id: None,
                    last_error: None,
                    next_check_at: None,
                })
            });
            next.insert(
                config.name.clone(),
                ManagedService {
                    config: config.clone(),
                    dynamic: false,
                    command_argv: None,
                    cwd: None,
                    env: Vec::new(),
                    created_by_run_id: None,
                    parent_event_id: None,
                    correlation_id: None,
                    reason: None,
                    stop_override,
                    active_run_id,
                    health,
                    restart_attempts: VecDeque::new(),
                    pending_restart,
                    next_start_at: None,
                    exhausted: false,
                    last_error: None,
                },
            );
        }
        for (_, service) in self.services.drain().filter(|(_, service)| service.dynamic) {
            next.insert(service.config.name.clone(), service);
        }
        self.services = next;
    }

    fn add_dynamic_service(&mut self, request: DynamicServiceRequest) -> ArmatureResult<()> {
        validate_dynamic_name("service", &request.name)?;
        if request.command.is_empty() {
            return Err(ArmatureError::invalid_input(
                "dynamic service command cannot be empty",
            ));
        }
        let cwd = request
            .cwd
            .map(|cwd| resolve_run_cwd(self.workspace.root(), Some(cwd)))
            .transpose()?;
        if self
            .services
            .get(&request.name)
            .is_some_and(|service| !service.dynamic)
        {
            return Err(ArmatureError::conflict(format!(
                "static service {:?} already exists",
                request.name
            )));
        }
        let run = command_display(&request.command);
        let config = ServiceConfig {
            name: request.name.clone(),
            run,
            enabled: true,
            supervision: SupervisionPolicyConfig {
                restart: request.restart,
                max_restarts: None,
                within: None,
                backoff: None,
                start_delay: None,
            },
            health: None,
            resources: ResourcePolicy::default(),
        };
        self.services.insert(
            request.name.clone(),
            ManagedService {
                config,
                dynamic: true,
                command_argv: Some(request.command),
                cwd,
                env: request.env,
                created_by_run_id: request.source_run_id,
                parent_event_id: request.parent_event_id,
                correlation_id: request.correlation_id,
                reason: request.reason,
                stop_override: false,
                active_run_id: None,
                health: None,
                restart_attempts: VecDeque::new(),
                pending_restart: None,
                next_start_at: None,
                exhausted: false,
                last_error: None,
            },
        );
        self.reconcile_services()
    }

    fn remove_dynamic_service(&mut self, name: &str) -> ArmatureResult<()> {
        let active_run_id = {
            let service = self.service(name)?;
            if !service.dynamic {
                return Err(ArmatureError::conflict(format!(
                    "static service {name:?} cannot be removed at runtime"
                )));
            }
            service.active_run_id.clone()
        };
        if let Some(run_id) = active_run_id {
            self.cancel_run(&run_id, StopReason::ServiceUpdate)?;
        }
        self.services.remove(name);
        Ok(())
    }

    fn add_dynamic_task(&mut self, request: DynamicTaskRequest) -> ArmatureResult<()> {
        validate_dynamic_name("task", &request.name)?;
        if request.command.is_empty() {
            return Err(ArmatureError::invalid_input(
                "dynamic task command cannot be empty",
            ));
        }
        let trigger_kinds = usize::from(request.on.is_some())
            + usize::from(!request.watch.is_empty())
            + usize::from(request.schedule.is_some());
        if trigger_kinds != 1 {
            return Err(ArmatureError::invalid_input(
                "dynamic task must define exactly one of --on, --watch, or --schedule",
            ));
        }
        if let Some(event_type) = &request.on {
            validate_nonempty_dynamic("dynamic task event trigger", event_type)?;
        }
        if let Some(schedule) = &request.schedule {
            validate_nonempty_dynamic("dynamic task schedule", schedule)?;
            parse_schedule(schedule)?;
        }
        for pattern in &request.watch {
            validate_nonempty_dynamic("dynamic task watch pattern", pattern)?;
        }
        if let Some(settle) = &request.settle {
            if request.watch.is_empty() {
                return Err(ArmatureError::invalid_input(
                    "dynamic task sets --settle without any --watch patterns",
                ));
            }
            parse_duration(settle)?;
        }
        let cwd = request
            .cwd
            .map(|cwd| resolve_run_cwd(self.workspace.root(), Some(cwd)))
            .transpose()?;
        if self
            .tasks
            .get(&request.name)
            .is_some_and(|task| !task.dynamic)
        {
            return Err(ArmatureError::conflict(format!(
                "static task {:?} already exists",
                request.name
            )));
        }

        let run = command_display(&request.command);
        let config = TaskConfig {
            name: request.name.clone(),
            run,
            trigger: TriggerConfig {
                schedule: request.schedule,
                watch: request.watch,
                on: request.on,
                settle: request.settle,
            },
            admission: AdmissionConfig::default(),
            supervision: SupervisionPolicyConfig::default(),
            resources: ResourcePolicy::default(),
        };
        self.tasks.insert(
            request.name.clone(),
            ManagedTask {
                config,
                dynamic: true,
                command_argv: Some(request.command),
                cwd,
                env: request.env,
                created_by_run_id: request.source_run_id,
                parent_event_id: request.parent_event_id,
                correlation_id: request.correlation_id,
                active_run_ids: Vec::new(),
                pending: VecDeque::new(),
            },
        );
        self.rebuild_schedules()?;
        self.rebuild_watches()
    }

    fn remove_dynamic_task(&mut self, name: &str) -> ArmatureResult<()> {
        let task = self
            .tasks
            .get(name)
            .ok_or_else(|| ArmatureError::not_found(format!("task {name:?} was not found")))?;
        if !task.dynamic {
            return Err(ArmatureError::conflict(format!(
                "static task {name:?} cannot be removed at runtime"
            )));
        }
        self.tasks.remove(name);
        self.schedules.remove(name);
        self.watches.remove(name);
        Ok(())
    }

    fn rebuild_tasks(&mut self) {
        let mut next = HashMap::new();
        for config in &self.config.tasks {
            let existing = self.tasks.remove(&config.name);
            next.insert(
                config.name.clone(),
                ManagedTask {
                    config: config.clone(),
                    dynamic: false,
                    command_argv: None,
                    cwd: None,
                    env: Vec::new(),
                    created_by_run_id: None,
                    parent_event_id: None,
                    correlation_id: None,
                    active_run_ids: existing
                        .as_ref()
                        .map(|entry| entry.active_run_ids.clone())
                        .unwrap_or_default(),
                    pending: existing.map(|entry| entry.pending).unwrap_or_default(),
                },
            );
        }
        for (_, task) in self.tasks.drain().filter(|(_, task)| task.dynamic) {
            next.insert(task.config.name.clone(), task);
        }
        self.tasks = next;
    }

    fn rebuild_schedules(&mut self) -> ArmatureResult<()> {
        let mut next = HashMap::new();
        for task in self.tasks.values() {
            if let Some(schedule) = &task.config.trigger.schedule {
                let parsed = parse_schedule(schedule)?;
                let next_fire_at = next_schedule_time(&parsed);
                next.insert(
                    task.config.name.clone(),
                    ScheduleState {
                        schedule: parsed,
                        next_fire_at,
                    },
                );
            }
        }
        self.schedules = next;
        Ok(())
    }

    fn rebuild_watches(&mut self) -> ArmatureResult<()> {
        let mut next = HashMap::new();
        for task in self.tasks.values() {
            if !task.config.trigger.watch.is_empty() {
                let settle_for = task
                    .config
                    .trigger
                    .settle
                    .as_deref()
                    .map(parse_duration)
                    .transpose()?
                    .unwrap_or(DEFAULT_WATCH_SETTLE);
                next.insert(
                    task.config.name.clone(),
                    WatchState {
                        settle_for,
                        known_files: HashMap::new(),
                        pending_paths: HashMap::new(),
                    },
                );
            }
        }
        self.watches = next;
        Ok(())
    }

    fn reconcile_services(&mut self) -> ArmatureResult<()> {
        let now = Instant::now();
        let service_names = self.services.keys().cloned().collect::<Vec<_>>();
        for name in service_names {
            let should_start = {
                let service = self.service(&name)?;
                service.config.enabled
                    && !service.stop_override
                    && service.active_run_id.is_none()
                    && !service.exhausted
                    && service
                        .next_start_at
                        .map(|deadline| deadline <= now)
                        .unwrap_or(true)
            };
            if should_start {
                self.spawn_service(&name)?;
            }
        }
        Ok(())
    }

    fn poll_health_checks(&mut self) -> ArmatureResult<()> {
        let now = Instant::now();
        let service_names = self.services.keys().cloned().collect::<Vec<_>>();
        for name in service_names {
            let should_check = {
                let service = self.service(&name)?;
                service.config.enabled
                    && !service.stop_override
                    && service.active_run_id.is_some()
                    && service
                        .health
                        .as_ref()
                        .map(|health| {
                            health.active_run_id.is_none()
                                && health
                                    .next_check_at
                                    .map(|deadline| deadline <= now)
                                    .unwrap_or(true)
                        })
                        .unwrap_or(false)
            };
            if should_check {
                self.spawn_health_check(&name)?;
            }
        }
        Ok(())
    }

    fn poll_schedules(&mut self) -> ArmatureResult<()> {
        let now = Utc::now();
        let task_names = self.schedules.keys().cloned().collect::<Vec<_>>();
        for task_name in task_names {
            let should_fire = self
                .schedules
                .get(&task_name)
                .and_then(|state| state.next_fire_at)
                .map(|deadline| deadline <= now)
                .unwrap_or(false);
            if !should_fire {
                continue;
            }

            let schedule_literal = self
                .tasks
                .get(&task_name)
                .and_then(|task| task.config.trigger.schedule.clone())
                .unwrap_or_default();
            let event = EventRecord {
                id: EventId::new(),
                event_type: "timer.fired".to_string(),
                time: Utc::now().to_rfc3339(),
                payload: json!({
                    "task": task_name,
                    "schedule": schedule_literal,
                    "time": Local::now().to_rfc3339(),
                }),
                routing: EventRouting::Schedule,
                config_version: Some(self.config.version.clone()),
                source: Some(format!("schedule:{task_name}")),
                source_run_id: None,
                parent_event_id: None,
                correlation_id: None,
            };
            self.store.record_event(&event)?;
            let _ = self.route_event(event, RouteTarget::Task(task_name.clone()))?;
            if let Some(state) = self.schedules.get_mut(&task_name) {
                state.next_fire_at = next_schedule_time(&state.schedule);
            }
        }
        Ok(())
    }

    fn poll_watches(&mut self) -> ArmatureResult<()> {
        let task_names = self.watches.keys().cloned().collect::<Vec<_>>();
        for task_name in task_names {
            let patterns = match self.tasks.get(&task_name) {
                Some(task) => task.config.trigger.watch.clone(),
                None => continue,
            };
            let watch = self
                .watches
                .get_mut(&task_name)
                .ok_or_else(|| ArmatureError::internal("watch state disappeared"))?;
            let changed = scan_watch_patterns(self.workspace.root(), &patterns, watch)?;
            let now = Instant::now();
            let ready = watch
                .pending_paths
                .iter()
                .filter_map(|(path, seen_at)| {
                    if now.duration_since(*seen_at) >= watch.settle_for {
                        Some(path.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if ready.is_empty() {
                let _ = changed;
                continue;
            }
            for path in &ready {
                watch.pending_paths.remove(path);
            }
            let relative_paths = ready
                .iter()
                .map(|path| relative_display(self.workspace.root(), path))
                .collect::<Vec<_>>();
            let event = EventRecord {
                id: EventId::new(),
                event_type: "file.changed".to_string(),
                time: Utc::now().to_rfc3339(),
                payload: json!({
                    "task": task_name,
                    "paths": relative_paths,
                }),
                routing: EventRouting::Watch,
                config_version: Some(self.config.version.clone()),
                source: Some(format!("watch:{task_name}")),
                source_run_id: None,
                parent_event_id: None,
                correlation_id: None,
            };
            self.store.record_event(&event)?;
            let _ = self.route_event(event, RouteTarget::Task(task_name.clone()))?;
        }
        Ok(())
    }

    fn route_event(
        &mut self,
        event: EventRecord,
        target: RouteTarget,
    ) -> ArmatureResult<RouteResult> {
        let task_names = match target {
            RouteTarget::AllMatching => self
                .tasks
                .values()
                .filter(|task| task.config.trigger.on.as_deref() == Some(event.event_type.as_str()))
                .map(|task| task.config.name.clone())
                .collect::<Vec<_>>(),
            RouteTarget::Task(name) => vec![name],
        };

        let mut started_run_id = None;
        for task_name in task_names {
            if !self.tasks.contains_key(&task_name) {
                continue;
            }
            let run_id = self.apply_admission(&task_name, event.clone())?;
            if started_run_id.is_none() {
                started_run_id = run_id;
            }
        }

        Ok(RouteResult { started_run_id })
    }

    fn apply_admission(
        &mut self,
        task_name: &str,
        event: EventRecord,
    ) -> ArmatureResult<Option<RunId>> {
        let policy = self
            .tasks
            .get(task_name)
            .map(|task| task.config.admission.when_busy.clone())
            .ok_or_else(|| ArmatureError::not_found(format!("task {task_name:?} was not found")))?;

        let is_busy = self
            .tasks
            .get(task_name)
            .map(|task| !task.active_run_ids.is_empty())
            .unwrap_or(false);

        if !is_busy {
            return self.start_task_run(task_name, event);
        }

        match policy {
            AdmissionPolicy::Allow => self.start_task_run(task_name, event),
            AdmissionPolicy::Reject => {
                self.record_trigger(
                    task_name,
                    &event,
                    policy,
                    TriggerOutcome::Rejected,
                    None,
                    Some("task already has an active run".to_string()),
                )?;
                Ok(None)
            }
            AdmissionPolicy::QueueAll => {
                self.queue_pending(task_name, event.clone())?;
                self.record_trigger(
                    task_name,
                    &event,
                    policy,
                    TriggerOutcome::Queued,
                    None,
                    Some("queued behind an active run".to_string()),
                )?;
                Ok(None)
            }
            AdmissionPolicy::QueueOne => {
                let already_pending = self
                    .tasks
                    .get(task_name)
                    .map(|task| !task.pending.is_empty())
                    .unwrap_or(false);
                if already_pending {
                    self.record_trigger(
                        task_name,
                        &event,
                        policy.clone(),
                        TriggerOutcome::Coalesced,
                        None,
                        Some("coalesced into the existing queued trigger".to_string()),
                    )?;
                    Ok(None)
                } else {
                    self.queue_pending(task_name, event.clone())?;
                    self.record_trigger(
                        task_name,
                        &event,
                        policy.clone(),
                        TriggerOutcome::Queued,
                        None,
                        Some("queued behind an active run".to_string()),
                    )?;
                    Ok(None)
                }
            }
            AdmissionPolicy::Restart => {
                let replaced = {
                    let task = self.task_mut(task_name)?;
                    task.pending.pop_back()
                };
                if let Some(previous) = replaced {
                    self.record_trigger(
                        task_name,
                        &previous.event,
                        policy.clone(),
                        TriggerOutcome::Superseded,
                        None,
                        Some("superseded by a newer restart trigger".to_string()),
                    )?;
                }
                self.queue_pending(task_name, event.clone())?;
                self.record_trigger(
                    task_name,
                    &event,
                    policy.clone(),
                    TriggerOutcome::Queued,
                    None,
                    Some("queued while active runs stop for restart admission".to_string()),
                )?;
                let active_run_ids = self
                    .tasks
                    .get(task_name)
                    .map(|task| task.active_run_ids.clone())
                    .unwrap_or_default();
                for run_id in active_run_ids {
                    self.cancel_run(&run_id, StopReason::AdmissionRestart)?;
                }
                Ok(None)
            }
        }
    }

    fn queue_pending(&mut self, task_name: &str, event: EventRecord) -> ArmatureResult<()> {
        let task = self.task_mut(task_name)?;
        task.pending.push_back(PendingTrigger { event });
        Ok(())
    }

    fn maybe_start_next_pending(&mut self, task_name: &str) -> ArmatureResult<()> {
        let event = {
            let task = self.task_mut(task_name)?;
            if !task.active_run_ids.is_empty() {
                return Ok(());
            }
            task.pending.pop_front().map(|pending| pending.event)
        };

        if let Some(event) = event {
            let _ = self.start_task_run(task_name, event)?;
        }
        Ok(())
    }

    fn start_task_run(
        &mut self,
        task_name: &str,
        event: EventRecord,
    ) -> ArmatureResult<Option<RunId>> {
        let task = self
            .tasks
            .get(task_name)
            .map(|entry| entry.config.clone())
            .ok_or_else(|| ArmatureError::not_found(format!("task {task_name:?} was not found")))?;
        let admission = task.admission.when_busy.clone();
        let run_id = self.spawn_task(task, Some(event.clone()))?;
        self.record_trigger(
            task_name,
            &event,
            admission,
            TriggerOutcome::Started,
            Some(run_id.clone()),
            None,
        )?;
        Ok(Some(run_id))
    }

    fn spawn_task(
        &mut self,
        task: TaskConfig,
        event: Option<EventRecord>,
    ) -> ArmatureResult<RunId> {
        let task_runtime = self.tasks.get(&task.name);
        let command_argv = task_runtime.and_then(|entry| entry.command_argv.clone());
        let cwd = task_runtime.and_then(|entry| entry.cwd.clone());
        let task_env = task_runtime
            .map(|entry| entry.env.clone())
            .unwrap_or_default();
        let kill_after = optional_duration(task.resources.kill_after.as_deref())?;
        let prepared = self.store.create_run(
            task.name.clone(),
            task.run.clone(),
            RunOrigin::Task,
            Some(self.config.version.clone()),
            event.as_ref().map(|record| record.id.clone()),
        )?;
        if let Some(event) = &event {
            write_event_artifact(&prepared.paths.event, event)?;
        }
        write_run_meta(&prepared.paths.meta, &prepared.record, "task")?;
        let run_cwd = cwd.as_deref().unwrap_or_else(|| self.workspace.root());
        let mut envs = run_envs(
            "task",
            &self.workspace,
            &self.runtime_paths,
            &prepared.record,
            self.config.version.as_str(),
            event.as_ref(),
            &prepared.paths.event,
        );
        envs.extend(task_env);
        let child = if let Some(command) = command_argv {
            spawn_command(
                &command,
                run_cwd,
                &prepared.paths.stdout,
                &prepared.paths.stderr,
                &envs,
            )
        } else {
            spawn_shell_command(
                &task.run,
                run_cwd,
                &prepared.paths.stdout,
                &prepared.paths.stderr,
                &envs,
            )
        }?;
        self.store
            .update_run_state(&prepared.record.id, ProcessState::Running)?;
        let mut record = prepared.record;
        record.state = ProcessState::Running;
        let run_id = record.id.clone();
        self.active_runs.insert(
            run_id.as_str().to_string(),
            ManagedRun {
                record,
                child,
                kind: RunKind::Task,
                task_name: Some(task.name.clone()),
                kill_after,
                started_at: Instant::now(),
                stop_started_at: None,
                stop_reason: None,
            },
        );
        let task_state = self.task_mut(&task.name)?;
        task_state.active_run_ids.push(run_id.as_str().to_string());
        Ok(run_id)
    }

    fn spawn_adhoc(
        &mut self,
        name: String,
        command: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        timeout: Option<Duration>,
        event: EventRecord,
    ) -> ArmatureResult<RunId> {
        let command_text = command_display(&command);
        let prepared = self.store.create_run(
            name.clone(),
            command_text,
            RunOrigin::Adhoc,
            Some(self.config.version.clone()),
            Some(event.id.clone()),
        )?;
        write_event_artifact(&prepared.paths.event, &event)?;
        write_run_meta(&prepared.paths.meta, &prepared.record, "adhoc")?;
        let cwd = resolve_run_cwd(self.workspace.root(), cwd)?;
        let mut envs = run_envs(
            "adhoc",
            &self.workspace,
            &self.runtime_paths,
            &prepared.record,
            self.config.version.as_str(),
            Some(&event),
            &prepared.paths.event,
        );
        envs.extend(env);
        let child = spawn_command(
            &command,
            &cwd,
            &prepared.paths.stdout,
            &prepared.paths.stderr,
            &envs,
        )?;
        self.store
            .update_run_state(&prepared.record.id, ProcessState::Running)?;
        let mut record = prepared.record;
        record.state = ProcessState::Running;
        let run_id = record.id.clone();
        self.active_runs.insert(
            run_id.as_str().to_string(),
            ManagedRun {
                record,
                child,
                kind: RunKind::Adhoc,
                task_name: None,
                kill_after: timeout,
                started_at: Instant::now(),
                stop_started_at: None,
                stop_reason: None,
            },
        );
        Ok(run_id)
    }

    fn spawn_service(&mut self, name: &str) -> ArmatureResult<()> {
        let service = self.service(name)?;
        let config = service.config.clone();
        let command_argv = service.command_argv.clone();
        let cwd = service.cwd.clone();
        let service_env = service.env.clone();
        let kill_after = optional_duration(config.resources.kill_after.as_deref())?;
        let pending_restart = self.service(name)?.pending_restart.clone();
        let prepared = self.store.create_run_record(NewRun {
            name: config.name.clone(),
            command: config.run.clone(),
            origin: if pending_restart.is_some() {
                RunOrigin::Restart
            } else {
                RunOrigin::Service
            },
            config_version: Some(self.config.version.clone()),
            event_id: None,
            restart_of: pending_restart
                .as_ref()
                .map(|lineage| lineage.restart_of.clone()),
            attempt: pending_restart.as_ref().map(|lineage| lineage.attempt),
        })?;
        write_run_meta(&prepared.paths.meta, &prepared.record, "service")?;
        let run_cwd = cwd.as_deref().unwrap_or_else(|| self.workspace.root());
        let mut envs = run_envs(
            "service",
            &self.workspace,
            &self.runtime_paths,
            &prepared.record,
            self.config.version.as_str(),
            None,
            &prepared.paths.event,
        );
        envs.extend(service_env);
        let child = if let Some(command) = command_argv {
            spawn_command(
                &command,
                run_cwd,
                &prepared.paths.stdout,
                &prepared.paths.stderr,
                &envs,
            )
        } else {
            spawn_shell_command(
                &config.run,
                run_cwd,
                &prepared.paths.stdout,
                &prepared.paths.stderr,
                &envs,
            )
        }?;
        self.store
            .update_run_state(&prepared.record.id, ProcessState::Running)?;
        let run_id = prepared.record.id.as_str().to_string();
        let mut record = prepared.record;
        record.state = ProcessState::Running;
        self.active_runs.insert(
            run_id.clone(),
            ManagedRun {
                record,
                child,
                kind: RunKind::Service,
                task_name: None,
                kill_after,
                started_at: Instant::now(),
                stop_started_at: None,
                stop_reason: None,
            },
        );
        let service = self.service_mut(name)?;
        service.active_run_id = Some(run_id);
        service.pending_restart = None;
        service.last_error = None;
        service.next_start_at = None;
        if let Some(health) = &mut service.health {
            health.next_check_at = Some(Instant::now());
        }
        Ok(())
    }

    fn spawn_health_check(&mut self, service_name: &str) -> ArmatureResult<()> {
        let config = self.service(service_name)?.config.clone();
        let health_config = config.health.as_ref().ok_or_else(|| {
            ArmatureError::invalid_state(format!(
                "service {service_name:?} does not have a health check configured"
            ))
        })?;
        let timeout = optional_duration(health_config.timeout.as_deref())?;
        let prepared = self.store.create_run(
            config.name.clone(),
            health_config.check.clone(),
            RunOrigin::HealthCheck,
            Some(self.config.version.clone()),
            None,
        )?;
        write_run_meta(&prepared.paths.meta, &prepared.record, "health_check")?;
        let child = spawn_shell_command(
            &health_config.check,
            self.workspace.root(),
            &prepared.paths.stdout,
            &prepared.paths.stderr,
            &run_envs(
                "health_check",
                &self.workspace,
                &self.runtime_paths,
                &prepared.record,
                self.config.version.as_str(),
                None,
                &prepared.paths.event,
            ),
        )?;
        self.store
            .update_run_state(&prepared.record.id, ProcessState::Running)?;
        let run_id = prepared.record.id.as_str().to_string();
        let mut record = prepared.record;
        record.state = ProcessState::Running;
        self.active_runs.insert(
            run_id.clone(),
            ManagedRun {
                record,
                child,
                kind: RunKind::HealthCheck,
                task_name: Some(config.name.clone()),
                kill_after: timeout,
                started_at: Instant::now(),
                stop_started_at: None,
                stop_reason: None,
            },
        );
        if let Some(health) = &mut self.service_mut(service_name)?.health {
            health.state = HealthState::Checking;
            health.active_run_id = Some(run_id);
            health.last_error = None;
        }
        Ok(())
    }

    fn cancel_run(&mut self, run_id: &str, reason: StopReason) -> ArmatureResult<()> {
        let managed = self
            .active_runs
            .get_mut(run_id)
            .ok_or_else(|| ArmatureError::not_found(format!("run {run_id} was not found")))?;
        if managed.stop_reason.is_none() {
            managed.stop_reason = Some(reason);
            managed.stop_started_at = Some(Instant::now());
            managed.record.state = ProcessState::Stopping;
            self.store
                .update_run_state(&managed.record.id, ProcessState::Stopping)?;
            signal_process_group(managed.child.id(), libc::SIGTERM)?;
        }
        Ok(())
    }

    fn stop_all_runs(&mut self, reason: StopReason) -> ArmatureResult<()> {
        let run_ids = self.active_runs.keys().cloned().collect::<Vec<_>>();
        for run_id in run_ids {
            self.cancel_run(&run_id, reason)?;
        }
        Ok(())
    }

    fn poll_runs(&mut self) -> ArmatureResult<()> {
        let run_ids = self.active_runs.keys().cloned().collect::<Vec<_>>();
        for run_id in run_ids {
            let mut finished = None;
            let mut should_kill = None;
            if let Some(managed) = self.active_runs.get_mut(&run_id) {
                if managed.stop_reason.is_none() {
                    if let Some(kill_after) = managed.kill_after {
                        if managed.started_at.elapsed() >= kill_after {
                            managed.stop_reason = Some(StopReason::TimedOut);
                            managed.stop_started_at = Some(Instant::now());
                            managed.record.state = ProcessState::Stopping;
                            self.store
                                .update_run_state(&managed.record.id, ProcessState::Stopping)?;
                            signal_process_group(managed.child.id(), libc::SIGTERM)?;
                        }
                    }
                } else if managed
                    .stop_started_at
                    .map(|started| started.elapsed() >= self.options.termination_grace)
                    .unwrap_or(false)
                {
                    should_kill = Some(managed.child.id());
                }

                if let Some(status) = managed.child.try_wait()? {
                    finished = Some(status);
                }
            }

            if let Some(pid) = should_kill {
                signal_process_group(pid, libc::SIGKILL)?;
            }

            if let Some(status) = finished {
                let managed = self
                    .active_runs
                    .remove(&run_id)
                    .ok_or_else(|| ArmatureError::internal("run disappeared during poll"))?;
                self.finish_run(managed, status)?;
            }
        }
        Ok(())
    }

    fn finish_run(&mut self, managed: ManagedRun, status: ExitStatus) -> ArmatureResult<()> {
        let success = status.success();
        let final_state = if success {
            ProcessState::Exited
        } else {
            ProcessState::Failed
        };
        let signal = status.signal();
        self.store.finish_run(
            &managed.record.id,
            final_state,
            status.code(),
            signal,
            signal.is_some(),
        )?;
        refresh_run_meta(
            &self.store,
            &managed.record.id,
            run_kind_meta_name(&managed.kind),
        )?;

        match managed.kind {
            RunKind::Service => {
                let active_health_run_id =
                    if let Some(service) = self.services.get_mut(&managed.record.name) {
                        service.active_run_id = None;
                        if let Some(reason) = managed.stop_reason {
                            match reason {
                                StopReason::ServiceUpdate | StopReason::Shutdown => {}
                                StopReason::Cancelled => {
                                    service.last_error = Some("cancelled".to_string());
                                }
                                StopReason::TimedOut => {
                                    service.last_error = Some("timed out".to_string());
                                    schedule_restart(
                                        service,
                                        &managed.record,
                                        &status,
                                        Instant::now(),
                                    )?;
                                }
                                StopReason::AdmissionRestart => {}
                            }
                        } else {
                            schedule_restart(service, &managed.record, &status, Instant::now())?;
                        }
                        service
                            .health
                            .as_ref()
                            .and_then(|health| health.active_run_id.clone())
                    } else {
                        None
                    };
                if let Some(run_id) = active_health_run_id {
                    self.cancel_run(&run_id, StopReason::ServiceUpdate)?;
                }
            }
            RunKind::Task => {
                if let Some(task_name) = &managed.task_name {
                    if let Some(task) = self.tasks.get_mut(task_name) {
                        task.active_run_ids
                            .retain(|active_id| active_id != managed.record.id.as_str());
                        self.maybe_start_next_pending(task_name)?;
                    }
                }
            }
            RunKind::HealthCheck => {
                if let Some(service_name) = &managed.task_name {
                    let every = self
                        .services
                        .get(service_name)
                        .and_then(|service| service.config.health.as_ref())
                        .map(|health| parse_duration(&health.every))
                        .transpose()?;
                    if let Some(service) = self.services.get_mut(service_name) {
                        if let Some(health) = &mut service.health {
                            health.active_run_id = None;
                            health.last_run_id = Some(managed.record.id.as_str().to_string());
                            if success {
                                health.state = HealthState::Healthy;
                                health.last_error = None;
                            } else {
                                match managed.stop_reason {
                                    Some(StopReason::TimedOut) => {
                                        health.state = HealthState::Unhealthy;
                                        health.last_error =
                                            Some("health check timed out".to_string());
                                        service.last_error = health.last_error.clone();
                                    }
                                    None => {
                                        health.state = HealthState::Unhealthy;
                                        health.last_error = Some("health check failed".to_string());
                                        service.last_error = health.last_error.clone();
                                    }
                                    Some(StopReason::ServiceUpdate) => {
                                        health.state = HealthState::Unknown;
                                        health.last_error =
                                            Some("health check stopped".to_string());
                                    }
                                    Some(StopReason::Shutdown) => {
                                        health.state = HealthState::Unknown;
                                        health.last_error =
                                            Some("health check stopped for shutdown".to_string());
                                    }
                                    Some(StopReason::Cancelled) => {
                                        health.state = HealthState::Unknown;
                                        health.last_error =
                                            Some("health check cancelled".to_string());
                                    }
                                    Some(StopReason::AdmissionRestart) => {
                                        health.state = HealthState::Unknown;
                                        health.last_error =
                                            Some("health check stopped".to_string());
                                    }
                                }
                            }
                            health.next_check_at = every.map(|duration| Instant::now() + duration);
                        }
                    }
                }
            }
            RunKind::Adhoc => {}
        }

        Ok(())
    }

    fn record_trigger(
        &self,
        task_name: &str,
        event: &EventRecord,
        admission: AdmissionPolicy,
        outcome: TriggerOutcome,
        run_id: Option<RunId>,
        detail: Option<String>,
    ) -> ArmatureResult<()> {
        self.store.record_trigger(&TriggerRecord {
            id: TriggerId::new(),
            task_name: task_name.to_string(),
            event_id: Some(event.id.clone()),
            event_type: event.event_type.clone(),
            routing: event.routing.clone(),
            admission,
            outcome,
            run_id,
            detail,
        })
    }

    fn service(&self, name: &str) -> ArmatureResult<&ManagedService> {
        self.services
            .get(name)
            .ok_or_else(|| ArmatureError::not_found(format!("service {name:?} was not found")))
    }

    fn service_mut(&mut self, name: &str) -> ArmatureResult<&mut ManagedService> {
        self.services
            .get_mut(name)
            .ok_or_else(|| ArmatureError::not_found(format!("service {name:?} was not found")))
    }

    fn task_mut(&mut self, name: &str) -> ArmatureResult<&mut ManagedTask> {
        self.tasks
            .get_mut(name)
            .ok_or_else(|| ArmatureError::not_found(format!("task {name:?} was not found")))
    }
}

struct RouteResult {
    started_run_id: Option<RunId>,
}

fn request_allowed_during_shutdown(request: &DaemonRequest) -> bool {
    matches!(
        request,
        DaemonRequest::Inspect
            | DaemonRequest::Events
            | DaemonRequest::Triggers
            | DaemonRequest::Runs
            | DaemonRequest::LockRenew { .. }
            | DaemonRequest::LockRelease { .. }
            | DaemonRequest::LockForceRelease { .. }
            | DaemonRequest::LockStatus
            | DaemonRequest::Shutdown
    )
}

fn recover_unfinished_runs(store: &SqliteStore) -> ArmatureResult<()> {
    for run in store.list_unfinished_runs()? {
        if let Some(stderr_path) = &run.stderr_path {
            append_recovery_note(Path::new(stderr_path), &run)?;
        }
        store.finish_run(&run.id, ProcessState::Failed, None, None, false)?;
        refresh_run_meta(store, &run.id, run_origin_meta_name(&run.origin))?;
    }
    Ok(())
}

fn append_recovery_note(path: &Path, run: &RunRecord) -> ArmatureResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(
        file,
        "[armature] {} recovery: run {} marked failed during daemon startup; previous state was {:?}; no exit code or signal was observed",
        Utc::now().to_rfc3339(),
        run.id.as_str(),
        run.state
    )?;
    Ok(())
}

fn refresh_run_meta(store: &SqliteStore, run_id: &RunId, kind: &str) -> ArmatureResult<()> {
    if let Some(run) = store.get_run(run_id)? {
        if let Some(run_directory) = &run.run_directory {
            write_run_meta(&Path::new(run_directory).join("meta.json"), &run, kind)?;
        }
    }
    Ok(())
}

fn run_kind_meta_name(kind: &RunKind) -> &'static str {
    match kind {
        RunKind::Task => "task",
        RunKind::Service => "service",
        RunKind::HealthCheck => "health_check",
        RunKind::Adhoc => "adhoc",
    }
}

fn run_origin_meta_name(origin: &RunOrigin) -> &'static str {
    match origin {
        RunOrigin::Task => "task",
        RunOrigin::Service => "service",
        RunOrigin::HealthCheck => "health_check",
        RunOrigin::Restart => "service",
        RunOrigin::Adhoc => "adhoc",
    }
}

fn bind_listener(path: impl AsRef<Path>) -> ArmatureResult<UnixListener> {
    UnixListener::bind(path)
        .map_err(|error| ArmatureError::internal(format!("failed to bind daemon socket: {error}")))
}

fn wait_for_startup(
    startup_rx: &mpsc::Receiver<ArmatureResult<()>>,
    join_handle: &JoinHandle<ArmatureResult<()>>,
    path: &Path,
    timeout: Duration,
) -> ArmatureResult<()> {
    match startup_rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(ArmatureError::unavailable(format!(
            "daemon socket did not appear at {} within {:?}",
            path.display(),
            timeout
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            if join_handle.is_finished() {
                Err(ArmatureError::internal(
                    "daemon thread exited during startup",
                ))
            } else {
                Err(ArmatureError::internal(
                    "daemon startup channel closed before readiness was reported",
                ))
            }
        }
    }
}

fn acquire_lock(path: &Path, pid: u32) -> ArmatureResult<()> {
    use std::fs::OpenOptions;
    loop {
        match OpenOptions::new().create_new(true).write(true).open(path) {
            Ok(file) => {
                drop(file);
                fs::write(path, format!("{pid}\n"))?;
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if lock_holder_is_alive(path) {
                    return Err(ArmatureError::conflict(format!(
                        "workspace daemon lock already exists at {}",
                        path.display()
                    )));
                }
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(ArmatureError::internal(error.to_string())),
        }
    }
}

fn lock_holder_is_alive(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return true;
    };
    let Ok(pid) = contents.trim().parse::<u32>() else {
        return true;
    };
    pid_is_alive(pid)
}

fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn lock_owner_id(owner_pid: u32) -> String {
    std::env::var("ARMATURE_RUN_ID").unwrap_or_else(|_| format!("pid:{owner_pid}"))
}

fn read_json_line<T: DeserializeOwned>(stream: &mut UnixStream) -> ArmatureResult<T> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(line.trim_end())
        .map_err(|error| ArmatureError::invalid_input(error.to_string()))
}

fn write_json_line<T: serde::Serialize>(stream: &mut UnixStream, value: &T) -> ArmatureResult<()> {
    let encoded =
        serde_json::to_string(value).map_err(|error| ArmatureError::internal(error.to_string()))?;
    stream.write_all(encoded.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

fn run_envs(
    kind: &str,
    workspace: &Workspace,
    runtime_paths: &WorkspaceRuntimePaths,
    record: &RunRecord,
    config_version: &str,
    event: Option<&EventRecord>,
    event_path: &Path,
) -> Vec<(String, String)> {
    let mut envs = vec![
        ("ARMATURE_KIND".to_string(), kind.to_string()),
        ("ARMATURE_NAME".to_string(), record.name.clone()),
        (
            "ARMATURE_WORKSPACE".to_string(),
            runtime_paths.workspace_root().display().to_string(),
        ),
        (
            "ARMATURE_WORKSPACE_ROOT".to_string(),
            runtime_paths.workspace_root().display().to_string(),
        ),
        (
            "ARMATURE_CONFIG_DIR".to_string(),
            workspace.config_dir().display().to_string(),
        ),
        (
            "ARMATURE_STATE_DIR".to_string(),
            runtime_paths.state_root().display().to_string(),
        ),
        (
            "ARMATURE_RUN_ID".to_string(),
            record.id.as_str().to_string(),
        ),
        (
            "ARMATURE_CONFIG_VERSION".to_string(),
            config_version.to_string(),
        ),
    ];
    if let Some(run_directory) = &record.run_directory {
        envs.push(("ARMATURE_RUN_DIR".to_string(), run_directory.clone()));
    }
    if let Some(stdout_path) = &record.stdout_path {
        envs.push(("ARMATURE_STDOUT_LOG".to_string(), stdout_path.clone()));
    }
    if let Some(stderr_path) = &record.stderr_path {
        envs.push(("ARMATURE_STDERR_LOG".to_string(), stderr_path.clone()));
    }
    if let Some(restart_of) = &record.restart_of {
        envs.push((
            "ARMATURE_RESTART_OF".to_string(),
            restart_of.as_str().to_string(),
        ));
    }
    if let Some(attempt) = record.attempt {
        envs.push(("ARMATURE_RESTART_ATTEMPT".to_string(), attempt.to_string()));
    }
    if let Some(event) = event {
        envs.push((
            "ARMATURE_EVENT_ID".to_string(),
            event.id.as_str().to_string(),
        ));
        envs.push(("ARMATURE_EVENT_TYPE".to_string(), event.event_type.clone()));
        envs.push((
            "ARMATURE_EVENT_JSON".to_string(),
            serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string()),
        ));
        let payload_json =
            serde_json::to_string(&event.payload).unwrap_or_else(|_| "null".to_string());
        envs.push((
            "ARMATURE_EVENT_PAYLOAD_JSON".to_string(),
            payload_json.clone(),
        ));
        envs.push(("ARMATURE_PAYLOAD_JSON".to_string(), payload_json));
        envs.push((
            "ARMATURE_EVENT_PATH".to_string(),
            event_path.display().to_string(),
        ));
        if let Some(correlation_id) = &event.correlation_id {
            envs.push((
                "ARMATURE_CORRELATION_ID".to_string(),
                correlation_id.clone(),
            ));
        }
    }
    envs
}

fn write_run_meta(path: &Path, record: &RunRecord, kind: &str) -> ArmatureResult<()> {
    let value = serde_json::json!({
        "run_id": record.id.as_str(),
        "name": record.name,
        "command": record.command,
        "kind": kind,
        "origin": format!("{:?}", record.origin).to_lowercase(),
        "state": format!("{:?}", record.state).to_lowercase(),
        "start_time": record.start_time,
        "end_time": record.end_time,
        "exit_code": record.exit_code,
        "signal": record.signal,
        "killed": record.killed,
        "config_version": record.config_version,
        "event_id": record.event_id.as_ref().map(|event_id| event_id.as_str()),
        "restartOf": record.restart_of.as_ref().map(|run_id| run_id.as_str()),
        "attempt": record.attempt,
        "run_directory": record.run_directory,
        "stdout_path": record.stdout_path,
        "stderr_path": record.stderr_path,
    });
    fs::write(
        path,
        serde_json::to_vec_pretty(&value)
            .map_err(|error| ArmatureError::internal(error.to_string()))?,
    )?;
    Ok(())
}

fn write_event_artifact(path: &Path, event: &EventRecord) -> ArmatureResult<()> {
    fs::write(
        path,
        serde_json::to_vec_pretty(event)
            .map_err(|error| ArmatureError::internal(error.to_string()))?,
    )?;
    Ok(())
}

fn validate_dynamic_name(kind: &str, name: &str) -> ArmatureResult<()> {
    if name.trim().is_empty() {
        return Err(ArmatureError::invalid_input(format!(
            "{kind} name is required"
        )));
    }
    if name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
    {
        return Ok(());
    }
    Err(ArmatureError::invalid_input(format!(
        "{kind} name {name:?} may only contain ASCII letters, digits, '_', '-', '.', or ':'"
    )))
}

fn validate_nonempty_dynamic(kind: &str, value: &str) -> ArmatureResult<()> {
    if value.trim().is_empty() {
        return Err(ArmatureError::invalid_input(format!("{kind} is required")));
    }
    Ok(())
}

fn optional_duration(value: Option<&str>) -> ArmatureResult<Option<Duration>> {
    value.map(parse_duration).transpose()
}

fn resolve_run_cwd(workspace_root: &Path, cwd: Option<PathBuf>) -> ArmatureResult<PathBuf> {
    let resolved = match cwd {
        Some(path) if path.is_absolute() => path,
        Some(path) => workspace_root.join(path),
        None => workspace_root.to_path_buf(),
    };
    if !resolved.is_dir() {
        return Err(ArmatureError::invalid_input(format!(
            "cwd {} is not a directory",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn command_display(command: &[String]) -> String {
    command
        .iter()
        .map(|part| {
            if part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':'))
            {
                part.clone()
            } else {
                format!("'{}'", part.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn schedule_restart(
    service: &mut ManagedService,
    run: &RunRecord,
    status: &ExitStatus,
    now: Instant,
) -> ArmatureResult<()> {
    let should_restart = match service.config.supervision.restart {
        RestartMode::Never => false,
        RestartMode::OnFailure => !status.success(),
        RestartMode::Always => true,
    };

    if !should_restart || service.stop_override || !service.config.enabled {
        return Ok(());
    }

    let window = optional_duration(service.config.supervision.within.as_deref())?;
    if let Some(window) = window {
        while service
            .restart_attempts
            .front()
            .map(|instant| now.duration_since(*instant) > window)
            .unwrap_or(false)
        {
            service.restart_attempts.pop_front();
        }
    } else {
        service.restart_attempts.clear();
    }

    let max_restarts = service.config.supervision.max_restarts.unwrap_or(u32::MAX) as usize;
    if service.restart_attempts.len() >= max_restarts {
        service.exhausted = true;
        service.last_error = Some("restart budget exhausted".to_string());
        service.pending_restart = None;
        return Ok(());
    }

    service.restart_attempts.push_back(now);
    service.pending_restart = Some(RestartLineage {
        restart_of: run.restart_of.clone().unwrap_or_else(|| run.id.clone()),
        attempt: run.attempt.unwrap_or(0).saturating_add(1),
    });
    let delay = if let Some(start_delay) = service.config.supervision.start_delay.as_deref() {
        parse_duration(start_delay)?
    } else {
        Duration::from_millis(0)
    };
    let next_delay = match service.config.supervision.backoff {
        Some(armature_core::BackoffMode::Exponential) => {
            let exponent = service.restart_attempts.len().saturating_sub(1) as u32;
            delay.saturating_mul(2_u32.saturating_pow(exponent))
        }
        _ => delay,
    };
    service.next_start_at = Some(now + next_delay);
    Ok(())
}

fn supervision_state(service: &ManagedService) -> &'static str {
    if service.stop_override {
        "stopped"
    } else if service.exhausted {
        "failed"
    } else if service.next_start_at.is_some() {
        "backoff"
    } else if service.active_run_id.is_some() {
        "running"
    } else {
        "idle"
    }
}

fn service_state(service: &ManagedService) -> ProcessState {
    if service.active_run_id.is_some() {
        ProcessState::Running
    } else if service.stop_override {
        ProcessState::Stopping
    } else if service.exhausted {
        ProcessState::Failed
    } else {
        ProcessState::Idle
    }
}

fn runtime_health_status(health: &ManagedHealth) -> RuntimeHealthStatus {
    RuntimeHealthStatus {
        state: health_state_label(health.state).to_string(),
        active_run_id: health
            .active_run_id
            .as_deref()
            .and_then(|value| RunId::parse(value).ok()),
        last_run_id: health
            .last_run_id
            .as_deref()
            .and_then(|value| RunId::parse(value).ok()),
        last_error: health.last_error.clone(),
    }
}

fn health_state_label(state: HealthState) -> &'static str {
    match state {
        HealthState::Unknown => "unknown",
        HealthState::Checking => "checking",
        HealthState::Healthy => "healthy",
        HealthState::Unhealthy => "unhealthy",
    }
}

fn admission_label(policy: AdmissionPolicy) -> &'static str {
    match policy {
        AdmissionPolicy::Allow => "allow",
        AdmissionPolicy::Reject => "reject",
        AdmissionPolicy::Restart => "restart",
        AdmissionPolicy::QueueOne => "queue_one",
        AdmissionPolicy::QueueAll => "queue_all",
    }
}

fn parse_schedule(expression: &str) -> ArmatureResult<Schedule> {
    let normalized = if expression.split_whitespace().count() == 5 {
        format!("0 {expression}")
    } else {
        expression.to_string()
    };
    Schedule::from_str(&normalized).map_err(|error| {
        ArmatureError::invalid_input(format!("invalid schedule {expression:?}: {error}"))
    })
}

fn next_schedule_time(schedule: &Schedule) -> Option<DateTime<Utc>> {
    schedule.upcoming(Utc).next()
}

fn scan_watch_patterns(
    workspace_root: &Path,
    patterns: &[String],
    watch: &mut WatchState,
) -> ArmatureResult<Vec<PathBuf>> {
    let mut current = HashMap::new();
    for pattern in patterns {
        let absolute_pattern = workspace_root.join(pattern);
        let Some(pattern_str) = absolute_pattern.to_str() else {
            return Err(ArmatureError::invalid_input(format!(
                "watch pattern {pattern:?} must be valid UTF-8"
            )));
        };
        let entries = glob::glob(pattern_str).map_err(|error| {
            ArmatureError::invalid_input(format!("invalid watch pattern: {error}"))
        })?;
        for entry in entries {
            let path = entry.map_err(|error| ArmatureError::internal(error.to_string()))?;
            let metadata = match fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => metadata,
                Ok(_) => continue,
                Err(_) => continue,
            };
            current.insert(
                path,
                FileFingerprint {
                    modified_at: metadata.modified().ok(),
                    len: metadata.len(),
                },
            );
        }
    }

    let mut changed = Vec::new();
    let now = Instant::now();
    for (path, fingerprint) in &current {
        if watch.known_files.get(path) != Some(fingerprint) {
            watch.pending_paths.insert(path.clone(), now);
            changed.push(path.clone());
        }
    }
    for path in watch.known_files.keys() {
        if !current.contains_key(path) {
            watch.pending_paths.insert(path.clone(), now);
            changed.push(path.clone());
        }
    }
    watch.known_files = current;
    Ok(changed)
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};

    use armature_core::{
        discover_workspace, AdmissionPolicy, ProcessState, RunOrigin, TriggerOutcome,
    };
    use serde_json::json;
    use tempfile::TempDir;

    use super::{acquire_lock, DaemonOptions, DaemonServer};

    #[test]
    fn acquire_lock_replaces_stale_dead_pid_lock() {
        let lock_dir = TempDir::new().unwrap();
        let lock_path = lock_dir.path().join("workspace.lock");
        fs::write(&lock_path, "999999999\n").unwrap();

        acquire_lock(&lock_path, std::process::id()).unwrap();

        assert_eq!(
            fs::read_to_string(lock_path).unwrap(),
            format!("{}\n", std::process::id())
        );
    }

    #[test]
    fn acquire_lock_rejects_live_pid_lock() {
        let lock_dir = TempDir::new().unwrap();
        let lock_path = lock_dir.path().join("workspace.lock");
        fs::write(&lock_path, format!("{}\n", std::process::id())).unwrap();

        let error = acquire_lock(&lock_path, std::process::id()).unwrap_err();

        assert_eq!(error.kind.as_ref(), "conflict");
        assert_eq!(
            fs::read_to_string(lock_path).unwrap(),
            format!("{}\n", std::process::id())
        );
    }

    #[test]
    fn manual_locks_round_trip_through_daemon_transport() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[task]]
name = "noop"
run = "true"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(100),
            },
        )
        .unwrap();
        let client = handle.client();

        assert!(client.locks().unwrap().is_empty());

        let lock = client
            .acquire_lock(
                "branch:main",
                Duration::from_secs(10),
                Some("runtime test".to_string()),
            )
            .unwrap();
        assert_eq!(lock.name, "branch:main");
        assert_eq!(lock.owner_pid, std::process::id());
        assert_eq!(lock.owner_id, format!("pid:{}", std::process::id()));
        assert_eq!(lock.reason.as_deref(), Some("runtime test"));
        assert!(lock.token.starts_with("lock_"));
        assert!(lock.manual);
        assert!(lock.expires_at_ms.is_some());

        let conflict = client
            .acquire_lock("branch:main", Duration::from_secs(10), None)
            .unwrap_err();
        assert_eq!(conflict.kind.as_ref(), "conflict");

        let stale_token = lock.token.clone();
        let renewed = client
            .renew_lock("branch:main", &lock.token, Duration::from_secs(20))
            .unwrap();
        assert_eq!(renewed.token, lock.token);
        assert!(renewed.renewed_at_ms.is_some());

        let locks = client.locks().unwrap();
        assert_eq!(locks.len(), 1);
        assert_eq!(locks[0].name, "branch:main");

        let mismatch = client
            .release_lock("branch:main", "wrong-token")
            .unwrap_err();
        assert_eq!(mismatch.kind.as_ref(), "conflict");

        client.release_lock("branch:main", stale_token).unwrap();
        assert!(client.locks().unwrap().is_empty());

        let expired = client
            .acquire_lock("branch:main", Duration::from_millis(50), None)
            .unwrap();
        thread::sleep(Duration::from_millis(80));
        let newer = client
            .acquire_lock("branch:main", Duration::from_secs(10), None)
            .unwrap();
        assert_ne!(expired.token, newer.token);
        let stale_release = client
            .release_lock("branch:main", expired.token)
            .unwrap_err();
        assert_eq!(stale_release.kind.as_ref(), "conflict");
        assert_eq!(client.locks().unwrap().len(), 1);
        client.release_lock("branch:main", newer.token).unwrap();

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn enabled_service_reconciles_and_survives_invalid_reload() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[service]]
name = "ticker"
run = "printf ready > service.txt && sleep 2"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(100),
            },
        )
        .unwrap();
        let client = handle.client();

        wait_for(
            || fixture.root().join("service.txt").is_file(),
            Duration::from_secs(2),
        );

        fs::write(
            fixture.root().join(".armature/armature.toml"),
            "[[service]]\nname = \"ticker\"\nrun =\n",
        )
        .unwrap();
        assert!(client.reload_config().is_err());

        let status = client.inspect().unwrap();
        assert_eq!(status.services[0].name, "ticker");
        assert_eq!(status.services[0].state, ProcessState::Running);

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn service_restarts_on_failure_and_tasks_do_not_restart_by_default() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[task]]
name = "once"
run = "echo task >> task.log"

[[service]]
name = "flaky"
run = "count=$(cat counter.txt 2>/dev/null || echo 0); count=$((count + 1)); echo $count > counter.txt; if [ \"$count\" -lt 2 ]; then exit 1; fi; sleep 1"

[service.supervision]
restart = "on_failure"
start_delay = "20ms"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(100),
            },
        )
        .unwrap();
        let client = handle.client();

        wait_for(
            || read_file(fixture.root().join("counter.txt")).trim() == "2",
            Duration::from_secs(3),
        );
        let runs = client.runs().unwrap();
        let original = runs
            .iter()
            .find(|run| run.name == "flaky" && run.origin == RunOrigin::Service)
            .unwrap();
        let restart = runs
            .iter()
            .find(|run| run.name == "flaky" && run.origin == RunOrigin::Restart)
            .unwrap();
        assert_eq!(restart.restart_of.as_ref(), Some(&original.id));
        assert_eq!(restart.attempt, Some(1));
        let meta_path = Path::new(restart.run_directory.as_ref().unwrap()).join("meta.json");
        let meta: serde_json::Value =
            serde_json::from_slice(&fs::read(meta_path).unwrap()).unwrap();
        assert_eq!(
            meta["restartOf"],
            restart.restart_of.as_ref().unwrap().as_str()
        );
        assert_eq!(meta["attempt"], 1);

        let run_id = client.start_task("once").unwrap();
        wait_for(
            || {
                client
                    .runs()
                    .unwrap()
                    .iter()
                    .find(|run| run.id == run_id)
                    .map(|run| matches!(run.state, ProcessState::Exited | ProcessState::Failed))
                    .unwrap_or(false)
            },
            Duration::from_secs(2),
        );

        thread::sleep(Duration::from_millis(150));
        let task_log = read_file(fixture.root().join("task.log"));
        assert_eq!(task_log.lines().count(), 1);

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn triggered_tasks_do_not_restart_by_default() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[task]]
name = "once"
on = "fail-once"
run = "count=$(cat task-count.txt 2>/dev/null || echo 0); count=$((count + 1)); echo $count > task-count.txt; exit 7"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(100),
            },
        )
        .unwrap();
        let client = handle.client();

        client
            .emit_event("fail-once", serde_json::json!({}), None)
            .unwrap();
        wait_for(
            || {
                client
                    .triggers()
                    .unwrap()
                    .iter()
                    .any(|trigger| trigger.task_name == "once" && trigger.run_id.is_some())
            },
            Duration::from_secs(2),
        );
        let trigger = client
            .triggers()
            .unwrap()
            .into_iter()
            .find(|trigger| trigger.task_name == "once")
            .unwrap();
        let run_id = trigger.run_id.unwrap();
        wait_for(
            || {
                client
                    .runs()
                    .unwrap()
                    .iter()
                    .find(|run| run.id == run_id)
                    .map(|run| run.state == ProcessState::Failed)
                    .unwrap_or(false)
            },
            Duration::from_secs(2),
        );

        thread::sleep(Duration::from_millis(150));
        assert_eq!(read_file(fixture.root().join("task-count.txt")).trim(), "1");

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn service_health_checks_are_recorded_and_exposed() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[service]]
name = "worker"
run = "sleep 5"

[service.health]
check = "count=$(cat health-count.txt 2>/dev/null || echo 0); count=$((count + 1)); echo $count > health-count.txt; if [ \"$count\" -lt 2 ]; then exit 0; else exit 7; fi"
every = "500ms"
timeout = "500ms"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(100),
            },
        )
        .unwrap();
        let client = handle.client();

        wait_for(
            || read_file(fixture.root().join("health-count.txt")).trim() == "2",
            Duration::from_secs(2),
        );
        wait_for(
            || {
                client
                    .inspect()
                    .unwrap()
                    .services
                    .iter()
                    .find(|service| service.name == "worker")
                    .and_then(|service| service.health.as_ref())
                    .map(|health| health.state.as_str() == "unhealthy")
                    .unwrap_or(false)
            },
            Duration::from_secs(2),
        );

        let status = client.inspect().unwrap();
        let health = status.services[0].health.as_ref().unwrap();
        assert_eq!(health.state, "unhealthy");
        assert_eq!(health.last_error.as_deref(), Some("health check failed"));
        assert!(health.last_run_id.is_some());

        let runs = client.runs().unwrap();
        assert!(runs.iter().any(|run| {
            run.name == "worker"
                && run.origin == RunOrigin::HealthCheck
                && run.state == ProcessState::Failed
        }));

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn service_health_check_timeout_is_unhealthy() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[service]]
name = "worker"
run = "sleep 5"

[service.health]
check = "sleep 5"
every = "500ms"
timeout = "50ms"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(50),
            },
        )
        .unwrap();
        let client = handle.client();

        wait_for(
            || {
                client
                    .inspect()
                    .unwrap()
                    .services
                    .iter()
                    .find(|service| service.name == "worker")
                    .and_then(|service| service.health.as_ref())
                    .map(|health| health.last_error.as_deref() == Some("health check timed out"))
                    .unwrap_or(false)
            },
            Duration::from_secs(2),
        );

        let status = client.inspect().unwrap();
        let health = status.services[0].health.as_ref().unwrap();
        assert_eq!(health.state, "unhealthy");
        assert_eq!(health.last_error.as_deref(), Some("health check timed out"));

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn cancel_run_terminates_process_group_after_timeout() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[task]]
name = "stubborn"
run = "trap '' TERM; sleep 10"

[task.resources]
kill_after = "5s"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(100),
            },
        )
        .unwrap();
        let client = handle.client();
        let run_id = client.start_task("stubborn").unwrap();

        client.cancel_run(run_id.as_str()).unwrap();
        wait_for(
            || {
                client
                    .runs()
                    .unwrap()
                    .iter()
                    .find(|run| run.id == run_id)
                    .map(|run| run.state == ProcessState::Failed)
                    .unwrap_or(false)
            },
            Duration::from_secs(3),
        );

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn rejects_new_work_after_shutdown_while_service_drains() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[task]]
name = "later"
run = "printf later > later.txt"

[[service]]
name = "slow-stop"
run = "trap 'sleep 1; exit 0' TERM; printf ready > service.txt; while true; do sleep 1; done"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_secs(2),
            },
        )
        .unwrap();
        let client = handle.client();

        wait_for(
            || fixture.root().join("service.txt").is_file(),
            Duration::from_secs(2),
        );

        client.shutdown().unwrap();
        wait_for(
            || !client.inspect().unwrap().active_runs.is_empty(),
            Duration::from_secs(1),
        );

        let error = client.start_task("later").unwrap_err();
        assert_eq!(error.kind.as_ref(), "invalid_state");
        assert_eq!(error.message.as_ref(), "daemon is shutting down");
        assert!(!fixture.root().join("later.txt").exists());

        handle.join().unwrap();
    }

    #[test]
    fn emit_event_routes_tasks_and_writes_event_artifacts() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[task]]
name = "react"
on = "tool.run.completed"
run = "printf '%s' \"$ARMATURE_EVENT_TYPE\" > event-type.txt; printf '%s' \"$ARMATURE_EVENT_ID\" > event-id.txt; printf '%s' \"$ARMATURE_EVENT_PATH\" > event-path.txt; printf '%s' \"$ARMATURE_EVENT_JSON\" > event-json.txt; printf '%s' \"$ARMATURE_EVENT_PAYLOAD_JSON\" > event-payload-json.txt; printf '%s' \"$ARMATURE_PAYLOAD_JSON\" > payload-json.txt; printf '%s' \"$ARMATURE_WORKSPACE\" > workspace.txt; printf '%s' \"$ARMATURE_WORKSPACE_ROOT\" > workspace-root.txt; printf '%s' \"$ARMATURE_CONFIG_DIR\" > config-dir.txt; printf '%s' \"$ARMATURE_STATE_DIR\" > state-dir.txt; printf '%s' \"$ARMATURE_RUN_DIR\" > run-dir.txt; printf '%s' \"$ARMATURE_CONFIG_VERSION\" > config-version.txt; cp \"$ARMATURE_EVENT_PATH\" event-copy.json"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(100),
            },
        )
        .unwrap();
        let client = handle.client();

        client
            .emit_event(
                "tool.run.completed",
                json!({ "runId": "abc123" }),
                Some("tests".to_string()),
            )
            .unwrap();

        wait_for(
            || fixture.root().join("event-type.txt").is_file(),
            Duration::from_secs(2),
        );

        let events = client.events().unwrap();
        let triggers = client.triggers().unwrap();
        assert_eq!(events[0].event_type, "tool.run.completed");
        assert_eq!(events[0].source.as_deref(), Some("tests"));
        assert_eq!(triggers[0].outcome, TriggerOutcome::Started);
        assert_eq!(
            read_file(fixture.root().join("event-type.txt")).trim(),
            "tool.run.completed"
        );

        let event_path = read_file(fixture.root().join("event-path.txt"));
        assert!(Path::new(event_path.trim()).is_file());

        let status = client.inspect().unwrap();
        let runs = client.runs().unwrap();
        let run = runs.iter().find(|run| run.name == "react").unwrap();
        assert_eq!(
            read_file(fixture.root().join("workspace.txt")).trim(),
            fs::canonicalize(fixture.root())
                .unwrap()
                .display()
                .to_string()
        );
        assert_eq!(
            read_file(fixture.root().join("workspace-root.txt")).trim(),
            read_file(fixture.root().join("workspace.txt")).trim()
        );
        assert_eq!(
            read_file(fixture.root().join("config-dir.txt")).trim(),
            fixture.root().join(".armature").display().to_string()
        );
        assert!(
            Path::new(read_file(fixture.root().join("state-dir.txt")).trim())
                .starts_with(fixture._state_home.path())
        );
        assert_eq!(
            read_file(fixture.root().join("run-dir.txt")).trim(),
            run.run_directory.as_deref().unwrap()
        );
        assert_eq!(
            read_file(fixture.root().join("config-version.txt")).trim(),
            status.config_version
        );
        assert_eq!(
            read_file(fixture.root().join("payload-json.txt")).trim(),
            r#"{"runId":"abc123"}"#
        );
        assert_eq!(
            read_file(fixture.root().join("event-payload-json.txt")).trim(),
            r#"{"runId":"abc123"}"#
        );
        let event_json: serde_json::Value =
            serde_json::from_str(&read_file(fixture.root().join("event-json.txt"))).unwrap();
        assert_eq!(event_json["payload"], json!({ "runId": "abc123" }));

        client.emit_event("unmatched", json!({}), None).unwrap();
        let events = client.events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "unmatched" && event.source.as_deref() == Some("user")
        }));

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn admission_outcomes_remain_inspectable() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[task]]
name = "rejector"
on = "reject"
run = "sleep 0.4"

[task.admission]
when_busy = "reject"

[[task]]
name = "queue-one"
on = "queue"
run = "printf 'q\\n' >> queue.log; sleep 0.2"

[task.admission]
when_busy = "queue_one"

[[task]]
name = "restarter"
on = "restart"
run = "printf 'r\\n' >> restart.log; sleep 0.4"

[task.admission]
when_busy = "restart"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(100),
            },
        )
        .unwrap();
        let client = handle.client();

        client.emit_event("reject", json!({}), None).unwrap();
        wait_for(
            || {
                client
                    .inspect()
                    .unwrap()
                    .tasks
                    .iter()
                    .any(|task| task.name == "rejector" && !task.active_run_ids.is_empty())
            },
            Duration::from_secs(2),
        );
        client.emit_event("reject", json!({}), None).unwrap();

        client.emit_event("queue", json!({ "n": 1 }), None).unwrap();
        wait_for(
            || {
                client
                    .inspect()
                    .unwrap()
                    .tasks
                    .iter()
                    .any(|task| task.name == "queue-one" && !task.active_run_ids.is_empty())
            },
            Duration::from_secs(2),
        );
        client.emit_event("queue", json!({ "n": 2 }), None).unwrap();
        client.emit_event("queue", json!({ "n": 3 }), None).unwrap();

        client
            .emit_event("restart", json!({ "n": 1 }), None)
            .unwrap();
        wait_for(
            || {
                client
                    .inspect()
                    .unwrap()
                    .tasks
                    .iter()
                    .any(|task| task.name == "restarter" && !task.active_run_ids.is_empty())
            },
            Duration::from_secs(2),
        );
        client
            .emit_event("restart", json!({ "n": 2 }), None)
            .unwrap();
        client
            .emit_event("restart", json!({ "n": 3 }), None)
            .unwrap();

        wait_for(
            || read_file(fixture.root().join("queue.log")).lines().count() >= 2,
            Duration::from_secs(3),
        );
        wait_for(
            || {
                read_file(fixture.root().join("restart.log"))
                    .lines()
                    .count()
                    >= 2
            },
            Duration::from_secs(3),
        );

        let triggers = client.triggers().unwrap();
        assert!(triggers.iter().any(|trigger| {
            trigger.task_name == "rejector" && trigger.outcome == TriggerOutcome::Rejected
        }));
        assert!(triggers.iter().any(|trigger| {
            trigger.task_name == "queue-one" && trigger.outcome == TriggerOutcome::Queued
        }));
        assert!(triggers.iter().any(|trigger| {
            trigger.task_name == "queue-one" && trigger.outcome == TriggerOutcome::Coalesced
        }));
        assert!(triggers.iter().any(|trigger| {
            trigger.task_name == "restarter" && trigger.outcome == TriggerOutcome::Superseded
        }));
        assert!(triggers.iter().any(|trigger| {
            trigger.task_name == "restarter"
                && trigger.admission == AdmissionPolicy::Restart
                && trigger.outcome == TriggerOutcome::Started
        }));

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn schedules_and_watches_emit_mechanical_events() {
        let _guard = lock_tests();
        let fixture = Fixture::new(
            r#"
[[task]]
name = "scheduled"
schedule = "*/1 * * * * *"
run = "printf s >> schedule.log"

[[task]]
name = "watcher"
watch = ["watched.txt"]
settle = "50ms"
run = "printf w >> watch.log"
"#,
        );
        let workspace = discover_workspace(fixture.root()).unwrap();
        let handle = DaemonServer::start_with_options(
            workspace,
            DaemonOptions {
                poll_interval: Duration::from_millis(20),
                termination_grace: Duration::from_millis(100),
            },
        )
        .unwrap();
        let client = handle.client();

        fs::write(fixture.root().join("watched.txt"), "hello").unwrap();

        wait_for(
            || !read_file(fixture.root().join("schedule.log")).is_empty(),
            Duration::from_secs(3),
        );
        wait_for(
            || !read_file(fixture.root().join("watch.log")).is_empty(),
            Duration::from_secs(2),
        );

        let events = client.events().unwrap();
        assert!(events
            .iter()
            .any(|event| event.routing == armature_core::EventRouting::Schedule));
        assert!(events
            .iter()
            .any(|event| event.routing == armature_core::EventRouting::Watch));

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    struct Fixture {
        root_dir: TempDir,
        _state_home: TempDir,
    }

    impl Fixture {
        fn new(config: &str) -> Self {
            let root_dir = TempDir::new().unwrap();
            let state_home = TempDir::new().unwrap();
            fs::create_dir_all(root_dir.path().join(".armature")).unwrap();
            fs::write(
                root_dir.path().join(".armature/armature.toml"),
                config.trim(),
            )
            .unwrap();
            std::env::set_var("XDG_STATE_HOME", state_home.path());
            Self {
                root_dir,
                _state_home: state_home,
            }
        }

        fn root(&self) -> &Path {
            self.root_dir.path()
        }
    }

    fn wait_for(predicate: impl Fn() -> bool, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for condition");
    }

    fn read_file(path: impl AsRef<Path>) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    fn test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn lock_tests() -> std::sync::MutexGuard<'static, ()> {
        test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
