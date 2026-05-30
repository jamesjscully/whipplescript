# WhippleScript v0.3 - Implementation Plan

This plan fixes the implementation decisions for the first build of WhippleScript v0.3.

It is subordinate to `spec/whipplescript-v0.3.md`. If this file appears to introduce workflow semantics, the normative specification wins.

## 1. Implementation Goal

WhippleScript v0.3 should produce a complete local daemon runtime:

```text
whip init
whip dev
whip run <task>
whip emit <event-type>
whip status
whip ps
whip tasks
whip services
whip runs
whip logs <run-id>
whip cancel <run-id>
whip config check
whip doctor
whip lock acquire <name>
whip lock release <name>
whip lock status
```

It should support:

```text
manual tasks
scheduled tasks
file-watch tasks
event-triggered tasks
long-running services
service supervision
health checks
basic admission policy
hard kill timeouts
named locks
event history
run history
separate stdout/stderr logs
per-run directories
hot config reload
runtime inspection
complete TypeScript SDK for core functionality
recipes as editable scaffolding
```

It should not include:

```text
Windows support
config migrations
built-in external adapters
capability policy
whip plan
cloud or distributed coordination
workflow DAGs
durable promises
agent graphs
semantic retries
semantic dedupe
daemon-owned traces
```

## 2. Runtime Shape

v0.3 supports a normal local daemon lifecycle and an explicit foreground mode.

`whip dev` starts the daemon in the foreground and owns the workspace until it exits.

`whip up` starts or reconciles the daemon runtime for the workspace without attaching it to the current terminal.

Foreground operation must be explicit through `whip dev` or an explicit foreground option. `whip up` without such an option should not run in the foreground.

The CLI may invoke runtime commands against the active daemon.

If no daemon owns the workspace, commands that require a running daemon should fail clearly.

Commands that do not require a running daemon may operate directly on the workspace:

```text
whip init
whip config check
whip doctor
```

`whip down` stops the daemon runtime for the workspace.

`whip restart` performs `down` then `up`.

The daemon should remain local to the machine and workspace. v0.3 should not install launchd or systemd services unless the user explicitly asks for that in a future version.

## 3. Language and Packages

The daemon and CLI should be implemented in Rust.

The TypeScript SDK should be implemented now and should be fully usable for v0.3 core functionality.

Recommended package layout:

```text
crates/
  whipplescript-cli/
  whipplescript-daemon/
  whipplescript-core/
packages/
  sdk/
```

The Rust CLI and daemon may initially live in one binary if that accelerates the first vertical slice, but the code should keep daemon, config, store, process, trigger, and CLI boundaries clear.

## 4. Workspace Discovery

Workspace discovery walks upward from the current working directory until it finds:

```text
.whipplescript/project.whip
```

WhippleScript must not search downward into subdirectories.

If a user is inside a nested project, the nearest ancestor with `.whipplescript/project.whip` is the workspace.

An explicit workspace flag should override discovery:

```bash
whip --workspace /path/to/workspace status
```

The workspace root is the directory containing `.whipplescript/`.

## 5. State Location

SQLite is the v0.3 store.

The SQLite database must not live in the working tree by default.

The workspace `.whipplescript/` directory may contain user-editable config and run artifacts, but the internal database should live under an WhippleScript-controlled state root outside the repository checkout.

Recommended default:

```text
$XDG_STATE_HOME/whipplescript/workspaces/<workspace-id>/whipplescript.sqlite
```

Fallback:

```text
~/.local/state/whipplescript/workspaces/<workspace-id>/whipplescript.sqlite
```

The workspace id should be a stable hash of the canonical workspace path.

The state root should also contain runtime control files:

```text
daemon.sock
daemon.pid
workspace.lock
```

The database is an internal daemon store. Users and agents may inspect exported JSON or CLI output, but should not edit the database directly.

## 6. Local Transport

The CLI talks to the daemon over a Unix domain socket.

The socket lives in the workspace state root, not in the repository checkout.

The protocol should be simple request/response JSON over the socket for v0.3.

The transport must support:

```text
emit event
run task
cancel run
query status
query tasks
query services
query runs
stream or fetch logs
lock acquire
lock release
```

No network listener is required for v0.3.

## 7. Config

Config format is TOML.

