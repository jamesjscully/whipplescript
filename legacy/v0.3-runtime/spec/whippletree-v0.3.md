# Whippletree v0.3 - Normative Specification

## 1. Definition

**Whippletree** is a lightweight local daemon and CLI for running **ordinary user-authored programs** in response to schedules, file changes, emitted events, and long-running process sources.

Whippletree exists to support reactive development and multi-agent workflows without becoming a workflow engine.

The core promise is:

**Write normal scripts. Whippletree keeps them wired to the world.**

Whippletree is best understood as:

```text
cron + file watcher + process supervisor + local event log + runtime inspector
```

It is **not**:

```text
Temporal
Airflow
LangGraph
an agent framework
a workflow DSL
a durable promise runtime
a semantic orchestration engine
```

Whippletree invokes user-authored orchestrators. It does not become the orchestrator.

---

## 2. Design Center

Whippletree assumes that the user may be a **competent coding agent** or a technically sophisticated human. Therefore, Whippletree should not hide meaningful control logic behind protective abstractions.

Whippletree should abstract only the machinery that every reliable reactive script runner would otherwise have to reimplement:

```text
detect triggers
launch commands
track processes
capture logs
record events
record runs
hot reload config
supervise long-running services
reconcile declared runtime state
apply explicit admission/resource policy
provide inspection/debugging commands
```

Whippletree should leave all domain-relevant behavior in user space:

```text
agent orchestration
semantic retries
deduplication
result evaluation
trace interpretation
heartbeat meaning
state-machine logic
fanout/join/race
workflow structure
domain-specific conflict detection
branch safety
review logic
success criteria
```

The fundamental boundary is:

**Whippletree owns invocation truth. User code owns operational meaning.**

---

## 3. Normative Language

The terms **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are used normatively.

A conforming Whippletree implementation **MUST** satisfy all **MUST** requirements.

---

## 4. Core Non-Goals

Whippletree **MUST NOT** require users to model work as:

```text
DAGs
workflows
activities
durable promises
agent graphs
state machines
supervisor trees
spans
routes
plans
```

Whippletree **MUST NOT** define privileged daemon-level meanings for:

```text
agent
heartbeat
review
trace
workflow
planner
worker
supervisor, beyond process supervision
retry, beyond mechanical process restart
dedupe
semantic success
semantic failure
```

Whippletree **MAY** provide recipes for common patterns, but recipes **MUST** expand into editable config and editable user scripts.

Whippletree **MUST NOT** require an SDK. Every core feature **SHOULD** be usable from shell scripts.

---

## 5. Core Objects

Whippletree has eight core objects:

```text
Task
Service
Trigger
Source
Event
Run
Log
Runtime
```

These objects are deliberately mechanical.

---

## 6. Task

A **Task** is a finite command run by Whippletree.

A task starts because a trigger fires or because a user invokes it manually.

A task is not a workflow.
A task is not an agent.
A task is not a graph node.

Example:

```toml
[[task]]
name = "test-on-change"
watch = ["src/**/*.ts"]
run = "npm test"
```

Tasks are appropriate for:

```text
run tests after file changes
respond to an emitted event
perform a scheduled check
launch a user-authored orchestration script
run a one-shot maintenance command
```

Tasks **MUST NOT** automatically restart by default.

---

## 7. Service

A **Service** is a long-running command managed by Whippletree.

A service usually exists to observe something, bridge an external tool, host a local webhook receiver, poll a system, or emit events.

Example:

```toml
[[service]]
name = "tool-event-source"
run = "tsx sources/tool-events.ts"

[service.supervision]
restart = "on_failure"
max_restarts = 5
within = "1m"
backoff = "exponential"
```

Services are appropriate for:

```text
event sources
polling loops
webhook listeners
log tailers
tool bridges
custom monitors
```

A service is where BEAM-style supervision most naturally belongs.

---

## 8. Trigger

A **Trigger** is a mechanical condition that starts a task.

Whippletree **SHOULD** support these trigger classes:

```text
manual trigger
schedule trigger
file-watch trigger
event trigger
```

A trigger only decides **when to invoke a command**.

A trigger does not decide what the command means.

For implementation consistency, primitive triggers **SHOULD** be normalized internally as Whippletree events before task admission is evaluated.

Examples:

```text
manual trigger       -> manual.run.requested
schedule trigger     -> timer.fired
file-watch trigger   -> file.changed
event trigger        -> the emitted event itself
```

This normalization is mechanical. It does not require users to subscribe to built-in event names when using config sugar such as `schedule`, `watch`, or manual `whip run`.

Config forms such as:

```toml
schedule = "0 9 * * *"
watch = ["src/**/*.ts"]
on = "tool.run.completed"
```

are declarative trigger shortcuts. The daemon may implement them through a common event-routing path.

---

## 9. Source

A **Source** is a process or built-in mechanism that emits Whippletree events.

Sources may be built in, such as schedules and file watchers, or user-authored, such as a TypeScript script that watches an external CLI, a webhook stream, a chat system, or a local log.

Sources **SHOULD** be shallow. They observe and emit events. They should not become hidden workflow engines.

Preferred source form:

```toml
[[service]]
name = "tool-source"
run = "tsx sources/tool-source.ts"

[service.supervision]
restart = "on_failure"
max_restarts = 5
within = "1m"
```

The service emits events using the CLI or SDK:

```bash
whip emit tool.run.completed --json '{"runId":"abc"}'
```

Built-in sources such as schedules and file watchers **SHOULD** emit mechanical Whippletree events into the same event log used by user-authored sources.

Whippletree **MUST NOT** emit both a hidden direct trigger and a separate routable event for the same occurrence in a way that starts the same task twice.

---

## 10. Event

