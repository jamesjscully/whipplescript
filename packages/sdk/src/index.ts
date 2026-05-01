import { execFile } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { Writable } from "node:stream";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

export type JsonPrimitive = null | boolean | number | string;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

export interface ArmatureEvent<TPayload extends JsonValue = JsonValue> {
  id?: string;
  type?: string;
  event_type: string;
  time?: string;
  payload: TPayload;
  routing?: string;
  config_version?: string | null;
  source?: string | null;
  source_run_id?: string | null;
  parent_event_id?: string | null;
  correlation_id?: string | null;
}

export interface RunContext {
  kind?: string;
  name?: string;
  runId?: string;
  runDirectory?: string;
  stdoutPath?: string;
  stderrPath?: string;
  configVersion?: string;
  eventId?: string;
  eventType?: string;
  eventJson?: string;
  eventPath?: string;
  correlationId?: string;
  workspace?: string;
}

export interface StatusSnapshot {
  workspace_root: string;
  config_path: string;
  config_version: string;
  socket_path: string;
  pid_path: string;
  services: number;
  tasks: number;
  active_runs: number;
}

export interface TaskStatus {
  name: string;
  run: string;
  schedule?: string;
  watch: string[];
  on?: string;
  admission: string;
  active_run_ids: string[];
  queued_triggers: number;
  schedule_active: boolean;
  watch_active: boolean;
}

export interface ServiceStatus {
  name: string;
  run: string;
  enabled: boolean;
  restart: string;
  state?: string;
  supervision_state?: string;
  active_run_id?: string | null;
  stop_override?: boolean;
  last_error?: string | null;
}

export interface RunRecord {
  id: string;
  name: string;
  command?: string;
  origin: string;
  state: string;
  start_time?: string;
  end_time?: string | null;
  exit_code?: number | null;
  signal?: number | null;
  killed?: boolean;
  config_version?: string | null;
  event_id?: string | null;
  restartOf?: string | null;
  attempt?: number | null;
  run_directory?: string | null;
  stdout_path?: string | null;
  stderr_path?: string | null;
}

export interface RunStartResult {
  run_id: string;
  task: string;
  correlation_id?: string | null;
}

export interface EmitResult<TPayload extends JsonValue = JsonValue> {
  emitted: true;
  type?: string;
  event_type: string;
  payload: TPayload;
  source?: string;
  source_run_id?: string | null;
  parent_event_id?: string | null;
  correlation_id?: string | null;
}

export interface ServiceCommandResult {
  service: string;
  action: "started" | "stopped" | "restarted";
}

export interface LogsResult {
  run_id: string;
  run?: RunRecord | null;
  run_directory?: string | null;
  stdout_path: string;
  stderr_path: string;
  stdout_bytes?: number;
  stderr_bytes?: number;
  stdout_lines?: number;
  stderr_lines?: number;
  stdout_truncated?: boolean;
  stderr_truncated?: boolean;
  stdout_missing?: boolean;
  stderr_missing?: boolean;
  stdout: string;
  stderr: string;
}

export interface CancelResult {
  cancelled: true;
  run_id: string;
}

export interface ConfigCheckResult {
  ok: true;
  workspace_root: string;
  config_path: string;
  config_version: string;
}

export interface DoctorResult {
  workspace_root: string;
  config_path: string;
  config_version?: string | null;
  config_error?: string | null;
  state_root: string;
  database_path: string;
  socket_path: string;
  pid_path: string;
  workspace_lock_path: string;
  daemon_running: boolean;
  daemon_error?: string | null;
  detached_stdout_path: string;
  detached_stderr_path: string;
}

export interface UpResult {
  started?: boolean;
  reloaded?: boolean;
  foreground?: boolean;
  mode?: string;
  workspace_root: string;
  socket_path?: string;
}

export interface DownResult {
  stopped: true;
  workspace_root: string;
}

