# Whippletree Dynamic Management Interface

Status: design proposal

This document describes the desired CLI/API shape for managing Whippletree as a
runtime scaffold for agent projects. It is intentionally separate from the v0.3
normative spec: not every command described here exists yet.

The goal is to make Whippletree a reusable coordination layer for ordinary
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

Whippletree should provide that plumbing with a clear object model and a predictable
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

**Whippletree owns invocation truth. User code owns operational meaning.**

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
whip task run test
whip run test

whip event emit plan.ready --json '{"requestId":"req-123"}'
whip emit plan.ready --json '{"requestId":"req-123"}'
```

The first form is canonical. The second form is an alias.

## 3. Ontology

Whippletree's interface should distinguish definitions from records.

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

Services are definitions with reconciliation. Whippletree should keep enabled
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

A source is anything that emits Whippletree events:

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
whip event emit plan.ready --correlation req-123 --json '{"ok":true}'
```

Events are coordination messages, not durable workflow promises. They may carry
mechanical provenance such as source run, parent event, and correlation id.

### 3.6 Trigger

A trigger is a routing/admission record produced by Whippletree. If an event causes
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
be good enough that Whippletree does not need a structured result object for v0.x.

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
whip task list
whip task show <name>
whip task run <name>
whip task add <name> --on EVENT -- <cmd...>
whip task add <name> --watch GLOB --settle 500ms -- <cmd...>
whip task add <name> --schedule CRON -- <cmd...>
whip task enable <name>
whip task disable <name>
whip task remove <name>
```

Task add creates a dynamic task definition. Dynamic task definitions should be
inspectable and marked as dynamic.

Initial implementations may support ephemeral dynamic tasks only. Persistent
dynamic tasks should be added only after the storage model is explicit.

### 4.2 Service Commands

```sh
whip service list
whip service show <name>
whip service add <name> -- <cmd...>
whip service start <name>
whip service stop <name>
whip service restart <name>
whip service enable <name>
whip service disable <name>
whip service remove <name>
whip service health <name>
```

Service add creates a dynamic service definition. Dynamic services should be
inspectable and marked as dynamic.

Initial implementations may support ephemeral dynamic services only.

### 4.3 Run Commands

```sh
whip run list
whip run show <run-id>
whip run start --name NAME -- <cmd...>
whip run cancel <run-id>
whip run logs <run-id>
whip run wait <run-id>
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
whip event list
whip event show <event-id>
whip event emit <type> [--json JSON | --payload-file PATH | --stdin]
whip event wait <type>
```

Useful list filters:

```sh
--type TYPE
--source SOURCE
--correlation ID
--limit N
```

Whippletree should not add a separate `publish` command. `emit` is the canonical
event creation verb.

### 4.5 Trigger Commands

```sh
whip trigger list
whip trigger show <trigger-id>
whip trigger wait --task NAME
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
whip trigger retry <trigger-id>
```

Retry should be deferred. If added, it must be defined mechanically as "create a
new run for the same task/event context" rather than a semantic workflow retry.

### 4.6 Log Commands

```sh
whip log show <run-id>
whip log tail <run-id> --lines 100
whip log follow <run-id>
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
whip lock acquire <name> --ttl 10m --reason "editing branch"
whip lock renew <name> --token lock_... --ttl 10m
whip lock release <name> --token lock_...
whip lock force-release <name> --reason "holder died"
whip lock list
whip lock show <name>
whip lock with <name> --ttl 10m -- <cmd...>
```

Lock semantics are detailed in section 7.

### 4.8 Wait Commands

```sh
whip wait event <type> --correlation req-123
whip wait run <run-id> --state exited
whip wait trigger --task reviewer --outcome started
whip wait service <name> --state running
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
whip subscribe events
whip subscribe runs
whip subscribe triggers
```

Subscribe should stream newline-delimited JSON. It should be an observation API,
not a broker protocol.

## 5. Ergonomic Aliases

Top-level aliases should exist for common operations:

```sh
whip tasks                 # task list
whip services              # service list
whip runs                  # run list
whip events                # event list
whippletree triggers              # trigger list

whip run <task>            # task run <task>
whip exec -- <cmd...>      # run start -- <cmd...>
whip emit <type>           # event emit <type>
whip logs <run-id>         # run logs <run-id>
whip cancel <run-id>       # run cancel <run-id>
whip ps                    # active runs view
```

Canonical docs should teach the object-oriented form when explaining the model
and the alias form when showing common workflows.

## 6. Dynamic Definitions

Dynamic definitions are runtime-created task/service definitions:

```sh
whip task add reviewer --on plan.ready -- node reviewer.mjs
whip service add github-source -- node sources/github.mjs
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
definitions should not rewrite the user's primary `.whippletree/project.whip`
without explicit consent. A separate Whippletree-managed dynamic definition file is
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
whip lock acquire repo:main --ttl 10m --reason "review req-482"
```

Acquire returns the lock record, including token.

If the lock exists and is not expired, acquire fails with conflict.

If the lock exists but is expired, the daemon may replace it.

When acquire is called from inside an Whippletree-managed run, owner fields should
be inferred from environment:

```text
WHIPPLETREE_RUN_ID
WHIPPLETREE_NAME
WHIPPLETREE_CORRELATION_ID
```

### 7.3 Renew

```sh
whip lock renew repo:main --token lock_... --ttl 10m
```

Renew requires the current fencing token.

Renew should update `renewed_at_ms` and `expires_at_ms`.

### 7.4 Release

Canonical release requires a token:

```sh
whip lock release repo:main --token lock_...
```

This prevents stale holders from releasing newer locks.

For ergonomics, tokenless release may be allowed only when the caller is the same
Whippletree run that owns the lock:

```sh
whip lock release repo:main
```

The daemon may accept this only if:

```text
caller WHIPPLETREE_RUN_ID == lock.owner_run_id
```

Outside the owning run context, tokenless release must fail and explain how to
use `--token` or `force-release`.

### 7.5 Force Release

Force release is the administrative recovery path:

```sh
whip lock force-release repo:main --reason "holder crashed and TTL is too long"
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
whip lock with repo:main --ttl 10m --reason "run tests" -- npm test
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

Whippletree should track mechanical causality, not semantic workflow traces.

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

When `event emit` is called from inside an Whippletree-managed run, Whippletree should
infer:

```text
source_run_id    from WHIPPLETREE_RUN_ID
parent_event_id  from WHIPPLETREE_EVENT_ID
correlation_id   from WHIPPLETREE_CORRELATION_ID, unless overridden
source           from WHIPPLETREE_NAME, unless overridden
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
whip event list --type plan.ready --correlation req-123
whip trigger list --task reviewer --outcome rejected
whip run list --name worker --state failed
whip lock list --expired
whip overview --json
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
WHIPPLETREE_FORMAT=json
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

Whippletree may carry labels, names, correlation IDs, and provenance. User code
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