An **Event** is a JSON record.

Whippletree events are generic. Event type strings are application space.

A conforming event envelope **MUST** contain:

```ts
type WhippletreeEvent = {
  id: string
  type: string
  time: string
  source: string
  payload: unknown
}
```

An event **MAY** contain:

```ts
type OptionalWhippletreeEventFields = {
  workspace?: string
  source_run_id?: string
  parent_event_id?: string
  correlation_id?: string
  labels?: Record<string, string>
}
```

Older SDKs or user payloads may use camelCase fields, but Whippletree-owned event
envelope fields **SHOULD** use the snake_case names above for consistency with
run and trigger records.

Whippletree **MUST NOT** assign domain semantics to event types.

These are all ordinary events:

```text
tool.run.completed
git.branch.changed
file.changed
timer.fired
review.requested
heartbeat.missed
trace.span.closed
```

The daemon records and routes them. User code interprets them.

### 10.1 Event Delivery

Whippletree v0.3 uses narrow local event delivery semantics.

When the daemon accepts an event, it **MUST** append the event atomically to the local event log before routing it to tasks.

Accepted events **SHOULD** be routed once against the active valid config version.

Whippletree v0.3 **MUST NOT** automatically replay historical events to newly added or changed tasks.

Whippletree v0.3 **MUST NOT** provide durable promise, subscription cursor, distributed queue, or exactly-once delivery semantics.

If the daemon is not running, `whip emit` **SHOULD** fail clearly rather than silently buffering events for later routing.

If future versions add offline event insertion, replay, or subscriptions, those behaviors **MUST** be explicit and inspectable.

Event-triggered tasks therefore have local, daemon-mediated, at-most-once routing for each accepted event and active config version. User code owns semantic deduplication and recovery.

---

## 11. Run

A **Run** is one execution of a task or service command.

Each run **MUST** have:

```text
run id
task/service name
command
start time
end time, if finished
status
exit code, if exited
signal, if killed
triggering event, if any
config version
stdout log
stderr log
run directory
```

Valid run statuses **SHOULD** include:

```text
pending
running
succeeded
failed
cancelled
rejected
crashed
timed_out
```

A run is a mechanical execution record. It is not a semantic workflow node.

For v0.3, each OS process spawn **SHOULD** create one run record.

Mechanical restarts **SHOULD** create new run records linked to the original run through explicit lineage metadata such as:

```text
restartOf
attempt
```

Logs remain per run. Whippletree **SHOULD NOT** merge logs from multiple restart attempts into one inseparable log stream.

---

## 12. Log

A **Log** is the captured output and metadata associated with a run.

Whippletree **MUST** keep stdout and stderr inspectable per run.

Concurrent runs **MUST NOT** produce inseparable logs.

Logs **SHOULD** be available through:

```bash
whip logs <run-id>
```

---

## 13. Runtime

A **Runtime** is the daemon's current mechanical view of configured and observed Whippletree-managed processes for a workspace.

Runtime state includes:

```text
daemon status
config version
enabled tasks
enabled services
active runs
pending runs, if any
supervised service states
watcher states
schedule states
recent failures
restart/backoff states
```

Runtime state does **not** include:

```text
workflow phase
semantic task status
agent quality
business meaning
domain progress
strategic completion
```

Whippletree **SHOULD** expose runtime state through CLI and JSON interfaces.

Examples:

```bash
whip overview
whip status
whip ps
whip services
whip tasks
whip runs
whip overview --json
whip status --json
whip ps --json
```

`whip overview` **SHOULD** provide a compact read-only projection of
mechanical runtime state for operators and agents. It should include configured
tasks and services, active runs, latest run per task/service where known, queued
trigger counts, recent trigger outcomes, recent events, and recent failures.
This overview **MUST NOT** infer semantic workflow status from user artifacts.

Runtime state is daemon-owned. Workflow meta-state is user-owned.

For services, runtime status **SHOULD** distinguish at least:

```text
configured state
user override state
observed process state
supervision state
```

These are mechanical desired-state facts. They do not describe domain progress.

---

## 14. System Shape

A typical Whippletree project contains:

```text
.whippletree/
  project.whip
  runs/
  events/
scripts/
sources/
```

The `.whippletree/` directory contains user-inspectable project config and artifacts.

Internal daemon stores, sockets, lock files, and indexes **MAY** live outside the repository checkout. If an implementation stores internal state outside `.whippletree/`, it **SHOULD** expose the state location through `whip doctor` or JSON status.

A typical process tree looks like:

```text
whippletreed
  |- tsx scripts/on-agent-complete.ts
  |- npm test
  |- tsx sources/tool-events.ts
  `- bash scripts/daily-status.sh
```

Each child process is an ordinary OS process.

Whippletree **MUST NOT** embed user TypeScript into the daemon process as the default execution model.

Whippletree **SHOULD** run user code out-of-process.

---

## 15. Daemon Responsibilities

The daemon **MUST** own mechanical runtime behavior.

Specifically, the daemon **MUST** be responsible for:

```text
loading config
validating config
hot reloading config
reconciling declared services with observed runtime state
detecting primitive triggers
starting task and service processes
tracking child processes
capturing stdout and stderr
recording run metadata
recording event metadata
providing run/event/runtime inspection
terminating processes on request
preserving separate logs for concurrent runs
```

The daemon **MAY** also provide:

```text
admission policy
hard process timeouts
global and per-task process limits
raw named locks
service supervision
health-check execution
crash-loop protection
dry-run reconciliation inspection
```

The daemon **MUST NOT** be responsible for:

```text
semantic retries
semantic dedupe
semantic cancellation
workflow ordering
agent quality evaluation
domain-specific conflict detection
interpreting stdout as success beyond exit status
constructing traces beyond raw causation metadata
```

---

## 16. User Responsibilities

User code owns all substantive orchestration.

User scripts **MAY**:

```text
spawn agents
call external CLIs
call coding-agent CLIs
call LLM CLIs
call git
call gh
call npm
call curl
call databases
use Promise.all
use Promise.race
use AbortController
use p-limit
implement queues
implement retries
implement deduplication
emit Whippletree events
write project state
construct traces
judge success/failure
```

Whippletree **MUST NOT** require these decisions to be expressed in Whippletree config.

Correct:

```ts
import { $ } from "zx"

