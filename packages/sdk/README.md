# `@armature/sdk`

Thin TypeScript helpers for Armature v0.3.

The SDK wraps the existing Armature CLI and runtime environment. It does not add a
second runtime, workflow DSL, or orchestration layer.

## Install Surface

Armature injects runtime context into tasks and services:

```ts
import { getEvent, getRunContext } from "@armature/sdk"

const context = getRunContext()
const event = getEvent<{ runId: string }>()

console.log(context.runId, event.payload.runId)
```

## Emit And Inspect

```ts
import { armature, emit } from "@armature/sdk"

await emit("tool.run.completed", { runId: "run_123", ok: true })

const status = await armature.status()
const services = await armature.services()
const runs = await armature.runs()
```

## Locks

```ts
import { withLock } from "@armature/sdk"

await withLock("branch:main", async () => {
  // do user-space coordination work here
}, { ttl: "2m" })
```

## Files And Structured Logs

```ts
import { log, readJson, writeJson } from "@armature/sdk"

await writeJson("result.json", { ok: true })
const result = await readJson<{ ok: boolean }>("result.json")

log({ level: "info", message: "result written", ok: result.ok })
```

## Client Options

Use `createArmature()` when you need an explicit binary path or workspace override:

```ts
import { createArmature } from "@armature/sdk"

const sdk = createArmature({
  bin: process.env.ARMATURE_BIN,
  workspace: "/path/to/workspace",
  lockTtl: "30s",
})

await sdk.run("build")
```
