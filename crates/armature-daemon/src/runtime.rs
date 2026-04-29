use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::str::FromStr;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use armature_core::{
    load_workspace_config, AdmissionPolicy, ArmatureConfig, ArmatureError, ArmatureResult, EventId,
    EventRecord, EventRouting, ProcessState, RestartMode, RunId, RunOrigin, RunRecord,
    ServiceConfig, TaskConfig, TriggerId, TriggerOutcome, TriggerRecord, Workspace,
    WorkspaceRuntimePaths,
};
use chrono::{DateTime, Local, Utc};
use cron::Schedule;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::duration::parse_duration;
use crate::process::{signal_process_group, spawn_shell_command};
use crate::protocol::{
    DaemonRequest, DaemonResponse, InspectResponse, ResponsePayload, RuntimeServiceStatus,
    RuntimeTaskStatus,
};
use crate::store::SqliteStore;

const LOOP_SLEEP: Duration = Duration::from_millis(25);
const TERMINATION_GRACE: Duration = Duration::from_secs(2);
const DEFAULT_WATCH_SETTLE: Duration = Duration::from_millis(300);

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
        let join_handle = thread::spawn(move || {
            let result = Runtime::new(workspace, runtime_paths, options)
                .and_then(|mut runtime| runtime.run());
            let _ = fs::remove_file(socket_path);
            let _ = fs::remove_file(pid_path);
            let _ = fs::remove_file(lock_path);
            result
        });

        wait_for_socket(&handle_socket_path, Duration::from_secs(1))?;

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
        match self.send(DaemonRequest::StartTask { name: name.into() })? {
            ResponsePayload::StartedRun { run_id } => Ok(run_id),
            _ => Err(ArmatureError::internal("unexpected task start response")),
        }
    }

    pub fn emit_event(
        &self,
        event_type: impl Into<String>,
        payload: Value,
        source: Option<String>,
    ) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::EmitEvent {
            event_type: event_type.into(),
            payload,
            source,
        })
    }

    pub fn cancel_run(&self, run_id: impl Into<String>) -> ArmatureResult<()> {
        self.expect_empty(DaemonRequest::CancelRun {
            run_id: run_id.into(),
        })
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
    stop_override: bool,
    active_run_id: Option<String>,
    restart_attempts: VecDeque<Instant>,
    next_start_at: Option<Instant>,
    exhausted: bool,
    last_error: Option<String>,
}