const event = JSON.parse(process.env.WHIPPLETREE_EVENT_JSON!)

const [review, tests] = await Promise.all([
  $`agent-cli review --run ${event.payload.runId}`,
  $`npm test`,
])

if (tests.exitCode !== 0) {
  await $`agent-cli fix --reason "tests failed"`
}
```

Incorrect as Whippletree core:

```toml
[workflow]
fanout = ["review", "test"]
join = "fixer"
```

Whippletree launches the script. The script owns the orchestration.

---

## 17. Configuration

Whippletree **SHOULD** use a human-editable config format. TOML is recommended.

Default project config path:

```text
.whippletree/project.whip
```

Minimal task:

```toml
[[task]]
name = "hello"
run = "echo hello"
```

Scheduled task:

```toml
[[task]]
name = "daily-status"
schedule = "0 9 * * 1-5"
run = "tsx scripts/daily-status.ts"
```

File-watch task:

```toml
[[task]]
name = "test-on-change"
watch = ["src/**/*.ts", "tests/**/*.ts"]
settle = "300ms"
run = "npm test"

[task.admission]
when_busy = "restart"
```

Event-triggered task:

```toml
[[task]]
name = "after-tool-run"
on = "tool.run.completed"
run = "tsx scripts/after-tool-run.ts"
```

Long-running service:

```toml
[[service]]
name = "tool-source"
run = "tsx sources/tool-source.ts"

[service.supervision]
restart = "on_failure"
max_restarts = 5
within = "1m"
backoff = "exponential"
```

Manual task:

```toml
[[task]]
name = "review-branch"
run = "tsx scripts/review-branch.ts"
```

Manual invocation:

```bash
whip run review-branch
```

If a task needs structured input, the recommended v0.3 path is to emit an event
with a payload and let an event-triggered task receive it, or to have the task
read project files. Future dynamic-management work may add explicit ad hoc run
payload support, but task invocation must remain a mechanical process launch,
not a workflow call.

---

## 18. Task Execution Protocol

When Whippletree starts a task or service, it **MUST** provide mechanical context.

Recommended environment variables:

```text
WHIPPLETREE_RUN_ID
WHIPPLETREE_NAME
WHIPPLETREE_KIND          # task | service
WHIPPLETREE_WORKSPACE
WHIPPLETREE_WORKSPACE_ROOT
WHIPPLETREE_CONFIG_DIR
WHIPPLETREE_STATE_DIR
WHIPPLETREE_RUN_DIR
WHIPPLETREE_EVENT_JSON
WHIPPLETREE_EVENT_PATH
WHIPPLETREE_EVENT_PAYLOAD_JSON
WHIPPLETREE_PAYLOAD_JSON
WHIPPLETREE_CONFIG_VERSION
WHIPPLETREE_CORRELATION_ID
```

For small events, `WHIPPLETREE_EVENT_JSON` is acceptable.

For large events, Whippletree **SHOULD** write the event to a file and provide `WHIPPLETREE_EVENT_PATH`.

Each run **MUST** have a private run directory, for example:

```text
.whippletree/runs/run_01HV.../
  event.json
  meta.json
  stdout.log
  stderr.log
  tmp/
```

Scripts **SHOULD** use `WHIPPLETREE_RUN_DIR` for temporary run-local files.

Whippletree **MUST** allow scripts to ignore all Whippletree-specific environment variables.

This must remain valid:

```toml
[[task]]
name = "test"
watch = ["src/**/*.ts"]
run = "npm test"
```

---

## 19. Concurrency Across Scripts

Whippletree **MUST** support concurrent task and service runs as independent OS processes.

If multiple triggers fire at once, Whippletree **MAY** run multiple scripts concurrently, subject only to explicit admission/resource policy.

Whippletree **MUST** record and expose each run independently.

Whippletree **MUST** isolate run metadata and logs.

Whippletree **MUST NOT** impose a workflow-level concurrency model across runs.

Semantic coordination among concurrent scripts belongs in user code.

Whippletree-owned concurrency mechanics include:

```text
process spawning
process accounting
per-run logs
per-run environment
per-run scratch directories
global process limits
per-task admission policy
optional named locks
process cancellation
atomic event insertion
atomic run status updates
```

User-owned concurrency semantics include:

```text
fanout
join
race
quorum
semantic cancellation
semantic dedupe
semantic retries
agent comparison
branch conflict policy
workflow dependencies
```

Normative boundary:

**Whippletree handles concurrent processes. User code handles concurrent meanings.**

---

## 20. Admission Policy

Admission policy controls what the daemon does when a trigger fires while a previous run of the same task is still active.

Admission policy is mechanical process hygiene. It is not workflow orchestration.

Supported values **SHOULD** include:

```text
allow       start another run
reject      do not start; record rejected trigger/run
restart     cancel active run and start a new one
queue_one   keep at most one pending run
queue_all   enqueue all triggered runs
```

Example:

```toml
[[task]]
name = "typecheck"
watch = ["src/**/*.ts"]
run = "npm run typecheck"