Default path:

```text
.whipplescript/project.whip
```

The daemon validates config on startup and reload.

Invalid reloads must be rejected while the previous valid config remains active.

Config version is a stable hash of normalized config content.

Each run and accepted event records the active config version.

## 8. Process Model

Each task, service, health check, and mechanical restart is an ordinary OS subprocess.

Each subprocess spawn creates one run record.

Each run receives:

```text
run id
run directory
stdout log
stderr log
environment variables
working directory
config version
triggering event, if any
```

The daemon should place each child in its own process group on Unix.

Cancelling or timing out a run should terminate the process group.

Process-group handling is a mechanical containment rule, not a workflow model.

## 9. Command Execution

String commands execute through the user shell for v0.3.

Unix behavior:

```text
sh -c "<command>"
```

This makes ordinary config ergonomic:

```toml
run = "npm test"
```

Future versions may add an argv-array form for exact execution. v0.3 does not need it.

## 10. IDs

Run ids and event ids should be ULIDs.

IDs should be sortable, compact enough for CLI use, and safe in filenames.

Recommended forms:

```text
run_01HV...
evt_01HV...
```

## 11. Event Model

Event delivery follows the narrow v0.3 semantics:

```text
append atomically
route once against active config
do not replay historical events automatically
do not buffer while daemon is down
do not promise exactly-once delivery
```

If `whip emit` cannot reach the daemon, it fails clearly.

Built-in schedules and file watchers emit mechanical events into the same event log as user-authored sources.

The daemon must avoid double-starting a task from both a direct hidden trigger and a routable event for the same occurrence.

## 12. Trigger Implementation

All trigger classes should feed the same event-routing path:

```text
manual trigger
schedule trigger
file-watch trigger
event trigger
```

Config sugar remains user-facing:

```toml
schedule = "0 9 * * *"
watch = ["src/**/*.ts"]
on = "tool.run.completed"
```

The implementation may normalize these internally as events before admission policy is evaluated.

## 13. Admission Policy

Default task admission is:

```text
allow
```

Supported v0.3 values:

```text
allow
reject
restart
queue_one
queue_all
```

If a trigger is rejected, queued, coalesced, or superseded, that fact must be recorded and inspectable.

Admission is per task. It does not create cross-task workflow ordering.

## 14. Services

Services are declared long-running commands.

The daemon reconciles enabled services toward running while `whip dev` is active, subject to:

```text
user stop overrides
restart budget exhaustion
backoff
invalid config reloads
spawn errors
health check failures
explicit shutdown
```

Service state should expose:

```text
configured enabled state
user override state
observed process state
supervision state
restart/backoff state
health state, if configured
```

`whip service stop <name>` sets a stop override.

`whip service start <name>` clears the stop override and starts/reconciles the service.

`whip service restart <name>` clears the stop override and restarts the service.

Config reload does not clear stop overrides.

## 15. Supervision

v0.3 supports one-for-one supervision only.

Supported restart modes:

```text
never
on_failure
always
```

Crash-loop controls:

```text
max_restarts
within
backoff
start_delay
```

Mechanical restarts create linked run records.

Task restart is opt-in. Services may use restart policies naturally.

No supervisor trees, group restart strategies, or domain retry semantics are included.

## 16. Health Checks

Health checks are included in v0.3.

Health checks are configured commands attached to services.

Example:

```toml
[service.health]
check = "tsx sources/tool-health.ts"
every = "30s"
timeout = "5s"
```

Health semantics are exit-code based:

```text
exit 0
  healthy

nonzero exit
  unhealthy

timeout
  unhealthy
```

The daemon must not infer domain health from logs or payloads.

Health check executions should be recorded as runs or as inspectable health-check records. If implemented as runs, they must be clearly identified as health runs.

## 17. File Watching

File watching is included in v0.3.

Watch patterns are read from task config.

File events should support settle behavior:

```toml
settle = "300ms"
```

Settling is limited to mechanical editor-write burst handling.

No semantic event dedupe is implemented.

## 18. Scheduling

Scheduled tasks are included in v0.3.

Schedules use cron syntax.

The daemon should emit mechanical timer events and route them through the same task admission path.

The implementation must document timezone behavior. Recommended v0.3 default is the local system timezone.