struct ManagedTask {
    config: TaskConfig,
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
            DaemonRequest::StartTask { name } => {
                let run_id = self.start_task(&name)?;
                Ok(ResponsePayload::StartedRun { run_id })
            }
            DaemonRequest::EmitEvent {
                event_type,
                payload,
                source,
            } => {
                self.emit_user_event(event_type, payload, source)?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::CancelRun { run_id } => {
                self.cancel_run(&run_id, StopReason::Cancelled)?;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::ServiceStart { name } => {
                let service = self.service_mut(&name)?;
                service.stop_override = false;
                service.exhausted = false;
                service.next_start_at = None;
                Ok(ResponsePayload::Empty)
            }
            DaemonRequest::ServiceStop { name } => {
                let service = self.service_mut(&name)?;
                service.stop_override = true;
                if let Some(run_id) = service.active_run_id.clone() {
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
                configured_enabled: service.config.enabled,
                stop_override: service.stop_override,
                state: service_state(service),
                supervision_state: supervision_state(service).to_string(),
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

    fn start_task(&mut self, name: &str) -> ArmatureResult<RunId> {
        if !self.tasks.contains_key(name) {
            return Err(ArmatureError::not_found(format!(
                "task {name:?} was not found"
            )));
        }

        let event = EventRecord {
            id: EventId::new(),
            event_type: "manual.run.requested".to_string(),
            payload: json!({ "task": name }),
            routing: EventRouting::Manual,
            config_version: Some(self.config.version.clone()),
            source: Some("manual".to_string()),
        };
        self.store.record_event(&event)?;

        let route_result = self.route_event(event, RouteTarget::Task(name.to_string()))?;
        route_result.started_run_id.ok_or_else(|| {
            ArmatureError::conflict(format!(
                "task {name:?} did not start immediately because admission policy queued or rejected it"
            ))
        })
    }

    fn emit_user_event(
        &mut self,
        event_type: String,
        payload: Value,
        source: Option<String>,
    ) -> ArmatureResult<()> {
        let event = EventRecord {
            id: EventId::new(),
            event_type,
            payload,
            routing: EventRouting::Event,
            config_version: Some(self.config.version.clone()),
            source,
        };
        self.store.record_event(&event)?;
        let _ = self.route_event(event, RouteTarget::AllMatching)?;
        Ok(())
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
            .filter_map(|(name, service)| {
                service
                    .active_run_id
                    .as_ref()
                    .map(|run_id| (name.clone(), run_id.clone()))
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
            let active_run_id = existing.and_then(|entry| entry.active_run_id);
            next.insert(
                config.name.clone(),
                ManagedService {
                    config: config.clone(),
                    stop_override,
                    active_run_id,
                    restart_attempts: VecDeque::new(),
                    next_start_at: None,
                    exhausted: false,
                    last_error: None,
                },
            );
        }
        self.services = next;
    }

    fn rebuild_tasks(&mut self) {
        let mut next = HashMap::new();
        for config in &self.config.tasks {
            let existing = self.tasks.remove(&config.name);
            next.insert(
                config.name.clone(),
                ManagedTask {
                    config: config.clone(),
                    active_run_ids: existing
                        .as_ref()
                        .map(|entry| entry.active_run_ids.clone())
                        .unwrap_or_default(),
                    pending: existing.map(|entry| entry.pending).unwrap_or_default(),
                },
            );
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
                payload: json!({
                    "task": task_name,
                    "schedule": schedule_literal,
                    "time": Local::now().to_rfc3339(),
                }),
                routing: EventRouting::Schedule,
                config_version: Some(self.config.version.clone()),
                source: Some(format!("schedule:{task_name}")),
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
                payload: json!({
                    "task": task_name,
                    "paths": relative_paths,
                }),
                routing: EventRouting::Watch,
                config_version: Some(self.config.version.clone()),
                source: Some(format!("watch:{task_name}")),
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
        let kill_after = optional_duration(task.resources.kill_after.as_deref())?;
        let prepared = self.store.create_run(
            task.name.clone(),
            RunOrigin::Task,
            Some(self.config.version.clone()),
            event.as_ref().map(|record| record.id.clone()),
        )?;
        if let Some(event) = &event {
            write_event_artifact(&prepared.paths.event, event)?;
        }
        write_run_meta(&prepared.paths.meta, &prepared.record, "task")?;
        let child = spawn_shell_command(
            &task.run,
            self.workspace.root(),
            &prepared.paths.stdout,
            &prepared.paths.stderr,
            &run_envs(
                "task",
                &prepared.record,
                self.config.version.as_str(),
                event.as_ref(),
                &prepared.paths.event,
            ),
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

    fn spawn_service(&mut self, name: &str) -> ArmatureResult<()> {
        let config = self.service(name)?.config.clone();
        let kill_after = optional_duration(config.resources.kill_after.as_deref())?;
        let prepared = self.store.create_run(
            config.name.clone(),
            RunOrigin::Service,
            Some(self.config.version.clone()),
            None,
        )?;
        write_run_meta(&prepared.paths.meta, &prepared.record, "service")?;
        let child = spawn_shell_command(
            &config.run,
            self.workspace.root(),
            &prepared.paths.stdout,
            &prepared.paths.stderr,
            &run_envs(
                "service",
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
        service.last_error = None;
        service.next_start_at = None;
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
        self.store
            .update_run_state(&managed.record.id, final_state)?;

        match managed.kind {
            RunKind::Service => {
                let service = self.service_mut(&managed.record.name)?;
                service.active_run_id = None;
                if let Some(reason) = managed.stop_reason {
                    match reason {
                        StopReason::ServiceUpdate | StopReason::Shutdown => {}
                        StopReason::Cancelled => {
                            service.last_error = Some("cancelled".to_string());
                        }
                        StopReason::TimedOut => {
                            service.last_error = Some("timed out".to_string());
                            schedule_restart(service, &status, Instant::now())?;
                        }
                        StopReason::AdmissionRestart => {}
                    }
                } else {
                    schedule_restart(service, &status, Instant::now())?;
                }
            }
            RunKind::Task => {
                if let Some(task_name) = &managed.task_name {
                    if let Some(task) = self.tasks.get_mut(task_name) {
                        task.active_run_ids
                            .retain(|active_id| active_id != managed.record.id.as_str());
                    }
                    self.maybe_start_next_pending(task_name)?;
                }
            }
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

fn bind_listener(path: impl AsRef<Path>) -> ArmatureResult<UnixListener> {
    UnixListener::bind(path)
        .map_err(|error| ArmatureError::internal(format!("failed to bind daemon socket: {error}")))
}

fn wait_for_socket(path: &Path, timeout: Duration) -> ArmatureResult<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(ArmatureError::unavailable(format!(
        "daemon socket did not appear at {} within {:?}",
        path.display(),
        timeout
    )))
}

fn acquire_lock(path: &Path, pid: u32) -> ArmatureResult<()> {
    use std::fs::OpenOptions;
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                ArmatureError::conflict(format!(
                    "workspace daemon lock already exists at {}",
                    path.display()
                ))
            } else {
                ArmatureError::internal(error.to_string())
            }
        })?;
    drop(file);
    fs::write(path, format!("{pid}\n"))?;
    Ok(())
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
    record: &RunRecord,
    config_version: &str,
    event: Option<&EventRecord>,
    event_path: &Path,
) -> Vec<(String, String)> {
    let mut envs = vec![
        ("ARMATURE_KIND".to_string(), kind.to_string()),
        ("ARMATURE_NAME".to_string(), record.name.clone()),
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
        envs.push((
            "ARMATURE_EVENT_PATH".to_string(),
            event_path.display().to_string(),
        ));
    }
    envs
}

fn write_run_meta(path: &Path, record: &RunRecord, kind: &str) -> ArmatureResult<()> {
    let value = serde_json::json!({
        "run_id": record.id.as_str(),
        "name": record.name,
        "kind": kind,
        "config_version": record.config_version,
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

fn optional_duration(value: Option<&str>) -> ArmatureResult<Option<Duration>> {
    value.map(parse_duration).transpose()
}

fn schedule_restart(
    service: &mut ManagedService,
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
        return Ok(());
    }

    service.restart_attempts.push_back(now);
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

    use armature_core::{discover_workspace, AdmissionPolicy, ProcessState, TriggerOutcome};
    use serde_json::json;
    use tempfile::TempDir;

    use super::{DaemonOptions, DaemonServer};

    #[test]
    fn enabled_service_reconciles_and_survives_invalid_reload() {
        let _guard = test_lock().lock().unwrap();
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
        let _guard = test_lock().lock().unwrap();
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
    fn cancel_run_terminates_process_group_after_timeout() {
        let _guard = test_lock().lock().unwrap();
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
    fn emit_event_routes_tasks_and_writes_event_artifacts() {
        let _guard = test_lock().lock().unwrap();
        let fixture = Fixture::new(
            r#"
[[task]]
name = "react"
on = "tool.run.completed"
run = "printf '%s' \"$ARMATURE_EVENT_TYPE\" > event-type.txt; printf '%s' \"$ARMATURE_EVENT_ID\" > event-id.txt; printf '%s' \"$ARMATURE_EVENT_PATH\" > event-path.txt; cp \"$ARMATURE_EVENT_PATH\" event-copy.json"
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
        assert_eq!(triggers[0].outcome, TriggerOutcome::Started);
        assert_eq!(
            read_file(fixture.root().join("event-type.txt")).trim(),
            "tool.run.completed"
        );

        let event_path = read_file(fixture.root().join("event-path.txt"));
        assert!(Path::new(event_path.trim()).is_file());

        client.shutdown().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn admission_outcomes_remain_inspectable() {
        let _guard = test_lock().lock().unwrap();
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
        let _guard = test_lock().lock().unwrap();
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
}
