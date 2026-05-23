# Workflow Storage

Status: design proposal

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

## Core Tables

### `workflow_instances`

Materialized workflow instance metadata:

```text
workflow_id TEXT PRIMARY KEY
workflow_name TEXT NOT NULL
workflow_version TEXT NOT NULL
ir_hash TEXT NOT NULL
current_state TEXT NOT NULL
status TEXT NOT NULL
blocked_reason TEXT
created_at TEXT NOT NULL
updated_at TEXT NOT NULL
```

### `workflow_context`

Materialized typed workflow-local context:

```text
workflow_id TEXT NOT NULL
path TEXT NOT NULL
schema_ref TEXT NOT NULL
value_json TEXT NOT NULL
updated_at TEXT NOT NULL
PRIMARY KEY (workflow_id, path)
```

Every write to this table must be validated against `context_schema`.

### `workflow_events`

Durable event queue:

```text
workflow_id TEXT NOT NULL
event_id TEXT NOT NULL
event_type TEXT NOT NULL
payload_json TEXT NOT NULL
source_json TEXT
occurred_at TEXT
enqueued_at TEXT NOT NULL
correlation_id TEXT
causation_id TEXT
dedupe_key TEXT
status TEXT NOT NULL
attempt_count INTEGER NOT NULL
last_error TEXT
workflow_version TEXT NOT NULL
PRIMARY KEY or UNIQUE (workflow_id, event_id)
```

Indexes:

```text
(workflow_id, status, enqueued_at, event_id)
(workflow_id, dedupe_key)
```

### `workflow_transitions`

Append-only transition records:

```text
transition_id TEXT PRIMARY KEY
workflow_id TEXT NOT NULL
from_state TEXT NOT NULL
to_state TEXT NOT NULL
event_id TEXT
guard_json TEXT
context_patch_json TEXT NOT NULL
sync_effects_json TEXT NOT NULL
async_effect_ids_json TEXT NOT NULL
diagnostics_json TEXT NOT NULL
created_at TEXT NOT NULL
```

No updates are allowed except possibly archival metadata added later.

### `workflow_effects`

Append-only intended and dispatched effect records:

```text
effect_id TEXT PRIMARY KEY
workflow_id TEXT NOT NULL
transition_id TEXT NOT NULL
effect TEXT NOT NULL
category TEXT NOT NULL
target TEXT
args_json TEXT NOT NULL
idempotency_key TEXT NOT NULL
required_capabilities_json TEXT NOT NULL
status TEXT NOT NULL
outcome_json TEXT
error TEXT
created_at TEXT NOT NULL
updated_at TEXT NOT NULL
```

Indexes:

```text
(workflow_id, status)
(idempotency_key)
```

### `workflow_artifacts`

Build/model/BAML artifacts:

```text
artifact_id TEXT PRIMARY KEY
workflow_id TEXT NOT NULL
kind TEXT NOT NULL
path TEXT NOT NULL
hash TEXT NOT NULL
created_at TEXT NOT NULL
```

## Transaction Rules

Event processing uses one SQLite transaction for prepare/commit:

```text
mark event processing
evaluate transition prepare work
append transition record
update materialized context/current state
append intended async effects
mark event processed or ignored
commit
```

Async effect dispatch happens after commit and records outcomes in separate
transactions.

The event row, materialized state, transition record, and intended effect
records are committed together. There must not be a visible state where an
event is marked processed but the state/log projection still reflects the old
state, or the reverse.

If the process crashes after commit and before dispatch completes, recovery
queries `workflow_effects` for intended/dispatched effects and reconciles by
idempotency key.

## Recovery

On startup:

1. Return stale `processing` events to `queued`.
2. Preserve and increment `attempt_count` on each dequeue so repeated recovery
   is visible in status and logs.
3. Find intended/dispatched effects without terminal outcomes.
4. Reconcile those effects through their adapter idempotency keys.
5. Enqueue overdue timers.
6. Resume normal event processing.

## Migration

The database stores a schema version. Runtime startup must reject databases with
newer unsupported schema versions and migrate older supported versions.

The first implementation records this in an `armature_meta` table:

```text
key TEXT PRIMARY KEY
value TEXT NOT NULL
```

with `schema_version = 1`. Migration code may create the metadata table for
older stores that predate version tracking, but it must fail closed when the
stored version is newer than the runtime supports.