[task.admission]
when_busy = "restart"
```

Whippletree **MUST NOT** silently drop triggers. If a trigger is rejected, coalesced, or superseded by admission policy, that fact **MUST** be inspectable.

Admission policy **MUST** be explicit when behavior other than the default is used.

Recommended default:

```text
allow
```

Alternative acceptable default:

```text
reject
```

The default **MUST** be documented.

---

## 21. Supervision Policy

Whippletree **MAY** expose BEAM-inspired supervision primitives for configured processes.

These primitives **MUST** be limited to mechanical process lifecycle management:

```text
spawn
monitor
restart
terminate
health-check
backoff
crash-loop prevention
```

Supervision **MUST NOT** encode domain-level retry, workflow recovery, agent evaluation, or semantic success criteria.

### 21.1 Tasks and Supervision

Triggered tasks **MUST NOT** restart automatically by default.

Default:

```toml
[task.supervision]
restart = "never"
```

Tasks **MAY** opt into mechanical restart:

```toml
[[task]]
name = "daily-summary"
schedule = "0 9 * * *"
run = "tsx scripts/daily-summary.ts"

[task.supervision]
restart = "on_failure"
max_restarts = 2
within = "5m"
backoff = "fixed"
```

This means only:

**If the process fails mechanically, rerun the same command according to this policy.**

It does not mean:

**The summary was bad, stale, incomplete, or semantically worth retrying.**

### 21.2 Services and Supervision

Services **SHOULD** support restart policies.

Supported restart modes **SHOULD** include:

```text
never
on_failure
always
```

Example:

```toml
[[service]]
name = "github-source"
run = "tsx sources/github-source.ts"

[service.supervision]
restart = "on_failure"
max_restarts = 5
within = "1m"
backoff = "exponential"
```

### 21.3 Crash-Loop Protection

Supervision policy **SHOULD** include crash-loop protection:

```text
max_restarts
within
backoff
start_delay
```

If a process exceeds its restart budget, Whippletree **SHOULD** mark it as failed and stop restarting it until user intervention or config reload.

### 21.4 Supervisor Strategy

Whippletree v0.3 **SHOULD** support only one-for-one supervision:

```text
if process X fails, restart process X according to X's policy
```

Whippletree **SHOULD NOT** implement BEAM-style group strategies initially:

```text
one_for_all
rest_for_one
supervisor trees
```

Those strategies risk hiding workflow structure behind daemon policy. Users can express group behavior in ordinary TypeScript if needed.

---

## 22. Health Checks

Whippletree **MAY** support health checks for services.

Health checks **MUST** be mechanical and exit-code based.

Example:

```toml
[[service]]
name = "tool-source"
run = "tsx sources/tool-source.ts"

[service.health]
check = "tsx sources/tool-health.ts"
every = "30s"
timeout = "5s"

[service.supervision]
restart = "on_failure"
max_restarts = 5
within = "1m"
```

Health semantics:

```text
exit 0      healthy
nonzero     unhealthy
timeout     unhealthy
```

Whippletree **MUST NOT** interpret domain-specific health from logs or payloads unless the user health-check script encodes that interpretation.

---

## 23. Resource Policy

Whippletree **MAY** enforce hard process resource limits.

Recommended fields:

```toml
[task.resources]
kill_after = "30m"
```

Resource policy is mechanical.

Acceptable daemon interpretation:

```text
this process exceeded a hard runtime limit; terminate it
```

Forbidden daemon interpretation:

```text
this agent seems stalled; launch a fixer
```

The latter belongs in user code.

---

## 24. Named Locks

Whippletree **MAY** provide named locks.

Locks are daemon-worthy because inter-process mutual exclusion is mechanical and error-prone.

Locks **MUST** be:

```text
explicit
named by user code
opaque to the daemon
inspectable
not inferred from domain concepts
bounded by a lease or equivalent recovery mechanism
```

CLI example:

```bash
whip lock acquire branch:main --ttl 10m --reason "review request req-482"
whip lock renew branch:main --token lock_... --ttl 10m
whip lock release branch:main --token lock_...
whip lock force-release branch:main --reason "holder exited"
whip lock with branch:main --ttl 2m --reason "run tests" -- npm test
```

TypeScript SDK example:

```ts
await withLock("branch:main", async () => {
  await mutateMainBranch()
}, { ttl: "2m", reason: "edit main" })
```

Whippletree does not know what `branch:main` means. It only provides atomic exclusion.

Whippletree **MUST NOT** implicitly lock based on task names, event types, files, branches, agents, or payload fields.

### 24.1 Lock Ownership

Named locks **SHOULD** have explicit mechanical ownership.

Recommended owner fields:

```text
lock name
fencing token
owner run id, if acquired by a run
owner name, if known
owner process id, if known
correlation id, if known
reason, if supplied
acquired time
renewed time
optional lease expiration
```

Lock release **SHOULD** require a fencing token so an old holder cannot release a newer lease with the same name.

An implementation **MAY** allow tokenless release only when the caller is mechanically identifiable as the owning Whippletree run.

If a lock is acquired by a run and that run exits, Whippletree **SHOULD** release the lock automatically or allow the lease to expire.

If a lock is acquired outside a run, Whippletree **SHOULD** require an explicit lease duration and make the owner inspectable.

Whippletree **SHOULD** provide an explicit administrative force-release operation that requires a reason and records the previous owner/token for audit.

Whippletree **SHOULD** provide an ergonomic `lock with` command or SDK helper that acquires a lock, runs a command or callback, and releases with the returned token.

Whippletree **SHOULD** expose held locks through a CLI or status interface.

Whippletree **MUST NOT** infer stale locks from domain concepts. Staleness, if implemented, is based only on mechanical facts such as owner process death or lease expiration.

---

## 25. Event Emission

Whippletree **SHOULD** allow scripts to emit events.

Required baseline mechanism:

```bash
whip emit review.ready --json '{"branch":"feature-x"}'
```

Payloads **SHOULD** also be accepted from a file or standard input for agent
ergonomics:

```bash
whip emit review.ready --payload-file payload.json
cat payload.json | whip emit review.ready --stdin
```

`emit` is the canonical event creation verb. Whippletree **SHOULD NOT** add a
separate `publish` command unless it is a pure alias with identical event
semantics and no broker-style guarantees.

Event emission **MUST** be usable from any language.

A TypeScript SDK **MAY** wrap it:

```ts
import { emit } from "@whippletree/sdk"

