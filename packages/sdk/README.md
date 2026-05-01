# `@armature/sdk`

Thin TypeScript helpers for Armature v0.3. The SDK shells out to the Armature CLI
and reads Armature-provided runtime environment variables; it does not add a
second runtime, workflow DSL, or orchestration layer.

## Setup

Make the `armature` binary available on `PATH`, or pass a binary path:

```ts
import { createArmature } from "@armature/sdk"

const armature = createArmature({
  bin: "/path/to/armature",
  workspace: "/path/to/workspace",
})
```

Every CLI-backed call requests JSON output and returns typed objects.

## Object Namespaces

The client exposes the canonical dynamic-management vocabulary directly:

```ts
await armature.task.run("test")
await armature.event.emit("build.completed", { ok: true })
await armature.run.start({
  name: "one-shot",
  command: ["node", "scripts/check.mjs"],
  correlation: "corr-123",
})

const events = await armature.event.list({ type: "build.completed", limit: 10 })
const runs = await armature.run.list({ correlation: "corr-123" })
const trigger = await armature.wait.trigger({
  task: "reviewer",
  outcome: "started",
  timeout: "30s",
})
```

These helpers are thin wrappers over object-oriented CLI commands such as
`armature task run`, `armature event emit`, `armature run start`, and
`armature wait trigger`. They do not add workflow state, semantic retries, or
agent graph behavior.

Dynamic definitions are runtime definitions:

```ts
await armature.service.add("github-source", ["node", "sources/github.mjs"], {
  restart: "on_failure",
  reason: "bridge github events",
})

await armature.task.add("reviewer", ["node", "agents/reviewer.mjs"], {
  on: "plan.ready",
  correlation: "corr-123",
})

console.log(await armature.service.list({ dynamic: true }))
console.log(await armature.task.list({ dynamic: true }))
await armature.task.remove("reviewer")
await armature.service.remove("github-source")
```

They live in the daemon runtime until removed, daemon shutdown, or workspace
reset. The SDK does not rewrite `.armature/armature.toml`.

## Runtime Context

Task and service processes can read their run and event context:

```ts
import { getEvent, getPayload, getRunContext } from "@armature/sdk"

const context = getRunContext()
const event = getEvent<{ runId: string; ok: boolean }>()
const payload = getPayload<{ runId: string; ok: boolean }>()

console.log(context.runId, context.runDirectory, event.event_type, payload.ok)
```

`getEvent()` reads `ARMATURE_EVENT_JSON`, `ARMATURE_EVENT`, or
`ARMATURE_EVENT_PATH`. `getRunContext()` also exposes
`ARMATURE_CORRELATION_ID` as `correlationId`.

When SDK code calls `emit()` from inside an Armature-managed task or service, the
CLI records mechanical provenance inherited from the process environment:
`source_run_id`, `parent_event_id`, and optional `correlation_id`.

```ts
await emit("review.ready", { ok: true }, { correlation: "corr-123" })
```

## Daemon And Inspection

```ts
import { armature } from "@armature/sdk"

await armature.up()
await armature.restart()

const snapshot = await armature.status()
const tasks = await armature.tasks()
const services = await armature.services()
const runs = await armature.runs()
const logOutput = await armature.logs(runs[0].id)

console.log(snapshot, tasks, services, logOutput)

await armature.down()
```

Equivalent named exports are available for common calls:

```ts
import { emit, logs, run, runs, services, status, tasks } from "@armature/sdk"

const started = await run("test")
await emit("build.completed", { runId: started.run_id, ok: true })
console.log(await status(), await tasks(), await services(), await runs())
console.log(await logs(started.run_id))
```

`logs(runId)` returns captured stdout/stderr plus the run record, run directory,
log paths, byte counts, line counts, truncation flags, and missing-file flags
exposed by `armature --format json logs`.

## Services

```ts
import { armature } from "@armature/sdk"

await armature.startService("worker")
await armature.restartService("worker")
await armature.stopService("worker")
```

## Locks

```ts
import { lock, locks, renewLock, unlock, withLock } from "@armature/sdk"

await withLock("branch:main", async () => {
  // user-space critical section
}, { ttl: "2m" })

const held = await lock("artifact-cache", "10m", "cache refresh")
await renewLock(held.name, held.token, "10m")
console.log(await locks())
await unlock(held.name, held.token)

await armature.lock.withCommand("repo:main", ["npm", "test"], {
  ttl: "2m",
  reason: "test main branch",
})
```

## Files And Structured Logs

```ts
import { log, readJson, writeJson } from "@armature/sdk"

await writeJson("result.json", { ok: true })
const result = await readJson<{ ok: boolean }>("result.json")

log({ level: "info", message: "result written", ok: result.ok })
```

## Errors

CLI failures and invalid JSON are reported as `ArmatureSdkError` with a stable
`kind` and optional `details` object:

```ts
import { ArmatureSdkError, status } from "@armature/sdk"

try {
  await status()
} catch (error) {
  if (error instanceof ArmatureSdkError) {
    console.error(error.kind, error.details)
  }
}
```