export interface ManualLockRecord {
  name: string;
  owner_pid: number;
  owner_id: string;
  reason?: string | null;
  token: string;
  acquired_at_ms: number;
  renewed_at_ms?: number | null;
  expires_at_ms?: number | null;
  manual: boolean;
}

export interface LockReleaseResult {
  released: true;
  name: string;
}

export type StructuredLogEntry = Record<string, JsonValue>;

export interface ArmatureClientOptions {
  bin?: string;
  workspace?: string;
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  lockTtl?: string;
}

export interface WithLockOptions {
  ttl?: string;
  reason?: string;
}

export interface RunOptions {
  correlation?: string;
}

export interface EmitOptions {
  source?: string;
  correlation?: string;
}

export interface ListOptions {
  limit?: number;
}

export interface TaskListOptions {
  dynamic?: boolean;
}

export interface TaskAddOptions {
  on?: string;
  watch?: string[];
  schedule?: string;
  settle?: string;
  correlation?: string;
  cwd?: string;
  env?: Record<string, string>;
}

export interface TaskMutationResult {
  task: string;
  action: "added" | "removed";
  dynamic: true;
  command?: string[];
}

export interface RunListOptions extends ListOptions {
  name?: string;
  origin?: string;
  state?: string;
  correlation?: string;
}

export interface RunStartOptions {
  name: string;
  command: string[];
  correlation?: string;
  cwd?: string;
  env?: Record<string, string>;
  timeout?: string;
  payload?: JsonValue;
  payloadFile?: string;
  stdin?: boolean;
}

export interface AdhocRunStartResult {
  run_id: string;
  name: string;
  origin?: "adhoc" | string;
  correlation_id?: string | null;
}

export interface EventListOptions extends ListOptions {
  type?: string;
  source?: string;
  correlation?: string;
}

export interface TriggerRecord {
  id: string;
  task_name: string;
  event_type: string;
  outcome: string;
  run_id?: string | null;
  correlation_id?: string | null;
}

export interface TriggerListOptions extends ListOptions {
  task?: string;
  event?: string;
  outcome?: string;
  correlation?: string;
}

export interface WaitOptions {
  timeout: string;
}

export interface WaitEventOptions extends WaitOptions {
  correlation?: string;
}

export interface WaitRunOptions extends WaitOptions {
  state: string;
}

export interface WaitTriggerOptions extends WaitOptions {
  task?: string;
  event?: string;
  outcome?: string;
  correlation?: string;
}

export interface WaitServiceOptions extends WaitOptions {
  state: string;
}

export interface ServiceListOptions {
  dynamic?: boolean;
}

export interface ServiceAddOptions {
  correlation?: string;
  cwd?: string;
  env?: Record<string, string>;
  restart?: "never" | "on_failure" | "always";
  reason?: string;
}

export interface ServiceMutationResult {
  service: string;
  action: "added" | "removed" | "started" | "stopped" | "restarted";
  dynamic?: true;
  command?: string[];
}

export interface LockListOptions {
  expired?: boolean;
}

export interface LockForceReleaseResult {
  forced: true;
  name: string;
  reason: string;
  released?: ManualLockRecord | null;
}

export interface LockWithCommandOptions {
  ttl: string;
  reason: string;
}

type RunNamespace = {
  (taskName: string, options?: RunOptions): Promise<RunStartResult>;
  start(options: RunStartOptions): Promise<AdhocRunStartResult>;
  list(options?: RunListOptions): Promise<RunRecord[]>;
  show(runId: string): Promise<RunRecord>;
  logs(runId: string): Promise<LogsResult>;
  cancel(runId: string): Promise<CancelResult>;
};

type LockNamespace = {
  (name: string, ttl?: string, reason?: string): Promise<ManualLockRecord>;
  acquire(name: string, ttl?: string, reason?: string): Promise<ManualLockRecord>;
  renew(name: string, token: string, ttl: string): Promise<ManualLockRecord>;
  release(name: string, token: string): Promise<LockReleaseResult>;
  forceRelease(name: string, reason: string): Promise<LockForceReleaseResult>;
  show(name: string): Promise<ManualLockRecord>;
  list(options?: LockListOptions): Promise<ManualLockRecord[]>;
  withCommand(name: string, command: string[], options: LockWithCommandOptions): Promise<LockReleaseResult>;
};