await emit("review.ready", { branch: "feature-x" })
```

When an event is emitted from inside a run, Whippletree **SHOULD** attach mechanical lineage:

```text
source_run_id
parent_event_id
correlation_id
```

Lineage is mechanical. It supports debugging. It is not a workflow model.

---

## 26. Causation and Correlation

Whippletree **SHOULD** support optional event causality fields:

```text
source_run_id
parent_event_id
correlation_id
```

Recommended meanings:

```text
source_run_id  the Whippletree run that emitted this event, if known
parent_event_id the Whippletree event visible to that run, if known
correlation_id a shared id for a broader episode, if known
```

Whippletree may propagate these mechanically.

Whippletree **MUST NOT** infer high-level traces, workflows, spans, or plans from these fields.

A user may build a trace projection over the event log, but that projection belongs in user space.

---

## 27. Runtime Reconciliation

Whippletree **SHOULD** automatically reconcile declared service state with observed runtime state.

In the steady state, if config declares an enabled service and the daemon is running, the service should be running unless:

```text
the service is disabled
the service is starting
the service is stopping
the service failed and is in backoff
the restart budget is exhausted
the command cannot be spawned
a health check is failing
the config is invalid and an older config remains active
the daemon is paused
the user explicitly stopped it
```

Whippletree **SHOULD** reconcile automatically after:

```text
daemon start
valid config reload
service crash
service exit
machine reboot, where applicable
explicit whip up
explicit service enable/disable
```

Automatic reconciliation is mechanical. It is not workflow planning.

Whippletree may know:

```text
service tool-source is declared enabled and not running; start it
```

Whippletree must not infer:

```text
review workflow is incomplete; launch fixer
```

### 27.1 Service Desired State

Whippletree v0.3 **SHOULD** model service reconciliation with a small desired-state contract.

At minimum, a service has:

```text
configured enabled state
user override state
observed process state
```

Recommended user override states:

```text
none
stopped
starting
```

A service declared enabled in config with no user override should converge toward running.

`whip service stop <name>` **SHOULD** set a user override that prevents automatic restart until the user clears it with `whip service start <name>`, `whip service restart <name>`, or an explicit project lifecycle command that documents override clearing.

`whip up` **SHOULD** clear service stop overrides by default and reconcile enabled services.

`whip down` **SHOULD** stop services as part of project shutdown without treating those stops as per-service user overrides.

Config reload **SHOULD NOT** clear a user stop override by default.

This prevents accidental restart loops while keeping the normal project lifecycle simple.

---

## 28. Runtime Status

Whippletree **MUST** expose runtime status.

Runtime status answers:

```text
what is declared?
what is enabled?
what is running?
what is pending?
what failed?
what is supervised?
what config version is active?
what restart/backoff state exists?
what watchers/schedules are active?
```

Required or recommended commands:

```bash
whip status
whip ps
whip services
whip tasks
whip runs
whip logs <run-id>
whip events
whippletree triggers
```

JSON form **SHOULD** be supported:

```bash
whip status --json
whip ps --json
whip services --json
whip runs --json
whip events --json
whippletree triggers --json
```

Record list commands **SHOULD** support practical mechanical filters such as
event type, source, task name, run state, trigger outcome, correlation id, and
limit. These filters are runtime inspection, not a workflow query language.

Runtime status is core because user scripts and coding agents may need to reason over mechanical runtime facts.

Example user-space policy:

```ts
import { whippletree } from "@whippletree/sdk"

const status = await whippletree.status()

const failedSources = status.services.filter(s =>
  s.state === "failed"
)

