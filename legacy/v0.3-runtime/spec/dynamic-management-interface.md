# Armature Dynamic Management Interface

Status: design proposal

This document describes the desired CLI/API shape for managing Armature as a
runtime scaffold for agent projects. It is intentionally separate from the v0.3
normative spec: not every command described here exists yet.

The goal is to make Armature a reusable coordination layer for ordinary
long-running processes without turning it into a workflow engine, tracing
system, or agent framework.

## 1. Design Goal

Agent projects repeatedly need the same mechanical runtime plumbing:

```text
launch processes
keep services alive
route events to handlers
capture logs
track run state
coordinate shared resources
wait for runtime conditions
recover from crashes
inspect what happened
```

Armature should provide that plumbing with a clear object model and a predictable
CLI. User code remains responsible for semantic meaning:

```text
planning
reviewing
approving
retry decisions
deduplication
workflow state
success criteria
domain-specific conflict handling
```

The boundary remains:

**Armature owns invocation truth. User code owns operational meaning.**

## 2. Interface Principles

The dynamic management interface should follow these principles:

1. Prefer canonical object-oriented commands.
2. Provide short ergonomic aliases for common operations.
3. Be permissive where ambiguity is low.
4. Make every dynamic thing inspectable.
5. Keep dynamic runtime state mechanical.
6. Avoid hidden workflow state.
7. Use familiar CLI patterns for agents and humans.
8. Keep simple cases short, and allow expressiveness incrementally.

For example:

```sh
armature task run test
armature run test

armature event emit plan.ready --json '{"requestId":"req-123"}'
armature emit plan.ready --json '{"requestId":"req-123"}'
```

The first form is canonical. The second form is an alias.

## 3. Ontology

Armature's interface should distinguish definitions from records.

Definitions describe desired behavior:

```text
Task       finite command template
Service    long-running command template
Source     event-producing service or mechanism
```

Records describe what happened:

```text
Run        process invocation
Event      recorded message
Trigger    routing/admission decision
Log        captured run output
Lock       resource lease
Health     service health observation/view
```

Other runtime concepts:

```text
Workspace  project/runtime boundary
Config     static definition set
Wait       blocking observation of runtime state
Subscribe  streaming observation of runtime state
```

### 3.1 Workspace

A workspace is the coordination boundary. It answers:

```text
Which config is active?
Where is daemon state?
Which tasks, services, locks, runs, and events belong together?
```

Agent projects usually map a workspace to a repository, checkout, or local
coordination domain.

### 3.2 Task

A task is a finite command template. It may be started manually or by a trigger:

```toml
[[task]]
name = "review-pr"
on = "pr.opened"
run = "node agents/review-pr.mjs"
```

Tasks are definitions. Each execution of a task creates a run.

Agents use tasks for stable capabilities:

```text
run tests
handle plan.ready
respond to file changes
perform scheduled checks
invoke a reusable worker script
```

### 3.3 Service

A service is a long-running command template:

```toml
[[service]]
name = "github-source"
run = "node sources/github-events.mjs"
```

Services are definitions with reconciliation. Armature should keep enabled
services running according to mechanical supervision policy.

Agents use services for:

```text
event sources
webhook listeners
pollers
worker pools
MCP bridges
model servers
log tailers
```

### 3.4 Source

A source is anything that emits Armature events:

```text
manual CLI emit
built-in schedule
built-in file watch
user-authored service
external bridge
```

Source is currently mostly a role. A future `source` command should only be
added if it proves clearer than representing user-authored sources as services
with source role metadata.

### 3.5 Event

An event is a recorded message:

```sh
armature event emit plan.ready --correlation req-123 --json '{"ok":true}'
```

Events are coordination messages, not durable workflow promises. They may carry
mechanical provenance such as source run, parent event, and correlation id.

### 3.6 Trigger

A trigger is a routing/admission record produced by Armature. If an event causes
a task to start, the trigger records:

```text
which event was evaluated
which task matched
which admission policy applied
whether the task started, queued, coalesced, rejected, or superseded
which run was created, if any
```

Users usually inspect triggers; they do not normally create triggers directly.

### 3.7 Run

A run is an actual process invocation. Runs are created by:

```text
manual task invocation
event/watch/schedule trigger
service reconciliation
ad hoc command execution
```

A run records command, lifecycle, logs, config version, event cause, provenance,
and correlation.

Runs are the primary answer to:

```text
What is currently happening?
What happened for this request?
What failed?
Where are the logs?
```

### 3.8 Log

A log is captured stdout/stderr plus mechanical metadata for a run. Logs should
be good enough that Armature does not need a structured result object for v0.x.

### 3.9 Lock

A lock is a TTL-backed lease on a named resource:

```text
repo:main
worktree:review-482
cache:index
port:3000
model:local-gpu
```

Locks are core to agent runtime safety because multiple agents may try to mutate
the same resource.

### 3.10 Wait / Subscribe

Wait and subscribe are observation primitives:

```text
wait       block until a condition is true
subscribe  stream future changes as newline-delimited JSON
```

