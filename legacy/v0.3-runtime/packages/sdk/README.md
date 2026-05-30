# `@whipplescript/sdk`

Thin TypeScript helpers for WhippleScript v0.3. The SDK shells out to the WhippleScript CLI
and reads WhippleScript-provided runtime environment variables; it does not add a
second runtime, workflow DSL, or orchestration layer.

## Setup

Make the `whip` binary available on `PATH`, or pass a binary path:

```ts
import { createWhippleScript } from "@whipplescript/sdk"

const whipplescript = createWhippleScript({
  bin: "/path/to/whipplescript",
  workspace: "/path/to/workspace",
})
```

Every CLI-backed call requests JSON output and returns typed objects.

## Object Namespaces

The client exposes the canonical dynamic-management vocabulary directly:

```ts
await whipplescript.task.run("test")
await whipplescript.event.emit("build.completed", { ok: true })
await whipplescript.run.start({
  name: "one-shot",
  command: ["node", "scripts/check.mjs"],
  correlation: "corr-123",
})

const events = await whipplescript.event.list({ type: "build.completed", limit: 10 })
const runs = await whipplescript.run.list({ correlation: "corr-123" })
const trigger = await whipplescript.wait.trigger({
  task: "reviewer",
  outcome: "started",
  timeout: "30s",
})
```

These helpers are thin wrappers over object-oriented CLI commands such as
`whip task run`, `whip event emit`, `whip run start`, and
`whip wait trigger`. They do not add workflow state, semantic retries, or
agent graph behavior.

Dynamic definitions are runtime definitions:

```ts
await whipplescript.service.add("github-source", ["node", "sources/github.mjs"], {
  restart: "on_failure",
  reason: "bridge github events",
})

await whipplescript.task.add("reviewer", ["node", "agents/reviewer.mjs"], {
  on: "plan.ready",
  correlation: "corr-123",
})

console.log(await whipplescript.service.list({ dynamic: true }))
console.log(await whipplescript.task.list({ dynamic: true }))
await whipplescript.task.remove("reviewer")
await whipplescript.service.remove("github-source")
```

They live in the daemon runtime until removed, daemon shutdown, or workspace
reset. The SDK does not rewrite `.whipplescript/project.whip`.

## Runtime Context

Task and service processes can read their run and event context:

```ts
import { getEvent, getPayload, getRunContext } from "@whipplescript/sdk"

const context = getRunContext()
const event = getEvent<{ runId: string; ok: boolean }>()
const payload = getPayload<{ runId: string; ok: boolean }>()

console.log(context.runId, context.runDirectory, event.event_type, payload.ok)
```

`getEvent()` reads `WHIPPLESCRIPT_EVENT_JSON`, `WHIPPLESCRIPT_EVENT`, or
`WHIPPLESCRIPT_EVENT_PATH`. `getRunContext()` also exposes
`WHIPPLESCRIPT_CORRELATION_ID` as `correlationId`.

When SDK code calls `emit()` from inside an WhippleScript-managed task or service, the
CLI records mechanical provenance inherited from the process environment:
`source_run_id`, `parent_event_id`, and optional `correlation_id`.

```ts
await emit("review.ready", { ok: true }, { correlation: "corr-123" })
```

## Daemon And Inspection

```ts
import { whipplescript } from "@whipplescript/sdk"

await whipplescript.up()
await whipplescript.restart()

const snapshot = await whipplescript.status()
const overview = await whipplescript.overview()
const tasks = await whipplescript.tasks()
const services = await whipplescript.services()
const runs = await whipplescript.runs()
const logOutput = await whipplescript.logs(runs[0].id)

console.log(snapshot, overview, tasks, services, logOutput)

await whipplescript.down()
```

Equivalent named exports are available for common calls:

```ts
import { emit, logs, overview, run, runs, services, status, tasks } from "@whipplescript/sdk"

const started = await run("test")
await emit("build.completed", { runId: started.run_id, ok: true })
console.log(await status(), await overview(), await tasks(), await services(), await runs())
console.log(await logs(started.run_id))
```

`overview()` wraps `whip overview` and returns the compact mechanical status
view: configured tasks/services, active runs, latest run per task/service, queued
trigger counts, recent failures, recent events, and recent triggers.

`logs(runId)` returns captured stdout/stderr plus the run record, run directory,
log paths, byte counts, line counts, truncation flags, and missing-file flags
exposed by `whip --format json logs`.

## Services

```ts
import { whipplescript } from "@whipplescript/sdk"

await whipplescript.startService("worker")
await whipplescript.restartService("worker")
await whipplescript.stopService("worker")
```

## Locks

```ts
import { lock, locks, renewLock, unlock, withLock } from "@whipplescript/sdk"

await withLock("branch:main", async () => {
  // user-space critical section
}, { ttl: "2m" })

const held = await lock("artifact-cache", "10m", "cache refresh")
await renewLock(held.name, held.token, "10m")
console.log(await locks())
await unlock(held.name, held.token)

await whipplescript.lock.withCommand("repo:main", ["npm", "test"], {
  ttl: "2m",
  reason: "test main branch",
})
```

## Files And Structured Logs

```ts
import { log, readJson, writeJson } from "@whipplescript/sdk"

await writeJson("result.json", { ok: true })
const result = await readJson<{ ok: boolean }>("result.json")

log({ level: "info", message: "result written", ok: result.ok })
```

## Errors

CLI failures and invalid JSON are reported as `WhippleScriptSdkError` with a stable
`kind` and optional `details` object:

```ts
import { WhippleScriptSdkError, status } from "@whipplescript/sdk"

try {
  await status()
} catch (error) {
  if (error instanceof WhippleScriptSdkError) {
    console.error(error.kind, error.details)
  }
}
```
