import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { readFile, writeFile } from "node:fs/promises";

const execFileAsync = promisify(execFile);

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

export interface RunContext {
  runId?: string;
  eventId?: string;
  workspaceRoot?: string;
  runDirectory?: string;
  stdoutPath?: string;
  stderrPath?: string;
  configVersion?: string;
  taskName?: string;
  serviceName?: string;
}

export interface ArmatureEvent<TPayload extends JsonValue = JsonValue> {
  id?: string;
  type: string;
  payload: TPayload;
  source?: string;
  configVersion?: string;
}

export interface TaskDefinition {
  name: string;
  run: string;
}

export interface ServiceDefinition {
  name: string;
  run: string;
  enabled: boolean;
}

export interface RunRecord {
  id: string;
  name: string;
  state: string;
}

export interface LogRecord {
  runId: string;
  stdoutPath: string;
  stderrPath: string;
}

export interface RuntimeSnapshot {
  tasks: TaskDefinition[];
  services: ServiceDefinition[];
  activeRuns: RunRecord[];
}

export interface LockResult {
  ok: boolean;
  name: string;
}

export class ArmatureSdkError extends Error {
  readonly kind: string;

  constructor(kind: string, message: string) {
    super(message);
    this.name = "ArmatureSdkError";
    this.kind = kind;
  }
}

function getRequiredEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new ArmatureSdkError("missing_env", `missing required environment variable ${name}`);
  }
  return value;
}

async function runCli<T>(args: string[]): Promise<T> {
  const command = process.env.ARMATURE_BIN ?? "armature";
  try {
    const { stdout } = await execFileAsync(command, args, { encoding: "utf8" });
    return stdout.length > 0 ? (JSON.parse(stdout) as T) : (undefined as T);
  } catch (error) {
    throw new ArmatureSdkError("transport", `armature CLI call failed for ${args.join(" ")}`);
  }
}

export function getRunContext(): RunContext {
  return {
    runId: process.env.ARMATURE_RUN_ID,
    eventId: process.env.ARMATURE_EVENT_ID,
    workspaceRoot: process.env.ARMATURE_WORKSPACE_ROOT,
    runDirectory: process.env.ARMATURE_RUN_DIR,
    stdoutPath: process.env.ARMATURE_STDOUT_LOG,
    stderrPath: process.env.ARMATURE_STDERR_LOG,
    configVersion: process.env.ARMATURE_CONFIG_VERSION,
    taskName: process.env.ARMATURE_TASK_NAME,
    serviceName: process.env.ARMATURE_SERVICE_NAME
  };
}

export function getEvent<TPayload extends JsonValue = JsonValue>(): ArmatureEvent<TPayload> {
  const raw = getRequiredEnv("ARMATURE_EVENT");
  return JSON.parse(raw) as ArmatureEvent<TPayload>;
}

export function getPayload<TPayload extends JsonValue = JsonValue>(): TPayload {
  return getEvent<TPayload>().payload;
}

export async function emit<TPayload extends JsonValue = JsonValue>(
  eventType: string,
  payload: TPayload
): Promise<void> {
  await runCli<void>(["emit", eventType, "--json", JSON.stringify(payload)]);
}

export async function run(taskName: string): Promise<RunRecord> {
  return runCli<RunRecord>(["run", taskName]);
}

export async function status(): Promise<RuntimeSnapshot> {
  return runCli<RuntimeSnapshot>(["status"]);
}

export async function tasks(): Promise<TaskDefinition[]> {
  return runCli<TaskDefinition[]>(["tasks"]);
}

export async function services(): Promise<ServiceDefinition[]> {
  return runCli<ServiceDefinition[]>(["services"]);
}

export async function runs(): Promise<RunRecord[]> {
  return runCli<RunRecord[]>(["runs"]);
}

export async function logs(runId: string): Promise<LogRecord> {
  return runCli<LogRecord>(["logs", runId]);
}

export async function cancel(runId: string): Promise<void> {
  await runCli<void>(["cancel", runId]);
}

export async function lock(name: string): Promise<LockResult> {
  return runCli<LockResult>(["lock", "acquire", name]);
}

export async function unlock(name: string): Promise<LockResult> {
  return runCli<LockResult>(["lock", "release", name]);
}

export async function withLock<T>(name: string, fn: () => Promise<T>): Promise<T> {
  await lock(name);
  try {
    return await fn();
  } finally {
    await unlock(name);
  }
}

export function log(entry: Record<string, JsonValue>): void {
  process.stdout.write(`${JSON.stringify(entry)}\n`);
}

export async function readJson<T>(path: string): Promise<T> {
  return JSON.parse(await readFile(path, "utf8")) as T;
}

export async function writeJson(path: string, value: JsonValue): Promise<void> {
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

