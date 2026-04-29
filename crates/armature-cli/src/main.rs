use std::fs::{self, File};
use std::io::{self};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use armature_core::{
    load_workspace_config, resolve_workspace, ArmatureConfig, ArmatureError, ArmatureResult, RunId,
    Workspace, WorkspaceRuntimePaths, CONFIG_DIR_NAME, CONFIG_FILE_NAME,
};
use armature_daemon::{
    store::SqliteStore, DaemonClient, DaemonServer, InspectResponse, RuntimeServiceStatus,
};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const DETACHED_DAEMON_STDOUT: &str = "daemon.stdout.log";
const DETACHED_DAEMON_STDERR: &str = "daemon.stderr.log";
const LOCKS_DIR_NAME: &str = "locks";
const LOCK_FILE_SUFFIX: &str = ".json";
const INIT_TEMPLATE: &str = "# Armature v0.3 config\n#\n# Add tasks and services here.\n#\n# [[task]]\n# name = \"example\"\n# run = \"echo hello from armature\"\n";
const RECIPE_FILE_WATCH_TESTS_CONFIG: &str = "# Armature v0.3 recipe: file-watch tests\n# Edit paths and commands to match your project.\n\n[[task]]\nname = \"test-on-change\"\nwatch = [\"src/**/*\", \"tests/**/*\"]\nrun = \"./scripts/run-tests.sh\"\n";
const RECIPE_FILE_WATCH_TESTS_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\necho \"running project tests\"\n# Replace this placeholder with your real test command.\n# Examples: cargo test, npm test, pytest\nprintf '%s\\n' \"TODO: run your test command here\"\n";
const RECIPE_SCHEDULED_STATUS_CONFIG: &str = "# Armature v0.3 recipe: scheduled status script\n# Replace the schedule and script body with your own status check.\n\n[[task]]\nname = \"scheduled-status\"\nschedule = \"*/15 * * * *\"\nrun = \"./scripts/scheduled-status.sh\"\n";
const RECIPE_SCHEDULED_STATUS_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\nnow=$(date -u +\"%Y-%m-%dT%H:%M:%SZ\")\necho \"status check at $now\"\n# Add whatever local inspection or reporting you need here.\n";
const RECIPE_EVENT_SOURCE_SERVICE_CONFIG: &str = "# Armature v0.3 recipe: generic event source service\n# This service emits a mechanical event on a fixed loop.\n\n[[service]]\nname = \"generic-event-source\"\nrun = \"./sources/generic-event-source.sh\"\n\n[service.supervision]\nrestart = \"on_failure\"\nmax_restarts = 5\nwithin = \"1m\"\nbackoff = \"exponential\"\n";
const RECIPE_EVENT_SOURCE_SERVICE_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\ninterval_seconds=\"${ARMATURE_EVENT_SOURCE_INTERVAL_SECONDS:-30}\"\n\necho \"generic event source started; interval=${interval_seconds}s\"\nwhile true; do\n  armature emit generic.event.tick --json \"$(date -u +'{\\\"emitted_at\\\":\\\"%Y-%m-%dT%H:%M:%SZ\\\",\\\"source\\\":\\\"generic-event-source\\\"}')\"\n  sleep \"$interval_seconds\"\ndone\n";
const RECIPE_EVENT_HOOK_TASK_CONFIG: &str = "# Armature v0.3 recipe: event hook task\n# Emit `hook.example` to trigger this task.\n\n[[task]]\nname = \"event-hook\"\non = \"hook.example\"\nrun = \"./scripts/on-hook-event.sh\"\n";
const RECIPE_EVENT_HOOK_TASK_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\necho \"received event: ${ARMATURE_EVENT_TYPE:-unknown}\"\necho \"payload: ${ARMATURE_EVENT_PAYLOAD_JSON:-null}\"\n# Extend this script with the local side effect you want.\n";
const RECIPE_NAMED_LOCK_CONFIG: &str = "# Armature v0.3 recipe: explicit named lock example\n# This task acquires and releases a named lock itself.\n\n[[task]]\nname = \"with-named-lock\"\nrun = \"./scripts/with-named-lock.sh\"\n";
const RECIPE_NAMED_LOCK_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\nlock_name=\"shared-resource\"\nacquired=0\ncleanup() {\n  if [ \"$acquired\" -eq 1 ]; then\n    armature lock release \"$lock_name\" >/dev/null\n  fi\n}\ntrap cleanup EXIT INT TERM\n\narmature lock acquire \"$lock_name\" --ttl 10m >/dev/null\nacquired=1\necho \"acquired lock: $lock_name\"\n# Put your critical section here.\nsleep 1\n";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> ArmatureResult<()> {
    let cli = Cli::parse();
    cli.command.execute(cli.workspace, cli.format)
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Parser)]
#[command(
    name = "armature",
    version,
    about = "Local daemon for triggering and supervising ordinary programs"
)]
struct Cli {
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,

    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init(InitArgs),
    Dev,
    Up(UpArgs),
    Down,
    Restart(UpArgs),
    Run(RunArgs),
    Emit(EmitArgs),
    Status,
    Ps,
    Tasks,
    Services,
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    Runs,
    Logs(LogsArgs),
    Cancel(CancelArgs),
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Doctor,
    Lock {
        #[command(subcommand)]
        command: LockCommand,
    },
    #[command(hide = true)]
    Internal {
        #[command(subcommand)]
        command: InternalCommand,
    },
}