export class ArmatureSdkError extends Error {
  readonly kind: string;
  readonly details?: Record<string, JsonValue>;

  constructor(kind: string, message: string, details?: Record<string, JsonValue>) {
    super(message);
    this.name = "ArmatureSdkError";
    this.kind = kind;
    this.details = details;
  }
}

function parseJson<T>(raw: string, context: string): T {
  try {
    return JSON.parse(raw) as T;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new ArmatureSdkError("invalid_json", `failed to parse ${context}: ${message}`, {
      context,
      raw
    });
  }
}

function resolveEventJson(env: NodeJS.ProcessEnv): string {
  const inline = env.ARMATURE_EVENT_JSON ?? env.ARMATURE_EVENT;
  if (inline) {
    return inline;
  }

  const path = env.ARMATURE_EVENT_PATH;
  if (path && existsSync(path)) {
    return readFileSync(path, "utf8");
  }

  throw new ArmatureSdkError(
    "missing_env",
    "missing Armature event payload; expected ARMATURE_EVENT_JSON or ARMATURE_EVENT_PATH"
  );
}

async function runCli<T>(args: string[], options: ArmatureClientOptions): Promise<T> {
  const command = options.bin ?? process.env.ARMATURE_BIN ?? "armature";
  const commandArgs = ["--format", "json"];

  if (options.workspace) {
    commandArgs.push("--workspace", options.workspace);
  }

  commandArgs.push(...args);

  try {
    const { stdout } = await execFileAsync(command, commandArgs, {
      cwd: options.cwd,
      env: options.env,
      encoding: "utf8"
    });
    return parseJson<T>(stdout, `CLI response for ${args.join(" ")}`);
  } catch (error) {
    if (error && typeof error === "object" && "stdout" in error) {
      const commandError = error as {
        code?: number | string;
        stdout?: string;
        stderr?: string;
        message?: string;
      };
      throw new ArmatureSdkError("cli_failed", commandError.message ?? "armature CLI call failed", {
        command,
        args: commandArgs,
        code: commandError.code == null ? null : String(commandError.code),
        stdout: commandError.stdout ?? "",
        stderr: commandError.stderr ?? ""
      });
    }

    const message = error instanceof Error ? error.message : String(error);
    throw new ArmatureSdkError("transport", `failed to execute ${command}: ${message}`, {
      command,
      args: commandArgs
    });
  }
}

function pushOptions(args: string[], values: object): void {
  for (const [key, value] of Object.entries(values)) {
    if (value === undefined || value === false) {
      continue;
    }
    args.push(`--${key}`);
    if (value !== true) {
      args.push(String(value));
    }
  }
}

function pushEnv(args: string[], env?: Record<string, string>): void {
  for (const [key, value] of Object.entries(env ?? {})) {
    args.push("--env", `${key}=${value}`);
  }
}

