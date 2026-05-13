# armature

Armature is a lightweight local daemon and CLI for running ordinary programs from
schedules, file changes, emitted events, and supervised long-running services.

## Install

Build the CLI from this repository:

```sh
cargo build -p armature-cli
alias armature="$PWD/target/debug/armature"
```

The TypeScript SDK lives in `packages/sdk` and wraps the installed CLI.

## Initialize A Workspace

```sh
armature init
```

This creates `.armature/armature.toml`. Armature discovers workspaces by searching
upward for that file, or you can pass `--workspace /path/to/workspace`.

Example config:

```toml
[[task]]
name = "test"
watch = ["src/**/*", "tests/**/*"]
settle = "500ms"
run = "cargo test"

[task.admission]
when_busy = "queue_one"

[[task]]
name = "nightly-status"
schedule = "0 9 * * *"
run = "./scripts/status.sh"

[[task]]
name = "on-build-event"
on = "build.completed"
run = "./scripts/on-build-completed.sh"

[[service]]
name = "worker"
run = "./scripts/worker.sh"

[service.supervision]
restart = "on_failure"
max_restarts = 5
within = "1m"
backoff = "exponential"

[service.health]
check = "./scripts/worker-health.sh"
every = "10s"
timeout = "2s"
```

Check the config before starting the daemon:

```sh
armature config check
```

Starter recipes are available with:

```sh
armature init recipe file-watch-tests
armature init recipe scheduled-status-script
armature init recipe event-source-service
armature init recipe event-hook-task
armature init recipe named-lock
```

## Run The Daemon

Start detached:

```sh
armature up
```

Run in the foreground while developing:

```sh
armature dev
# equivalent to:
armature up --foreground
```

Reload after config changes:

```sh
armature up
```

If a daemon is already running, `armature up` validates and hot-reloads the
current config. Invalid reloads are rejected and the previous valid config stays
active. Removed or changed services are stopped and reconciled from the new
config; in-flight task runs are left to finish unless they are explicitly
cancelled or the daemon is stopped. `armature restart` stops active runs as part
of daemon shutdown, then starts the daemon again from the latest valid config.

Stop the daemon:

```sh
armature down
```

## Trigger Work

Run a task manually:

```sh
armature task run test
# alias:
armature run test
```

Emit an event for `on = "..."` tasks:

```sh
armature event emit build.completed --json '{"runId":"run_123","ok":true}'
# alias:
armature emit build.completed --json '{"runId":"run_123","ok":true}'
armature emit build.completed --correlation corr-123 --json '{"ok":true}'
armature emit build.completed --payload-file event.json
printf '%s\n' '{"runId":"run_123","ok":true}' | armature emit build.completed --stdin
```

`armature emit` defaults to an empty object payload (`{}`) and records the event
source as `cli`. Override that provenance with `--source`, for example
`--source agent:reviewer`. Use exactly one of `--json`, `--payload-file`, or
`--stdin` for payload input. For machine-readable command output, put the global
format flag before the command: `armature --format json emit ...`.

Event payloads are JSON values recorded with the event. For shell scripts,
prefer `--payload-file` or `--stdin` once payloads stop being tiny; this avoids
quoting bugs and makes script-side validation easier. Event-triggered task
processes receive the recorded event through environment variables:
`ARMATURE_EVENT_TYPE`, `ARMATURE_EVENT_JSON`, `ARMATURE_EVENT_PATH`, and
`ARMATURE_EVENT_PAYLOAD_JSON`. Scripts should validate payload shape before
turning an event into project state.

Accepted events are appended to the local event log before task routing and can
be inspected with `armature event list --json` or the `armature events --json`
alias. Trigger admission is inspectable with `armature trigger list --json` or
the `armature triggers --json` alias: busy tasks may record `started`, `queued`,
`coalesced`, `rejected`, or `superseded` outcomes depending on `when_busy`.

Armature does not provide durable queues, replay cursors, or exactly-once
delivery. If the daemon is down, `armature emit` fails instead of buffering for
later delivery; duplicate emits are recorded as separate events, and user code
owns semantic deduplication.

Task and service processes receive Armature context in environment variables such
as `ARMATURE_RUN_ID`, `ARMATURE_RUN_DIR`, `ARMATURE_WORKSPACE_ROOT`,
`ARMATURE_EVENT_TYPE`, `ARMATURE_EVENT_JSON`, and `ARMATURE_EVENT_PATH`.
When `armature emit` or `armature run` is called from inside an
Armature-managed process, the CLI forwards mechanical provenance from
`ARMATURE_RUN_ID` and `ARMATURE_EVENT_ID`; recorded events may include
`source_run_id`, `parent_event_id`, and `correlation_id`. Correlation can be set
with `--correlation` or propagated with `ARMATURE_CORRELATION_ID`. These fields
track invocation causality only; Armature does not infer workflows or traces from
them.

## Inspect Work

