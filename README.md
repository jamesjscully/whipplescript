# armature

Armature is a lightweight local daemon and CLI for running ordinary programs from
schedules, file changes, emitted events, and supervised long-running services.

## Install

```sh
cargo build -p armature-cli
alias armature="$PWD/target/debug/armature"
```

## Initialize

```sh
armature init
armature config check
```

Example config:

```toml
[[task]]
name = "test"
watch = ["src/**/*", "tests/**/*"]
run = "cargo test"

[[task]]
name = "on-build-event"
on = "build.completed"
run = "./scripts/on-build-completed.sh"

[[service]]
name = "worker"
run = "./scripts/worker.sh"
```

## Run The Daemon

```sh
armature up
armature dev
armature down
```

`armature up` starts the daemon detached, or reloads config when a daemon is
already running. `armature dev` runs the daemon in the foreground.

## Trigger Work

```sh
armature task run test
armature event emit build.completed --json '{"runId":"run_123","ok":true}'
```

The v0.3 aliases remain available:

```sh
armature run test
armature emit build.completed --json '{"runId":"run_123","ok":true}'
```

## Inspect Runtime Objects

Canonical object-oriented commands:

```sh
armature task list
armature task show test
armature service list
armature service show worker
armature run list
armature run show <run-id>
armature run logs <run-id>
armature run cancel <run-id>
armature event list
armature event show <event-id>
armature trigger list
armature trigger show <trigger-id>
armature log show <run-id>
armature log tail <run-id> --lines 100
```

Existing inspection aliases still work:

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

Use `--format json` globally for machine-readable output:

```sh
armature --format json run list
armature --format json event list
```

## Services And Locks

```sh
armature service start worker
armature service stop worker
armature service restart worker

armature --format json lock acquire branch:main --ttl 10m --reason "edit branch"
armature lock list
armature lock show branch:main
armature lock renew branch:main --token lock_... --ttl 10m
armature lock release branch:main --token lock_...
armature lock force-release branch:main --reason "holder exited"
armature lock with branch:main --ttl 2m --reason "run tests" -- npm test
```

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
armature run list --correlation req-1
armature lock with repo:main --ttl 1m --reason "final check" -- echo ok
armature down
```

User-authored scripts own planning, retries, deduplication, fanout, review
logic, and success criteria. Armature records and supervises the mechanical
runtime facts.

## TypeScript SDK

`@armature/sdk` is a thin CLI-backed helper layer:

```ts
import { createArmature, getEvent, getRunContext } from "@armature/sdk"

const armature = createArmature({ workspace: process.cwd() })
const context = getRunContext()
const event = getEvent<{ requestId: string }>()

await armature.task.add("reviewer", ["node", "reviewer.mjs"], {
  on: "plan.ready",
  correlation: event.correlation_id ?? event.payload.requestId,
})
await armature.event.emit("review.registered", {
  runId: context.runId,
})
await armature.wait.event("review.completed", {
  correlation: event.correlation_id ?? event.payload.requestId,
  timeout: "5m",
})
```

The SDK wraps the same CLI/runtime environment surface available to shell
scripts. It does not create a second runtime.

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

Armature owns mechanical invocation truth: triggers, launches, process state,
logs, events, locks, and runtime inspection. User code owns semantic meaning.
