use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use whipplescript_core::{
    load_workspace_config, resolve_workspace, WhippleScriptConfig, WhippleScriptError, WhippleScriptResult,
    EventId, ProcessState, RestartMode, RunId, Workspace, WorkspaceRuntimePaths, CONFIG_DIR_NAME,
    CONFIG_FILE_NAME,
};
use whipplescript_daemon::{
    store::{EventFilter, RunFilter, SqliteStore, TriggerFilter},
    DaemonClient, DaemonServer, InspectResponse, ManualLockRecord, RuntimeServiceStatus,
};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{json, Value};

const DETACHED_DAEMON_STDOUT: &str = "daemon.stdout.log";
const DETACHED_DAEMON_STDERR: &str = "daemon.stderr.log";
const INIT_TEMPLATE: &str = "# WhippleScript v0.3 config\n#\n# Add tasks and services here.\n#\n# [[task]]\n# name = \"example\"\n# run = \"echo hello from whipplescript\"\n";
const RECIPE_FILE_WATCH_TESTS_CONFIG: &str = "# WhippleScript v0.3 recipe: file-watch tests\n# Edit paths and commands to match your project.\n\n[[task]]\nname = \"test-on-change\"\nwatch = [\"src/**/*\", \"tests/**/*\"]\nrun = \"./scripts/run-tests.sh\"\n";
const RECIPE_FILE_WATCH_TESTS_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\necho \"running project tests\"\n# Replace this placeholder with your real test command.\n# Examples: cargo test, npm test, pytest\nprintf '%s\\n' \"TODO: run your test command here\"\n";
const RECIPE_SCHEDULED_STATUS_CONFIG: &str = "# WhippleScript v0.3 recipe: scheduled status script\n# Replace the schedule and script body with your own status check.\n\n[[task]]\nname = \"scheduled-status\"\nschedule = \"*/15 * * * *\"\nrun = \"./scripts/scheduled-status.sh\"\n";
const RECIPE_SCHEDULED_STATUS_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\nnow=$(date -u +\"%Y-%m-%dT%H:%M:%SZ\")\necho \"status check at $now\"\n# Add whatever local inspection or reporting you need here.\n";
const RECIPE_EVENT_SOURCE_SERVICE_CONFIG: &str = "# WhippleScript v0.3 recipe: generic event source service\n# This service emits a mechanical event on a fixed loop.\n\n[[service]]\nname = \"generic-event-source\"\nrun = \"./sources/generic-event-source.sh\"\n\n[service.supervision]\nrestart = \"on_failure\"\nmax_restarts = 5\nwithin = \"1m\"\nbackoff = \"exponential\"\n";
const RECIPE_EVENT_SOURCE_SERVICE_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\ninterval_seconds=\"${WHIPPLESCRIPT_EVENT_SOURCE_INTERVAL_SECONDS:-30}\"\n\necho \"generic event source started; interval=${interval_seconds}s\"\nwhile true; do\n  whip emit generic.event.tick --source generic-event-source --json \"$(date -u +'{\\\"emitted_at\\\":\\\"%Y-%m-%dT%H:%M:%SZ\\\"}')\"\n  sleep \"$interval_seconds\"\ndone\n";
const RECIPE_EVENT_HOOK_TASK_CONFIG: &str = "# WhippleScript v0.3 recipe: event hook task\n# Emit `hook.example` to trigger this task.\n\n[[task]]\nname = \"event-hook\"\non = \"hook.example\"\nrun = \"./scripts/on-hook-event.sh\"\n";
const RECIPE_EVENT_HOOK_TASK_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\necho \"received event: ${WHIPPLESCRIPT_EVENT_TYPE:-unknown}\"\necho \"payload: ${WHIPPLESCRIPT_EVENT_PAYLOAD_JSON:-null}\"\n# Extend this script with the local side effect you want.\n";
const RECIPE_NAMED_LOCK_CONFIG: &str = "# WhippleScript v0.3 recipe: explicit named lock example\n# This task acquires and releases a named lock itself.\n\n[[task]]\nname = \"with-named-lock\"\nrun = \"./scripts/with-named-lock.sh\"\n";
const RECIPE_NAMED_LOCK_SCRIPT: &str = "#!/usr/bin/env sh\nset -eu\n\nlock_name=\"shared-resource\"\nlock_token=\"\"\ncleanup() {\n  if [ -n \"$lock_token\" ]; then\n    whip lock release \"$lock_name\" --token \"$lock_token\" >/dev/null\n  fi\n}\ntrap cleanup EXIT INT TERM\n\nlock_token=$(whip --format json lock acquire \"$lock_name\" --ttl 10m --reason \"named-lock recipe\" | sed -n 's/.*\"token\": *\"\\([^\"]*\\)\".*/\\1/p')\necho \"acquired lock: $lock_name\"\n# Put your critical section here.\nsleep 1\n";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> WhippleScriptResult<()> {
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
    name = "whip",
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
    Exec(AdhocRunArgs),
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    Emit(EmitArgs),
    Event {
        #[command(subcommand)]
        command: EventCommand,
    },
    Events(EventsArgs),
    Trigger {
        #[command(subcommand)]
        command: TriggerCommand,
    },
    Triggers(TriggersArgs),
    Wait {
        #[command(subcommand)]
        command: WaitCommand,
    },
    Subscribe {
        #[command(subcommand)]
        command: SubscribeCommand,
    },
    Overview(OverviewArgs),
    Status(JsonOutputArgs),
    Ps(JsonOutputArgs),
    Tasks,
    Services(JsonOutputArgs),
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    Runs(RunsArgs),
    Logs(LogsArgs),
    Log {
        #[command(subcommand)]
        command: LogCommand,
    },
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
    fn execute(self, workspace: Option<PathBuf>, format: OutputFormat) -> WhippleScriptResult<()> {
        match self {
            Self::Init(args) => init_workspace(workspace, args, format),
            Self::Dev => dev_workspace(workspace, format),
            Self::Up(args) => up_workspace(workspace, args, format),
            Self::Down => down_workspace(workspace, format),
            Self::Restart(args) => restart_workspace(workspace, args, format),
            Self::Exec(args) => run_adhoc(workspace, args, format),
            Self::Run { command } => run_command(workspace, command, format),
            Self::Task { command } => task_command(workspace, command, format),
            Self::Emit(args) => emit_event(workspace, args, format),
            Self::Event { command } => event_command(workspace, command, format),
            Self::Events(args) => events_workspace(workspace, args, format),
            Self::Trigger { command } => trigger_command(workspace, command, format),
            Self::Triggers(args) => triggers_workspace(workspace, args, format),
            Self::Wait { command } => wait_command(workspace, command, format),
            Self::Subscribe { command } => subscribe_command(workspace, command),
            Self::Overview(args) => overview_workspace(workspace, args, format),
            Self::Status(args) => status_workspace(workspace, args.apply(format)),
            Self::Ps(args) => ps_workspace(workspace, args.apply(format)),
            Self::Tasks => tasks_workspace(workspace, format),
            Self::Services(args) => services_workspace(workspace, args.apply(format)),
            Self::Service { command } => service_command(workspace, command, format),
            Self::Runs(args) => runs_workspace(workspace, args, format),
            Self::Logs(args) => logs_workspace(workspace, args, format),
            Self::Log { command } => log_command(workspace, command, format),
            Self::Cancel(args) => cancel_run(workspace, args, format),
            Self::Config { command } => command.execute(workspace, format),
            Self::Doctor => doctor_workspace(workspace, format),
            Self::Lock { command } => lock_command(workspace, command, format),
            Self::Internal { command } => command.execute(),
        }
    }
}

impl ConfigCommand {
    fn execute(self, workspace: Option<PathBuf>, format: OutputFormat) -> WhippleScriptResult<()> {
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
    fn execute(self) -> WhippleScriptResult<()> {
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
    #[arg(long)]
    correlation: Option<String>,
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    List(TaskListArgs),
    Show(TaskNameArgs),
    Run(RunArgs),
    Add(TaskAddArgs),
    Remove(TaskNameArgs),
}

#[derive(Debug, Args)]
struct TaskListArgs {
    #[command(flatten)]
    output: JsonOutputArgs,
    #[arg(long, default_value_t = false)]
    dynamic: bool,
}

#[derive(Debug, Args)]
struct TaskAddArgs {
    name: String,
    #[arg(long)]
    on: Option<String>,
    #[arg(long)]
    watch: Vec<String>,
    #[arg(long)]
    schedule: Option<String>,
    #[arg(long)]
    settle: Option<String>,
    #[arg(long)]
    correlation: Option<String>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "env")]
    env: Vec<String>,
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

#[derive(Debug, Args)]
struct TaskNameArgs {
    name: String,
}

#[derive(Debug, Subcommand)]
enum RunCommand {
    Start(AdhocRunArgs),
    List(RunsArgs),
    Show(RunShowArgs),
    Logs(LogsArgs),
    Cancel(CancelArgs),
    #[command(external_subcommand)]
    TaskAlias(Vec<OsString>),
}

#[derive(Debug, Args)]
struct RunShowArgs {
    run_id: String,
}

#[derive(Debug, Args)]
#[command(
    trailing_var_arg = true,
    after_help = "Starts one finite ad hoc command through the daemon. Payload input defaults to an empty JSON object. Use -- to separate WhippleScript options from the command."
)]
struct AdhocRunArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    correlation: Option<String>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "env", value_name = "KEY=VALUE")]
    env: Vec<String>,
    #[arg(long)]
    timeout: Option<String>,
    #[arg(long, conflicts_with_all = ["payload_file", "stdin"])]
    json: Option<String>,
    #[arg(long, value_name = "PATH", conflicts_with_all = ["json", "stdin"])]
    payload_file: Option<PathBuf>,
    #[arg(long, default_value_t = false, conflicts_with_all = ["json", "payload_file"])]
    stdin: bool,
    #[arg(required = true, num_args = 1.., allow_hyphen_values = true)]
    command: Vec<String>,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Payload input defaults to an empty JSON object. Use exactly one of --json, --payload-file, or --stdin. This command emits an event to the running daemon; it does not publish, buffer, or replay events."
)]
struct EmitArgs {
    /// Event type matched by task `on = "..."`
    event_type: String,
    /// Inline JSON payload.
    ///
    /// Kept for compatibility with existing scripts. Use global `--format json`
    /// before the command for machine-readable command output.
    #[arg(long, conflicts_with_all = ["payload_file", "stdin"])]
    json: Option<String>,
    /// Read the JSON payload from a file.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["json", "stdin"])]
    payload_file: Option<PathBuf>,
    /// Read the JSON payload from stdin.
    #[arg(long, default_value_t = false, conflicts_with_all = ["json", "payload_file"])]
    stdin: bool,
    /// Event source recorded in the event log.
    ///
    /// Defaults to `cli` for compatibility with existing CLI-emitted events.
    #[arg(long, default_value = "cli")]
    source: String,
    #[arg(long)]
    correlation: Option<String>,
}

#[derive(Debug, Subcommand)]
enum EventCommand {
    List(EventsArgs),
    Show(EventShowArgs),
    Emit(EmitArgs),
}