```sh
armature overview
armature status
armature task list
armature task show test
armature service list
armature service show worker
armature ps
armature run list
armature run show <run-id>
armature run logs <run-id>
armature log tail <run-id> --lines 100
armature run cancel <run-id>
armature doctor
```

`armature overview` is the compact operational view for agent-style projects. It
summarizes configured tasks and services, active run ids, queued trigger counts,
latest run per task/service, recent failures, recent events, and recent trigger
outcomes. It is a read-only projection over Armature's mechanical state; it does
not infer workflow status from your project artifacts.

The v0.3 aliases remain available:

```sh
armature tasks
armature services
armature runs
armature logs <run-id>
armature logs --tail 100 <run-id>
armature cancel <run-id>
armature events
armature triggers
```

Use `--format json` globally, or `--json` on supported inspection commands:

```sh
armature --format json status
armature runs --json
```

`armature logs` prints run metadata, stdout/stderr paths, stream sizes, line
counts, and the captured stream contents. Use `--tail <lines>` to limit each
stream while keeping the original byte and line counts visible. If the daemon
recovers a run that was active when a previous daemon exited, stderr and
`meta.json` include a recovery note and final failed state.

## Services And Health

Configured services are reconciled by the daemon. They can be controlled manually:

```sh
armature service list
armature service show worker
armature service start worker
armature service stop worker
armature service restart worker
```

Health checks run as configured under `[service.health]`; service state and recent
errors are visible through `armature services` and `armature status`.

## Manual Locks

Use named locks for simple local coordination between scripts:

```sh
armature --format json lock acquire branch:main --ttl 10m --reason "deploy"
armature lock list
armature lock show branch:main
armature lock renew branch:main --token "$TOKEN" --ttl 10m
armature lock release branch:main --token "$TOKEN"
armature lock force-release branch:main --reason "holder exited"
armature lock with branch:main --ttl 2m --reason "run tests" -- npm test
```

Locks are workspace-scoped and TTL-backed. Acquire returns a fencing token; renew
and release require that token so an expired holder cannot release a newer lock.
Use `force-release` only as an explicit recovery action.

## Dynamic Runtime Definitions

Tasks and services can also be registered at runtime without editing
`.armature/armature.toml`:

```sh
armature service add github-source --restart on_failure --reason "event bridge" -- node sources/github.mjs
armature task add reviewer --on plan.ready -- node agents/reviewer.mjs

armature service list --dynamic
armature task list --dynamic
armature task remove reviewer
armature service remove github-source
```

Dynamic definitions are ephemeral runtime definitions. They are inspectable and
marked `dynamic: true`, but they are not workflow state and Armature does not
persist them into user config.

## Agent Desire Path

```sh
armature up
armature service add github-source -- node sources/github.mjs
armature task add planner --on agent.requested -- node planner.mjs
armature event emit agent.requested --correlation req-1 --payload-file request.json
armature wait event work.completed --correlation req-1 --timeout 5m
armature overview --json
armature run list --correlation req-1
armature lock with repo:main --ttl 1m --reason "final check" -- echo ok
armature down
```

User-authored scripts own planning, retries, deduplication, fanout, review
logic, and success criteria. Armature records and supervises the mechanical
runtime facts.

For recurring agent loops, a common pattern is a scheduled director task that
checks script-owned state, emits request events when work should start, and
exits without emitting when active project work already exists. Event-triggered
tasks then handle request/completion/quality-gate events. Keep state such as
`tasks.json`, artifacts, quality decisions, and locks in the repo scripts; use
Armature to make the timers, dispatch, logs, and run history inspectable.

## TypeScript SDK

The SDK provides typed helpers over the CLI and Armature runtime environment:

```ts
import { createArmature, emit, getEvent, getRunContext, status, withLock } from "@armature/sdk"

const armature = createArmature({ workspace: process.cwd() })
const context = getRunContext()
const event = getEvent<{ runId: string; ok: boolean }>()

await armature.task.add("reviewer", ["node", "reviewer.mjs"], {
  on: "plan.ready",
  correlation: event.correlation_id ?? event.payload.runId,
})
await emit("build.completed", { runId: context.runId ?? "manual", ok: true })
await armature.wait.event("review.completed", {
  correlation: event.correlation_id ?? event.payload.runId,
  timeout: "5m",
})
await withLock("branch:main", async () => {
  console.log(await status())
}, { ttl: "2m", reason: "inspect status" })
```

The SDK wraps the same CLI/runtime environment surface available to shell
scripts. It does not create a second runtime. See `packages/sdk/README.md` for
the full SDK surface.

## Migration Notes

Canonical object commands are preferred in new docs and scripts:

```text
armature tasks             -> armature task list
armature services          -> armature service list
armature runs              -> armature run list
armature logs <run-id>     -> armature run logs <run-id>
armature cancel <run-id>   -> armature run cancel <run-id>
armature emit <type>       -> armature event emit <type>
armature run <task>        -> armature task run <task>
```

## Current Caveat

Armature v0.3 uses Unix sockets and Unix process groups, so the current runtime
targets Unix-like systems.
