import assert from "node:assert/strict";
import { chmod, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { PassThrough } from "node:stream";

import {
  ArmatureSdkError,
  armature,
  createArmature,
  emit,
  getEvent,
  getPayload,
  getRunContext,
  lock,
  locks,
  log,
  readJson,
  renewLock,
  run,
  status,
  tasks,
  unlock,
  withLock,
  writeJson
} from "./index.js";

async function createStubArmature() {
  const directory = await mkdtemp(join(tmpdir(), "armature-sdk-cli-"));
  const callsPath = join(directory, "calls.jsonl");
  const binPath = join(directory, "armature");

const source = `#!/usr/bin/env node
const fs = require("node:fs");
const send = (value) => fs.writeSync(1, JSON.stringify(value) + "\\n");

const args = process.argv.slice(2);
const callsPath = process.env.CALLS_PATH;
fs.appendFileSync(callsPath, JSON.stringify(args) + "\\n", "utf8");

const filtered = args.filter((arg, index) => {
  if (arg === "--format" || arg === "--workspace") {
    return false;
  }
  if (index > 0 && (args[index - 1] === "--format" || args[index - 1] === "--workspace")) {
    return false;
  }
  return true;
});

const command = filtered.join(" ");

if (command === "status") {
  send({
    workspace_root: "/workspace",
    config_path: "/workspace/.armature/armature.toml",
    config_version: "cfg_123",
    socket_path: "/state/daemon.sock",
    pid_path: "/state/daemon.pid",
    services: 2,
    tasks: 3,
    active_runs: 1
  });
} else if (command === "tasks") {
  send([{
    name: "watch",
    run: "npm test",
    watch: ["src/**/*.ts"],
    admission: "queue_one",
    active_run_ids: ["run_1"],
    queued_triggers: 0,
    schedule_active: false,
    watch_active: true
  }]);
} else if (command === "run build") {
  send({ run_id: "run_123", task: "build" });
} else if (command === "emit runtime.tick --json {\\"ok\\":true}") {
  send({ emitted: true, event_type: "runtime.tick", payload: { ok: true } });
} else if (command === "emit runtime.tick --source sdk --correlation corr-sdk --json {\\"ok\\":true}") {
  send({
    emitted: true,
    event_type: "runtime.tick",
    payload: { ok: true },
    source: "sdk",
    correlation_id: "corr-sdk"
  });
} else if (command === "lock acquire branch:main --ttl 30s" || command === "lock acquire branch:main --ttl 5m") {
  send({
    name: "branch:main",
    owner_pid: 4242,
    owner_id: "pid:4242",
    reason: null,
    token: "lock_main",
    acquired_at_ms: 10,
    renewed_at_ms: null,
    expires_at_ms: 40,
    manual: true
  });
} else if (command === "lock acquire branch:release --ttl 2m --reason deploy") {
  send({
    name: "branch:release",
    owner_pid: 4242,
    owner_id: "pid:4242",
    reason: "deploy",
    token: "lock_release",
    acquired_at_ms: 10,
    renewed_at_ms: null,
    expires_at_ms: 130,
    manual: true
  });
} else if (command === "lock renew branch:main --token lock_main --ttl 60s") {
  send({
    name: "branch:main",
    owner_pid: 4242,
    owner_id: "pid:4242",
    reason: null,
    token: "lock_main",
    acquired_at_ms: 10,
    renewed_at_ms: 20,
    expires_at_ms: 80,
    manual: true
  });
} else if (command === "lock release branch:main --token lock_main") {
  send({ released: true, name: "branch:main" });
} else if (command === "lock release branch:release --token lock_release") {
  send({ released: true, name: "branch:release" });
} else if (command === "lock status") {
  send([{
    name: "branch:main",
    owner_pid: 4242,
    owner_id: "pid:4242",
    reason: null,
    token: "lock_main",
    acquired_at_ms: 10,
    renewed_at_ms: null,
    expires_at_ms: 40,
    manual: true
  }]);
} else if (command === "logs run_123") {
  send({
    run_id: "run_123",
    run: {
      id: "run_123",
      name: "build",
      command: "npm test",
      origin: "task",
      state: "exited",
      start_time: "2026-04-29T12:00:00Z",
      end_time: "2026-04-29T12:00:01Z",
      exit_code: 0,
      signal: null,
      killed: false,
      config_version: "cfg_123",
      event_id: null,
      run_directory: "/runs/run_123",
      stdout_path: "/runs/run_123/stdout.log",
      stderr_path: "/runs/run_123/stderr.log"
    },
    run_directory: "/runs/run_123",
    stdout_path: "/runs/run_123/stdout.log",
    stderr_path: "/runs/run_123/stderr.log",
    stdout_bytes: 3,
    stderr_bytes: 0,
    stdout_lines: 1,
    stderr_lines: 0,
    stdout_truncated: false,
    stderr_truncated: false,
    stdout_missing: false,
    stderr_missing: false,
    stdout: "ok\\n",
    stderr: ""
  });
} else if (command === "cancel run_123") {
  send({ cancelled: true, run_id: "run_123" });
} else if (command === "services") {
  send([{
    name: "watcher",
    run: "tsx watcher.ts",
    enabled: true,
    restart: "on_failure",
    state: "running",
    supervision_state: "healthy",
    active_run_id: "run_srv",
    stop_override: false,
    last_error: null
  }]);
} else if (command === "down") {
  send({ stopped: true, workspace_root: "/workspace" });
} else {
  console.error("unexpected command", command);
  process.exit(2);
}
`;

  await writeFile(binPath, source, "utf8");
  await chmod(binPath, 0o755);

  return { binPath, callsPath };
}

test("getRunContext reads Armature runtime environment", () => {
  const context = getRunContext({
    ARMATURE_KIND: "task",
    ARMATURE_NAME: "status",
    ARMATURE_RUN_ID: "run_01ARZ3NDEKTSV4RRFFQ69G5FAV",
    ARMATURE_RUN_DIR: "/tmp/run",
    ARMATURE_EVENT_TYPE: "runtime.tick",
    ARMATURE_CORRELATION_ID: "corr-env"
  });

  assert.equal(context.kind, "task");
  assert.equal(context.name, "status");
  assert.equal(context.runId, "run_01ARZ3NDEKTSV4RRFFQ69G5FAV");
  assert.equal(context.runDirectory, "/tmp/run");
  assert.equal(context.eventType, "runtime.tick");
  assert.equal(context.correlationId, "corr-env");
});

test("getEvent prefers ARMATURE_EVENT_JSON and parses payloads", () => {
  const event = getEvent<{ ok: boolean }>({
    ARMATURE_EVENT_JSON: JSON.stringify({
      id: "evt_123",
      event_type: "tool.run.completed",
      payload: { ok: true }
    })
  });

  assert.equal(event.event_type, "tool.run.completed");
  assert.deepEqual(getPayload<{ ok: boolean }>({ ARMATURE_EVENT_JSON: JSON.stringify(event) }), {
    ok: true
  });
});

test("getEvent falls back to ARMATURE_EVENT_PATH", async () => {
  const directory = await mkdtemp(join(tmpdir(), "armature-sdk-event-"));
  const path = join(directory, "event.json");
  await writeFile(path, JSON.stringify({ event_type: "runtime.tick", payload: { count: 1 } }));

  const event = getEvent<{ count: number }>({ ARMATURE_EVENT_PATH: path });

  assert.equal(event.event_type, "runtime.tick");
  assert.equal(event.payload.count, 1);
});

test("getEvent raises a typed error when event context is absent", () => {
  assert.throws(() => getEvent({}), (error: unknown) => {
    assert.ok(error instanceof ArmatureSdkError);
    assert.equal(error.kind, "missing_env");
    return true;
  });
});

test("readJson and writeJson round-trip formatted files", async () => {
  const directory = await mkdtemp(join(tmpdir(), "armature-sdk-json-"));
  const path = join(directory, "payload.json");

  await writeJson(path, { ok: true, count: 1 });

  assert.equal(await readFile(path, "utf8"), '{\n  "ok": true,\n  "count": 1\n}\n');
  assert.deepEqual(await readJson(path), { ok: true, count: 1 });
});

test("log writes structured JSON lines", async () => {
  const stream = new PassThrough();
  const chunks: string[] = [];
  stream.on("data", (chunk) => {
    chunks.push(chunk.toString("utf8"));
  });

  log({ level: "info", message: "tick", ok: true }, stream);
  stream.end();

  assert.equal(chunks.join(""), '{"level":"info","message":"tick","ok":true}\n');
});

test("client wrappers call the CLI with json output and workspace options", async () => {
  const { binPath, callsPath } = await createStubArmature();
  const sdk = createArmature({
    bin: binPath,
    workspace: "/workspace",
    env: { ...process.env, CALLS_PATH: callsPath }
  });

  const currentStatus = await sdk.status();
  const currentTasks = await sdk.tasks();
  const runResult = await sdk.run("build");
  const emitResult = await sdk.emit("runtime.tick", { ok: true });
  const serviceList = await sdk.services();
  const runLogs = await sdk.logs("run_123");
  const cancelled = await sdk.cancel("run_123");
  const downResult = await sdk.down();

  assert.equal(currentStatus.config_version, "cfg_123");
  assert.equal(currentTasks[0]?.name, "watch");
  assert.equal(runResult.run_id, "run_123");
  assert.equal(emitResult.payload.ok, true);
  assert.equal(serviceList[0]?.name, "watcher");
  assert.equal(runLogs.stdout, "ok\n");
  assert.equal(runLogs.run?.name, "build");
  assert.equal(runLogs.stdout_lines, 1);
  assert.equal(runLogs.stdout_truncated, false);
  assert.equal(cancelled.cancelled, true);
  assert.equal(downResult.stopped, true);

  const calls = (await readFile(callsPath, "utf8"))
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line) as string[]);

  assert.deepEqual(calls[0], ["--format", "json", "--workspace", "/workspace", "status"]);
  assert.deepEqual(calls[1], ["--format", "json", "--workspace", "/workspace", "tasks"]);
  assert.deepEqual(calls[2], ["--format", "json", "--workspace", "/workspace", "run", "build"]);
  assert.deepEqual(calls[3], [
    "--format",
    "json",
    "--workspace",
    "/workspace",
    "emit",
    "runtime.tick",
    "--json",
    '{"ok":true}'
  ]);
});