for (const service of failedSources) {
  await whippletree.emit("runtime.service_failed", {
    service: service.name,
  })
}
```

The daemon exposes facts. The script interprets them.

---

## 29. Runtime Lifecycle

Whippletree **SHOULD** support explicit project runtime lifecycle commands.

Foreground operation **MUST** be explicit. A user who wants the runtime attached to the current terminal should request `whip dev` or an explicit foreground option.

Non-foreground lifecycle commands such as `whip up` may start or contact a daemon without attaching it to the current terminal, but that behavior **MUST** be documented and inspectable.

Recommended commands:

```bash
whip dev
whip up
whip down
whip restart
```

### 29.1 `whip dev`

`whip dev` **SHOULD** run the daemon in the foreground for the current workspace.

Foreground dev mode **SHOULD** use the same config validation, event log, run records, and process supervision semantics as the normal daemon runtime.

Only one daemon instance **SHOULD** manage a workspace at a time. If another daemon already owns the workspace lock, `whip dev` **SHOULD** fail clearly or offer an explicit takeover option.

On Ctrl-C or normal terminal termination, `whip dev` **SHOULD** perform graceful foreground shutdown.

If graceful shutdown times out, the implementation **MAY** terminate remaining child process groups mechanically.

### 29.2 `whip up`

`whip up` **SHOULD**:

```text
start the daemon if needed
load valid config
start enabled services
activate watchers
activate schedules
begin accepting events
```

`whip up` is a project lifecycle command. It **SHOULD** reconcile the project toward the active config, including clearing per-service stop overrides unless an implementation documents a stricter option.

`whip up` **SHOULD NOT** run in the foreground unless the user explicitly requests foreground behavior.

### 29.3 `whip down`

`whip down` **SHOULD**:

```text
stop accepting new triggers
stop watchers
stop schedules
terminate or gracefully stop services
handle active task runs according to explicit option
```

Useful options:

```bash
whip down --graceful
whip down --kill
whip down --services-only
whip down --leave-runs
```

Lifecycle commands are runtime control-plane actions. They are not workflow transitions.

### 29.4 Command Names

Whippletree command names **SHOULD** keep project lifecycle separate from service lifecycle.

Recommended meanings:

```text
whip dev                run the project runtime in the foreground
whip up                 start/reconcile the project runtime
whip down               stop the project runtime
whip restart            down then up
whip service start      clear stop override and start one service
whip service stop       stop one service and set stop override
whip service restart    restart one service and clear stop override
```

Bare `whip start` and `whip stop` **SHOULD** either be omitted or documented aliases for `whip up` and `whip down`.

---

## 30. Dry-Run Reconciliation / Plan

Whippletree **MAY** expose a dry-run reconciliation command:

```bash
whip plan
```

This command is optional and primarily for debugging, preflight, and developer inspection.

`whip plan` answers:

```text
Given current config and current observed runtime state,
what mechanical reconciliation actions would Whippletree take?
```

It should not be part of normal steady-state operation.

A well-functioning daemon should usually reconcile automatically within a short interval, so `whip plan` should often say:

```text
System converged.
No actions pending.
```

Acceptable `plan` output:

```text
Would start:
  tool-source

Would stop:
  github-source, disabled in config

Would restart:
  webhook-listener, running under old config version

Would do nothing:
  test-on-change
```

Forbidden `plan` output:

```text
Would launch fixer because review workflow is semantically incomplete
```

`plan` is mechanical reconciliation introspection, not workflow planning.

---

## 31. State

Whippletree **MUST** maintain internal state sufficient to inspect events, runs, logs, services, and runtime status.

Whippletree **SHOULD NOT** make an application key-value store part of the core conceptual model.

Instead, Whippletree **SHOULD** expose useful paths:

```text
WHIPPLETREE_CONFIG_DIR
WHIPPLETREE_RUN_DIR
WHIPPLETREE_STATE_DIR
WHIPPLETREE_WORKSPACE
```

User scripts can choose their own state mechanism:

```text
JSON files
SQLite
Dolt
Postgres
Redis
Git
project-specific databases
```

Whippletree **MAY** offer a convenience scratchpad later, but it **MUST NOT** become the recommended application-state abstraction.

---

## 32. Schema Validation

Whippletree **MUST** validate its own event envelope and config.

Whippletree **SHOULD NOT** validate domain payload schemas in the daemon core.

User scripts or SDK helpers may validate payloads.

Example:

```ts
const AgentCompleted = z.object({
  runId: z.string(),
  branch: z.string().optional(),
})

const event = getEvent(AgentCompleted)
```

This belongs in user space or SDK space, not daemon core.

---

## 33. File Watching and Settling

File watching is a primitive trigger source.

Whippletree **SHOULD** support file-settling behavior because editors often produce bursts of writes.

Example:

```toml
[[task]]
name = "test"
watch = ["src/**/*.ts"]
settle = "300ms"
run = "npm test"
```

Whippletree **SHOULD NOT** generalize this into broad event-stream semantic debounce or dedupe.

File settling is mechanical. General event coalescing is semantic and belongs in user code.

---

## 34. Sources and Adapters

Whippletree should treat integrations suspiciously. Adapters are where domain semantics often sneak into the daemon.

Preferred model:

```toml
[[service]]
name = "tool-source"
run = "tsx sources/tool-source.ts"