impl Command {
    fn execute(self, workspace: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
        match self {
            Self::Init(args) => init_workspace(workspace, args, format),
            Self::Dev => dev_workspace(workspace, format),
            Self::Up(args) => up_workspace(workspace, args, format),
            Self::Down => down_workspace(workspace, format),
            Self::Restart(args) => restart_workspace(workspace, args, format),
            Self::Run(args) => run_task(workspace, args, format),
            Self::Emit(args) => emit_event(workspace, args, format),
            Self::Status => status_workspace(workspace, format),
            Self::Ps => ps_workspace(workspace, format),
            Self::Tasks => tasks_workspace(workspace, format),
            Self::Services => services_workspace(workspace, format),
            Self::Service { command } => service_command(workspace, command, format),
            Self::Runs => runs_workspace(workspace, format),
            Self::Logs(args) => logs_workspace(workspace, args, format),
            Self::Cancel(args) => cancel_run(workspace, args, format),
            Self::Config { command } => command.execute(workspace, format),
            Self::Doctor => doctor_workspace(workspace, format),
            Self::Lock { command } => lock_command(workspace, command, format),
            Self::Internal { command } => command.execute(),
        }
    }
}

impl ConfigCommand {
    fn execute(self, workspace: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
        match self {
            Self::Check => {
                let workspace = resolve_workspace_arg(workspace)?;
                let config = load_workspace_config(&workspace)?;
                print_data(
                    format,
                    &json!({
                        "ok": true,
                        "workspace_root": workspace.root(),
                        "config_path": workspace.config_path(),
                        "config_version": config.version,
                    }),
                    &[
                        "ok".to_string(),
                        workspace.config_path().display().to_string(),
                        config.version,
                    ]
                    .join(" "),
                )
            }
        }
    }
}

impl InternalCommand {
    fn execute(self) -> ArmatureResult<()> {
        match self {
            Self::Serve(args) => serve_workspace(args.workspace),
        }
    }
}

#[derive(Debug, Args)]
struct InitArgs {
    #[command(subcommand)]
    command: Option<InitCommand>,
}

#[derive(Debug, Subcommand)]
enum InitCommand {
    Recipe(RecipeArgs),
}

#[derive(Debug, Args)]
struct RecipeArgs {
    name: RecipeName,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum RecipeName {
    FileWatchTests,
    ScheduledStatusScript,
    EventSourceService,
    EventHookTask,
    NamedLock,
}

impl RecipeName {
    fn scaffold(self) -> RecipeScaffold {
        match self {
            Self::FileWatchTests => RecipeScaffold {
                config: RECIPE_FILE_WATCH_TESTS_CONFIG,
                files: vec![RecipeFile {
                    relative_path: PathBuf::from("scripts/run-tests.sh"),
                    contents: RECIPE_FILE_WATCH_TESTS_SCRIPT,
                    executable: true,
                }],
            },
            Self::ScheduledStatusScript => RecipeScaffold {
                config: RECIPE_SCHEDULED_STATUS_CONFIG,
                files: vec![RecipeFile {
                    relative_path: PathBuf::from("scripts/scheduled-status.sh"),
                    contents: RECIPE_SCHEDULED_STATUS_SCRIPT,
                    executable: true,
                }],
            },
            Self::EventSourceService => RecipeScaffold {
                config: RECIPE_EVENT_SOURCE_SERVICE_CONFIG,
                files: vec![RecipeFile {
                    relative_path: PathBuf::from("sources/generic-event-source.sh"),
                    contents: RECIPE_EVENT_SOURCE_SERVICE_SCRIPT,
                    executable: true,
                }],
            },
            Self::EventHookTask => RecipeScaffold {
                config: RECIPE_EVENT_HOOK_TASK_CONFIG,
                files: vec![RecipeFile {
                    relative_path: PathBuf::from("scripts/on-hook-event.sh"),
                    contents: RECIPE_EVENT_HOOK_TASK_SCRIPT,
                    executable: true,
                }],
            },
            Self::NamedLock => RecipeScaffold {
                config: RECIPE_NAMED_LOCK_CONFIG,
                files: vec![RecipeFile {
                    relative_path: PathBuf::from("scripts/with-named-lock.sh"),
                    contents: RECIPE_NAMED_LOCK_SCRIPT,
                    executable: true,
                }],
            },
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::FileWatchTests => "file-watch-tests",
            Self::ScheduledStatusScript => "scheduled-status-script",
            Self::EventSourceService => "event-source-service",
            Self::EventHookTask => "event-hook-task",
            Self::NamedLock => "named-lock",
        }
    }
}

#[derive(Debug, Clone)]
struct RecipeScaffold {
    config: &'static str,
    files: Vec<RecipeFile>,
}

#[derive(Debug, Clone)]
struct RecipeFile {
    relative_path: PathBuf,
    contents: &'static str,
    executable: bool,
}

#[derive(Debug, Args, Clone, Copy)]
struct UpArgs {
    #[arg(long, default_value_t = false)]
    foreground: bool,
}

#[derive(Debug, Args)]
struct RunArgs {
    task_name: String,
}

#[derive(Debug, Args)]
struct EmitArgs {
    event_type: String,
    #[arg(long)]
    json: Option<String>,
}

#[derive(Debug, Args)]
struct LogsArgs {
    run_id: String,
}

#[derive(Debug, Args)]
struct CancelArgs {
    run_id: String,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Check,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    Start(ServiceNameArgs),
    Stop(ServiceNameArgs),
    Restart(ServiceNameArgs),
}

#[derive(Debug, Args)]
struct ServiceNameArgs {
    name: String,
}

#[derive(Debug, Subcommand)]
enum LockCommand {
    Acquire(LockAcquireArgs),
    Release(LockNameArgs),
    Status,
}

#[derive(Debug, Args)]
struct LockAcquireArgs {
    name: String,
    #[arg(long)]
    ttl: Option<String>,
}

#[derive(Debug, Args)]
struct LockNameArgs {
    name: String,
}

#[derive(Debug, Subcommand)]
enum InternalCommand {
    Serve(InternalServeArgs),
}

#[derive(Debug, Args)]
struct InternalServeArgs {
    #[arg(long = "workspace-root")]
    workspace: PathBuf,
}

#[derive(Debug, Serialize)]
struct StatusOutput<'a> {
    workspace_root: &'a Path,
    config_path: &'a Path,
    config_version: String,
    socket_path: String,
    pid_path: String,
    services: usize,
    tasks: usize,
    active_runs: usize,
}