test("emit options pass source and correlation through the CLI", async () => {
  const { binPath, callsPath } = await createStubArmature();
  const sdk = createArmature({
    bin: binPath,
    workspace: "/workspace",
    env: { ...process.env, CALLS_PATH: callsPath }
  });

  const result = await sdk.emit("runtime.tick", { ok: true }, {
    source: "sdk",
    correlation: "corr-sdk"
  });

  assert.equal(result.source, "sdk");
  assert.equal(result.correlation_id, "corr-sdk");

  const calls = (await readFile(callsPath, "utf8"))
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line) as string[]);

  assert.deepEqual(calls[0], [
    "--format",
    "json",
    "--workspace",
    "/workspace",
    "emit",
    "runtime.tick",
    "--source",
    "sdk",
    "--correlation",
    "corr-sdk",
    "--json",
    '{"ok":true}'
  ]);
});

test("top-level helpers use the default client", async () => {
  const { binPath, callsPath } = await createStubArmature();
  process.env.ARMATURE_BIN = binPath;
  process.env.CALLS_PATH = callsPath;

  try {
    assert.equal((await status()).config_version, "cfg_123");
    assert.equal((await tasks())[0]?.name, "watch");
    assert.equal((await run("build")).task, "build");
    assert.equal((await emit("runtime.tick", { ok: true })).event_type, "runtime.tick");
  } finally {
    delete process.env.ARMATURE_BIN;
    delete process.env.CALLS_PATH;
  }

  assert.ok(armature);
});