They should not add workflow semantics. They only observe mechanical runtime
state.

### 3.11 Overview

Overview is a compact read-only runtime projection for humans and agents:

```text
configured tasks and services
active runs
latest run per task/service
queued trigger counts
recent trigger outcomes
recent events
recent failures
```

Overview should reduce custom status scripting for agent projects without
becoming a workflow verdict. It must not inspect or interpret repo-owned
application state such as task ledgers, quality decisions, or artifacts.

## 4. Canonical CLI

The canonical CLI should map to object types.

### 4.1 Task Commands

```sh
armature task list
armature task show <name>
armature task run <name>
armature task add <name> --on EVENT -- <cmd...>
armature task add <name> --watch GLOB --settle 500ms -- <cmd...>
armature task add <name> --schedule CRON -- <cmd...>
armature task enable <name>
armature task disable <name>
armature task remove <name>
```

Task add creates a dynamic task definition. Dynamic task definitions should be
inspectable and marked as dynamic.

Initial implementations may support ephemeral dynamic tasks only. Persistent
dynamic tasks should be added only after the storage model is explicit.

### 4.2 Service Commands

```sh
armature service list
armature service show <name>
armature service add <name> -- <cmd...>
armature service start <name>
armature service stop <name>
armature service restart <name>
armature service enable <name>
armature service disable <name>
armature service remove <name>
armature service health <name>
```

Service add creates a dynamic service definition. Dynamic services should be
inspectable and marked as dynamic.

Initial implementations may support ephemeral dynamic services only.

### 4.3 Run Commands

```sh
armature run list
armature run show <run-id>
armature run start --name NAME -- <cmd...>
armature run cancel <run-id>
armature run logs <run-id>
armature run wait <run-id>
```

`run start` creates an ad hoc tracked process. It does not create a task
definition and does not register future trigger behavior.

Useful options:

```sh
--name NAME
--correlation ID
--cwd DIR
--env KEY=VALUE
--timeout DURATION
--payload-file PATH
--stdin
```

### 4.4 Event Commands

```sh
armature event list
armature event show <event-id>
armature event emit <type> [--json JSON | --payload-file PATH | --stdin]
armature event wait <type>
```

Useful list filters:

```sh
--type TYPE
--source SOURCE
--correlation ID
--limit N
```

Armature should not add a separate `publish` command. `emit` is the canonical
event creation verb.

### 4.5 Trigger Commands

```sh
armature trigger list
armature trigger show <trigger-id>
armature trigger wait --task NAME
```

Useful list filters:

```sh
--task NAME
--event EVENT_TYPE
--outcome OUTCOME
--correlation ID
--limit N
```

Potential future command:

```sh
armature trigger retry <trigger-id>
```

Retry should be deferred. If added, it must be defined mechanically as "create a
new run for the same task/event context" rather than a semantic workflow retry.

### 4.6 Log Commands

```sh
armature log show <run-id>
armature log tail <run-id> --lines 100
armature log follow <run-id>
```

Logs should expose:

```text
run metadata
run directory
stdout/stderr paths
byte counts
line counts
truncation flags
missing-file flags
stream contents
```

### 4.7 Lock Commands

```sh
armature lock acquire <name> --ttl 10m --reason "editing branch"
armature lock renew <name> --token lock_... --ttl 10m
armature lock release <name> --token lock_...
armature lock force-release <name> --reason "holder died"
armature lock list
armature lock show <name>
armature lock with <name> --ttl 10m -- <cmd...>
```

Lock semantics are detailed in section 7.

### 4.8 Wait Commands

```sh
armature wait event <type> --correlation req-123
armature wait run <run-id> --state exited
armature wait trigger --task reviewer --outcome started
armature wait service <name> --state running
```

Useful options:

```sh
--timeout DURATION
--json
```

Wait commands should exit successfully when the condition is met and non-zero on
timeout.

### 4.9 Subscribe Commands

```sh
armature subscribe events
armature subscribe runs
armature subscribe triggers
```

Subscribe should stream newline-delimited JSON. It should be an observation API,
not a broker protocol.

## 5. Ergonomic Aliases

Top-level aliases should exist for common operations:

```sh
armature tasks                 # task list
armature services              # service list
armature runs                  # run list
armature events                # event list
armature triggers              # trigger list

armature run <task>            # task run <task>
armature exec -- <cmd...>      # run start -- <cmd...>
armature emit <type>           # event emit <type>
armature logs <run-id>         # run logs <run-id>
armature cancel <run-id>       # run cancel <run-id>
armature ps                    # active runs view
```

Canonical docs should teach the object-oriented form when explaining the model
and the alias form when showing common workflows.

## 6. Dynamic Definitions

Dynamic definitions are runtime-created task/service definitions:

```sh
armature task add reviewer --on plan.ready -- node reviewer.mjs
armature service add github-source -- node sources/github.mjs
```

Dynamic definitions must be inspectable:

```json
{
  "name": "reviewer",
  "kind": "task",
  "dynamic": true,
  "created_by_run_id": "run_...",
  "correlation_id": "req-482",
  "command": "node reviewer.mjs"
}
```

