# Workflow Storage

Status: implemented compact schema plus future normalization notes

The first implementation should use SQLite for durable workflow state.

The runtime should not begin as an in-memory interpreter with persistence added
later. Event queues, transition logs, effect logs, current state, and status
projection data should be durable from the first executable slice.

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
.armature/state/workflows.sqlite3
```

The database may contain multiple workflow instances. Each table includes
`workflow_id`.

## Implemented Core Tables

The current implementation uses a compact schema. Structured envelopes are
stored as JSON records, while the fields needed for durable lookup and status
queries are indexed as columns. This keeps the first implementation simple
without losing typed validation at the runtime boundary.

### `armature_meta`

Schema metadata:

```text
key TEXT PRIMARY KEY NOT NULL
value TEXT NOT NULL
```

The implemented schema version is `2`. Runtime startup rejects databases with a
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
policy blockers, and recent failures from this append-only log.

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
`workflow_events`, `workflow_log`, and `coerce_calls` already provide durable
state, queueing, audit, coerce replay, and status projection.

## Transaction Rules

Event processing uses SQLite transactions around each durable phase:

```text
mark event processing
evaluate transition prepare work
append transition/effect log records
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

If the process crashes while an event is `processing`, recovery requeues it and
preserves `attempt_count` for operator visibility.

## Recovery

On startup:

1. Return stale `processing` events to `queued`.
2. Preserve and increment `attempt_count` on each dequeue so repeated recovery
   is visible in status and logs.
3. Resume normal event processing.

Durable timers are reserved for a later runtime slice; current recovery has no
timer queue to scan.

## Migration

The database stores a schema version in `armature_meta`.
Migration code may create the metadata table for older stores that predate
version tracking, but it must fail closed when the stored version is newer than
the runtime supports.