export class ArmatureClient {
  readonly options: ArmatureClientOptions;
  readonly task: {
    list: (options?: TaskListOptions) => Promise<TaskStatus[]>;
    show: (name: string) => Promise<TaskStatus>;
    run: (name: string, options?: RunOptions) => Promise<RunStartResult>;
    add: (name: string, command: string[], options?: TaskAddOptions) => Promise<TaskMutationResult>;
    remove: (name: string) => Promise<TaskMutationResult>;
  };
  readonly run: RunNamespace;
  readonly event: {
    list: (options?: EventListOptions) => Promise<ArmatureEvent[]>;
    show: (eventId: string) => Promise<ArmatureEvent>;
    emit: <TPayload extends JsonValue = JsonValue>(
      eventType: string,
      payload?: TPayload,
      options?: EmitOptions
    ) => Promise<EmitResult<TPayload>>;
  };
  readonly trigger: {
    list: (options?: TriggerListOptions) => Promise<TriggerRecord[]>;
    show: (triggerId: string) => Promise<TriggerRecord>;
  };
  readonly wait: {
    event: (eventType: string, options: WaitEventOptions) => Promise<ArmatureEvent>;
    run: (runId: string, options: WaitRunOptions) => Promise<RunRecord>;
    trigger: (options: WaitTriggerOptions) => Promise<TriggerRecord>;
    service: (name: string, options: WaitServiceOptions) => Promise<ServiceStatus>;
  };
  readonly service: {
    list: (options?: ServiceListOptions) => Promise<ServiceStatus[]>;
    show: (name: string) => Promise<ServiceStatus>;
    add: (name: string, command: string[], options?: ServiceAddOptions) => Promise<ServiceMutationResult>;
    remove: (name: string) => Promise<ServiceMutationResult>;
    start: (name: string) => Promise<ServiceCommandResult>;
    stop: (name: string) => Promise<ServiceCommandResult>;
    restart: (name: string) => Promise<ServiceCommandResult>;
  };
  readonly lock: LockNamespace;

  constructor(options: ArmatureClientOptions = {}) {
    this.options = { ...options };
    this.task = {
      list: (taskOptions = {}) => this.listTasks(taskOptions),
      show: (name) => runCli<TaskStatus>(["task", "show", name], this.options),
      run: (name, runOptions = {}) => this.runTask(name, runOptions),
      add: (name, command, taskOptions = {}) => this.addTask(name, command, taskOptions),
      remove: (name) => runCli<TaskMutationResult>(["task", "remove", name], this.options)
    };
    this.run = Object.assign(
      (taskName: string, runOptions: RunOptions = {}) => this.runTask(taskName, runOptions),
      {
        start: (runOptions: RunStartOptions) => this.startRun(runOptions),
        list: (runOptions: RunListOptions = {}) => this.listRuns(runOptions),
        show: (runId: string) => runCli<RunRecord>(["run", "show", runId], this.options),
        logs: (runId: string) => runCli<LogsResult>(["run", "logs", runId], this.options),
        cancel: (runId: string) => runCli<CancelResult>(["run", "cancel", runId], this.options)
      }
    );
    this.event = {
      list: (eventOptions = {}) => this.listEvents(eventOptions),
      show: (eventId) => runCli<ArmatureEvent>(["event", "show", eventId], this.options),
      emit: <TPayload extends JsonValue = JsonValue>(
        eventType: string,
        payload: TPayload = {} as TPayload,
        emitOptions: EmitOptions = {}
      ) => this.emit(eventType, payload, emitOptions)
    };
    this.trigger = {
      list: (triggerOptions = {}) => this.listTriggers(triggerOptions),
      show: (triggerId) => runCli<TriggerRecord>(["trigger", "show", triggerId], this.options)
    };
    this.wait = {
      event: (eventType, waitOptions) => this.waitEvent(eventType, waitOptions),
      run: (runId, waitOptions) => runCli<RunRecord>(
        ["wait", "run", runId, "--state", waitOptions.state, "--timeout", waitOptions.timeout],
        this.options
      ),
      trigger: (waitOptions) => this.waitTrigger(waitOptions),
      service: (name, waitOptions) => runCli<ServiceStatus>(
        ["wait", "service", name, "--state", waitOptions.state, "--timeout", waitOptions.timeout],
        this.options
      )
    };
    this.service = {
      list: (serviceOptions = {}) => this.listServices(serviceOptions),
      show: (name) => runCli<ServiceStatus>(["service", "show", name], this.options),
      add: (name, command, serviceOptions = {}) => this.addService(name, command, serviceOptions),
      remove: (name) => runCli<ServiceMutationResult>(["service", "remove", name], this.options),
      start: (name) => this.startService(name),
      stop: (name) => this.stopService(name),
      restart: (name) => this.restartService(name)
    };
    this.lock = Object.assign(
      (name: string, ttl = this.options.lockTtl ?? "5m", reason?: string) =>
        this.acquireLock(name, ttl, reason),
      {
        acquire: (name: string, ttl = this.options.lockTtl ?? "5m", reason?: string) =>
          this.acquireLock(name, ttl, reason),
        renew: (name: string, token: string, ttl: string) => this.renewLock(name, token, ttl),
        release: (name: string, token: string) => this.unlock(name, token),
        forceRelease: (name: string, reason: string) =>
          runCli<LockForceReleaseResult>(["lock", "force-release", name, "--reason", reason], this.options),
        show: (name: string) => runCli<ManualLockRecord>(["lock", "show", name], this.options),
        list: (lockOptions: LockListOptions = {}) => this.listLocks(lockOptions),
        withCommand: (name: string, command: string[], lockOptions: LockWithCommandOptions) =>
          runCli<LockReleaseResult>(
            ["lock", "with", name, "--ttl", lockOptions.ttl, "--reason", lockOptions.reason, ...command],
            this.options
          )
      }
    );
  }

