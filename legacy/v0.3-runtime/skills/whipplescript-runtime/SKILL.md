---
name: whipplescript-runtime
description: Use when coding agents need to run, supervise, observe, or coordinate local project automation with WhippleScript; configure WhippleScript workspaces; write tasks, services, event handlers, locks, or TypeScript SDK integrations; or debug WhippleScript-managed runs, logs, triggers, and daemon state.
---

# WhippleScript Runtime

Use WhippleScript as local runtime plumbing for ordinary programs. Keep the boundary clear:

- WhippleScript owns mechanical invocation truth: process start/stop, supervision, events, triggers, runs, logs, locks, waits, and daemon state.
- User code owns operational meaning: planning, approval, retry decisions, deduplication, fanout, workflow state, and success criteria.

Do not introduce workflow DAGs, durable promises, semantic retries, semantic dedupe, agent graphs, or hidden workflow state unless the user explicitly asks for a layer outside WhippleScript.

## First Checks

1. Confirm the CLI is available: `whip --help`. If working from source, build it with `cargo build -p whipplescript-cli` and use `target/debug/whip`.
2. Find or create the workspace. WhippleScript discovers `.whipplescript/project.whip` by walking upward, or accepts `--workspace /path/to/workspace`.
3. Validate config before starting or reloading: `whip config check`.
4. Start the daemon with `whip up`, or use `whip dev` / `whip up --foreground` while debugging.
5. Prefer machine-readable output while scripting: put `--format json` before the command, for example `whip --format json status`.
6. Use `whip overview` for the compact operational picture before writing custom status scripts.

## Static Definitions

Use `.whipplescript/project.whip` for stable repository behavior.

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

Use `whip up` after config edits. If a daemon is already running, `up` validates and hot-reloads; invalid reloads are rejected and the previous valid config remains active.

## Dynamic Definitions

Use dynamic definitions for ephemeral runtime wiring that should not edit `.whipplescript/project.whip`.

```sh
whip service add github-source --restart on_failure --reason "event bridge" -- node sources/github.mjs
whip task add reviewer --on plan.ready -- node agents/reviewer.mjs
whip service list --dynamic
whip task list --dynamic
whip task remove reviewer
whip service remove github-source
```

Dynamic definitions are inspectable and marked `dynamic: true`, but they are not workflow state and are not persisted into config.

## Canonical Commands

Prefer object-oriented commands in new scripts and docs:

```sh
whip task list
whip task show test
whip task run test
whip service list
whip service show worker
whip service start worker
whip service stop worker
whip event emit plan.ready --correlation req-123 --json '{"ok":true}'
whip event list --correlation req-123 --json
whip trigger list --task reviewer --json
whip run list --correlation req-123
whip run show <run-id>
whip run logs <run-id>
whip run cancel <run-id>
whip overview --json
whip wait event work.completed --correlation req-123 --timeout 5m
whip subscribe events
```

Aliases such as `whip tasks`, `whip services`, `whip runs`, `whip logs`, `whip emit`, and `whip run <task>` exist for compatibility, but canonical commands are clearer for agents.

## Events And Provenance

Emit events to route work to `on = "..."` tasks:

```sh
whip event emit build.completed --source agent:builder --correlation req-123 --json '{"ok":true}'
whip event emit build.completed --payload-file event.json
printf '%s\n' '{"ok":true}' | whip event emit build.completed --stdin
```

Use exactly one payload source: `--json`, `--payload-file`, or `--stdin`. If no payload is supplied, WhippleScript records `{}`.

Prefer `--payload-file` or `--stdin` for non-trivial payloads. Shell quoting bugs can quickly become project state if scheduled/event-triggered scripts trust malformed input. Event handlers should validate payload shape before mutating repo-owned state.

WhippleScript records accepted events before routing them. Trigger outcomes are inspectable and can be `started`, `queued`, `coalesced`, `rejected`, or `superseded` depending on admission policy.

WhippleScript does not provide durable queues, replay cursors, exactly-once delivery, or semantic deduplication. If the daemon is down, emit fails instead of buffering.

## Runtime Context