#[derive(Debug, Args)]
struct EventShowArgs {
    event_id: String,
}

#[derive(Debug, Subcommand)]
enum TriggerCommand {
    List(TriggersArgs),
    Show(TriggerShowArgs),
}

#[derive(Debug, Args)]
struct TriggerShowArgs {
    trigger_id: String,
}

#[derive(Debug, Args, Clone, Copy)]
struct JsonOutputArgs {
    #[arg(long, default_value_t = false)]
    json: bool,
}

impl JsonOutputArgs {
    fn apply(self, format: OutputFormat) -> OutputFormat {
        if self.json {
            OutputFormat::Json
        } else {
            format
        }
    }
}

#[derive(Debug, Args)]
struct EventsArgs {
    #[command(flatten)]
    output: JsonOutputArgs,
    #[arg(long = "type")]
    event_type: Option<String>,
    #[arg(long)]
    source: Option<String>,
    #[arg(long)]
    correlation: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
struct TriggersArgs {
    #[command(flatten)]
    output: JsonOutputArgs,
    #[arg(long)]
    task: Option<String>,
    #[arg(long = "event", alias = "event-type")]
    event_type: Option<String>,
    #[arg(long)]
    outcome: Option<String>,
    #[arg(long)]
    correlation: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
struct RunsArgs {
    #[command(flatten)]
    output: JsonOutputArgs,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    origin: Option<String>,
    #[arg(long)]
    state: Option<String>,
    #[arg(long)]
    correlation: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Subcommand)]
enum WaitCommand {
    Event(WaitEventArgs),
    Run(WaitRunArgs),
    Trigger(WaitTriggerArgs),
    Service(WaitServiceArgs),
}

#[derive(Debug, Args)]
struct WaitEventArgs {
    event_type: String,
    #[arg(long)]
    correlation: Option<String>,
    #[arg(long)]
    timeout: String,
}

#[derive(Debug, Args)]
struct WaitRunArgs {
    run_id: String,
    #[arg(long)]
    state: String,
    #[arg(long)]
    timeout: String,
}

#[derive(Debug, Args)]
struct WaitTriggerArgs {
    #[arg(long)]
    task: Option<String>,
    #[arg(long = "event", alias = "event-type")]
    event_type: Option<String>,
    #[arg(long)]
    outcome: Option<String>,
    #[arg(long)]
    correlation: Option<String>,
    #[arg(long)]
    timeout: String,
}

#[derive(Debug, Args)]
struct WaitServiceArgs {
    name: String,
    #[arg(long)]
    state: String,
    #[arg(long)]
    timeout: String,
}

#[derive(Debug, Subcommand)]
enum SubscribeCommand {
    Events,
    Runs,
    Triggers,
}

#[derive(Debug, Args)]
struct OverviewArgs {
    #[command(flatten)]
    output: JsonOutputArgs,
    #[arg(long, default_value_t = 10)]
    recent: usize,
}

#[derive(Debug, Args)]
struct LogsArgs {
    #[arg(long, value_name = "LINES")]
    tail: Option<usize>,
    #[arg(long, default_value_t = false)]
    follow: bool,
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
    List(ServiceListArgs),
    Show(ServiceShowArgs),
    Add(ServiceAddArgs),
    Remove(ServiceNameArgs),
    Start(ServiceNameArgs),
    Stop(ServiceNameArgs),
    Restart(ServiceNameArgs),
}

#[derive(Debug, Args)]
struct ServiceListArgs {
    #[command(flatten)]
    output: JsonOutputArgs,
    #[arg(long, default_value_t = false)]
    dynamic: bool,
}

#[derive(Debug, Args)]
struct ServiceAddArgs {
    name: String,
    #[arg(long)]
    correlation: Option<String>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "env")]
    env: Vec<String>,
    #[arg(long, value_enum, default_value_t = ServiceRestartArg::OnFailure)]
    restart: ServiceRestartArg,
    #[arg(long)]
    reason: Option<String>,
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum ServiceRestartArg {
    Never,
    OnFailure,
    Always,
}

impl From<ServiceRestartArg> for RestartMode {
    fn from(value: ServiceRestartArg) -> Self {
        match value {
            ServiceRestartArg::Never => RestartMode::Never,
            ServiceRestartArg::OnFailure => RestartMode::OnFailure,
            ServiceRestartArg::Always => RestartMode::Always,
        }
    }
}

#[derive(Debug, Args)]
struct ServiceNameArgs {
    name: String,
}

#[derive(Debug, Args)]
struct ServiceShowArgs {
    #[command(flatten)]
    output: JsonOutputArgs,
    name: String,
}

#[derive(Debug, Subcommand)]
enum LogCommand {
    Show(LogsArgs),
    Tail(LogTailArgs),
    Follow(LogFollowArgs),
}

#[derive(Debug, Args)]
struct LogTailArgs {
    run_id: String,
    #[arg(long, value_name = "LINES", default_value_t = 100)]
    lines: usize,
}

#[derive(Debug, Args)]
struct LogFollowArgs {
    run_id: String,
}

#[derive(Debug, Subcommand)]
enum LockCommand {
    Acquire(LockAcquireArgs),
    Renew(LockRenewArgs),
    Release(LockNameArgs),
    ForceRelease(LockForceReleaseArgs),
    Show(LockShowArgs),
    List(LockListArgs),
    With(LockWithArgs),
    Status,
}

#[derive(Debug, Args)]
struct LockListArgs {
    #[arg(long, default_value_t = false)]
    expired: bool,
}

#[derive(Debug, Args)]
struct LockShowArgs {
    name: String,
}