[service.supervision]
restart = "on_failure"
```

The source script observes an external tool and emits events:

```bash
whip emit tool.run.completed --json '{"runId":"abc"}'
```

Whippletree **MAY** ship shallow built-in adapters, but built-in adapters **MUST** only observe, translate, and emit events.

Acceptable:

```text
tool.run.started
tool.run.completed
tool.run.failed
```

Not acceptable in daemon core:

```text
tool.agent_is_stuck
tool.review_needed
tool.should_retry
tool.output_is_bad
```

Those are user-space interpretations.

---

## 35. Recipes

Whippletree **SHOULD** provide recipes as scaffolding.

A recipe **MUST** generate ordinary editable files.

Example:

```bash
whip init recipe external-review-loop
```

May create:

```text
.whippletree/project.whip
scripts/on-tool-complete.ts
scripts/review.ts
sources/tool-source.ts
```

Bad recipe model:

```bash
whip enable external-review-loop
```

where behavior is hidden in the daemon.

Recipes **MUST NOT** create hidden daemon behavior.

A recipe is copyable code, not a privileged runtime feature.

---

## 36. TypeScript SDK

Whippletree **SHOULD** provide a TypeScript SDK as the golden-path ergonomic layer.

The SDK **MUST** be optional.

The SDK **MUST NOT** create a second runtime.

The SDK **SHOULD** be thin sugar over:

```text
environment variables
event parsing
whip emit
runtime status queries
subprocess execution
structured logging
optional named locks
canonical object CLI commands
```

Acceptable SDK helpers:

```ts
getEvent()
emit()
run()
status()
runs()
services()
withLock()
whippletree.task.list()
whippletree.task.run()
whippletree.task.add()
whippletree.service.add()
whippletree.run.start()
whippletree.run.list()
whippletree.event.emit()
whippletree.wait.event()
whippletree.lock.withCommand()
log()
readJson()
```

Dynamic task and service SDK helpers **MUST** remain wrappers over runtime
definition commands. They **MUST NOT** persist hidden workflow state, rewrite
user config without explicit consent, or add daemon-level meanings for retries,
deduplication, fanout, joins, or agent graphs.

Suspicious SDK helpers:

```ts
workflow()
activity()
durable()
agentGraph()
managedRace()
managedJoin()
semanticRetry()
```

The decisive test:

**Could this helper be explained as a thin wrapper around environment variables, subprocesses, and the Whippletree CLI?**

If yes, it likely belongs.
If no, it likely introduces framework creep.

---

## 37. CLI

Whippletree **MUST** provide a CLI.

The CLI **SHOULD** expose a stable object model. The v0.3 top-level commands are
acceptable ergonomic aliases, but future dynamic-management work should prefer
canonical object-oriented forms described in
`spec/dynamic-management-interface.md`.

A conforming v0.3 CLI **SHOULD** include commands equivalent to:

```bash
whip init
whip dev
whip up
whip down
whip restart
whip status
whip ps
whip tasks
whip services
whip runs
whip logs <run-id>
whip events
whippletree triggers
whip emit <event-type> --json <payload>
whip run <task-name>
whip cancel <run-id>
whip config check
whip doctor
```

Optional but recommended:

```bash
whip plan
whip lock acquire <name> --ttl <duration>
whip lock renew <name> --token <token> --ttl <duration>
whip lock release <name> --token <token>
whip service restart <name>
whip service stop <name>
whip service start <name>
```

The CLI should make Whippletree feel inspectable, not magical.

Canonical future object forms include:

```bash
whip task list
whip task run <name>
whip task add <name> --on <event> -- <cmd...>
whip service list
whip service add <name> -- <cmd...>
whip run list
whip run start --name <name> -- <cmd...>
whip event emit <type> --json <payload>
whip trigger list
whip lock with <name> --ttl <duration> -- <cmd...>
whip wait event <type> --correlation <id>
```

These object forms **MUST** remain mechanical. Dynamic task/service creation
creates runtime definitions; it does not create workflow state.

---

## 38. Config Reload

Whippletree **SHOULD** hot reload config.

Reload semantics:

```text
valid new config replaces old config
invalid new config is rejected
old config remains active
running task processes are not mutated
new task runs use the new config
service reconciliation occurs after valid reload
each run records the config version it started under
```

A config version **SHOULD** be a stable content-derived identifier or monotonically increasing daemon-local revision recorded in run and event metadata.

Config versioning is for mechanical inspection and reconciliation only. It does not imply semantic workflow versioning.

A bad config **MUST NOT** crash the daemon.

Running scripts **MUST NOT** be affected by config reload unless explicitly cancelled by the user or by a declared daemon policy.

For services, config reload may cause mechanical reconciliation:

```text
new enabled service starts
removed service stops
changed service restarts, if required
unchanged service remains running
```

These are runtime reconciliation actions, not workflow actions.

---

## 39. Failure Semantics

Whippletree **MUST** distinguish mechanical process failure from semantic failure.

Mechanical failures include:

```text
command failed to spawn
process exited nonzero
process was terminated by signal
process exceeded hard timeout
health check failed
restart budget exceeded
```

Semantic failures include:

```text
agent wrote bad code
review was inadequate
test suite was insufficient
output was stale
patch was unacceptable
event was duplicate
retry is warranted
```

Whippletree owns the first category. User code owns the second.

Exit code semantics:

```text
exit 0      process success
nonzero     process failure
signal      process killed/crashed
timeout     process timed out
```

Whippletree **MUST NOT** inspect domain output to override this interpretation.

---

## 40. Security Model

Whippletree executes user commands. Therefore, Whippletree is a code execution tool.

Whippletree **MUST NOT** pretend configured scripts are safe.

Whippletree **SHOULD** provide strong inspection:

```bash
whip tasks
whip services
whip status
whip config check
whip doctor
```

Future versions **MAY** add optional capability controls.

Example:

```toml
[[task]]
name = "review"
run = "tsx scripts/review.ts"

[task.capabilities]
allow_exec = ["agent-cli", "git"]
network = "none"
```

Capability controls are optional and out of scope for the core v0.3 model.

If introduced, they **MUST** remain understandable and must not become a hidden policy language.

---

## 41. Examples

### 41.1 File Watch Test

```toml
[[task]]
name = "test"
watch = ["src/**/*.ts"]
settle = "300ms"
run = "npm test"

[task.admission]
when_busy = "restart"
```

This is a mechanical watcher. The daemon does not know what the tests mean.

---

### 41.2 External Tool Completion Hook

```toml
[[task]]
name = "after-tool-run"
on = "tool.run.completed"
run = "tsx scripts/after-tool-run.ts"
```

```ts
import { $ } from "zx"

const event = JSON.parse(process.env.WHIPPLETREE_EVENT_JSON!)

const runId = event.payload.runId

await $`tool-cli logs ${runId} --tail 200 > ${process.env.WHIPPLETREE_RUN_DIR}/tool.log`

await $`review-cli request --run ${runId} --log ${process.env.WHIPPLETREE_RUN_DIR}/tool.log`

await $`whip emit review.requested --json ${JSON.stringify({ runId })}`
```

Whippletree does not know what a review is.

---

### 41.3 User-Owned Multi-Agent Concurrency

```toml
[[task]]
name = "parallel-attempts"
run = "tsx scripts/parallel-attempts.ts"
```

```ts
import { $ } from "zx"

const prompt = process.argv.slice(2).join(" ")

const attempts = await Promise.all([
  $`agent-cli attempt --strategy baseline ${prompt}`,
  $`agent-cli attempt --strategy review-first ${prompt}`,
  $`agent-cli attempt --strategy alternate ${prompt}`,
])