Initial semantics should be ephemeral:

```text
dynamic definitions live until removed, daemon shutdown, or workspace reset
```

Persistence should be designed separately. If added, persistent dynamic
definitions should not rewrite the user's primary `.armature/armature.toml`
without explicit consent. A separate Armature-managed dynamic definition file is
preferable.

## 7. Lock Semantics

Locks must avoid both unsafe release and permanent stuck state.

### 7.1 Lock Record

A lock record should contain:

```json
{
  "name": "repo:main",
  "token": "lock_...",
  "owner_pid": 1234,
  "owner_run_id": "run_...",
  "owner_name": "worker",
  "correlation_id": "req-482",
  "reason": "editing branch",
  "acquired_at_ms": 123,
  "renewed_at_ms": 456,
  "expires_at_ms": 789
}
```

`token` is a fencing token. It proves that a caller is acting on the current
lease, not a stale lease with the same name.

### 7.2 Acquire

```sh
armature lock acquire repo:main --ttl 10m --reason "review req-482"
```

Acquire returns the lock record, including token.

If the lock exists and is not expired, acquire fails with conflict.

If the lock exists but is expired, the daemon may replace it.

When acquire is called from inside an Armature-managed run, owner fields should
be inferred from environment:

```text
ARMATURE_RUN_ID
ARMATURE_NAME
ARMATURE_CORRELATION_ID
```

### 7.3 Renew

```sh
armature lock renew repo:main --token lock_... --ttl 10m
```

Renew requires the current fencing token.

Renew should update `renewed_at_ms` and `expires_at_ms`.

### 7.4 Release

Canonical release requires a token:

```sh
armature lock release repo:main --token lock_...
```

This prevents stale holders from releasing newer locks.

For ergonomics, tokenless release may be allowed only when the caller is the same
Armature run that owns the lock:

```sh
armature lock release repo:main
```

The daemon may accept this only if:

```text
caller ARMATURE_RUN_ID == lock.owner_run_id
```

Outside the owning run context, tokenless release must fail and explain how to
use `--token` or `force-release`.

### 7.5 Force Release

Force release is the administrative recovery path:

```sh
armature lock force-release repo:main --reason "holder crashed and TTL is too long"
```

Force release should require a reason and should be inspectable. It should record
an audit event or equivalent record containing:

```text
lock name
previous owner
previous token
reason
time
caller context, if available
```

Force release is necessary because tokens and TTLs improve safety but must not
create unrecoverable deadlocks.

### 7.6 With-Lock

`with-lock` is the ergonomic safe path:

```sh
armature lock with repo:main --ttl 10m --reason "run tests" -- npm test
```

The daemon/CLI should:

```text
acquire the lock
run the command
release with the returned token
release on interruption when possible
return the command exit code
```

Optional future behavior:

```text
automatic renewal while the command is running
```

## 8. Provenance and Correlation

Armature should track mechanical causality, not semantic workflow traces.

Good mechanical provenance fields:

```text
source_run_id
parent_event_id
correlation_id
config_version
run_id
event_id
trigger_id
```

When `event emit` is called from inside an Armature-managed run, Armature should
infer:

```text
source_run_id    from ARMATURE_RUN_ID
parent_event_id  from ARMATURE_EVENT_ID
correlation_id   from ARMATURE_CORRELATION_ID, unless overridden
source           from ARMATURE_NAME, unless overridden
```

These fields answer:

```text
which process emitted this event?
which event caused that process?
which records belong to the same request?
```

They must not imply:

```text
workflow step
semantic success
business state
approval status
retry policy
```

## 9. Query Design

Every record list should support practical filters.

Examples:

```sh
armature event list --type plan.ready --correlation req-123
armature trigger list --task reviewer --outcome rejected
armature run list --name worker --state failed
armature lock list --expired
armature overview --json
```

Filtering is not a workflow query language. It is runtime inspection.

## 10. Machine-Oriented Behavior

Agent callers need predictable machine behavior:

```text
JSON output
structured errors
stable exit codes
timeouts
idempotency hooks where appropriate
```

Recommended environment default:

```sh
ARMATURE_FORMAT=json
```

Potential future flags:

```sh
--idempotency-key KEY
--timeout DURATION
```

Idempotency should be added carefully and only for mechanical effects such as
event creation or ad hoc run creation.

## 11. Non-Goals

The dynamic management interface must not introduce:

```text
workflow DAGs
durable promises
semantic retries
approval semantics
planner/worker roles as daemon concepts
agent graphs
business state machines
semantic deduplication
```

Armature may carry labels, names, correlation IDs, and provenance. User code
decides what they mean.

## 12. Suggested Implementation Order

1. Stabilize object-oriented list/show aliases.
2. Add `run start` / `exec` for ad hoc tracked commands.
3. Add `wait` for events, runs, triggers, and services.
4. Add `lock with` and force-release audit.
5. Add ephemeral `service add/remove`.
6. Add ephemeral `task add/remove`.
7. Evaluate dynamic persistence after real usage.

This order gives agents immediate value while preserving a simple mental model.