## 19. Logs and Run Directories

Each run has a private run directory.

Recommended default:

```text
.whipplescript/runs/<run-id>/
  event.json
  meta.json
  stdout.log
  stderr.log
  tmp/
```

Run artifacts are intentionally in the workspace because users and agents need to inspect them.

The SQLite database is outside the workspace; run artifacts are inside the workspace.

Concurrent runs must never share stdout or stderr log files.

## 20. Named Locks

Named locks are included in v0.3.

Locks are raw mechanical mutexes.

Supported commands:

```bash
whip lock acquire <name>
whip lock release <name>
whip lock status
```

Run-owned locks are preferred.

Manual locks should require a lease or be marked as manually owned and inspectable.

Recommended manual lock syntax:

```bash
whip lock acquire branch:main --ttl 10m
```

Lock records include:

```text
name
owner run id, if any
owner process id, if known
acquired time
lease expiration, if any
manual ownership flag
```

The daemon may release locks mechanically when the owning run exits, the owner process is gone, or the lease expires.

The daemon must not infer lock meaning from names.

## 21. TypeScript SDK

The TypeScript SDK is part of v0.3, not deferred.

It must be optional and must not create a second runtime.

It should be complete for v0.3 core functionality while remaining a thin wrapper over WhippleScript runtime facts, environment variables, CLI commands, or daemon transport.

It should wrap:

```text
environment variable access
event parsing
payload parsing
event emission
runtime status queries
task invocation
run queries
service queries
log access
named locks
structured logging
JSON file helpers for run directories
```

Required SDK helpers:

```ts
getRunContext()
getEvent()
getPayload()
emit()
run()
status()
tasks()
services()
runs()
logs()
cancel()
withLock()
lock()
unlock()
log()
readJson()
writeJson()
```

SDK helpers should either use environment variables or call the WhippleScript CLI/daemon transport.

The SDK must not expose workflow, activity, durable promise, agent graph, managed join, managed race, semantic retry, or semantic dedupe helpers.

## 22. Recipes

Recipes are included in v0.3 as editable scaffolding.

Recipes generate ordinary files and do not create hidden daemon behavior.

Recommended command:

```bash
whip init recipe <name>
```

Recipes may create:

```text
.whipplescript/project.whip
scripts/
sources/
package.json
```

Recipe output should be plain project code that users and agents can edit.

## 23. CLI Scope

Required v0.3 commands:

```bash
whip init
whip init recipe <name>
whip dev
whip up
whip down
whip restart
whip run <task-name>
whip emit <event-type> --json <payload>
whip status
whip ps
whip tasks
whip services
whip service start <name>
whip service stop <name>
whip service restart <name>
whip runs
whip logs <run-id>
whip cancel <run-id>
whip config check
whip doctor
whip lock acquire <name>
whip lock release <name>
whip lock status
```

Reserved or deferred:

```bash
whip plan
```

Foreground operation must remain explicit through `whip dev` or a foreground option.

## 24. Validation Boundary

The daemon validates:

```text
WhippleScript config schema
event envelope schema
run status transitions
service state transitions
lock ownership records
resource policy values
health check config
duration syntax
cron syntax
watch pattern syntax
```

The daemon does not validate:

```text
domain payload schemas
agent states
review quality
workflow phase
semantic success
semantic failure
business progress
trace meaning
heartbeat meaning
branch safety
```

The TypeScript SDK may provide user-space payload validation helpers, but those helpers do not affect daemon semantics.

## 25. First Vertical Slice

Build the first implementation in this order:

1. Rust workspace and CLI skeleton.
2. Workspace discovery and `whip init`.
3. TOML config parse and `whip config check`.
4. Daemon lifecycle with workspace lock and Unix socket.
5. SQLite store outside the workspace.
6. Manual task execution with per-run directory and logs.
7. `runs`, `logs`, `status`, and `ps`.
8. `emit` and event-triggered tasks.
9. Services and service reconciliation.
10. Supervision and restart accounting.
11. File watching and settling.
12. Scheduling.
13. Health checks.
14. Named locks.
15. TypeScript SDK.
16. Recipes.
17. Doctor checks and polish.

Each step should preserve the core boundary:

```text
WhippleScript records and controls mechanical runtime facts.
User code owns operational meaning.
```
