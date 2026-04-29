use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use armature_core::{
    load_workspace_config, ArmatureConfig, ArmatureError, ArmatureResult, ProcessState,
    RestartMode, RunId, RunOrigin, RunRecord, ServiceConfig, TaskConfig, Workspace,
    WorkspaceRuntimePaths,
};
use serde::de::DeserializeOwned;

use crate::duration::parse_duration;
use crate::process::{signal_process_group, spawn_shell_command};
use crate::protocol::{
    DaemonRequest, DaemonResponse, InspectResponse, ResponsePayload, RuntimeServiceStatus,
};
use crate::store::SqliteStore;

const LOOP_SLEEP: Duration = Duration::from_millis(25);
const TERMINATION_GRACE: Duration = Duration::from_secs(2);

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
    pub fn inspect(&self) -> ArmatureResult<InspectResponse> {
        match self.send(DaemonRequest::Inspect)? {
            ResponsePayload::Inspect(response) => Ok(response),
            _ => Err(ArmatureError::internal("unexpected inspect response")),
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

struct ManagedRun {
    record: RunRecord,
    child: Child,
    kind: RunKind,
    kill_after: Option<Duration>,
    started_at: Instant,
    stop_started_at: Option<Instant>,
    stop_reason: Option<StopReason>,
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
            active_runs: HashMap::new(),
            shutdown_requested: false,
            options,
        };
        runtime.rebuild_services();
        runtime.reconcile_services()?;
        Ok(runtime)
    }

    fn run(&mut self) -> ArmatureResult<()> {
        while !self.shutdown_requested || !self.active_runs.is_empty() {
            self.accept_requests()?;
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
            DaemonRequest::Runs => Ok(ResponsePayload::Runs {
                runs: self.store.list_runs()?,
            }),
            DaemonRequest::StartTask { name } => {
                let run_id = self.start_task(&name)?;
                Ok(ResponsePayload::StartedRun { run_id })
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
            active_runs,
        }
    }

    fn start_task(&mut self, name: &str) -> ArmatureResult<RunId> {
        let task = self
            .config
            .tasks
            .iter()
            .find(|task| task.name == name)
            .cloned()
            .ok_or_else(|| ArmatureError::not_found(format!("task {name:?} was not found")))?;
        self.spawn_task(task)
    }

    fn spawn_task(&mut self, task: TaskConfig) -> ArmatureResult<RunId> {
        let kill_after = optional_duration(task.resources.kill_after.as_deref())?;
        let prepared = self.store.create_run(
            task.name.clone(),
            RunOrigin::Task,
            Some(self.config.version.clone()),
            None,
        )?;
        write_run_meta(&prepared.paths.meta, &prepared.record, "task")?;
        let child = spawn_shell_command(
            &task.run,
            self.workspace.root(),
            &prepared.paths.stdout,
            &prepared.paths.stderr,
            &run_envs("task", &prepared.record, self.config.version.as_str()),
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
                kill_after,
                started_at: Instant::now(),
                stop_started_at: None,
                stop_reason: None,
            },
        );
        Ok(run_id)
    }

    fn reload_config(&mut self) -> ArmatureResult<()> {
        let next = load_workspace_config(&self.workspace)?;
        let previous = self.config.clone();
        self.config = next;
        self.apply_service_config_changes(&previous)?;
        self.rebuild_services();
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
            &run_envs("service", &prepared.record, self.config.version.as_str()),
        )?;
        self.store
            .update_run_state(&prepared.record.id, ProcessState::Running)?;
        let run_id = prepared.record.id.as_str().to_string();
        let pid = child.id();
        let mut record = prepared.record;
        record.state = ProcessState::Running;
        self.active_runs.insert(
            run_id.clone(),
            ManagedRun {
                record,
                child,
                kind: RunKind::Service,
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
        let _ = pid;
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

        if managed.kind == RunKind::Service {
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
                }
            } else {
                schedule_restart(service, &status, Instant::now())?;
            }
        }

        Ok(())
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

fn run_envs(kind: &str, record: &RunRecord, config_version: &str) -> Vec<(String, String)> {
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};

    use armature_core::{discover_workspace, ProcessState};
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
