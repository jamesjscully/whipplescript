# Workflow Storage

Status: implemented compact schema plus planned native agent ledger

The first implementation should use SQLite for durable workflow state.

The runtime should not begin as an in-memory interpreter with persistence added
later. Event queues, transition logs, effect logs, current state, agent
invocations, and status projection data should be durable from the first
executable slice.

## Why SQLite

SQLite is the right first storage layer because it gives:

- durable transactions
- simple local deployment
- indexed status queries
- event status updates
- recovery leases
- append-only audit records
- room for migrations

Flat files are attractive initially, but event retries, status filters, crash
recovery, and idempotency reconciliation become awkward quickly.

## Database Scope

One workspace owns one workflow database by default:

```text
.whipplescript/state/workflows.sqlite3
```

The database may contain multiple workflow instances. Each table includes
`workflow_id`.

## Implemented Core Tables

The current implementation uses a compact schema. Structured envelopes are
stored as JSON records, while the fields needed for durable lookup and status
queries are indexed as columns. This keeps the first implementation simple
without losing typed validation at the runtime boundary.

### `whipplescript_meta`

Schema metadata:

```text
key TEXT PRIMARY KEY NOT NULL
value TEXT NOT NULL
```

The implemented schema version is `4`. Runtime startup rejects databases with a
newer unsupported schema version.

### `workflow_state`

Current state and workflow data projection:

```text
workflow_id TEXT PRIMARY KEY NOT NULL
workflow_name TEXT NOT NULL
current_state TEXT NOT NULL
context_json TEXT NOT NULL
```

Every state write is validated by the interpreter against WorkflowIR semantics
before persistence. A future normalized store may split individual data paths
into a separate table, but v1 status reads from `context_json`.

### `workflow_events`

Durable event queue:

```text
seq INTEGER PRIMARY KEY AUTOINCREMENT
workflow_id TEXT NOT NULL
event_id TEXT NOT NULL
status TEXT NOT NULL
event_json TEXT NOT NULL
UNIQUE(workflow_id, event_id)
```

Indexes:

```text
(workflow_id, status, seq)
```

`event_json` contains the full typed event envelope:

```json
{
  "event_id": "evt_...",
  "workflow_id": "ImplementationLoop",
  "event_type": "finished",
  "payload": {},
  "source": null,
  "occurred_at": null,
  "enqueued_at": null,
  "correlation_id": null,
  "causation_id": null,
  "dedupe_key": null,
  "status": "queued",
  "attempt_count": 0,
  "last_error": null
}
```

The separate `status` column mirrors the JSON status for efficient queue and
inspection queries. `attempt_count` and `last_error` currently live inside the
event envelope.

### `workflow_log`

Append-only transition and effect records:

```text
seq INTEGER PRIMARY KEY AUTOINCREMENT
workflow_id TEXT NOT NULL
record_json TEXT NOT NULL
```

`record_json` stores typed transition and effect log records. Status and
overview projections derive latest transitions, effects, active invocations,
policy blockers, current effect failures, current blockers, and historical
recent failures from this append-only log plus durable event/coerce records.

### `coerce_calls`

Durable synchronous value-call records for BAML-backed `coerce` calls:

```text
seq INTEGER PRIMARY KEY AUTOINCREMENT
coerce_call_id TEXT NOT NULL UNIQUE
workflow_id TEXT NOT NULL
workflow_version TEXT NOT NULL
transition_id TEXT
event_id TEXT
step_path TEXT NOT NULL
function_name TEXT NOT NULL
idempotency_key TEXT NOT NULL
backend_json TEXT NOT NULL
args_json TEXT NOT NULL
status TEXT NOT NULL
http_status INTEGER
raw_response_json TEXT
parsed_output_json TEXT
error TEXT
duration_ms INTEGER
created_at TEXT NOT NULL
```

Indexes:

```text
(workflow_id, function_name, seq DESC)
UNIQUE (workflow_id, idempotency_key) WHERE status = 'succeeded'
```

Rules:

- successful records are reused by idempotency key during replay
- failed attempts are append-only and remain visible in status
- argument and output JSON are schema-validated against WorkflowIR before use
- raw response storage is controlled by policy and may be replaced with a
  redaction marker before persistence, but parsed output and errors must be
  durable enough for audit and replay
- status projections include latest successful coerce decisions and latest
  failures

## Planned Native Agent Ledger Tables

The JSON agent-file bridge is scaffolding, not the target storage model. The
native harness should use the same SQLite workflow database as the runtime.
Agent work is first-class durable workflow state, not an adapter side file.

The first native harness migration should bump the schema version and add these
tables.

### `agent_invocations`

Queued, claimed, running, and completed agent work:

```text
seq INTEGER PRIMARY KEY AUTOINCREMENT
workflow_id TEXT NOT NULL
invocation_id TEXT NOT NULL UNIQUE
agent TEXT NOT NULL
effect_id TEXT NOT NULL
transition_id TEXT NOT NULL
event_id TEXT
idempotency_key TEXT NOT NULL
input_json TEXT NOT NULL
requested_profile TEXT
resolved_profile TEXT
profile_enforcement TEXT
status TEXT NOT NULL
claimed_by TEXT
claim_expires_at TEXT
provider TEXT
provider_run_id TEXT
run_dir TEXT
stdout_path TEXT
stderr_path TEXT
exit_code INTEGER
error TEXT
created_at TEXT NOT NULL
updated_at TEXT NOT NULL
UNIQUE(workflow_id, idempotency_key)
```