  up(): Promise<UpResult> {
    return runCli<UpResult>(["up"], this.options);
  }

  down(): Promise<DownResult> {
    return runCli<DownResult>(["down"], this.options);
  }

  restart(): Promise<UpResult> {
    return runCli<UpResult>(["restart"], this.options);
  }

  private runTask(taskName: string, options: RunOptions = {}): Promise<RunStartResult> {
    const args = ["run", taskName];
    if (options.correlation) {
      args.push("--correlation", options.correlation);
    }
    return runCli<RunStartResult>(args, this.options);
  }

  emit<TPayload extends JsonValue = JsonValue>(
    eventType: string,
    payload: TPayload = {} as TPayload,
    options: EmitOptions = {}
  ): Promise<EmitResult<TPayload>> {
    const args = ["emit", eventType];
    if (options.source) {
      args.push("--source", options.source);
    }
    if (options.correlation) {
      args.push("--correlation", options.correlation);
    }
    args.push("--json", JSON.stringify(payload));
    return runCli<EmitResult<TPayload>>(args, this.options);
  }

  status(): Promise<StatusSnapshot> {
    return runCli<StatusSnapshot>(["status"], this.options);
  }

  inspect(): Promise<StatusSnapshot> {
    return this.status();
  }

  tasks(): Promise<TaskStatus[]> {
    return this.task.list();
  }

  services(): Promise<ServiceStatus[]> {
    return this.service.list();
  }

  startService(name: string): Promise<ServiceCommandResult> {
    return runCli<ServiceCommandResult>(["service", "start", name], this.options);
  }

  stopService(name: string): Promise<ServiceCommandResult> {
    return runCli<ServiceCommandResult>(["service", "stop", name], this.options);
  }

  restartService(name: string): Promise<ServiceCommandResult> {
    return runCli<ServiceCommandResult>(["service", "restart", name], this.options);
  }

  runs(): Promise<RunRecord[]> {
    return this.run.list();
  }

  logs(runId: string): Promise<LogsResult> {
    return this.run.logs(runId);
  }

  cancel(runId: string): Promise<CancelResult> {
    return this.run.cancel(runId);
  }

  configCheck(): Promise<ConfigCheckResult> {
    return runCli<ConfigCheckResult>(["config", "check"], this.options);
  }

  doctor(): Promise<DoctorResult> {
    return runCli<DoctorResult>(["doctor"], this.options);
  }

  private acquireLock(
    name: string,
    ttl = this.options.lockTtl ?? "5m",
    reason?: string
  ): Promise<ManualLockRecord> {
    const args = ["lock", "acquire", name, "--ttl", ttl];
    if (reason) {
      args.push("--reason", reason);
    }
    return runCli<ManualLockRecord>(args, this.options);
  }