test("lock helpers include a ttl and release locks around the callback", async () => {
  const { binPath, callsPath } = await createStubArmature();
  const sdk = createArmature({
    bin: binPath,
    env: { ...process.env, CALLS_PATH: callsPath }
  });

  const held = await sdk.lock("branch:main", "30s");
  assert.equal(held.name, "branch:main");
  assert.equal(held.token, "lock_main");
  assert.equal((await sdk.renewLock("branch:main", held.token, "60s")).renewed_at_ms, 20);

  const result = await sdk.withLock(
    "branch:release",
    async () => "ok",
    { ttl: "2m", reason: "deploy" }
  );

  assert.equal(result, "ok");

  const calls = (await readFile(callsPath, "utf8"))
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line) as string[]);

  assert.deepEqual(calls[0], ["--format", "json", "lock", "acquire", "branch:main", "--ttl", "30s"]);
  assert.deepEqual(calls[1], [
    "--format",
    "json",
    "lock",
    "renew",
    "branch:main",
    "--token",
    "lock_main",
    "--ttl",
    "60s"
  ]);
  assert.deepEqual(calls[2], [
    "--format",
    "json",
    "lock",
    "acquire",
    "branch:release",
    "--ttl",
    "2m",
    "--reason",
    "deploy"
  ]);
  assert.deepEqual(calls[3], [
    "--format",
    "json",
    "lock",
    "release",
    "branch:release",
    "--token",
    "lock_release"
  ]);
});

test("top-level lock helpers use default ttl and status commands", async () => {
  const { binPath, callsPath } = await createStubArmature();
  process.env.ARMATURE_BIN = binPath;
  process.env.CALLS_PATH = callsPath;

  try {
    assert.equal((await lock("branch:main")).manual, true);
    assert.equal((await renewLock("branch:main", "lock_main", "60s")).expires_at_ms, 80);
    assert.equal((await locks())[0]?.name, "branch:main");
    assert.equal((await unlock("branch:main", "lock_main")).released, true);
    await withLock("branch:release", async () => undefined, { ttl: "2m", reason: "deploy" });
  } finally {
    delete process.env.ARMATURE_BIN;
    delete process.env.CALLS_PATH;
  }

  const calls = (await readFile(callsPath, "utf8"))
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line) as string[]);

  assert.deepEqual(calls[0], ["--format", "json", "lock", "acquire", "branch:main", "--ttl", "5m"]);
  assert.deepEqual(calls[1], ["--format", "json", "lock", "renew", "branch:main", "--token", "lock_main", "--ttl", "60s"]);
  assert.deepEqual(calls[2], ["--format", "json", "lock", "status"]);
  assert.deepEqual(calls[3], ["--format", "json", "lock", "release", "branch:main", "--token", "lock_main"]);
});

test("CLI failures surface typed details", async () => {
  const { binPath, callsPath } = await createStubArmature();
  const sdk = createArmature({
    bin: binPath,
    env: { ...process.env, CALLS_PATH: callsPath }
  });

  await assert.rejects(
    sdk.run("missing"),
    (error: unknown) => {
      assert.ok(error instanceof ArmatureSdkError);
      assert.equal(error.kind, "cli_failed");
      assert.equal(error.details?.code, "2");
      return true;
    }
  );
});