#[derive(Debug, Serialize)]
struct PsOutput {
    runs: Vec<armature_core::RunRecord>,
}

#[derive(Debug, Serialize)]
struct TaskView {
    name: String,
    run: String,
    schedule: Option<String>,
    watch: Vec<String>,
    on: Option<String>,
    admission: String,
    active_run_ids: Vec<String>,
    queued_triggers: usize,
    schedule_active: bool,
    watch_active: bool,
}

#[derive(Debug, Serialize)]
struct ServiceView {
    name: String,
    run: String,
    enabled: bool,
    restart: String,
    state: Option<String>,
    supervision_state: Option<String>,
    active_run_id: Option<String>,
    stop_override: Option<bool>,
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct LogsOutput {
    run_id: String,
    stdout_path: String,
    stderr_path: String,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Serialize)]
struct DoctorOutput {
    workspace_root: PathBuf,
    config_path: PathBuf,
    config_version: Option<String>,
    config_error: Option<String>,
    state_root: PathBuf,
    database_path: PathBuf,
    socket_path: PathBuf,
    pid_path: PathBuf,
    workspace_lock_path: PathBuf,
    daemon_running: bool,
    daemon_error: Option<String>,
    detached_stdout_path: PathBuf,
    detached_stderr_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManualLockRecord {
    name: String,
    owner_pid: u32,
    acquired_at_ms: i64,
    expires_at_ms: Option<i64>,
    manual: bool,
}

fn init_workspace(
    workspace_arg: Option<PathBuf>,
    args: InitArgs,
    format: OutputFormat,
) -> ArmatureResult<()> {
    if let Some(InitCommand::Recipe(recipe)) = args.command {
        return init_recipe_workspace(workspace_arg, recipe.name, format);
    }

    let root = workspace_arg.unwrap_or(std::env::current_dir()?);
    let root = if root.exists() {
        root.canonicalize()?
    } else {
        root
    };
    let config_dir = root.join(CONFIG_DIR_NAME);
    let config_path = config_dir.join(CONFIG_FILE_NAME);

    if config_path.exists() {
        return Err(ArmatureError::conflict(format!(
            "workspace already initialized at {}",
            config_path.display()
        )));
    }

    fs::create_dir_all(&config_dir)?;
    fs::write(&config_path, INIT_TEMPLATE)?;

    print_data(
        format,
        &json!({
            "workspace_root": root,
            "config_path": config_path,
            "created": true,
        }),
        &format!("initialized {}", config_path.display()),
    )
}

fn init_recipe_workspace(
    workspace_arg: Option<PathBuf>,
    recipe_name: RecipeName,
    format: OutputFormat,
) -> ArmatureResult<()> {
    let root = resolve_init_root(workspace_arg)?;
    let config_dir = root.join(CONFIG_DIR_NAME);
    let config_path = config_dir.join(CONFIG_FILE_NAME);
    let scaffold = recipe_name.scaffold();

    ensure_recipe_targets_available(&config_path, &root, &scaffold)?;

    fs::create_dir_all(&config_dir)?;
    fs::write(&config_path, scaffold.config)?;

    let mut created_paths = vec![config_path.clone()];
    for file in scaffold.files {
        let path = root.join(&file.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, file.contents)?;
        if file.executable {
            make_executable(&path)?;
        }
        created_paths.push(path);
    }

    let created = created_paths
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap_or(path)
                .display()
                .to_string()
        })
        .collect::<Vec<_>>();

    let text = format!(
        "initialized recipe {} in {}\n{}",
        recipe_name.as_str(),
        root.display(),
        created.join("\n")
    );

    print_data(
        format,
        &json!({
            "workspace_root": root,
            "recipe": recipe_name.as_str(),
            "config_path": config_path,
            "created": created_paths,
        }),
        &text,
    )
}

