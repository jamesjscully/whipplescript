---
name: armature-runtime
description: Use when coding agents need to run, supervise, observe, or coordinate local project automation with Armature; configure Armature workspaces; write tasks, services, event handlers, locks, or TypeScript SDK integrations; or debug Armature-managed runs, logs, triggers, and daemon state.
---

# Armature Runtime

Use Armature as local runtime plumbing for ordinary programs. Keep the boundary clear:

- Armature owns mechanical invocation truth: process start/stop, supervision, events, triggers, runs, logs, locks, waits, and daemon state.
- User code owns operational meaning: planning, approval, retry decisions, deduplication, fanout, workflow state, and success criteria.

Do not introduce workflow DAGs, durable promises, semantic retries, semantic dedupe, agent graphs, or hidden workflow state unless the user explicitly asks for a layer outside Armature.

## First Checks

1. Confirm the CLI is available: `armature --help`. If working from source, build it with `cargo build -p armature-cli` and use `target/debug/armature`.
2. Find or create the workspace. Armature discovers `.armature/armature.toml` by walking upward, or accepts `--workspace /path/to/workspace`.
3. Validate config before starting or reloading: `armature config check`.
4. Start the daemon with `armature up`, or use `armature dev` / `armature up --foreground` while debugging.
5. Prefer machine-readable output while scripting: put `--format json` before the command, for example `armature --format json status`.

## Static Definitions

Use `.armature/armature.toml` for stable repository behavior.

```toml
[[task]]
name = "test"
watch = ["src/**/*", "tests/**/*"]
settle = "500ms"
run = "cargo test"

[task.admission]
when_busy = "queue_one"

[[task]]
name = "on-plan-ready"
on = "plan.ready"
run = "node agents/reviewer.mjs"

[[service]]
name = "github-source"
run = "node sources/github.mjs"

[service.supervision]
restart = "on_failure"
max_restarts = 5
within = "1m"
backoff = "exponential"
```

Use `armature up` after config edits. If a daemon is already running, `up` validates and hot-reloads; invalid reloads are rejected and the previous valid config remains active.

## Dynamic Definitions

Use dynamic definitions for ephemeral runtime wiring that should not edit `.armature/armature.toml`.

```sh
armature service add github-source --restart on_failure --reason "event bridge" -- node sources/github.mjs
armature task add reviewer --on plan.ready -- node agents/reviewer.mjs
armature service list --dynamic
armature task list --dynamic
armature task remove reviewer
armature service remove github-source
```

Dynamic definitions are inspectable and marked `dynamic: true`, but they are not workflow state and are not persisted into config.

## Canonical Commands

Prefer object-oriented commands in new scripts and docs:

```sh
armature task list
armature task show test
armature task run test
armature service list
armature service show worker
armature service start worker
armature service stop worker
armature event emit plan.ready --correlation req-123 --json '{"ok":true}'
armature event list --correlation req-123 --json
armature trigger list --task reviewer --json
armature run list --correlation req-123
armature run show <run-id>
armature run logs <run-id>
armature run cancel <run-id>
armature wait event work.completed --correlation req-123 --timeout 5m
armature subscribe events
```

Aliases such as `armature tasks`, `armature services`, `armature runs`, `armature logs`, `armature emit`, and `armature run <task>` exist for compatibility, but canonical commands are clearer for agents.

## Events And Provenance

Emit events to route work to `on = "..."` tasks:

```sh
armature event emit build.completed --source agent:builder --correlation req-123 --json '{"ok":true}'
armature event emit build.completed --payload-file event.json
printf '%s\n' '{"ok":true}' | armature event emit build.completed --stdin
```

Use exactly one payload source: `--json`, `--payload-file`, or `--stdin`. If no payload is supplied, Armature records `{}`.

Armature records accepted events before routing them. Trigger outcomes are inspectable and can be `started`, `queued`, `coalesced`, `rejected`, or `superseded` depending on admission policy.

Armature does not provide durable queues, replay cursors, exactly-once delivery, or semantic deduplication. If the daemon is down, emit fails instead of buffering.

## Runtime Context

Armature-managed task and service processes receive context through environment variables including:

- `ARMATURE_RUN_ID`
- `ARMATURE_RUN_DIR`
- `ARMATURE_WORKSPACE_ROOT`
- `ARMATURE_EVENT_TYPE`
- `ARMATURE_EVENT_JSON`
- `ARMATURE_EVENT_PATH`
- `ARMATURE_CORRELATION_ID`

When an Armature-managed process calls `armature event emit` or `armature task run`, the CLI forwards mechanical provenance from the environment so records may include `source_run_id`, `parent_event_id`, and `correlation_id`.

## Locks

Use named locks for local critical sections:

```sh
armature --format json lock acquire branch:main --ttl 10m --reason "deploy"
armature lock renew branch:main --token "$TOKEN" --ttl 10m
armature lock release branch:main --token "$TOKEN"
armature lock with branch:main --ttl 2m --reason "run tests" -- npm test
armature lock list --expired
armature lock force-release branch:main --reason "holder exited"
```

Locks are workspace-scoped and TTL-backed. Acquire returns a fencing token; renew and release require that token so an expired holder cannot release a newer lock. Treat `force-release` as an explicit recovery action.

## TypeScript SDK

Use `@armature/sdk` when writing Node/TypeScript task or service code. It shells out to the same CLI and reads the same runtime environment; it is not a second runtime.

```ts
import { createArmature, emit, getEvent, getRunContext, withLock } from "@armature/sdk";

const armature = createArmature({ workspace: process.cwd() });
const context = getRunContext();
const event = getEvent<{ requestId: string }>();

await armature.task.add("reviewer", ["node", "agents/reviewer.mjs"], {
  on: "plan.ready",
  correlation: event.correlation_id ?? event.payload.requestId,
});

await emit("review.completed", { ok: true }, {
  correlation: event.correlation_id ?? event.payload.requestId,
});

await withLock("repo:main", async () => {
  console.log(await armature.status());
}, { ttl: "2m", reason: "inspect status" });
```

## Debugging

Start with:

```sh
armature status
armature doctor
armature task list
armature service list
armature run list
armature event list --json
armature trigger list --json
```

For a failed run, inspect `armature run show <run-id>` and `armature run logs <run-id>`. Logs include stdout/stderr paths, byte counts, line counts, and captured stream contents. Recovered runs may include a recovery note if they were active when a previous daemon exited.

## Validation

For this repository, run:

```sh
cargo test
npm test --workspace @armature/sdk
cargo clippy --all-targets -- -D warnings
```

There is an opt-in CLI stress test:

```sh
cargo test -p armature-cli --test e2e -- --ignored sustained_stress_many_events_watch_changes_and_services
```

If CLI e2e tests uniformly fail with `No such file or directory` while spawning `armature`, stale Cargo artifacts may contain an old absolute `CARGO_BIN_EXE_armature` path. Run `cargo clean -p armature-cli` and retry.

## References

Read these repo files when changing Armature itself or when details matter:

- `README.md` for the current user-facing command surface.
- `spec/armature-v0.3.md` for the normative v0.3 behavior.
- `spec/dynamic-management-interface.md` for the runtime management design boundary.
- `packages/sdk/README.md` for SDK usage.
