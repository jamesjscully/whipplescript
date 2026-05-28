# `@whippletree/sdk`

Thin TypeScript helpers for Whippletree v0.3. The SDK shells out to the Whippletree CLI
and reads Whippletree-provided runtime environment variables; it does not add a
second runtime, workflow DSL, or orchestration layer.

## Setup

Make the `whip` binary available on `PATH`, or pass a binary path:

```ts
import { createWhippletree } from "@whippletree/sdk"

const whippletree = createWhippletree({
  bin: "/path/to/whippletree",
  workspace: "/path/to/workspace",
})
```

Every CLI-backed call requests JSON output and returns typed objects.

## Object Namespaces

The client exposes the canonical dynamic-management vocabulary directly:

```ts
await whippletree.task.run("test")
await whippletree.event.emit("build.completed", { ok: true })
await whippletree.run.start({
  name: "one-shot",
  command: ["node", "scripts/check.mjs"],
  correlation: "corr-123",
})

const events = await whippletree.event.list({ type: "build.completed", limit: 10 })
const runs = await whippletree.run.list({ correlation: "corr-123" })
const trigger = await whippletree.wait.trigger({
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
await whippletree.service.add("github-source", ["node", "sources/github.mjs"], {
  restart: "on_failure",
  reason: "bridge github events",
})

await whippletree.task.add("reviewer", ["node", "agents/reviewer.mjs"], {
  on: "plan.ready",
  correlation: "corr-123",
})

console.log(await whippletree.service.list({ dynamic: true }))
console.log(await whippletree.task.list({ dynamic: true }))
await whippletree.task.remove("reviewer")
await whippletree.service.remove("github-source")
```

They live in the daemon runtime until removed, daemon shutdown, or workspace
reset. The SDK does not rewrite `.whippletree/project.whip`.

## Runtime Context

Task and service processes can read their run and event context:

```ts
import { getEvent, getPayload, getRunContext } from "@whippletree/sdk"

const context = getRunContext()
const event = getEvent<{ runId: string; ok: boolean }>()
const payload = getPayload<{ runId: string; ok: boolean }>()

console.log(context.runId, context.runDirectory, event.event_type, payload.ok)
```

`getEvent()` reads `WHIPPLETREE_EVENT_JSON`, `WHIPPLETREE_EVENT`, or
`WHIPPLETREE_EVENT_PATH`. `getRunContext()` also exposes
`WHIPPLETREE_CORRELATION_ID` as `correlationId`.

When SDK code calls `emit()` from inside an Whippletree-managed task or service, the
CLI records mechanical provenance inherited from the process environment:
`source_run_id`, `parent_event_id`, and optional `correlation_id`.

```ts
await emit("review.ready", { ok: true }, { correlation: "corr-123" })
```

## Daemon And Inspection

```ts
import { whippletree } from "@whippletree/sdk"

await whippletree.up()
await whippletree.restart()

const snapshot = await whippletree.status()
const overview = await whippletree.overview()
const tasks = await whippletree.tasks()
const services = await whippletree.services()
const runs = await whippletree.runs()
const logOutput = await whippletree.logs(runs[0].id)

console.log(snapshot, overview, tasks, services, logOutput)

await whippletree.down()
```

Equivalent named exports are available for common calls:

```ts
import { emit, logs, overview, run, runs, services, status, tasks } from "@whippletree/sdk"

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
import { whippletree } from "@whippletree/sdk"

await whippletree.startService("worker")
await whippletree.restartService("worker")
await whippletree.stopService("worker")
```

## Locks

```ts
import { lock, locks, renewLock, unlock, withLock } from "@whippletree/sdk"

await withLock("branch:main", async () => {
  // user-space critical section
}, { ttl: "2m" })

const held = await lock("artifact-cache", "10m", "cache refresh")
await renewLock(held.name, held.token, "10m")
console.log(await locks())
await unlock(held.name, held.token)

await whippletree.lock.withCommand("repo:main", ["npm", "test"], {
  ttl: "2m",
  reason: "test main branch",
})
```

## Files And Structured Logs

```ts
import { log, readJson, writeJson } from "@whippletree/sdk"

await writeJson("result.json", { ok: true })
const result = await readJson<{ ok: boolean }>("result.json")

log({ level: "info", message: "result written", ok: result.ok })
```

## Errors

CLI failures and invalid JSON are reported as `WhippletreeSdkError` with a stable
`kind` and optional `details` object:

```ts
import { WhippletreeSdkError, status } from "@whippletree/sdk"

try {
  await status()
} catch (error) {
  if (error instanceof WhippletreeSdkError) {
    console.error(error.kind, error.details)
  }
}
```