fn resolve_init_root(workspace_arg: Option<PathBuf>) -> ArmatureResult<PathBuf> {
    let root = workspace_arg.unwrap_or(std::env::current_dir()?);
    if root.exists() {
        Ok(root.canonicalize()?)
    } else {
        Ok(root)
    }
}

fn ensure_recipe_targets_available(
    config_path: &Path,
    root: &Path,
    scaffold: &RecipeScaffold,
) -> ArmatureResult<()> {
    if config_path.exists() {
        return Err(ArmatureError::conflict(format!(
            "workspace already initialized at {}",
            config_path.display()
        )));
    }
    for file in &scaffold.files {
        let path = root.join(&file.relative_path);
        if path.exists() {
            return Err(ArmatureError::conflict(format!(
                "recipe target already exists at {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn make_executable(path: &Path) -> ArmatureResult<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn dev_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    if daemon_client(&workspace).inspect().is_ok() {
        return Err(ArmatureError::conflict(format!(
            "daemon is already running for workspace {}",
            workspace.root().display()
        )));
    }
    let handle = DaemonServer::start(workspace.clone())?;
    let client = handle.client();
    install_shutdown_handler(client)?;
    print_data(
        format,
        &json!({
            "mode": "foreground",
            "workspace_root": workspace.root(),
            "socket_path": WorkspaceRuntimePaths::for_workspace(&workspace)?.socket_path(),
        }),
        &format!("dev {}", workspace.root().display()),
    )?;
    handle.join()
}

fn up_workspace(
    workspace_arg: Option<PathBuf>,
    args: UpArgs,
    format: OutputFormat,
) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    if args.foreground {
        let handle = DaemonServer::start(workspace.clone())?;
        let client = handle.client();
        install_shutdown_handler(client)?;
        print_data(
            format,
            &json!({
                "mode": "foreground",
                "workspace_root": workspace.root(),
            }),
            &format!("up --foreground {}", workspace.root().display()),
        )?;
        return handle.join();
    }

    load_workspace_config(&workspace)?;
    let client = daemon_client(&workspace);
    if client.inspect().is_ok() {
        client.reload_config()?;
        return print_data(
            format,
            &json!({
                "started": false,
                "reloaded": true,
                "workspace_root": workspace.root(),
            }),
            &format!("daemon already running for {}", workspace.root().display()),
        );
    }

    spawn_detached_daemon(&workspace)?;
    wait_for_daemon(&workspace, Duration::from_secs(3))?;
    print_data(
        format,
        &json!({
            "started": true,
            "foreground": false,
            "workspace_root": workspace.root(),
            "socket_path": WorkspaceRuntimePaths::for_workspace(&workspace)?.socket_path(),
        }),
        &format!("daemon started for {}", workspace.root().display()),
    )
}

fn down_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    daemon_client(&workspace).shutdown()?;
    print_data(
        format,
        &json!({
            "stopped": true,
            "workspace_root": workspace.root(),
        }),
        &format!("daemon stopped for {}", workspace.root().display()),
    )
}

fn restart_workspace(
    workspace_arg: Option<PathBuf>,
    args: UpArgs,
    format: OutputFormat,
) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let client = daemon_client(&workspace);
    let _ = client.shutdown();
    wait_for_daemon_stop(&workspace, Duration::from_secs(3));
    up_workspace(Some(workspace.root().to_path_buf()), args, format)
}

fn run_task(
    workspace_arg: Option<PathBuf>,
    args: RunArgs,
    format: OutputFormat,
) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let run_id = daemon_client(&workspace).start_task(args.task_name.clone())?;
    print_data(
        format,
        &json!({
            "run_id": run_id,
            "task": args.task_name,
        }),
        &format!("started {}", run_id.as_str()),
    )
}

fn emit_event(
    workspace_arg: Option<PathBuf>,
    args: EmitArgs,
    format: OutputFormat,
) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let payload = match args.json {
        Some(raw) => serde_json::from_str::<Value>(&raw).map_err(|error| {
            ArmatureError::invalid_input(format!("invalid JSON payload: {error}"))
        })?,
        None => json!({}),
    };
    daemon_client(&workspace).emit_event(args.event_type.clone(), payload.clone(), None)?;
    print_data(
        format,
        &json!({
            "emitted": true,
            "event_type": args.event_type,
            "payload": payload,
        }),
        "event emitted",
    )
}