WhippleScript-managed task and service processes receive context through environment variables including:

- `WHIPPLESCRIPT_RUN_ID`
- `WHIPPLESCRIPT_RUN_DIR`
- `WHIPPLESCRIPT_WORKSPACE_ROOT`
- `WHIPPLESCRIPT_EVENT_TYPE`
- `WHIPPLESCRIPT_EVENT_JSON`
- `WHIPPLESCRIPT_EVENT_PATH`
- `WHIPPLESCRIPT_CORRELATION_ID`

When an WhippleScript-managed process calls `whip event emit` or `whip task run`, the CLI forwards mechanical provenance from the environment so records may include `source_run_id`, `parent_event_id`, and `correlation_id`.

## Locks

Use named locks for local critical sections:

```sh
whip --format json lock acquire branch:main --ttl 10m --reason "deploy"
whip lock renew branch:main --token "$TOKEN" --ttl 10m
whip lock release branch:main --token "$TOKEN"
whip lock with branch:main --ttl 2m --reason "run tests" -- npm test
whip lock list --expired
whip lock force-release branch:main --reason "holder exited"
```

Locks are workspace-scoped and TTL-backed. Acquire returns a fencing token; renew and release require that token so an expired holder cannot release a newer lock. Treat `force-release` as an explicit recovery action.

## TypeScript SDK

Use `@whipplescript/sdk` when writing Node/TypeScript task or service code. It shells out to the same CLI and reads the same runtime environment; it is not a second runtime.

```ts
import { createWhippleScript, emit, getEvent, getRunContext, withLock } from "@whipplescript/sdk";

const whipplescript = createWhippleScript({ workspace: process.cwd() });
const context = getRunContext();
const event = getEvent<{ requestId: string }>();

await whipplescript.task.add("reviewer", ["node", "agents/reviewer.mjs"], {
  on: "plan.ready",
  correlation: event.correlation_id ?? event.payload.requestId,
});

await emit("review.completed", { ok: true }, {
  correlation: event.correlation_id ?? event.payload.requestId,
});

await withLock("repo:main", async () => {
  console.log(await whipplescript.status());
}, { ttl: "2m", reason: "inspect status" });
```

## Debugging

Start with:

```sh
whip overview
whip status
whip doctor
whip task list
whip service list
whip run list
whip event list --json
whip trigger list --json
```

`whip overview` summarizes configured tasks/services, live active run ids, queued trigger counts, latest run per task/service, recent failures, recent events, and recent trigger outcomes. Treat it as a mechanical status view, not a workflow verdict.

For a failed run, inspect `whip run show <run-id>` and `whip run logs <run-id>`. Logs include stdout/stderr paths, byte counts, line counts, and captured stream contents. Recovered runs may include a recovery note if they were active when a previous daemon exited.

For recurring agent loops, a good pattern is:

1. A scheduled director task wakes up on a cron interval.
2. The director reads repo-owned state, exits if active work already exists, and emits a request event only when new work should start.
3. Event-triggered tasks handle request/completion/quality events.
4. Repo scripts own `tasks.json`, artifacts, quality gates, locking, dedupe, and semantic success/failure.
5. WhippleScript owns timers, event dispatch, run capture, logs, trigger outcomes, waits, and compact inspection.

## Validation

For this repository, run:

```sh
cargo test
npm test --workspace @whipplescript/sdk
cargo clippy --all-targets -- -D warnings
```

There is an opt-in CLI stress test:

```sh
cargo test -p whipplescript-cli --test e2e -- --ignored sustained_stress_many_events_watch_changes_and_services
```

If CLI e2e tests uniformly fail with `No such file or directory` while spawning `whip`, stale Cargo artifacts may contain an old absolute `CARGO_BIN_EXE_whip` path. Run `cargo clean -p whipplescript-cli` and retry.

## References

Read these repo files when changing WhippleScript itself or when details matter:

- `README.md` for the current user-facing command surface.
- `spec/whipplescript-v0.3.md` for the normative v0.3 behavior.
- `spec/dynamic-management-interface.md` for the runtime management design boundary.
- `packages/sdk/README.md` for SDK usage.