  renewLock(name: string, token: string, ttl: string): Promise<ManualLockRecord> {
    return runCli<ManualLockRecord>(["lock", "renew", name, "--token", token, "--ttl", ttl], this.options);
  }

  unlock(name: string, token: string): Promise<LockReleaseResult> {
    return runCli<LockReleaseResult>(["lock", "release", name, "--token", token], this.options);
  }

  locks(): Promise<ManualLockRecord[]> {
    return this.lock.list();
  }

  async withLock<T>(
    name: string,
    fn: () => Promise<T> | T,
    options: WithLockOptions = {}
  ): Promise<T> {
    const lock = await this.lock.acquire(name, options.ttl, options.reason);
    try {
      return await fn();
    } finally {
      await this.unlock(name, lock.token);
    }
  }

  private listTasks(options: TaskListOptions): Promise<TaskStatus[]> {
    const args = ["task", "list"];
    pushOptions(args, { dynamic: options.dynamic });
    return runCli<TaskStatus[]>(args, this.options);
  }

  private addTask(name: string, command: string[], options: TaskAddOptions): Promise<TaskMutationResult> {
    const args = ["task", "add", name];
    pushOptions(args, {
      on: options.on,
      schedule: options.schedule,
      settle: options.settle,
      correlation: options.correlation,
      cwd: options.cwd
    });
    for (const watch of options.watch ?? []) {
      args.push("--watch", watch);
    }
    pushEnv(args, options.env);
    args.push(...command);
    return runCli<TaskMutationResult>(args, this.options);
  }

  private startRun(options: RunStartOptions): Promise<AdhocRunStartResult> {
    const args = ["run", "start", "--name", options.name];
    pushOptions(args, {
      correlation: options.correlation,
      cwd: options.cwd,
      timeout: options.timeout,
      stdin: options.stdin
    });
    pushEnv(args, options.env);
    if (options.payload !== undefined) {
      args.push("--json", JSON.stringify(options.payload));
    }
    if (options.payloadFile) {
      args.push("--payload-file", options.payloadFile);
    }
    args.push(...options.command);
    return runCli<AdhocRunStartResult>(args, this.options);
  }

  private listRuns(options: RunListOptions): Promise<RunRecord[]> {
    const args = ["run", "list"];
    pushOptions(args, options);
    return runCli<RunRecord[]>(args, this.options);
  }

  private listEvents(options: EventListOptions): Promise<ArmatureEvent[]> {
    const args = ["event", "list"];
    const { type, ...rest } = options;
    pushOptions(args, { ...rest, type });
    return runCli<ArmatureEvent[]>(args, this.options);
  }

  private listTriggers(options: TriggerListOptions): Promise<TriggerRecord[]> {
    const args = ["trigger", "list"];
    pushOptions(args, options);
    return runCli<TriggerRecord[]>(args, this.options);
  }

  private waitEvent(eventType: string, options: WaitEventOptions): Promise<ArmatureEvent> {
    const args = ["wait", "event", eventType];
    pushOptions(args, { correlation: options.correlation, timeout: options.timeout });
    return runCli<ArmatureEvent>(args, this.options);
  }

  private waitTrigger(options: WaitTriggerOptions): Promise<TriggerRecord> {
    const args = ["wait", "trigger"];
    pushOptions(args, {
      task: options.task,
      event: options.event,
      outcome: options.outcome,
      correlation: options.correlation,
      timeout: options.timeout
    });
    return runCli<TriggerRecord>(args, this.options);
  }

  private listServices(options: ServiceListOptions): Promise<ServiceStatus[]> {
    const args = ["service", "list"];
    pushOptions(args, { dynamic: options.dynamic });
    return runCli<ServiceStatus[]>(args, this.options);
  }

  private addService(
    name: string,
    command: string[],
    options: ServiceAddOptions
  ): Promise<ServiceMutationResult> {
    const args = ["service", "add", name];
    pushOptions(args, {
      correlation: options.correlation,
      cwd: options.cwd,
      restart: options.restart,
      reason: options.reason
    });
    pushEnv(args, options.env);
    args.push(...command);
    return runCli<ServiceMutationResult>(args, this.options);
  }