Indexes:

```text
(workflow_id, status, seq)
(workflow_id, agent, status)
(claim_expires_at)
(provider, provider_run_id)
```

Statuses:

```text
queued
claimed
running
succeeded
failed
cancelled
timed_out
completion_rejected
```

Rules:

- `start` inserts an invocation in the same transaction as the state transition
  and effect log.
- `idempotency_key` deduplicates replay of the same committed start step.
- `requested_profile` records the source-level semantic profile, when present;
  `resolved_profile` and `profile_enforcement` are filled by the harness after
  policy resolution.
- the harness claims `queued` or expired `claimed`/`running` work with a lease.
- active invocation projections count `queued`, `claimed`, and `running`
  invocations until a valid completion retires them.
- provider stdout/stderr are referenced by path, not embedded in the row.

### `agent_messages`

Durable messages sent to existing agents or threads:

```text
seq INTEGER PRIMARY KEY AUTOINCREMENT
workflow_id TEXT NOT NULL
message_id TEXT NOT NULL UNIQUE
agent TEXT NOT NULL
invocation_id TEXT
effect_id TEXT NOT NULL
transition_id TEXT NOT NULL
event_id TEXT
idempotency_key TEXT NOT NULL
message_json TEXT NOT NULL
status TEXT NOT NULL
created_at TEXT NOT NULL
updated_at TEXT NOT NULL
UNIQUE(workflow_id, idempotency_key)
```

Rules:

- `send` records a durable message in the same transaction as the transition
  that produced it.
- if `invocation_id` is absent, the harness/provider adapter decides whether
  the message targets a named long-lived agent, a thread, or is unsupported.
- message delivery failures are harness events, not silent state changes.

### `agent_completions`

Validated provider completion records:

```text
seq INTEGER PRIMARY KEY AUTOINCREMENT
workflow_id TEXT NOT NULL
completion_id TEXT NOT NULL UNIQUE
invocation_id TEXT NOT NULL
agent TEXT NOT NULL
status TEXT NOT NULL
summary TEXT
exit_code INTEGER
event_id TEXT
payload_json TEXT NOT NULL
created_at TEXT NOT NULL
UNIQUE(workflow_id, invocation_id)
```

Rules:

- the harness writes the completion record and queued workflow event in one
  transaction.
- `payload_json` must match the workflow's declared completion event schema
  before `workflow_events` receives the event.
- duplicate completion attempts for the same invocation are idempotent when
  payloads match and rejected when payloads conflict.

### `harness_events`

Append-only operational observations from the harness:

```text
seq INTEGER PRIMARY KEY AUTOINCREMENT
workflow_id TEXT NOT NULL
event_id TEXT NOT NULL UNIQUE
invocation_id TEXT
kind TEXT NOT NULL
payload_json TEXT NOT NULL
created_at TEXT NOT NULL
```

Indexes:

```text
(workflow_id, seq DESC)
(workflow_id, kind, seq DESC)
(invocation_id, seq)
```

Useful initial kinds:

```text
invocation_claimed
provider_started
provider_exited
completion_enqueued
completion_schema_mismatch
provider_command_failed
lease_expired
idle_without_work
desire_path_observed
```

Rules:

- harness events are local operational evidence, not workflow input.
- status may summarize recent harness events, but statechart handlers do not
  consume them directly.
- desire-path observations should preserve enough context to improve the UX
  without storing unbounded raw logs in SQLite.

## Future Normalized Tables

These tables remain reasonable future normalization targets once the compact
schema proves insufficient for operations or analytics:

```text
workflow_instances
workflow_context
workflow_transitions
workflow_effects
workflow_artifacts
```

They are not required for the current runtime because `workflow_state`,
`workflow_events`, `workflow_log`, `coerce_calls`, and the native agent ledger
tables provide durable state, queueing, audit, coerce replay, agent execution
tracking, and status projection.

## Transaction Rules

Event processing uses SQLite transactions around each durable phase:

```text
mark event processing
evaluate transition prepare work
append transition/effect log records
insert native agent invocations/messages produced by start/send
update current state and workflow data
mark event processed or ignored
commit
```

Dequeue marks the event `processing` and increments `attempt_count`.
Successful/ignored event completion commits the terminal event status, state,
and log records together. Failed transitions mark the event `failed` and append
failure log records without saving tentative state.

Async effect dispatch records outcomes in durable effect log records. There
must not be a visible state where an event is marked processed but the
state/log projection still reflects the old state, or the reverse.

Native agent invocation records follow the same rule. There must not be a
visible state where a transition has started an agent but the invocation row is
missing, or where the invocation row exists but the transition/effect log that
created it is absent.

If the process crashes while an event is `processing`, recovery requeues it and
preserves `attempt_count` for operator visibility.

## Recovery

On startup:

1. Return stale `processing` events to `queued`.
2. Preserve and increment `attempt_count` on each dequeue so repeated recovery
   is visible in status and logs.
3. Return expired agent claims to `queued` or mark them failed according to the
   provider status and retry policy.
4. Resume normal event processing and harness claiming.

Durable timers are reserved for a later runtime slice; current recovery has no
timer queue to scan.

## Migration

The database stores a schema version in `whipplescript_meta`.
Migration code may create the metadata table for older stores that predate
version tracking, but it must fail closed when the stored version is newer than
the runtime supports.
