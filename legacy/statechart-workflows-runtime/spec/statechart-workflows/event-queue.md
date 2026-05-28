# Durable Event Queue

Status: implemented v0 contract plus future queue policy notes

The event queue is the interpreter's durable input boundary. Events are typed,
persisted, ordered, and processed with explicit status.

## Event Record

Every event record contains:

```text
event_id
workflow_id
event_type
payload
source
occurred_at
enqueued_at
correlation_id
causation_id
dedupe_key
status
attempt_count
last_error
```

`event_id` is immutable and unique within its `workflow_id`. `workflow_id`
scopes the event to one workflow instance, so two workflows may contain the same
external event id without colliding.

## Statuses

Event statuses:

```text
queued
processing
processed
ignored
failed
dead_lettered
```

Ignored events are durable records with reasons stored in `last_error`. They
are not silent drops.

Valid status transitions:

```text
queued -> processing
processing -> processed
processing -> ignored
processing -> failed
failed -> queued
failed -> dead_lettered
dead_lettered -> queued, only through explicit administrative retry
```

No other status transition is valid.

## Ordering

Within one workflow instance, the interpreter processes events in insertion
order:

```text
SQLite seq
```

The runtime processes one event at a time per workflow instance. Different
workflow instances may process events concurrently.

## Delivery Semantics

The queue provides at-least-once internal delivery to the interpreter.

The interpreter must make event handling idempotent by recording:

- event status
- transition id
- effect idempotency keys
- processed event cursor/checkpoint

After a crash:

- `processing` events are returned to `queued` on runtime startup
- `attempt_count` is preserved and incremented on the next dequeue
- committed terminal events are not re-applied

The first implementation does not persist a recovery lease or a separate
intended-effect table. Effect outcomes are durable log records, and future
adapter-specific reconciliation can add idempotency repair without changing the
event envelope.

## Event Admission

Events may enter from:

```text
CLI emit
adapters
workflow `raise` effects
compatibility bridges
```

Before enqueue:

- event type must be declared by the workflow, unless the event is a built-in
  runtime diagnostic event
- payload must match the event schema
- dedupe policy must be applied, if configured
- policy must allow the source to emit that event

## Dedupe

Dedupe is optional and explicit.

If an event has a `dedupe_key`, a future workflow revision may configure:

```text
allow_duplicates
drop_if_queued_or_processed
replace_queued
coalesce_payload
```

The implemented default is `allow_duplicates`. Semantic dedupe should usually be
represented in workflow data, such as `seenRunIds`, because that is modelable
and visible in status.

## Fanout

An event is queued for exactly one workflow instance.

If an adapter observes an external fact that multiple workflows care about, the
adapter or router creates one event record per workflow. This avoids shared
cursor ambiguity.

## Timers

Timers are reserved for a later runtime slice. The implemented v0 event queue
does not own timer records or enqueue overdue timer events.

## Runtime Observation Events

Some events are produced by runtime observers.

Example:

```text
idle
```

`idle` is an observation event emitted by an adapter or built-in observer when
configured idle conditions are true, such as no active invocations and
unfinished work.

Observation events must still be declared in the workflow event schema and
record their source.

## Retention

Processed, ignored, and failed events remain inspectable.

Retention policy should be configurable:

```text
keep_all
keep_last_n
keep_for_duration
archive
```

The default during early development should be `keep_all`.

## CLI

Event commands:

```text
whip emit workflow.whip --event <event-type> --payload <json>
whip events workflow.whip
whip events workflow.whip --status failed
whip events workflow.whip --status dead_lettered
whip events workflow.whip --json
whip retry-event workflow.whip --event-id <event-id>
```

`retry-event` is an administrative command. It should require that the event is
currently failed or dead-lettered. The implemented command requeues the event,
clears `last_error`, and preserves `attempt_count` for operator visibility.

## Type Validation

Event payload validation happens twice:

```text
enqueue time
processing time
```

Enqueue-time validation protects the queue. Processing-time validation protects
the interpreter against schema drift after workflow updates.

If a workflow schema changes while events are queued, the runtime must either:

- process the event with the workflow version that admitted it, or
- reject it with a durable schema-version diagnostic.