const ids = attempts.map(a => a.stdout.trim())

await $`agent-cli compare --runs ${ids.join(",")}`
```

The user owns the fanout and join. Whippletree only launched the script and recorded the run.

---

### 41.4 Supervised Source Service

```toml
[[service]]
name = "tool-source"
run = "tsx sources/tool-source.ts"

[service.supervision]
restart = "on_failure"
max_restarts = 5
within = "1m"
backoff = "exponential"
```

```ts
import { emit } from "@whippletree/sdk"

while (true) {
  const event = await waitForToolEvent()

  await emit(`tool.run.${event.status}`, {
    runId: event.runId,
    status: event.status,
  })
}
```

If this source crashes, Whippletree may restart it. That is process supervision, not workflow orchestration.

---

### 41.5 Explicit Named Lock

```ts
import { withLock } from "@whippletree/sdk"
import { $ } from "zx"

await withLock("branch:main", async () => {
  await $`git checkout main`
  await $`agent-cli apply --target main --source accepted-fix`
})
```

Whippletree does not know what `branch:main` means.

---

### 41.6 Runtime Status as User-Space Input

```toml
[[task]]
name = "runtime-monitor"
schedule = "*/5 * * * *"
run = "tsx scripts/runtime-monitor.ts"
```

```ts
import { whippletree, emit } from "@whippletree/sdk"

const status = await whippletree.status()

const failed = status.services.filter(s =>
  s.state === "failed" || s.restartBudgetExhausted
)

if (failed.length > 0) {
  await emit("runtime.services_unhealthy", { failed })
}
```

Whippletree exposes runtime facts. The script decides what they mean.

---

### 41.7 Morning Boot Policy in User Space

```toml
[[task]]
name = "morning-boot"
schedule = "0 8 * * 1-5"
run = "tsx scripts/morning-boot.ts"
```

```ts
import { whippletree, emit } from "@whippletree/sdk"

await whippletree.up()

const status = await whippletree.status()

const notRunning = status.services.filter(s =>
  s.enabled && s.desired === "running" && s.actual !== "running"
)

if (notRunning.length > 0) {
  await emit("runtime.boot_incomplete", { notRunning })
}
```

The daemon provides lifecycle controls. The script provides the operating policy.

---

## 42. Design Tests

Every proposed Whippletree feature should pass these tests.

### 42.1 The Normal Script Test

Can the interesting part remain ordinary TypeScript, bash, Python, Rust, or another normal language?

If not, the feature is probably too framework-like.

### 42.2 The No-SDK Test

Can the feature be used without importing an SDK?

If not, it may be too coupled to a runtime.

### 42.3 The No-Ontology Test

Does the daemon need to know what an agent, review, heartbeat, trace, or workflow means?

If yes, the feature probably belongs in a recipe, source script, or user library.

### 42.4 The Coding-Agent Test

Would a competent coding agent want to inspect or modify this logic?

If yes, it belongs in user space.

### 42.5 The Reimplementation Test

Would every reactive script runner otherwise have to reimplement this just to operate reliably?

If yes, it probably belongs in the daemon.

### 42.6 The Replaceability Test

Could an advanced user replace Whippletree with cron, watchexec, shell scripts, tmux, and systemd user services, but prefer Whippletree because it is cleaner?

If yes, Whippletree is in the right design space.

If not, it may be becoming a platform.

### 42.7 The Runtime-vs-Workflow Test

Is this state about declared and observed Whippletree-managed processes?

If yes, it may belong in runtime status.

Is this state about domain progress, agent quality, business meaning, or workflow phase?

If yes, it belongs in user space.

---

## 43. MVP Scope

A v0.3 MVP **SHOULD** include:

```text
project init
foreground dev mode
TOML config
manual tasks
scheduled tasks
file-watch tasks
event-triggered tasks
long-running services
automatic service reconciliation
service supervision
runtime status
run history
event history
stdout/stderr logs
per-run directories
config hot reload
config validation
event emission CLI
basic admission policy
hard kill timeout
process cancellation
optional raw named locks
thin TypeScript SDK
```

A v0.3 MVP **SHOULD NOT** include:

```text
workflow DAGs
durable promises
agent graphs
visual workflow builder
distributed execution
cloud coordination
embedded TypeScript runtime
semantic retry system
semantic dedupe system
daemon-owned traces
daemon-owned heartbeat semantics
BEAM supervisor trees
capability policy language
mandatory dry-run planning
```

`whip plan` may exist as a developer/debug tool, but the normal runtime should reconcile automatically.

---

## 44. Recommended Implementation

A practical implementation:

```text
daemon: Rust
store: SQLite or append-only JSONL + metadata index
config: TOML
watching: native file watcher
scheduling: cron parser
processes: OS subprocesses and process groups
SDK: TypeScript package
scripts: arbitrary executables
```

Rust is a good daemon language because the daemon is mostly process supervision, filesystem watching, signal handling, and local durability.

TypeScript is a good user-space language because the user wants normal async/concurrency primitives and package ecosystem access.

But the spec does not require Rust or TypeScript.

The spec requires:

**ordinary processes in, ordinary logs out, no hidden workflow model.**

---

## 45. Final Boundary Statement

Whippletree core should contain only what is necessary to make user-authored reactive scripts reliable:

```text
trigger
launch
monitor
record
supervise
reconcile runtime
inspect
```

Everything that determines the meaning of the work belongs in user space:

```text
interpret
decide
coordinate
retry semantically
dedupe semantically
judge
plan
orchestrate
```

One sentence:

**Whippletree is a small local daemon for reliably invoking, supervising, reconciling, and inspecting ordinary user programs in response to events, while leaving all orchestration semantics in normal user code.**
