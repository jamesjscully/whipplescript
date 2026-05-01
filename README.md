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
armature lock status
armature lock renew branch:main --token "$TOKEN" --ttl 10m
armature lock release branch:main --token "$TOKEN"
```

Locks are workspace-scoped and TTL-backed. Acquire returns a fencing token; renew
and release require that token so an expired holder cannot release a newer lock.

## TypeScript SDK

The SDK provides typed helpers over the CLI and Armature runtime environment:

```ts
import { emit, getEvent, getRunContext, status, withLock } from "@armature/sdk"

const context = getRunContext()
const event = getEvent<{ runId: string; ok: boolean }>()

await emit("build.completed", { runId: context.runId ?? "manual", ok: true })
await withLock("branch:main", async () => {
  console.log(await status())
}, { ttl: "2m" })
```

See `packages/sdk/README.md` for the full SDK surface.

## Current Caveat

Armature v0.3 uses Unix sockets and Unix process groups, so the current runtime
targets Unix-like systems.