fn status_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let inspect = daemon_client(&workspace).inspect()?;
    let output = StatusOutput {
        workspace_root: workspace.root(),
        config_path: workspace.config_path(),
        config_version: inspect.config_version.clone(),
        socket_path: inspect.socket_path.clone(),
        pid_path: inspect.pid_path.clone(),
        services: inspect.services.len(),
        tasks: inspect.tasks.len(),
        active_runs: inspect.active_runs.len(),
    };
    let text = format!(
        "workspace {}\nconfig {}\nservices {} tasks {} active_runs {}\nsocket {}\npid {}",
        output.workspace_root.display(),
        output.config_version,
        output.services,
        output.tasks,
        output.active_runs,
        output.socket_path,
        output.pid_path
    );
    print_data(format, &output, &text)
}

fn ps_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let inspect = daemon_client(&workspace).inspect()?;
    let output = PsOutput {
        runs: inspect.active_runs.clone(),
    };
    let text = if output.runs.is_empty() {
        "no active runs".to_string()
    } else {
        output
            .runs
            .iter()
            .map(|run| format!("{}\t{}\t{:?}", run.id.as_str(), run.name, run.state))
            .collect::<Vec<_>>()
            .join("\n")
    };
    print_data(format, &output, &text)
}

fn tasks_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let config = load_workspace_config(&workspace)?;
    let inspect = daemon_client(&workspace).inspect().ok();
    let tasks = build_task_views(&config, inspect.as_ref());
    let text = if tasks.is_empty() {
        "no tasks configured".to_string()
    } else {
        tasks
            .iter()
            .map(|task| {
                format!(
                    "{}\t{}\tactive={}\tqueued={}",
                    task.name,
                    task.admission,
                    task.active_run_ids.len(),
                    task.queued_triggers
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    print_data(format, &tasks, &text)
}

fn services_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let config = load_workspace_config(&workspace)?;
    let inspect = daemon_client(&workspace).inspect().ok();
    let services = build_service_views(&config, inspect.as_ref());
    let text = if services.is_empty() {
        "no services configured".to_string()
    } else {
        services
            .iter()
            .map(|service| {
                format!(
                    "{}\tenabled={}\trestart={}\tstate={}",
                    service.name,
                    service.enabled,
                    service.restart,
                    service.state.as_deref().unwrap_or("not_running")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    print_data(format, &services, &text)
}

fn service_command(
    workspace_arg: Option<PathBuf>,
    command: ServiceCommand,
    format: OutputFormat,
) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let client = daemon_client(&workspace);
    let (name, action) = match command {
        ServiceCommand::Start(args) => {
            client.service_start(args.name.clone())?;
            (args.name, "started")
        }
        ServiceCommand::Stop(args) => {
            client.service_stop(args.name.clone())?;
            (args.name, "stopped")
        }
        ServiceCommand::Restart(args) => {
            client.service_restart(args.name.clone())?;
            (args.name, "restarted")
        }
    };
    print_data(
        format,
        &json!({
            "service": name,
            "action": action,
        }),
        &format!("service {} {}", name, action),
    )
}

fn runs_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let store = SqliteStore::open(&workspace)?;
    let runs = store.list_runs()?;
    let text = if runs.is_empty() {
        "no runs recorded".to_string()
    } else {
        runs.iter()
            .map(|run| {
                format!(
                    "{}\t{}\t{:?}\t{:?}",
                    run.id.as_str(),
                    run.name,
                    run.origin,
                    run.state
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    print_data(format, &runs, &text)
}

fn logs_workspace(
    workspace_arg: Option<PathBuf>,
    args: LogsArgs,
    format: OutputFormat,
) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let run_id = RunId::parse(args.run_id)?;
    let store = SqliteStore::open(&workspace)?;
    let logs = store.get_logs(&run_id)?.ok_or_else(|| {
        ArmatureError::not_found(format!("logs for {} were not found", run_id.as_str()))
    })?;
    let stdout = read_optional_file(Path::new(&logs.stdout_path))?;
    let stderr = read_optional_file(Path::new(&logs.stderr_path))?;
    let output = LogsOutput {
        run_id: run_id.as_str().to_string(),
        stdout_path: logs.stdout_path.clone(),
        stderr_path: logs.stderr_path.clone(),
        stdout,
        stderr,
    };
    let text = format!(
        "stdout {}\n{}\n---\nstderr {}\n{}",
        output.stdout_path, output.stdout, output.stderr_path, output.stderr
    );
    print_data(format, &output, &text)
}

fn cancel_run(
    workspace_arg: Option<PathBuf>,
    args: CancelArgs,
    format: OutputFormat,
) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    daemon_client(&workspace).cancel_run(args.run_id.clone())?;
    print_data(
        format,
        &json!({
            "cancelled": true,
            "run_id": args.run_id,
        }),
        "run cancelled",
    )
}

fn doctor_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let runtime_paths = WorkspaceRuntimePaths::for_workspace(&workspace)?;
    let config_result = load_workspace_config(&workspace);
    let daemon = daemon_client(&workspace).inspect();
    let output = DoctorOutput {
        workspace_root: workspace.root().to_path_buf(),
        config_path: workspace.config_path().to_path_buf(),
        config_version: config_result
            .as_ref()
            .ok()
            .map(|config| config.version.clone()),
        config_error: config_result.err().map(|error| error.to_string()),
        state_root: runtime_paths.state_root().to_path_buf(),
        database_path: runtime_paths.database_path(),
        socket_path: runtime_paths.socket_path(),
        pid_path: runtime_paths.pid_path(),
        workspace_lock_path: runtime_paths.workspace_lock_path(),
        daemon_running: daemon.is_ok(),
        daemon_error: daemon.err().map(|error| error.to_string()),
        detached_stdout_path: runtime_paths.state_root().join(DETACHED_DAEMON_STDOUT),
        detached_stderr_path: runtime_paths.state_root().join(DETACHED_DAEMON_STDERR),
    };
    let text = format!(
        "workspace {}\nconfig {}\nstate {}\ndaemon_running {}\nsocket {}\ndatabase {}",
        output.workspace_root.display(),
        output.config_path.display(),
        output.state_root.display(),
        output.daemon_running,
        output.socket_path.display(),
        output.database_path.display()
    );
    print_data(format, &output, &text)
}

fn lock_command(
    workspace_arg: Option<PathBuf>,
    command: LockCommand,
    format: OutputFormat,
) -> ArmatureResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let runtime_paths = WorkspaceRuntimePaths::for_workspace(&workspace)?;
    let lock_dir = runtime_paths.state_root().join(LOCKS_DIR_NAME);
    fs::create_dir_all(&lock_dir)?;

    match command {
        LockCommand::Acquire(args) => {
            let ttl = args
                .ttl
                .as_deref()
                .ok_or_else(|| {
                    ArmatureError::invalid_input(
                        "manual lock acquire requires --ttl so ownership remains inspectable",
                    )
                })
                .and_then(parse_duration)
                .map(Some)?;
            let record = acquire_manual_lock(&lock_dir, args.name, ttl)?;
            print_data(format, &record, &format!("lock acquired {}", record.name))
        }
        LockCommand::Release(args) => {
            release_manual_lock(&lock_dir, &args.name)?;
            print_data(
                format,
                &json!({
                    "released": true,
                    "name": args.name,
                }),
                "lock released",
            )
        }
        LockCommand::Status => {
            let locks = list_manual_locks(&lock_dir)?;
            let text = if locks.is_empty() {
                "no locks held".to_string()
            } else {
                locks
                    .iter()
                    .map(|lock| {
                        format!(
                            "{}\tpid={}\texpires_at={}",
                            lock.name,
                            lock.owner_pid,
                            lock.expires_at_ms
                                .map(timestamp_text)
                                .unwrap_or_else(|| "none".to_string())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            print_data(format, &locks, &text)
        }
    }
}

fn serve_workspace(workspace_root: PathBuf) -> ArmatureResult<()> {
    let workspace = resolve_workspace(Some(&workspace_root), &workspace_root)?;
    let handle = DaemonServer::start(workspace)?;
    handle.join()
}

fn resolve_workspace_arg(workspace_arg: Option<PathBuf>) -> ArmatureResult<Workspace> {
    let cwd = std::env::current_dir()?;
    resolve_workspace(workspace_arg.as_ref(), &cwd)
}

fn daemon_client(workspace: &Workspace) -> DaemonClient {
    let runtime_paths = WorkspaceRuntimePaths::for_workspace(workspace)
        .expect("workspace runtime paths should resolve for a valid workspace");
    DaemonClient::from_socket_path(runtime_paths.socket_path())
}

fn install_shutdown_handler(client: DaemonClient) -> ArmatureResult<()> {
    let (sender, receiver) = mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })
    .map_err(|error| {
        ArmatureError::internal(format!("failed to install signal handler: {error}"))
    })?;
    std::thread::spawn(move || {
        let _ = receiver.recv();
        let _ = client.shutdown();
    });
    Ok(())
}

fn spawn_detached_daemon(workspace: &Workspace) -> ArmatureResult<()> {
    let runtime_paths = WorkspaceRuntimePaths::for_workspace(workspace)?;
    runtime_paths.ensure_state_root()?;
    let stdout_path = runtime_paths.state_root().join(DETACHED_DAEMON_STDOUT);
    let stderr_path = runtime_paths.state_root().join(DETACHED_DAEMON_STDERR);
    let stdout = File::create(stdout_path)?;
    let stderr = File::create(stderr_path)?;
    let exe = std::env::current_exe()?;
    let mut command = ProcessCommand::new(exe);
    command
        .arg("internal")
        .arg("serve")
        .arg("--workspace-root")
        .arg(workspace.root())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    command.spawn().map_err(|error| {
        ArmatureError::internal(format!("failed to spawn detached daemon: {error}"))
    })?;
    Ok(())
}

fn wait_for_daemon(workspace: &Workspace, timeout: Duration) -> ArmatureResult<()> {
    let start = SystemTime::now();
    let client = daemon_client(workspace);
    loop {
        if client.inspect().is_ok() {
            return Ok(());
        }
        if elapsed_since(start) >= timeout {
            return Err(ArmatureError::unavailable(format!(
                "daemon did not become ready for workspace {}",
                workspace.root().display()
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_daemon_stop(workspace: &Workspace, timeout: Duration) {
    let start = SystemTime::now();
    let client = daemon_client(workspace);
    while elapsed_since(start) < timeout {
        if client.inspect().is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn build_task_views(config: &ArmatureConfig, inspect: Option<&InspectResponse>) -> Vec<TaskView> {
    let mut views = config
        .tasks
        .iter()
        .map(|task| {
            let runtime = inspect.and_then(|status| {
                status
                    .tasks
                    .iter()
                    .find(|runtime| runtime.name == task.name)
            });
            TaskView {
                name: task.name.clone(),
                run: task.run.clone(),
                schedule: task.trigger.schedule.clone(),
                watch: task.trigger.watch.clone(),
                on: task.trigger.on.clone(),
                admission: format!("{:?}", task.admission.when_busy).to_lowercase(),
                active_run_ids: runtime
                    .map(|runtime| {
                        runtime
                            .active_run_ids
                            .iter()
                            .map(|run_id| run_id.as_str().to_string())
                            .collect()
                    })
                    .unwrap_or_default(),
                queued_triggers: runtime.map(|runtime| runtime.queued_triggers).unwrap_or(0),
                schedule_active: runtime
                    .map(|runtime| runtime.schedule_active)
                    .unwrap_or(false),
                watch_active: runtime.map(|runtime| runtime.watch_active).unwrap_or(false),
            }
        })
        .collect::<Vec<_>>();
    views.sort_by(|left, right| left.name.cmp(&right.name));
    views
}

fn build_service_views(
    config: &ArmatureConfig,
    inspect: Option<&InspectResponse>,
) -> Vec<ServiceView> {
    let mut views = config
        .services
        .iter()
        .map(|service| {
            let runtime = inspect.and_then(|status| {
                status
                    .services
                    .iter()
                    .find(|runtime| runtime.name == service.name)
            });
            ServiceView {
                name: service.name.clone(),
                run: service.run.clone(),
                enabled: service.enabled,
                restart: format!("{:?}", service.supervision.restart).to_lowercase(),
                state: runtime.map(service_state_text),
                supervision_state: runtime.map(|runtime| runtime.supervision_state.clone()),
                active_run_id: runtime
                    .and_then(|runtime| runtime.active_run_id.as_ref())
                    .map(|run_id| run_id.as_str().to_string()),
                stop_override: runtime.map(|runtime| runtime.stop_override),
                last_error: runtime.and_then(|runtime| runtime.last_error.clone()),
            }
        })
        .collect::<Vec<_>>();
    views.sort_by(|left, right| left.name.cmp(&right.name));
    views
}

fn service_state_text(service: &RuntimeServiceStatus) -> String {
    format!("{:?}", service.state).to_lowercase()
}

fn read_optional_file(path: &Path) -> ArmatureResult<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

fn parse_duration(input: &str) -> ArmatureResult<Duration> {
    let trimmed = input.trim();
    let units = [("ms", 1_u64), ("s", 1_000), ("m", 60_000), ("h", 3_600_000)];
    for (suffix, multiplier) in units {
        if let Some(number) = trimmed.strip_suffix(suffix) {
            let value = number.trim().parse::<u64>().map_err(|error| {
                ArmatureError::invalid_input(format!("invalid duration {input:?}: {error}"))
            })?;
            return Ok(Duration::from_millis(value.saturating_mul(multiplier)));
        }
    }
    Err(ArmatureError::invalid_input(format!(
        "invalid duration {input:?}: expected suffix ms, s, m, or h"
    )))
}

fn acquire_manual_lock(
    lock_dir: &Path,
    name: String,
    ttl: Option<Duration>,
) -> ArmatureResult<ManualLockRecord> {
    let path = lock_file_path(lock_dir, &name);
    if let Some(existing) = read_lock_if_fresh(&path)? {
        return Err(ArmatureError::conflict(format!(
            "lock {:?} is already held by pid {}",
            existing.name, existing.owner_pid
        )));
    }

    let acquired_at_ms = now_millis();
    let expires_at_ms = ttl.map(|ttl| acquired_at_ms + ttl.as_millis() as i64);
    let record = ManualLockRecord {
        name,
        owner_pid: std::process::id(),
        acquired_at_ms,
        expires_at_ms,
        manual: true,
    };
    let contents = serde_json::to_vec_pretty(&record)
        .map_err(|error| ArmatureError::internal(error.to_string()))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| match error.kind() {
            io::ErrorKind::AlreadyExists => {
                ArmatureError::conflict(format!("lock {:?} is already held", record.name))
            }
            _ => ArmatureError::internal(error.to_string()),
        })?;
    use std::io::Write as _;
    file.write_all(&contents)?;
    Ok(record)
}

fn release_manual_lock(lock_dir: &Path, name: &str) -> ArmatureResult<()> {
    let path = lock_file_path(lock_dir, name);
    if !path.exists() {
        return Err(ArmatureError::not_found(format!(
            "lock {:?} is not held",
            name
        )));
    }
    fs::remove_file(path)?;
    Ok(())
}

fn list_manual_locks(lock_dir: &Path) -> ArmatureResult<Vec<ManualLockRecord>> {
    let mut locks = Vec::new();
    for entry in fs::read_dir(lock_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if let Some(lock) = read_lock_if_fresh(&path)? {
            locks.push(lock);
        }
    }
    locks.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(locks)
}

fn read_lock_if_fresh(path: &Path) -> ArmatureResult<Option<ManualLockRecord>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read(path)?;
    let record: ManualLockRecord = serde_json::from_slice(&raw)
        .map_err(|error| ArmatureError::internal(format!("invalid lock record: {error}")))?;
    if lock_is_stale(&record) {
        let _ = fs::remove_file(path);
        return Ok(None);
    }
    Ok(Some(record))
}

fn lock_is_stale(record: &ManualLockRecord) -> bool {
    if let Some(expires_at_ms) = record.expires_at_ms {
        if now_millis() >= expires_at_ms {
            return true;
        }
    }
    if record.manual {
        return false;
    }
    !process_exists(record.owner_pid)
}

fn process_exists(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return true;
    }
    let error = io::Error::last_os_error();
    !matches!(error.raw_os_error(), Some(code) if code == libc::ESRCH)
}

fn lock_file_path(lock_dir: &Path, name: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    lock_dir.join(format!("{digest}{LOCK_FILE_SUFFIX}"))
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_millis() as i64
}

fn elapsed_since(start: SystemTime) -> Duration {
    start.elapsed().unwrap_or_default()
}

fn timestamp_text(timestamp_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| timestamp_ms.to_string())
}

fn print_data<T>(format: OutputFormat, value: &T, text: &str) -> ArmatureResult<()>
where
    T: Serialize,
{
    match format {
        OutputFormat::Text => {
            println!("{text}");
        }
        OutputFormat::Json => {
            let encoded = serde_json::to_string_pretty(value)
                .map_err(|error| ArmatureError::internal(error.to_string()))?;
            println!("{encoded}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use clap::CommandFactory;
    use tempfile::TempDir;

    use super::{
        init_recipe_workspace, list_manual_locks, lock_file_path, parse_duration, Cli,
        ManualLockRecord, OutputFormat, RecipeName, CONFIG_DIR_NAME, CONFIG_FILE_NAME,
    };

    #[test]
    fn cli_command_tree_builds() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_ttl_literals() {
        assert_eq!(
            parse_duration("5s").unwrap(),
            std::time::Duration::from_secs(5)
        );
        assert!(parse_duration("oops").is_err());
    }

    #[test]
    fn status_lists_manual_locks_from_disk() {
        let dir = TempDir::new().unwrap();
        let record = ManualLockRecord {
            name: "branch:main".to_string(),
            owner_pid: std::process::id(),
            acquired_at_ms: 1,
            expires_at_ms: None,
            manual: true,
        };
        let path = lock_file_path(dir.path(), &record.name);
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();

        let locks = list_manual_locks(dir.path()).unwrap();
        assert_eq!(locks.len(), 1);
        assert_eq!(locks[0].name, "branch:main");
    }

    #[test]
    fn init_recipe_creates_expected_scaffolding() {
        let cases = [
            (
                RecipeName::FileWatchTests,
                "scripts/run-tests.sh",
                "watch = [\"src/**/*\", \"tests/**/*\"]",
                "TODO: run your test command here",
            ),
            (
                RecipeName::ScheduledStatusScript,
                "scripts/scheduled-status.sh",
                "schedule = \"*/15 * * * *\"",
                "status check at",
            ),
            (
                RecipeName::EventSourceService,
                "sources/generic-event-source.sh",
                "name = \"generic-event-source\"",
                "armature emit generic.event.tick",
            ),
            (
                RecipeName::EventHookTask,
                "scripts/on-hook-event.sh",
                "on = \"hook.example\"",
                "ARMATURE_EVENT_PAYLOAD_JSON",
            ),
            (
                RecipeName::NamedLock,
                "scripts/with-named-lock.sh",
                "name = \"with-named-lock\"",
                "armature lock acquire",
            ),
        ];

        for (recipe, script_path, config_fragment, script_fragment) in cases {
            let dir = TempDir::new().unwrap();
            init_recipe_workspace(Some(dir.path().to_path_buf()), recipe, OutputFormat::Json)
                .unwrap();

            let config_path = dir.path().join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME);
            let config = fs::read_to_string(&config_path).unwrap();
            assert!(
                config.contains(config_fragment),
                "{recipe:?} config mismatch"
            );

            let script_path = dir.path().join(script_path);
            let script = fs::read_to_string(&script_path).unwrap();
            assert!(
                script.contains(script_fragment),
                "{recipe:?} script mismatch"
            );
            assert_ne!(
                fs::metadata(script_path).unwrap().permissions().mode() & 0o111,
                0
            );
        }
    }

    #[test]
    fn init_recipe_rejects_existing_target_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("scripts/with-named-lock.sh");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "existing").unwrap();

        let error = init_recipe_workspace(
            Some(dir.path().to_path_buf()),
            RecipeName::NamedLock,
            OutputFormat::Json,
        )
        .unwrap_err();

        assert!(error.to_string().contains("recipe target already exists"));
        assert!(!dir
            .path()
            .join(CONFIG_DIR_NAME)
            .join(CONFIG_FILE_NAME)
            .exists());
    }
}