  private listLocks(options: LockListOptions): Promise<ManualLockRecord[]> {
    const args = ["lock", "list"];
    pushOptions(args, { expired: options.expired });
    return runCli<ManualLockRecord[]>(args, this.options);
  }
}

export function createArmature(options: ArmatureClientOptions = {}): ArmatureClient {
  return new ArmatureClient(options);
}

export const armature = createArmature();

export function getRunContext(env: NodeJS.ProcessEnv = process.env): RunContext {
  return {
    kind: env.ARMATURE_KIND,
    name: env.ARMATURE_NAME,
    runId: env.ARMATURE_RUN_ID,
    runDirectory: env.ARMATURE_RUN_DIR,
    stdoutPath: env.ARMATURE_STDOUT_LOG,
    stderrPath: env.ARMATURE_STDERR_LOG,
    configVersion: env.ARMATURE_CONFIG_VERSION,
    eventId: env.ARMATURE_EVENT_ID,
    eventType: env.ARMATURE_EVENT_TYPE,
    eventJson: env.ARMATURE_EVENT_JSON ?? env.ARMATURE_EVENT,
    eventPath: env.ARMATURE_EVENT_PATH,
    correlationId: env.ARMATURE_CORRELATION_ID,
    workspace: env.ARMATURE_WORKSPACE ?? env.ARMATURE_WORKSPACE_ROOT
  };
}

export function getEvent<TPayload extends JsonValue = JsonValue>(
  env: NodeJS.ProcessEnv = process.env
): ArmatureEvent<TPayload> {
  return parseJson<ArmatureEvent<TPayload>>(resolveEventJson(env), "Armature event");
}

export function getPayload<TPayload extends JsonValue = JsonValue>(
  env: NodeJS.ProcessEnv = process.env
): TPayload {
  return getEvent<TPayload>(env).payload;
}

export function log(entry: StructuredLogEntry, stream: Writable = process.stdout): void {
  stream.write(`${JSON.stringify(entry)}\n`);
}

export async function readJson<T>(path: string): Promise<T> {
  return parseJson<T>(await readFile(path, "utf8"), path);
}

export async function writeJson(path: string, value: JsonValue): Promise<void> {
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

export async function emit<TPayload extends JsonValue = JsonValue>(
  eventType: string,
  payload: TPayload = {} as TPayload,
  options: EmitOptions = {}
): Promise<EmitResult<TPayload>> {
  return armature.emit(eventType, payload, options);
}

export async function run(taskName: string, options: RunOptions = {}): Promise<RunStartResult> {
  return armature.run(taskName, options);
}

export async function status(): Promise<StatusSnapshot> {
  return armature.status();
}

export async function tasks(): Promise<TaskStatus[]> {
  return armature.tasks();
}

export async function services(): Promise<ServiceStatus[]> {
  return armature.services();
}

export async function runs(): Promise<RunRecord[]> {
  return armature.runs();
}

export async function logs(runId: string): Promise<LogsResult> {
  return armature.logs(runId);
}

export async function cancel(runId: string): Promise<CancelResult> {
  return armature.cancel(runId);
}

export async function lock(name: string, ttl?: string, reason?: string): Promise<ManualLockRecord> {
  return armature.lock(name, ttl, reason);
}

export async function renewLock(name: string, token: string, ttl: string): Promise<ManualLockRecord> {
  return armature.renewLock(name, token, ttl);
}

export async function unlock(name: string, token: string): Promise<LockReleaseResult> {
  return armature.unlock(name, token);
}

export async function locks(): Promise<ManualLockRecord[]> {
  return armature.locks();
}

export async function withLock<T>(
  name: string,
  fn: () => Promise<T> | T,
  options: WithLockOptions = {}
): Promise<T> {
  return armature.withLock(name, fn, options);
}