#[derive(Debug, Args)]
struct LockAcquireArgs {
    name: String,
    #[arg(long)]
    ttl: Option<String>,
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
struct LockRenewArgs {
    name: String,
    #[arg(long)]
    token: String,
    #[arg(long)]
    ttl: String,
}

#[derive(Debug, Args)]
struct LockNameArgs {
    name: String,
    #[arg(long)]
    token: Option<String>,
}

#[derive(Debug, Args)]
struct LockForceReleaseArgs {
    name: String,
    #[arg(long)]
    reason: String,
}

#[derive(Debug, Args)]
struct LockWithArgs {
    name: String,
    #[arg(long)]
    ttl: String,
    #[arg(long)]
    reason: String,
    #[arg(required = true, num_args = 1.., allow_hyphen_values = true, trailing_var_arg = true)]
    command: Vec<String>,
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
    runs: Vec<whipplescript_core::RunRecord>,
}

#[derive(Debug, Serialize)]
struct OverviewOutput {
    workspace_root: PathBuf,
    config_path: PathBuf,
    config_version: String,
    daemon_running: bool,
    socket_path: Option<String>,
    pid_path: Option<String>,
    tasks: Vec<TaskOverview>,
    services: Vec<ServiceOverview>,
    active_runs: Vec<RunSummary>,
    recent_events: Vec<whipplescript_core::EventRecord>,
    recent_triggers: Vec<whipplescript_core::TriggerRecord>,
    recent_failures: Vec<RunSummary>,
}

#[derive(Debug, Serialize)]
struct TaskOverview {
    name: String,
    run: String,
    dynamic: bool,
    schedule: Option<String>,
    watch: Vec<String>,
    on: Option<String>,
    admission: String,
    active_run_ids: Vec<String>,
    queued_triggers: usize,
    latest_run: Option<RunSummary>,
    latest_failure: Option<RunSummary>,
}

#[derive(Debug, Serialize)]
struct ServiceOverview {
    name: String,
    run: String,
    enabled: bool,
    dynamic: bool,
    restart: String,
    state: Option<String>,
    active_run_id: Option<String>,
    stop_override: Option<bool>,
    health_state: Option<String>,
    last_error: Option<String>,
    latest_run: Option<RunSummary>,
    latest_failure: Option<RunSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct RunSummary {
    id: String,
    name: String,
    command: String,
    origin: String,
    state: String,
    start_time: String,
    end_time: Option<String>,
    exit_code: Option<i32>,
    signal: Option<i32>,
    killed: bool,
    event_id: Option<String>,
    stdout_path: Option<String>,
    stderr_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct TaskView {
    name: String,
    run: String,
    dynamic: bool,
    cwd: Option<String>,
    env: Vec<(String, String)>,
    created_by_run_id: Option<String>,
    parent_event_id: Option<String>,
    correlation_id: Option<String>,
    schedule: Option<String>,
    watch: Vec<String>,
    settle: Option<String>,
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
    dynamic: bool,
    cwd: Option<String>,
    env: Vec<(String, String)>,
    created_by_run_id: Option<String>,
    parent_event_id: Option<String>,
    correlation_id: Option<String>,
    reason: Option<String>,
    restart: String,
    state: Option<String>,
    supervision_state: Option<String>,
    active_run_id: Option<String>,
    stop_override: Option<bool>,
    last_error: Option<String>,
    health_state: Option<String>,
    health_active_run_id: Option<String>,
    health_last_run_id: Option<String>,
    health_last_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct LogsOutput {
    run_id: String,
    run: Option<whipplescript_core::RunRecord>,
    run_directory: Option<String>,
    stdout_path: String,
    stderr_path: String,
    stdout_bytes: u64,
    stderr_bytes: u64,
    stdout_lines: usize,
    stderr_lines: usize,
    stdout_truncated: bool,
    stderr_truncated: bool,
    stdout_missing: bool,
    stderr_missing: bool,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
struct LogFileSnapshot {
    contents: String,
    bytes: u64,
    lines: usize,
    truncated: bool,
    missing: bool,
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

fn init_workspace(
    workspace_arg: Option<PathBuf>,
    args: InitArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
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
        return Err(WhippleScriptError::conflict(format!(
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
) -> WhippleScriptResult<()> {
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

fn resolve_init_root(workspace_arg: Option<PathBuf>) -> WhippleScriptResult<PathBuf> {
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
) -> WhippleScriptResult<()> {
    if config_path.exists() {
        return Err(WhippleScriptError::conflict(format!(
            "workspace already initialized at {}",
            config_path.display()
        )));
    }
    for file in &scaffold.files {
        let path = root.join(&file.relative_path);
        if path.exists() {
            return Err(WhippleScriptError::conflict(format!(
                "recipe target already exists at {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn make_executable(path: &Path) -> WhippleScriptResult<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn dev_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    if daemon_client(&workspace).inspect().is_ok() {
        return Err(WhippleScriptError::conflict(format!(
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
) -> WhippleScriptResult<()> {
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

fn down_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    daemon_client(&workspace).shutdown()?;
    wait_for_daemon_stop(&workspace, Duration::from_secs(3))?;
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
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let client = daemon_client(&workspace);
    client.shutdown()?;
    wait_for_daemon_stop(&workspace, Duration::from_secs(3))?;
    up_workspace(Some(workspace.root().to_path_buf()), args, format)
}

fn task_command(
    workspace_arg: Option<PathBuf>,
    command: TaskCommand,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    match command {
        TaskCommand::List(args) => {
            tasks_workspace_filtered(workspace_arg, args.output.apply(format), args.dynamic)
        }
        TaskCommand::Show(args) => task_show(workspace_arg, args, format),
        TaskCommand::Run(args) => run_task(workspace_arg, args, format),
        TaskCommand::Add(args) => task_add(workspace_arg, args, format),
        TaskCommand::Remove(args) => {
            let workspace = resolve_workspace_arg(workspace_arg)?;
            daemon_client(&workspace).task_remove(args.name.clone())?;
            print_data(
                format,
                &json!({
                    "task": args.name,
                    "action": "removed",
                    "dynamic": true,
                }),
                "task removed",
            )
        }
    }
}

fn run_command(
    workspace_arg: Option<PathBuf>,
    command: RunCommand,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    match command {
        RunCommand::Start(args) => run_adhoc(workspace_arg, args, format),
        RunCommand::List(args) => runs_workspace(workspace_arg, args, format),
        RunCommand::Show(args) => run_show(workspace_arg, args, format),
        RunCommand::Logs(args) => logs_workspace(workspace_arg, args, format),
        RunCommand::Cancel(args) => cancel_run(workspace_arg, args, format),
        RunCommand::TaskAlias(raw) => run_task(workspace_arg, parse_run_alias(raw)?, format),
    }
}

fn parse_run_alias(raw: Vec<OsString>) -> WhippleScriptResult<RunArgs> {
    let mut values = raw.into_iter();
    let task_name = values
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| WhippleScriptError::invalid_input("expected task name"))?;
    let mut correlation = None;
    while let Some(value) = values.next() {
        let value = value
            .into_string()
            .map_err(|_| WhippleScriptError::invalid_input("invalid run argument"))?;
        if value == "--correlation" {
            let next = values
                .next()
                .and_then(|value| value.into_string().ok())
                .ok_or_else(|| WhippleScriptError::invalid_input("--correlation requires a value"))?;
            correlation = Some(next);
        } else {
            return Err(WhippleScriptError::invalid_input(format!(
                "unknown run alias argument {value:?}"
            )));
        }
    }
    Ok(RunArgs {
        task_name,
        correlation,
    })
}

fn run_task(
    workspace_arg: Option<PathBuf>,
    args: RunArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let provenance = provenance_from_env(args.correlation)?;
    let run_id = daemon_client(&workspace).start_task_with_provenance(
        args.task_name.clone(),
        provenance.source_run_id,
        provenance.parent_event_id,
        provenance.correlation_id.clone(),
    )?;
    print_data(
        format,
        &json!({
            "run_id": run_id,
            "task": args.task_name,
            "correlation_id": provenance.correlation_id,
        }),
        &format!("started {}", run_id.as_str()),
    )
}

fn run_adhoc(
    workspace_arg: Option<PathBuf>,
    args: AdhocRunArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let (payload, payload_source) = read_adhoc_payload(&args)?;
    let correlation = args
        .correlation
        .clone()
        .or_else(|| payload_correlation_id(&payload));
    let provenance = provenance_from_env(correlation)?;
    let env = parse_env_pairs(&args.env)?;
    let timeout = args.timeout.as_deref().map(parse_duration).transpose()?;
    let run_id = daemon_client(&workspace).start_adhoc(
        args.name.clone(),
        args.command.clone(),
        args.cwd.clone(),
        env,
        timeout,
        payload.clone(),
        provenance.source_run_id.clone(),
        provenance.parent_event_id.clone(),
        provenance.correlation_id.clone(),
    )?;
    print_data(
        format,
        &json!({
            "run_id": run_id,
            "name": args.name,
            "origin": "adhoc",
            "command": args.command,
            "payload": payload,
            "payload_source": payload_source,
            "source_run_id": provenance.source_run_id,
            "parent_event_id": provenance.parent_event_id,
            "correlation_id": provenance.correlation_id,
        }),
        &format!("started {}", run_id.as_str()),
    )
}

fn emit_event(
    workspace_arg: Option<PathBuf>,
    args: EmitArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let (payload, payload_source) = read_emit_payload(&args)?;
    let correlation = args
        .correlation
        .clone()
        .or_else(|| payload_correlation_id(&payload));
    let provenance = provenance_from_env(correlation)?;
    daemon_client(&workspace).emit_event_with_provenance(
        args.event_type.clone(),
        payload.clone(),
        Some(args.source.clone()),
        provenance.source_run_id.clone(),
        provenance.parent_event_id.clone(),
        provenance.correlation_id.clone(),
    )?;
    print_data(
        format,
        &json!({
            "emitted": true,
            "type": args.event_type,
            "event_type": args.event_type,
            "payload": payload,
            "payload_source": payload_source,
            "source": args.source,
            "source_run_id": provenance.source_run_id,
            "parent_event_id": provenance.parent_event_id,
            "correlation_id": provenance.correlation_id,
        }),
        "event emitted",
    )
}

fn event_command(
    workspace_arg: Option<PathBuf>,
    command: EventCommand,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    match command {
        EventCommand::List(args) => events_workspace(workspace_arg, args, format),
        EventCommand::Show(args) => event_show(workspace_arg, args, format),
        EventCommand::Emit(args) => emit_event(workspace_arg, args, format),
    }
}

fn trigger_command(
    workspace_arg: Option<PathBuf>,
    command: TriggerCommand,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    match command {
        TriggerCommand::List(args) => triggers_workspace(workspace_arg, args, format),
        TriggerCommand::Show(args) => trigger_show(workspace_arg, args, format),
    }
}

fn read_emit_payload(args: &EmitArgs) -> WhippleScriptResult<(Value, String)> {
    if let Some(raw) = args.json.as_deref() {
        return parse_emit_payload(raw, "--json").map(|payload| (payload, "json".to_string()));
    }

    if let Some(path) = args.payload_file.as_deref() {
        let raw = fs::read_to_string(path).map_err(|error| {
            WhippleScriptError::invalid_input(format!(
                "failed to read JSON payload file {}: {error}",
                path.display()
            ))
        })?;
        return parse_emit_payload(&raw, "--payload-file")
            .map(|payload| (payload, format!("file:{}", path.display())));
    }

    if args.stdin {
        let mut raw = String::new();
        io::stdin().read_to_string(&mut raw).map_err(|error| {
            WhippleScriptError::invalid_input(format!("failed to read JSON payload from stdin: {error}"))
        })?;
        return parse_emit_payload(&raw, "--stdin").map(|payload| (payload, "stdin".to_string()));
    }

    Ok((json!({}), "default-empty-object".to_string()))
}

fn read_adhoc_payload(args: &AdhocRunArgs) -> WhippleScriptResult<(Value, String)> {
    if let Some(raw) = args.json.as_deref() {
        return parse_emit_payload(raw, "--json").map(|payload| (payload, "json".to_string()));
    }

    if let Some(path) = args.payload_file.as_deref() {
        let raw = fs::read_to_string(path).map_err(|error| {
            WhippleScriptError::invalid_input(format!(
                "failed to read JSON payload file {}: {error}",
                path.display()
            ))
        })?;
        return parse_emit_payload(&raw, "--payload-file")
            .map(|payload| (payload, format!("file:{}", path.display())));
    }

    if args.stdin {
        let mut raw = String::new();
        io::stdin().read_to_string(&mut raw).map_err(|error| {
            WhippleScriptError::invalid_input(format!("failed to read JSON payload from stdin: {error}"))
        })?;
        return parse_emit_payload(&raw, "--stdin").map(|payload| (payload, "stdin".to_string()));
    }

    Ok((json!({}), "default-empty-object".to_string()))
}

fn parse_env_pairs(raw: &[String]) -> WhippleScriptResult<Vec<(String, String)>> {
    raw.iter()
        .map(|pair| {
            let (key, value) = pair.split_once('=').ok_or_else(|| {
                WhippleScriptError::invalid_input(format!(
                    "invalid env value {pair:?}: expected KEY=VALUE"
                ))
            })?;
            if key.is_empty() {
                return Err(WhippleScriptError::invalid_input(
                    "env key cannot be empty in KEY=VALUE",
                ));
            }
            Ok((key.to_string(), value.to_string()))
        })
        .collect()
}

#[derive(Debug, Clone)]
struct EventProvenance {
    source_run_id: Option<RunId>,
    parent_event_id: Option<EventId>,
    correlation_id: Option<String>,
}

fn provenance_from_env(correlation: Option<String>) -> WhippleScriptResult<EventProvenance> {
    let source_run_id = optional_env("WHIPPLESCRIPT_RUN_ID")
        .map(RunId::parse)
        .transpose()?;
    let parent_event_id = optional_env("WHIPPLESCRIPT_EVENT_ID")
        .map(EventId::parse)
        .transpose()?;
    let correlation_id = correlation
        .or_else(|| optional_env("WHIPPLESCRIPT_CORRELATION_ID"))
        .filter(|value| !value.is_empty());

    Ok(EventProvenance {
        source_run_id,
        parent_event_id,
        correlation_id,
    })
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn payload_correlation_id(payload: &Value) -> Option<String> {
    payload
        .get("correlation_id")
        .or_else(|| payload.get("correlationId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_emit_payload(raw: &str, source: &str) -> WhippleScriptResult<Value> {
    serde_json::from_str::<Value>(raw).map_err(|error| {
        WhippleScriptError::invalid_input(format!("invalid JSON payload from {source}: {error}"))
    })
}

fn events_workspace(
    workspace_arg: Option<PathBuf>,
    args: EventsArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let store = SqliteStore::open(&workspace)?;
    let format = args.output.apply(format);
    let events = store.list_events_filtered(&EventFilter {
        event_type: args.event_type,
        source: args.source,
        correlation: args.correlation,
        limit: args.limit,
    })?;
    let text = if events.is_empty() {
        "no events recorded".to_string()
    } else {
        events
            .iter()
            .map(|event| {
                format!(
                    "{}\t{}\t{}\t{}\t{}",
                    event.id.as_str(),
                    event.time,
                    event.event_type,
                    format!("{:?}", event.routing).to_lowercase(),
                    event
                        .source
                        .as_deref()
                        .unwrap_or(whipplescript_core::model::DEFAULT_EVENT_SOURCE)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    print_data(format, &events, &text)
}

fn event_show(
    workspace_arg: Option<PathBuf>,
    args: EventShowArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let event_id = EventId::parse(args.event_id)?;
    let store = SqliteStore::open(&workspace)?;
    let event = store.get_event(&event_id)?.ok_or_else(|| {
        WhippleScriptError::not_found(format!("event {} was not found", event_id.as_str()))
    })?;
    let text = format!(
        "{}\t{}\t{}",
        event.id.as_str(),
        event.event_type,
        event
            .source
            .as_deref()
            .unwrap_or(whipplescript_core::model::DEFAULT_EVENT_SOURCE)
    );
    print_data(format, &event, &text)
}

fn triggers_workspace(
    workspace_arg: Option<PathBuf>,
    args: TriggersArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let store = SqliteStore::open(&workspace)?;
    let format = args.output.apply(format);
    let triggers = store.list_triggers_filtered(&TriggerFilter {
        task: args.task,
        event_type: args.event_type,
        outcome: args.outcome,
        correlation: args.correlation,
        limit: args.limit,
    })?;
    let text = if triggers.is_empty() {
        "no triggers recorded".to_string()
    } else {
        triggers
            .iter()
            .map(|trigger| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\trun={}",
                    trigger.id.as_str(),
                    trigger.task_name,
                    trigger.event_type,
                    format!("{:?}", trigger.admission).to_lowercase(),
                    format!("{:?}", trigger.outcome).to_lowercase(),
                    trigger
                        .run_id
                        .as_ref()
                        .map(|run_id| run_id.as_str())
                        .unwrap_or("none")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    print_data(format, &triggers, &text)
}

fn trigger_show(
    workspace_arg: Option<PathBuf>,
    args: TriggerShowArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let store = SqliteStore::open(&workspace)?;
    let trigger = store
        .list_triggers_filtered(&TriggerFilter {
            task: None,
            event_type: None,
            outcome: None,
            correlation: None,
            limit: None,
        })?
        .into_iter()
        .find(|trigger| trigger.id.as_str() == args.trigger_id)
        .ok_or_else(|| {
            WhippleScriptError::not_found(format!("trigger {:?} was not found", args.trigger_id))
        })?;
    let text = format!(
        "{}\t{}\t{}\t{:?}",
        trigger.id.as_str(),
        trigger.task_name,
        trigger.event_type,
        trigger.outcome
    );
    print_data(format, &trigger, &text)
}

fn wait_command(
    workspace_arg: Option<PathBuf>,
    command: WaitCommand,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    match command {
        WaitCommand::Event(args) => wait_event(workspace_arg, args, format),
        WaitCommand::Run(args) => wait_run(workspace_arg, args, format),
        WaitCommand::Trigger(args) => wait_trigger(workspace_arg, args, format),
        WaitCommand::Service(args) => wait_service(workspace_arg, args, format),
    }
}

fn wait_event(
    workspace_arg: Option<PathBuf>,
    args: WaitEventArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let timeout = parse_duration(&args.timeout)?;
    let event = wait_until_observed(timeout, "event", || {
        let store = SqliteStore::open(&workspace)?;
        Ok(store
            .list_events_filtered(&EventFilter {
                event_type: Some(args.event_type.clone()),
                source: None,
                correlation: args.correlation.clone(),
                limit: Some(1),
            })?
            .into_iter()
            .next())
    })?;
    let text = format!("event {}", event.id.as_str());
    print_data(format, &event, &text)
}

fn wait_run(
    workspace_arg: Option<PathBuf>,
    args: WaitRunArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let timeout = parse_duration(&args.timeout)?;
    let run_id = RunId::parse(args.run_id)?;
    let run = wait_until_observed(timeout, "run", || {
        let store = SqliteStore::open(&workspace)?;
        Ok(store.get_run(&run_id)?.filter(|run| {
            enum_text(&run.state)
                .map(|state| state == args.state)
                .unwrap_or(false)
        }))
    })?;
    let text = format!("run {} {}", run.id.as_str(), enum_text(&run.state)?);
    print_data(format, &run, &text)
}

fn wait_trigger(
    workspace_arg: Option<PathBuf>,
    args: WaitTriggerArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let timeout = parse_duration(&args.timeout)?;
    let trigger = wait_until_observed(timeout, "trigger", || {
        let store = SqliteStore::open(&workspace)?;
        Ok(store
            .list_triggers_filtered(&TriggerFilter {
                task: args.task.clone(),
                event_type: args.event_type.clone(),
                outcome: args.outcome.clone(),
                correlation: args.correlation.clone(),
                limit: Some(1),
            })?
            .into_iter()
            .next())
    })?;
    let text = format!("trigger {}", trigger.id.as_str());
    print_data(format, &trigger, &text)
}

fn wait_service(
    workspace_arg: Option<PathBuf>,
    args: WaitServiceArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let timeout = parse_duration(&args.timeout)?;
    let service = wait_until_observed(timeout, "service", || {
        let inspect = daemon_client(&workspace).inspect()?;
        Ok(inspect.services.into_iter().find(|service| {
            service.name == args.name
                && enum_text(&service.state)
                    .map(|state| state == args.state)
                    .unwrap_or(false)
        }))
    })?;
    let text = format!("service {} {}", service.name, enum_text(&service.state)?);
    print_data(format, &service, &text)
}

fn subscribe_command(
    workspace_arg: Option<PathBuf>,
    command: SubscribeCommand,
) -> WhippleScriptResult<()> {
    match command {
        SubscribeCommand::Events => subscribe_events(workspace_arg),
        SubscribeCommand::Runs => subscribe_runs(workspace_arg),
        SubscribeCommand::Triggers => subscribe_triggers(workspace_arg),
    }
}

fn subscribe_events(workspace_arg: Option<PathBuf>) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let mut seen = SqliteStore::open(&workspace)?
        .list_events()?
        .into_iter()
        .map(|event| event.id.as_str().to_string())
        .collect::<HashSet<_>>();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let mut events = SqliteStore::open(&workspace)?.list_events()?;
        events.reverse();
        for event in events {
            if seen.insert(event.id.as_str().to_string()) {
                write_ndjson(&event)?;
            }
        }
    }
}

fn subscribe_runs(workspace_arg: Option<PathBuf>) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let mut seen = SqliteStore::open(&workspace)?
        .list_runs()?
        .into_iter()
        .map(|run| Ok(format!("{}:{}", run.id.as_str(), enum_text(&run.state)?)))
        .collect::<WhippleScriptResult<HashSet<_>>>()?;
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let mut runs = SqliteStore::open(&workspace)?.list_runs()?;
        runs.reverse();
        for run in runs {
            let key = format!("{}:{}", run.id.as_str(), enum_text(&run.state)?);
            if seen.insert(key) {
                write_ndjson(&run)?;
            }
        }
    }
}

fn subscribe_triggers(workspace_arg: Option<PathBuf>) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let mut seen = SqliteStore::open(&workspace)?
        .list_triggers()?
        .into_iter()
        .map(|trigger| trigger.id.as_str().to_string())
        .collect::<HashSet<_>>();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let mut triggers = SqliteStore::open(&workspace)?.list_triggers()?;
        triggers.reverse();
        for trigger in triggers {
            if seen.insert(trigger.id.as_str().to_string()) {
                write_ndjson(&trigger)?;
            }
        }
    }
}

fn wait_until_observed<T>(
    timeout: Duration,
    label: &str,
    mut observe: impl FnMut() -> WhippleScriptResult<Option<T>>,
) -> WhippleScriptResult<T> {
    let start = Instant::now();
    while start.elapsed() <= timeout {
        if let Some(value) = observe()? {
            return Ok(value);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(WhippleScriptError::unavailable(format!(
        "timed out waiting for {label}"
    )))
}

fn write_ndjson<T: Serialize>(value: &T) -> WhippleScriptResult<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer(&mut handle, value)
        .map_err(|error| WhippleScriptError::internal(error.to_string()))?;
    handle.write_all(b"\n")?;
    handle.flush()?;
    Ok(())
}

fn status_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> WhippleScriptResult<()> {
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

fn overview_workspace(
    workspace_arg: Option<PathBuf>,
    args: OverviewArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let config = load_workspace_config(&workspace)?;
    let store = SqliteStore::open(&workspace)?;
    let inspect = daemon_client(&workspace).inspect().ok();
    let runs = store.list_runs()?;
    let recent_events = store.list_events_filtered(&EventFilter {
        event_type: None,
        source: None,
        correlation: None,
        limit: Some(args.recent),
    })?;
    let recent_triggers = store.list_triggers_filtered(&TriggerFilter {
        task: None,
        event_type: None,
        outcome: None,
        correlation: None,
        limit: Some(args.recent),
    })?;
    let recent_failures = runs
        .iter()
        .filter(|run| run_failed(run))
        .take(args.recent)
        .map(run_summary)
        .collect::<Vec<_>>();
    let task_views = build_task_views(&config, inspect.as_ref());
    let service_views = build_service_views(&config, inspect.as_ref());
    let tasks = task_views
        .into_iter()
        .map(|task| {
            let latest_run = runs
                .iter()
                .find(|run| {
                    run.name == task.name && matches!(run.origin, whipplescript_core::RunOrigin::Task)
                })
                .map(run_summary);
            let latest_failure = runs
                .iter()
                .find(|run| {
                    run.name == task.name
                        && matches!(run.origin, whipplescript_core::RunOrigin::Task)
                        && run_failed(run)
                })
                .map(run_summary);
            TaskOverview {
                name: task.name,
                run: task.run,
                dynamic: task.dynamic,
                schedule: task.schedule,
                watch: task.watch,
                on: task.on,
                admission: task.admission,
                active_run_ids: task.active_run_ids,
                queued_triggers: task.queued_triggers,
                latest_run,
                latest_failure,
            }
        })
        .collect::<Vec<_>>();
    let services = service_views
        .into_iter()
        .map(|service| {
            let latest_run = runs
                .iter()
                .find(|run| {
                    run.name == service.name
                        && matches!(
                            run.origin,
                            whipplescript_core::RunOrigin::Service | whipplescript_core::RunOrigin::Restart
                        )
                })
                .map(run_summary);
            let latest_failure = runs
                .iter()
                .find(|run| {
                    run.name == service.name
                        && matches!(
                            run.origin,
                            whipplescript_core::RunOrigin::Service | whipplescript_core::RunOrigin::Restart
                        )
                        && run_failed(run)
                })
                .map(run_summary);
            ServiceOverview {
                name: service.name,
                run: service.run,
                enabled: service.enabled,
                dynamic: service.dynamic,
                restart: service.restart,
                state: service.state,
                active_run_id: service.active_run_id,
                stop_override: service.stop_override,
                health_state: service.health_state,
                last_error: service.last_error,
                latest_run,
                latest_failure,
            }
        })
        .collect::<Vec<_>>();
    let active_runs = inspect
        .as_ref()
        .map(|inspect| inspect.active_runs.iter().map(run_summary).collect())
        .unwrap_or_default();
    let output = OverviewOutput {
        workspace_root: workspace.root().to_path_buf(),
        config_path: workspace.config_path().to_path_buf(),
        config_version: config.version,
        daemon_running: inspect.is_some(),
        socket_path: inspect.as_ref().map(|inspect| inspect.socket_path.clone()),
        pid_path: inspect.as_ref().map(|inspect| inspect.pid_path.clone()),
        tasks,
        services,
        active_runs,
        recent_events,
        recent_triggers,
        recent_failures,
    };
    let text = format_overview_text(&output);
    print_data(args.output.apply(format), &output, &text)
}

fn ps_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> WhippleScriptResult<()> {
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

fn tasks_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> WhippleScriptResult<()> {
    tasks_workspace_filtered(workspace_arg, format, false)
}

fn tasks_workspace_filtered(
    workspace_arg: Option<PathBuf>,
    format: OutputFormat,
    dynamic_only: bool,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let config = load_workspace_config(&workspace)?;
    let inspect = daemon_client(&workspace).inspect()?;
    let tasks = build_task_views(&config, Some(&inspect))
        .into_iter()
        .filter(|task| !dynamic_only || task.dynamic)
        .collect::<Vec<_>>();
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

fn task_add(
    workspace_arg: Option<PathBuf>,
    args: TaskAddArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let env = parse_env_pairs(&args.env)?;
    let provenance = provenance_from_env(args.correlation.clone())?;
    daemon_client(&workspace).task_add(
        args.name.clone(),
        args.command.clone(),
        args.on.clone(),
        args.watch.clone(),
        args.schedule.clone(),
        args.settle.clone(),
        args.cwd.clone(),
        env.clone(),
        provenance.source_run_id.clone(),
        provenance.parent_event_id.clone(),
        provenance.correlation_id.clone(),
    )?;
    print_data(
        format,
        &json!({
            "task": args.name,
            "action": "added",
            "dynamic": true,
            "command": args.command,
            "on": args.on,
            "watch": args.watch,
            "schedule": args.schedule,
            "settle": args.settle,
            "cwd": args.cwd,
            "env": env,
            "source_run_id": provenance.source_run_id,
            "parent_event_id": provenance.parent_event_id,
            "correlation_id": provenance.correlation_id,
        }),
        "task added",
    )
}

fn task_show(
    workspace_arg: Option<PathBuf>,
    args: TaskNameArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let config = load_workspace_config(&workspace)?;
    let inspect = daemon_client(&workspace).inspect()?;
    let task = build_task_views(&config, Some(&inspect))
        .into_iter()
        .find(|task| task.name == args.name)
        .ok_or_else(|| WhippleScriptError::not_found(format!("task {:?} was not found", args.name)))?;
    let text = format!(
        "{}\nrun {}\nadmission {}\nactive_runs {}",
        task.name,
        task.run,
        task.admission,
        task.active_run_ids.len()
    );
    print_data(format, &task, &text)
}

fn services_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> WhippleScriptResult<()> {
    services_workspace_filtered(workspace_arg, format, false)
}

fn services_workspace_filtered(
    workspace_arg: Option<PathBuf>,
    format: OutputFormat,
    dynamic_only: bool,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let config = load_workspace_config(&workspace)?;
    let inspect = daemon_client(&workspace).inspect()?;
    let services = build_service_views(&config, Some(&inspect))
        .into_iter()
        .filter(|service| !dynamic_only || service.dynamic)
        .collect::<Vec<_>>();
    let text = if services.is_empty() {
        "no services configured".to_string()
    } else {
        services
            .iter()
            .map(|service| {
                let health = service.health_state.as_deref().unwrap_or("not_configured");
                format!(
                    "{}\tenabled={}\trestart={}\tstate={}\thealth={}",
                    service.name,
                    service.enabled,
                    service.restart,
                    service.state.as_deref().unwrap_or("not_running"),
                    health
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    print_data(format, &services, &text)
}

fn service_show(
    workspace_arg: Option<PathBuf>,
    args: ServiceShowArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let config = load_workspace_config(&workspace)?;
    let inspect = daemon_client(&workspace).inspect()?;
    let format = args.output.apply(format);
    let service = build_service_views(&config, Some(&inspect))
        .into_iter()
        .find(|service| service.name == args.name)
        .ok_or_else(|| {
            WhippleScriptError::not_found(format!("service {:?} was not found", args.name))
        })?;
    let text = format!(
        "{}\nrun {}\nenabled {}\nstate {}",
        service.name,
        service.run,
        service.enabled,
        service.state.as_deref().unwrap_or("not_running")
    );
    print_data(format, &service, &text)
}

fn service_add(
    workspace_arg: Option<PathBuf>,
    args: ServiceAddArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let env = parse_env_pairs(&args.env)?;
    let provenance = provenance_from_env(args.correlation.clone())?;
    daemon_client(&workspace).service_add(
        args.name.clone(),
        args.command.clone(),
        args.cwd.clone(),
        env.clone(),
        args.restart.into(),
        args.reason.clone(),
        provenance.source_run_id.clone(),
        provenance.parent_event_id.clone(),
        provenance.correlation_id.clone(),
    )?;
    print_data(
        format,
        &json!({
            "service": args.name,
            "action": "added",
            "dynamic": true,
            "command": args.command,
            "cwd": args.cwd,
            "env": env,
            "restart": format!("{:?}", RestartMode::from(args.restart)).to_lowercase(),
            "reason": args.reason,
            "source_run_id": provenance.source_run_id,
            "parent_event_id": provenance.parent_event_id,
            "correlation_id": provenance.correlation_id,
        }),
        "service added",
    )
}

fn service_command(
    workspace_arg: Option<PathBuf>,
    command: ServiceCommand,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    match command {
        ServiceCommand::List(args) => {
            return services_workspace_filtered(
                workspace_arg,
                args.output.apply(format),
                args.dynamic,
            )
        }
        ServiceCommand::Show(args) => return service_show(workspace_arg, args, format),
        ServiceCommand::Add(args) => return service_add(workspace_arg, args, format),
        _ => {}
    }
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let client = daemon_client(&workspace);
    let (name, action) = match command {
        ServiceCommand::List(_) | ServiceCommand::Show(_) => unreachable!(),
        ServiceCommand::Add(_) => unreachable!(),
        ServiceCommand::Remove(args) => {
            client.service_remove(args.name.clone())?;
            (args.name, "removed")
        }
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

fn runs_workspace(
    workspace_arg: Option<PathBuf>,
    args: RunsArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let store = SqliteStore::open(&workspace)?;
    let format = args.output.apply(format);
    let runs = store.list_runs_filtered(&RunFilter {
        name: args.name,
        origin: args.origin,
        state: args.state,
        correlation: args.correlation,
        limit: args.limit,
    })?;
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

fn run_show(
    workspace_arg: Option<PathBuf>,
    args: RunShowArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let run_id = RunId::parse(args.run_id)?;
    let store = SqliteStore::open(&workspace)?;
    let run = store.get_run(&run_id)?.ok_or_else(|| {
        WhippleScriptError::not_found(format!("run {} was not found", run_id.as_str()))
    })?;
    let text = format!(
        "{}\t{}\t{:?}\t{:?}",
        run.id.as_str(),
        run.name,
        run.origin,
        run.state
    );
    print_data(format, &run, &text)
}

fn logs_workspace(
    workspace_arg: Option<PathBuf>,
    args: LogsArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let run_id = RunId::parse(args.run_id)?;
    let store = SqliteStore::open(&workspace)?;
    let logs = store.get_logs(&run_id)?.ok_or_else(|| {
        WhippleScriptError::not_found(format!("logs for {} were not found", run_id.as_str()))
    })?;
    if args.follow && format == OutputFormat::Json {
        return Err(WhippleScriptError::invalid_input(
            "logs --follow streams text output and cannot be combined with JSON output",
        ));
    }
    let run = store.get_run(&run_id)?;
    let stdout = read_log_file(Path::new(&logs.stdout_path), args.tail)?;
    let stderr = read_log_file(Path::new(&logs.stderr_path), args.tail)?;
    let output = LogsOutput {
        run_id: run_id.as_str().to_string(),
        run_directory: run.as_ref().and_then(|record| record.run_directory.clone()),
        run,
        stdout_path: logs.stdout_path.clone(),
        stderr_path: logs.stderr_path.clone(),
        stdout_bytes: stdout.bytes,
        stderr_bytes: stderr.bytes,
        stdout_lines: stdout.lines,
        stderr_lines: stderr.lines,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        stdout_missing: stdout.missing,
        stderr_missing: stderr.missing,
        stdout: stdout.contents,
        stderr: stderr.contents,
    };
    let text = format_logs_text(&output, args.tail);
    if args.follow {
        println!("{text}");
        follow_logs_until_complete(
            &store,
            &run_id,
            Path::new(&logs.stdout_path),
            Path::new(&logs.stderr_path),
            output.stdout_bytes,
            output.stderr_bytes,
        )?;
        return Ok(());
    }
    print_data(format, &output, &text)
}

fn log_command(
    workspace_arg: Option<PathBuf>,
    command: LogCommand,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    match command {
        LogCommand::Show(args) => logs_workspace(workspace_arg, args, format),
        LogCommand::Tail(args) => logs_workspace(
            workspace_arg,
            LogsArgs {
                run_id: args.run_id,
                tail: Some(args.lines),
                follow: false,
            },
            format,
        ),
        LogCommand::Follow(args) => logs_workspace(
            workspace_arg,
            LogsArgs {
                run_id: args.run_id,
                tail: None,
                follow: true,
            },
            format,
        ),
    }
}

fn cancel_run(
    workspace_arg: Option<PathBuf>,
    args: CancelArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
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

fn doctor_workspace(workspace_arg: Option<PathBuf>, format: OutputFormat) -> WhippleScriptResult<()> {
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
) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace_arg(workspace_arg)?;
    let client = daemon_client(&workspace);

    match command {
        LockCommand::Acquire(args) => {
            let ttl = args
                .ttl
                .as_deref()
                .ok_or_else(|| {
                    WhippleScriptError::invalid_input(
                        "manual lock acquire requires --ttl so ownership remains inspectable",
                    )
                })
                .and_then(parse_duration)?;
            let record = client.acquire_lock(args.name, ttl, args.reason)?;
            print_data(format, &record, &format!("lock acquired {}", record.name))
        }
        LockCommand::Renew(args) => {
            let ttl = parse_duration(&args.ttl)?;
            let record = client.renew_lock(args.name, args.token, ttl)?;
            print_data(format, &record, &format!("lock renewed {}", record.name))
        }
        LockCommand::Release(args) => {
            let token = args.token.ok_or_else(|| {
                WhippleScriptError::invalid_input(
                    "manual lock release requires --token from lock acquire",
                )
            })?;
            client.release_lock(&args.name, token)?;
            print_data(
                format,
                &json!({
                    "released": true,
                    "name": args.name,
                }),
                "lock released",
            )
        }
        LockCommand::ForceRelease(args) => {
            if args.reason.trim().is_empty() {
                return Err(WhippleScriptError::invalid_input(
                    "lock force-release requires a non-empty --reason",
                ));
            }
            let lock = client.force_release_lock(args.name.clone(), args.reason.clone())?;
            print_data(
                format,
                &json!({
                    "forced": true,
                    "name": args.name,
                    "reason": args.reason,
                    "released": lock,
                }),
                "lock force-released",
            )
        }
        LockCommand::Show(args) => {
            let lock = client
                .locks()?
                .into_iter()
                .find(|lock| lock.name == args.name)
                .ok_or_else(|| {
                    WhippleScriptError::not_found(format!("lock {:?} is not held", args.name))
                })?;
            print_lock_show(format, &lock)
        }
        LockCommand::List(args) => {
            let locks = if args.expired {
                list_expired_locks(&workspace)?
            } else {
                client.locks()?
            };
            print_lock_list(format, &locks)
        }
        LockCommand::With(args) => lock_with(&client, args, format),
        LockCommand::Status => {
            let locks = client.locks()?;
            print_lock_list(format, &locks)
        }
    }
}

fn print_lock_show(format: OutputFormat, lock: &ManualLockRecord) -> WhippleScriptResult<()> {
    let text = format!(
        "{}\nowner {}\npid {}\nexpires_at {}\nreason {}",
        lock.name,
        lock.owner_id,
        lock.owner_pid,
        lock.expires_at_ms
            .map(timestamp_text)
            .unwrap_or_else(|| "none".to_string()),
        lock.reason.as_deref().unwrap_or("")
    );
    print_data(format, lock, &text)
}

fn print_lock_list(format: OutputFormat, locks: &[ManualLockRecord]) -> WhippleScriptResult<()> {
    let text = if locks.is_empty() {
        "no locks held".to_string()
    } else {
        locks
            .iter()
            .map(|lock| {
                format!(
                    "{}\towner={}\tpid={}\texpires_at={}\treason={}",
                    lock.name,
                    lock.owner_id,
                    lock.owner_pid,
                    lock.expires_at_ms
                        .map(timestamp_text)
                        .unwrap_or_else(|| "none".to_string()),
                    lock.reason.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    print_data(format, &locks, &text)
}

fn lock_with(
    client: &DaemonClient,
    args: LockWithArgs,
    format: OutputFormat,
) -> WhippleScriptResult<()> {
    let ttl = parse_duration(&args.ttl)?;
    if args.reason.trim().is_empty() {
        return Err(WhippleScriptError::invalid_input(
            "lock with requires a non-empty --reason",
        ));
    }
    let lock = client.acquire_lock(args.name.clone(), ttl, Some(args.reason.clone()))?;
    let status = run_locked_command(&args.command, &lock);
    let release_result = client.release_lock(&args.name, lock.token.clone());
    match (status, release_result) {
        (Ok(status), Ok(())) if status.success() => print_data(
            format,
            &json!({
                "name": args.name,
                "released": true,
                "exit_code": status.code(),
                "signal": status.signal(),
            }),
            "lock released",
        ),
        (Ok(status), Ok(())) => {
            if let Some(code) = status.code() {
                std::process::exit(code);
            }
            std::process::exit(128 + status.signal().unwrap_or(1));
        }
        (Ok(status), Err(error)) if status.success() => Err(error),
        (Ok(status), Err(error)) => {
            eprintln!("{error}");
            if let Some(code) = status.code() {
                std::process::exit(code);
            }
            std::process::exit(128 + status.signal().unwrap_or(1));
        }
        (Err(error), release_result) => {
            if let Err(release_error) = release_result {
                eprintln!("{release_error}");
            }
            Err(error)
        }
    }
}

fn run_locked_command(
    command: &[String],
    lock: &ManualLockRecord,
) -> WhippleScriptResult<std::process::ExitStatus> {
    let Some((program, args)) = command.split_first() else {
        return Err(WhippleScriptError::invalid_input("lock with requires a command"));
    };
    ProcessCommand::new(program)
        .args(args)
        .env("WHIPPLESCRIPT_LOCK_NAME", &lock.name)
        .env("WHIPPLESCRIPT_LOCK_TOKEN", &lock.token)
        .env("WHIPPLESCRIPT_LOCK_OWNER_ID", &lock.owner_id)
        .status()
        .map_err(|error| WhippleScriptError::internal(format!("failed to run locked command: {error}")))
}

fn list_expired_locks(workspace: &Workspace) -> WhippleScriptResult<Vec<ManualLockRecord>> {
    let runtime_paths = WorkspaceRuntimePaths::for_workspace(workspace)?;
    let lock_dir = runtime_paths.state_root().join("locks");
    let mut locks = Vec::new();
    if !lock_dir.exists() {
        return Ok(locks);
    }
    let now = now_millis();
    for entry in fs::read_dir(lock_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read(&path)?;
        let record = serde_json::from_slice::<ManualLockRecord>(&raw).map_err(|error| {
            WhippleScriptError::internal(format!("invalid lock record {}: {error}", path.display()))
        })?;
        if record
            .expires_at_ms
            .map(|expires_at| expires_at <= now)
            .unwrap_or(false)
        {
            locks.push(record);
        }
    }
    locks.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(locks)
}

fn serve_workspace(workspace_root: PathBuf) -> WhippleScriptResult<()> {
    let workspace = resolve_workspace(Some(&workspace_root), &workspace_root)?;
    let handle = DaemonServer::start(workspace)?;
    handle.join()
}

fn resolve_workspace_arg(workspace_arg: Option<PathBuf>) -> WhippleScriptResult<Workspace> {
    let cwd = std::env::current_dir()?;
    resolve_workspace(workspace_arg.as_ref(), &cwd)
}

fn daemon_client(workspace: &Workspace) -> DaemonClient {
    let runtime_paths = WorkspaceRuntimePaths::for_workspace(workspace)
        .expect("workspace runtime paths should resolve for a valid workspace");
    DaemonClient::from_socket_path(runtime_paths.socket_path())
}

fn install_shutdown_handler(client: DaemonClient) -> WhippleScriptResult<()> {
    let (sender, receiver) = mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })
    .map_err(|error| {
        WhippleScriptError::internal(format!("failed to install signal handler: {error}"))
    })?;
    std::thread::spawn(move || {
        let _ = receiver.recv();
        let _ = client.shutdown();
    });
    Ok(())
}

fn spawn_detached_daemon(workspace: &Workspace) -> WhippleScriptResult<()> {
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
        WhippleScriptError::internal(format!("failed to spawn detached daemon: {error}"))
    })?;
    Ok(())
}

fn wait_for_daemon(workspace: &Workspace, timeout: Duration) -> WhippleScriptResult<()> {
    let start = SystemTime::now();
    let client = daemon_client(workspace);
    loop {
        if client.inspect().is_ok() {
            return Ok(());
        }
        if elapsed_since(start) >= timeout {
            return Err(WhippleScriptError::unavailable(format!(
                "daemon did not become ready for workspace {}",
                workspace.root().display()
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_daemon_stop(workspace: &Workspace, timeout: Duration) -> WhippleScriptResult<()> {
    let start = SystemTime::now();
    let client = daemon_client(workspace);
    while elapsed_since(start) < timeout {
        if client.inspect().is_err() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(WhippleScriptError::unavailable(format!(
        "daemon did not stop for workspace {}",
        workspace.root().display()
    )))
}

fn build_task_views(config: &WhippleScriptConfig, inspect: Option<&InspectResponse>) -> Vec<TaskView> {
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
                dynamic: runtime.map(|runtime| runtime.dynamic).unwrap_or(false),
                cwd: runtime
                    .and_then(|runtime| runtime.cwd.as_ref())
                    .map(|cwd| cwd.display().to_string()),
                env: runtime
                    .map(|runtime| runtime.env.clone())
                    .unwrap_or_default(),
                created_by_run_id: runtime
                    .and_then(|runtime| runtime.created_by_run_id.as_ref())
                    .map(|run_id| run_id.as_str().to_string()),
                parent_event_id: runtime
                    .and_then(|runtime| runtime.parent_event_id.as_ref())
                    .map(|event_id| event_id.as_str().to_string()),
                correlation_id: runtime.and_then(|runtime| runtime.correlation_id.clone()),
                schedule: task.trigger.schedule.clone(),
                watch: task.trigger.watch.clone(),
                settle: task.trigger.settle.clone(),
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
    if let Some(inspect) = inspect {
        let existing_names = views
            .iter()
            .map(|view| view.name.clone())
            .collect::<HashSet<_>>();
        for runtime in inspect
            .tasks
            .iter()
            .filter(|runtime| runtime.dynamic && !existing_names.contains(&runtime.name))
        {
            views.push(TaskView {
                name: runtime.name.clone(),
                run: runtime.command.clone(),
                dynamic: true,
                cwd: runtime.cwd.as_ref().map(|cwd| cwd.display().to_string()),
                env: runtime.env.clone(),
                created_by_run_id: runtime
                    .created_by_run_id
                    .as_ref()
                    .map(|run_id| run_id.as_str().to_string()),
                parent_event_id: runtime
                    .parent_event_id
                    .as_ref()
                    .map(|event_id| event_id.as_str().to_string()),
                correlation_id: runtime.correlation_id.clone(),
                schedule: runtime.schedule.clone(),
                watch: runtime.watch.clone(),
                settle: runtime.settle.clone(),
                on: runtime.event_trigger.clone(),
                admission: runtime.admission.clone(),
                active_run_ids: runtime
                    .active_run_ids
                    .iter()
                    .map(|run_id| run_id.as_str().to_string())
                    .collect(),
                queued_triggers: runtime.queued_triggers,
                schedule_active: runtime.schedule_active,
                watch_active: runtime.watch_active,
            });
        }
    }
    views.sort_by(|left, right| left.name.cmp(&right.name));
    views
}

fn build_service_views(
    config: &WhippleScriptConfig,
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
                dynamic: runtime.map(|runtime| runtime.dynamic).unwrap_or(false),
                cwd: runtime
                    .and_then(|runtime| runtime.cwd.as_ref())
                    .map(|cwd| cwd.display().to_string()),
                env: runtime
                    .map(|runtime| runtime.env.clone())
                    .unwrap_or_default(),
                created_by_run_id: runtime
                    .and_then(|runtime| runtime.created_by_run_id.as_ref())
                    .map(|run_id| run_id.as_str().to_string()),
                parent_event_id: runtime
                    .and_then(|runtime| runtime.parent_event_id.as_ref())
                    .map(|event_id| event_id.as_str().to_string()),
                correlation_id: runtime.and_then(|runtime| runtime.correlation_id.clone()),
                reason: runtime.and_then(|runtime| runtime.reason.clone()),
                restart: format!("{:?}", service.supervision.restart).to_lowercase(),
                state: runtime.map(service_state_text),
                supervision_state: runtime.map(|runtime| runtime.supervision_state.clone()),
                active_run_id: runtime
                    .and_then(|runtime| runtime.active_run_id.as_ref())
                    .map(|run_id| run_id.as_str().to_string()),
                stop_override: runtime.map(|runtime| runtime.stop_override),
                last_error: runtime.and_then(|runtime| runtime.last_error.clone()),
                health_state: runtime
                    .and_then(|runtime| runtime.health.as_ref())
                    .map(|health| health.state.clone()),
                health_active_run_id: runtime
                    .and_then(|runtime| runtime.health.as_ref())
                    .and_then(|health| health.active_run_id.as_ref())
                    .map(|run_id| run_id.as_str().to_string()),
                health_last_run_id: runtime
                    .and_then(|runtime| runtime.health.as_ref())
                    .and_then(|health| health.last_run_id.as_ref())
                    .map(|run_id| run_id.as_str().to_string()),
                health_last_error: runtime
                    .and_then(|runtime| runtime.health.as_ref())
                    .and_then(|health| health.last_error.clone()),
            }
        })
        .collect::<Vec<_>>();
    if let Some(inspect) = inspect {
        let existing_names = views
            .iter()
            .map(|view| view.name.clone())
            .collect::<HashSet<_>>();
        for runtime in inspect
            .services
            .iter()
            .filter(|runtime| runtime.dynamic && !existing_names.contains(&runtime.name))
        {
            views.push(ServiceView {
                name: runtime.name.clone(),
                run: runtime.command.clone(),
                enabled: runtime.configured_enabled,
                dynamic: true,
                cwd: runtime.cwd.as_ref().map(|cwd| cwd.display().to_string()),
                env: runtime.env.clone(),
                created_by_run_id: runtime
                    .created_by_run_id
                    .as_ref()
                    .map(|run_id| run_id.as_str().to_string()),
                parent_event_id: runtime
                    .parent_event_id
                    .as_ref()
                    .map(|event_id| event_id.as_str().to_string()),
                correlation_id: runtime.correlation_id.clone(),
                reason: runtime.reason.clone(),
                restart: runtime.restart.clone(),
                state: Some(service_state_text(runtime)),
                supervision_state: Some(runtime.supervision_state.clone()),
                active_run_id: runtime
                    .active_run_id
                    .as_ref()
                    .map(|run_id| run_id.as_str().to_string()),
                stop_override: Some(runtime.stop_override),
                last_error: runtime.last_error.clone(),
                health_state: runtime.health.as_ref().map(|health| health.state.clone()),
                health_active_run_id: runtime
                    .health
                    .as_ref()
                    .and_then(|health| health.active_run_id.as_ref())
                    .map(|run_id| run_id.as_str().to_string()),
                health_last_run_id: runtime
                    .health
                    .as_ref()
                    .and_then(|health| health.last_run_id.as_ref())
                    .map(|run_id| run_id.as_str().to_string()),
                health_last_error: runtime
                    .health
                    .as_ref()
                    .and_then(|health| health.last_error.clone()),
            });
        }
    }
    views.sort_by(|left, right| left.name.cmp(&right.name));
    views
}

fn run_summary(run: &whipplescript_core::RunRecord) -> RunSummary {
    RunSummary {
        id: run.id.as_str().to_string(),
        name: run.name.clone(),
        command: run.command.clone(),
        origin: format!("{:?}", run.origin).to_lowercase(),
        state: format!("{:?}", run.state).to_lowercase(),
        start_time: run.start_time.clone(),
        end_time: run.end_time.clone(),
        exit_code: run.exit_code,
        signal: run.signal,
        killed: run.killed,
        event_id: run
            .event_id
            .as_ref()
            .map(|event_id| event_id.as_str().to_string()),
        stdout_path: run.stdout_path.clone(),
        stderr_path: run.stderr_path.clone(),
    }
}

fn run_failed(run: &whipplescript_core::RunRecord) -> bool {
    matches!(run.state, ProcessState::Failed)
        || run.exit_code.map(|code| code != 0).unwrap_or(false)
        || run.signal.is_some()
        || run.killed
}

fn format_overview_text(output: &OverviewOutput) -> String {
    let mut lines = vec![
        format!("workspace {}", output.workspace_root.display()),
        format!("config {}", output.config_version),
        format!(
            "daemon {}",
            if output.daemon_running {
                "running"
            } else {
                "not_running"
            }
        ),
    ];
    if let Some(socket_path) = &output.socket_path {
        lines.push(format!("socket {socket_path}"));
    }
    lines.push(String::new());
    lines.push("tasks".to_string());
    if output.tasks.is_empty() {
        lines.push("  none".to_string());
    } else {
        for task in &output.tasks {
            let triggers = overview_task_triggers(task);
            let latest = task
                .latest_run
                .as_ref()
                .map(format_run_inline)
                .unwrap_or_else(|| "none".to_string());
            let failure = task
                .latest_failure
                .as_ref()
                .map(format_run_inline)
                .unwrap_or_else(|| "none".to_string());
            lines.push(format!(
                "  {}\t{}\tactive={}\tqueued={}\tlatest={}\tfailure={}",
                task.name,
                triggers,
                task.active_run_ids.len(),
                task.queued_triggers,
                latest,
                failure
            ));
        }
    }
    lines.push(String::new());
    lines.push("services".to_string());
    if output.services.is_empty() {
        lines.push("  none".to_string());
    } else {
        for service in &output.services {
            let latest = service
                .latest_run
                .as_ref()
                .map(format_run_inline)
                .unwrap_or_else(|| "none".to_string());
            let failure = service
                .latest_failure
                .as_ref()
                .map(format_run_inline)
                .unwrap_or_else(|| "none".to_string());
            lines.push(format!(
                "  {}\tstate={}\trestart={}\tactive={}\tlatest={}\tfailure={}",
                service.name,
                service.state.as_deref().unwrap_or("not_running"),
                service.restart,
                service.active_run_id.as_deref().unwrap_or("none"),
                latest,
                failure
            ));
        }
    }
    lines.push(String::new());
    lines.push("active runs".to_string());
    if output.active_runs.is_empty() {
        lines.push("  none".to_string());
    } else {
        for run in &output.active_runs {
            lines.push(format!("  {}", format_run_inline(run)));
        }
    }
    lines.push(String::new());
    lines.push("recent failures".to_string());
    if output.recent_failures.is_empty() {
        lines.push("  none".to_string());
    } else {
        for run in &output.recent_failures {
            lines.push(format!("  {}", format_run_inline(run)));
        }
    }
    lines.push(String::new());
    lines.push("recent events".to_string());
    if output.recent_events.is_empty() {
        lines.push("  none".to_string());
    } else {
        for event in &output.recent_events {
            lines.push(format!(
                "  {}\t{}\tsource={}\tcorrelation={}",
                event.id.as_str(),
                event.event_type,
                event
                    .source
                    .as_deref()
                    .unwrap_or(whipplescript_core::model::DEFAULT_EVENT_SOURCE),
                event.correlation_id.as_deref().unwrap_or("none")
            ));
        }
    }
    lines.push(String::new());
    lines.push("recent triggers".to_string());
    if output.recent_triggers.is_empty() {
        lines.push("  none".to_string());
    } else {
        for trigger in &output.recent_triggers {
            lines.push(format!(
                "  {}\t{}\t{}\toutcome={}\trun={}",
                trigger.id.as_str(),
                trigger.task_name,
                trigger.event_type,
                format!("{:?}", trigger.outcome).to_lowercase(),
                trigger
                    .run_id
                    .as_ref()
                    .map(|run_id| run_id.as_str())
                    .unwrap_or("none")
            ));
        }
    }
    lines.join("\n")
}

fn overview_task_triggers(task: &TaskOverview) -> String {
    let mut triggers = Vec::new();
    if let Some(schedule) = &task.schedule {
        triggers.push(format!("schedule={schedule}"));
    }
    if let Some(event_type) = &task.on {
        triggers.push(format!("event={event_type}"));
    }
    if !task.watch.is_empty() {
        triggers.push(format!("watch={}", task.watch.len()));
    }
    if triggers.is_empty() {
        "manual".to_string()
    } else {
        triggers.join(",")
    }
}

fn format_run_inline(run: &RunSummary) -> String {
    let mut text = format!("{}:{}:{}", run.id, run.name, run.state);
    if let Some(exit_code) = run.exit_code {
        text.push_str(&format!(" exit={exit_code}"));
    }
    if let Some(signal) = run.signal {
        text.push_str(&format!(" signal={signal}"));
    }
    text
}

fn service_state_text(service: &RuntimeServiceStatus) -> String {
    format!("{:?}", service.state).to_lowercase()
}

fn read_log_file(path: &Path, tail: Option<usize>) -> WhippleScriptResult<LogFileSnapshot> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LogFileSnapshot {
                contents: String::new(),
                bytes: 0,
                lines: 0,
                truncated: false,
                missing: true,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let bytes = fs::metadata(path)?.len();
    let lines = contents.lines().count();
    let (contents, truncated) = match tail {
        Some(0) => (String::new(), lines > 0),
        Some(limit) => tail_lines(&contents, limit),
        None => (contents, false),
    };

    Ok(LogFileSnapshot {
        contents,
        bytes,
        lines,
        truncated,
        missing: false,
    })
}

fn tail_lines(contents: &str, limit: usize) -> (String, bool) {
    let lines = contents.lines().count();
    if lines <= limit {
        return (contents.to_string(), false);
    }

    let mut selected = contents
        .split_inclusive('\n')
        .rev()
        .take(limit)
        .collect::<Vec<_>>();
    selected.reverse();
    (selected.concat(), true)
}

fn format_logs_text(output: &LogsOutput, tail: Option<usize>) -> String {
    let mut lines = vec![format!("run {}", output.run_id)];
    if let Some(run) = &output.run {
        lines.push(format!("name: {}", run.name));
        lines.push(format!(
            "origin: {}  state: {}",
            format!("{:?}", run.origin).to_lowercase(),
            format!("{:?}", run.state).to_lowercase()
        ));
        lines.push(format!("command: {}", run.command));
        lines.push(format!("started: {}", run.start_time));
        if let Some(end_time) = &run.end_time {
            lines.push(format!("ended: {end_time}"));
        }
        if let Some(exit_code) = run.exit_code {
            lines.push(format!("exit_code: {exit_code}"));
        }
        if let Some(signal) = run.signal {
            lines.push(format!("signal: {signal}"));
        }
        if run.killed {
            lines.push("killed: true".to_string());
        }
        if let Some(config_version) = &run.config_version {
            lines.push(format!("config_version: {config_version}"));
        }
        if let Some(event_id) = &run.event_id {
            lines.push(format!("event_id: {}", event_id.as_str()));
        }
        if let Some(restart_of) = &run.restart_of {
            lines.push(format!("restart_of: {}", restart_of.as_str()));
        }
        if let Some(attempt) = run.attempt {
            lines.push(format!("attempt: {attempt}"));
        }
    }
    if let Some(run_directory) = &output.run_directory {
        lines.push(format!("run_directory: {run_directory}"));
    }
    if let Some(limit) = tail {
        lines.push(format!("tail: last {limit} lines per stream"));
    }

    lines.push(String::new());
    lines.push(format_log_stream_text(
        "stdout",
        &output.stdout_path,
        output.stdout_bytes,
        output.stdout_lines,
        output.stdout_truncated,
        output.stdout_missing,
        &output.stdout,
    ));
    lines.push(format_log_stream_text(
        "stderr",
        &output.stderr_path,
        output.stderr_bytes,
        output.stderr_lines,
        output.stderr_truncated,
        output.stderr_missing,
        &output.stderr,
    ));
    lines.join("\n")
}

fn format_log_stream_text(
    label: &str,
    path: &str,
    bytes: u64,
    lines: usize,
    truncated: bool,
    missing: bool,
    contents: &str,
) -> String {
    let mut output = format!("{label} {path} ({bytes} bytes, {lines} lines");
    if truncated {
        output.push_str(", truncated");
    }
    if missing {
        output.push_str(", missing");
    }
    output.push_str(")\n");
    if contents.is_empty() {
        output.push_str("(empty)\n");
    } else {
        output.push_str(contents);
        if !contents.ends_with('\n') {
            output.push('\n');
        }
    }
    output
}

fn follow_logs_until_complete(
    store: &SqliteStore,
    run_id: &RunId,
    stdout_path: &Path,
    stderr_path: &Path,
    mut stdout_offset: u64,
    mut stderr_offset: u64,
) -> WhippleScriptResult<()> {
    loop {
        std::thread::sleep(Duration::from_millis(200));
        stdout_offset = print_appended_log_bytes("stdout", stdout_path, stdout_offset)?;
        stderr_offset = print_appended_log_bytes("stderr", stderr_path, stderr_offset)?;

        let Some(run) = store.get_run(run_id)? else {
            return Ok(());
        };
        if !matches!(
            run.state,
            ProcessState::Starting | ProcessState::Running | ProcessState::Stopping
        ) {
            let _ = print_appended_log_bytes("stdout", stdout_path, stdout_offset)?;
            let _ = print_appended_log_bytes("stderr", stderr_path, stderr_offset)?;
            return Ok(());
        }
    }
}

fn print_appended_log_bytes(label: &str, path: &Path, offset: u64) -> WhippleScriptResult<u64> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(offset),
        Err(error) => return Err(error.into()),
    };
    if bytes.len() as u64 <= offset {
        return Ok(offset);
    }

    let appended = &bytes[offset as usize..];
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "--- {label} appended ---")?;
    stdout.write_all(appended)?;
    if !appended.ends_with(b"\n") {
        writeln!(stdout)?;
    }
    stdout.flush()?;
    Ok(bytes.len() as u64)
}

fn parse_duration(input: &str) -> WhippleScriptResult<Duration> {
    let trimmed = input.trim();
    let units = [("ms", 1_u64), ("s", 1_000), ("m", 60_000), ("h", 3_600_000)];
    for (suffix, multiplier) in units {
        if let Some(number) = trimmed.strip_suffix(suffix) {
            let value = number.trim().parse::<u64>().map_err(|error| {
                WhippleScriptError::invalid_input(format!("invalid duration {input:?}: {error}"))
            })?;
            return Ok(Duration::from_millis(value.saturating_mul(multiplier)));
        }
    }
    Err(WhippleScriptError::invalid_input(format!(
        "invalid duration {input:?}: expected suffix ms, s, m, or h"
    )))
}

fn elapsed_since(start: SystemTime) -> Duration {
    start.elapsed().unwrap_or_default()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn enum_text<T>(value: &T) -> WhippleScriptResult<String>
where
    T: Serialize,
{
    let encoded =
        serde_json::to_string(value).map_err(|error| WhippleScriptError::internal(error.to_string()))?;
    encoded
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .map(str::to_string)
        .ok_or_else(|| WhippleScriptError::internal("expected enum to serialize as a JSON string"))
}

fn timestamp_text(timestamp_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| timestamp_ms.to_string())
}

fn print_data<T>(format: OutputFormat, value: &T, text: &str) -> WhippleScriptResult<()>
where
    T: Serialize,
{
    match format {
        OutputFormat::Text => {
            println!("{text}");
        }
        OutputFormat::Json => {
            let encoded = serde_json::to_string_pretty(value)
                .map_err(|error| WhippleScriptError::internal(error.to_string()))?;
            println!("{encoded}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use whipplescript_core::{resolve_workspace, WorkspaceRuntimePaths};
    use clap::{CommandFactory, Parser};
    use tempfile::TempDir;

    use super::{
        doctor_workspace, init_recipe_workspace, overview_workspace, parse_duration,
        services_workspace, tasks_workspace, wait_for_daemon_stop, Cli, Command, JsonOutputArgs,
        OutputFormat, OverviewArgs, RecipeName, CONFIG_DIR_NAME, CONFIG_FILE_NAME,
    };

    #[test]
    fn cli_command_tree_builds() {
        Cli::command().debug_assert();
    }

    #[test]
    fn runtime_status_commands_accept_json_output_alias() {
        for command in [
            "overview", "status", "ps", "services", "runs", "events", "triggers",
        ] {
            let cli = Cli::try_parse_from(["whipplescript", command, "--json"]).unwrap();

            assert_eq!(cli.format, OutputFormat::Text);
            let output = match cli.command {
                Command::Overview(args) => args.output,
                Command::Status(args) | Command::Ps(args) | Command::Services(args) => args,
                Command::Runs(args) => args.output,
                Command::Events(args) => args.output,
                Command::Triggers(args) => args.output,
                other => panic!("unexpected command parsed for {command}: {other:?}"),
            };
            assert!(output.json);
            assert_eq!(output.apply(cli.format), OutputFormat::Json);
        }
    }

    #[test]
    fn global_format_json_still_controls_runtime_status_output() {
        let cli = Cli::try_parse_from(["whipplescript", "--format", "json", "status"]).unwrap();

        assert_eq!(cli.format, OutputFormat::Json);
        let Command::Status(args) = cli.command else {
            panic!("expected status command");
        };
        assert!(!args.json);
        assert_eq!(args.apply(cli.format), OutputFormat::Json);
    }

    #[test]
    fn emit_json_still_parses_payload_json() {
        let cli = Cli::try_parse_from([
            "whipplescript",
            "emit",
            "hook.example",
            "--json",
            "{\"ok\":true}",
        ])
        .unwrap();

        assert_eq!(cli.format, OutputFormat::Text);
        let Command::Emit(args) = cli.command else {
            panic!("expected emit command");
        };
        assert_eq!(args.event_type, "hook.example");
        assert_eq!(args.json.as_deref(), Some("{\"ok\":true}"));
        assert_eq!(args.payload_file, None);
        assert!(!args.stdin);
        assert_eq!(args.source, "cli");
    }

    #[test]
    fn emit_accepts_explicit_event_source() {
        let cli = Cli::try_parse_from(["whipplescript", "emit", "hook.example", "--source", "manual"])
            .unwrap();

        let Command::Emit(args) = cli.command else {
            panic!("expected emit command");
        };
        assert_eq!(args.source, "manual");
    }

    #[test]
    fn emit_accepts_payload_file_and_stdin_sources() {
        let cli = Cli::try_parse_from([
            "whipplescript",
            "emit",
            "hook.example",
            "--payload-file",
            "payload.json",
        ])
        .unwrap();

        let Command::Emit(args) = cli.command else {
            panic!("expected emit command");
        };
        assert_eq!(
            args.payload_file.as_deref(),
            Some(Path::new("payload.json"))
        );
        assert!(!args.stdin);

        let cli = Cli::try_parse_from(["whipplescript", "emit", "hook.example", "--stdin"]).unwrap();

        let Command::Emit(args) = cli.command else {
            panic!("expected emit command");
        };
        assert!(args.payload_file.is_none());
        assert!(args.stdin);
    }

    #[test]
    fn emit_payload_sources_are_mutually_exclusive() {
        assert!(Cli::try_parse_from([
            "whipplescript",
            "emit",
            "hook.example",
            "--json",
            "{}",
            "--stdin",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "whipplescript",
            "emit",
            "hook.example",
            "--json",
            "{}",
            "--payload-file",
            "payload.json",
        ])
        .is_err());
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
    fn tasks_requires_running_daemon() {
        let dir = TempDir::new().unwrap();
        write_workspace_config(
            dir.path(),
            "[[task]]\nname = \"test\"\nrun = \"echo test\"\n",
        );

        let error = tasks_workspace(Some(dir.path().to_path_buf()), OutputFormat::Json)
            .expect_err("tasks should require a live daemon");

        assert_eq!(error.kind.as_ref(), "unavailable");
        assert!(error.to_string().contains("failed to connect to daemon"));
    }

    #[test]
    fn services_requires_running_daemon() {
        let dir = TempDir::new().unwrap();
        write_workspace_config(
            dir.path(),
            "[[service]]\nname = \"worker\"\nrun = \"echo worker\"\n",
        );

        let error = services_workspace(Some(dir.path().to_path_buf()), OutputFormat::Json)
            .expect_err("services should require a live daemon");

        assert_eq!(error.kind.as_ref(), "unavailable");
        assert!(error.to_string().contains("failed to connect to daemon"));
    }

    #[test]
    fn doctor_remains_offline_tolerant() {
        let dir = TempDir::new().unwrap();
        write_workspace_config(
            dir.path(),
            "[[task]]\nname = \"test\"\nrun = \"echo test\"\n",
        );

        doctor_workspace(Some(dir.path().to_path_buf()), OutputFormat::Json).unwrap();
    }

    #[test]
    fn overview_remains_offline_tolerant() {
        let dir = TempDir::new().unwrap();
        write_workspace_config(
            dir.path(),
            "[[task]]\nname = \"test\"\nschedule = \"*/15 * * * *\"\nrun = \"echo test\"\n",
        );

        overview_workspace(
            Some(dir.path().to_path_buf()),
            OverviewArgs {
                output: JsonOutputArgs { json: true },
                recent: 5,
            },
            OutputFormat::Text,
        )
        .unwrap();
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
                "whip emit generic.event.tick",
            ),
            (
                RecipeName::EventHookTask,
                "scripts/on-hook-event.sh",
                "on = \"hook.example\"",
                "WHIPPLESCRIPT_EVENT_PAYLOAD_JSON",
            ),
            (
                RecipeName::NamedLock,
                "scripts/with-named-lock.sh",
                "name = \"with-named-lock\"",
                "lock acquire",
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

    #[test]
    fn wait_for_daemon_stop_succeeds_when_daemon_is_unreachable() {
        let dir = TempDir::new().unwrap();
        write_workspace_config(dir.path(), "");
        let workspace = resolve_workspace(Some(dir.path()), dir.path()).unwrap();

        wait_for_daemon_stop(&workspace, std::time::Duration::from_millis(10)).unwrap();
    }

    #[test]
    fn wait_for_daemon_stop_errors_when_daemon_stays_reachable() {
        let dir = TempDir::new().unwrap();
        write_workspace_config(dir.path(), "");
        let workspace = resolve_workspace(Some(dir.path()), dir.path()).unwrap();
        let runtime_paths = WorkspaceRuntimePaths::for_workspace(&workspace).unwrap();
        runtime_paths.ensure_state_root().unwrap();
        let socket_path = runtime_paths.socket_path();
        let _ = fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let stop_server = Arc::clone(&stop);
        let server = std::thread::spawn(move || {
            while !stop_server.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut request = String::new();
                        BufReader::new(stream.try_clone().unwrap())
                            .read_line(&mut request)
                            .unwrap();
                        stream
                            .write_all(
                                br#"{"status":"ok","payload":{"kind":"inspect","config_version":"0.3","socket_path":"","pid_path":"","services":[],"tasks":[],"active_runs":[]}}"#,
                            )
                            .unwrap();
                        stream.write_all(b"\n").unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(_) => break,
                }
            }
        });

        let error =
            wait_for_daemon_stop(&workspace, std::time::Duration::from_millis(30)).unwrap_err();
        stop.store(true, Ordering::SeqCst);
        server.join().unwrap();
        let _ = fs::remove_file(socket_path);

        assert_eq!(error.kind.as_ref(), "unavailable");
        assert!(error.message.contains("daemon did not stop"));
    }

    fn write_workspace_config(root: &std::path::Path, contents: &str) {
        let config_dir = root.join(CONFIG_DIR_NAME);
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join(CONFIG_FILE_NAME), contents).unwrap();
    }
}
