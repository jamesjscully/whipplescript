# Fact Provenance

Status: draft

Facts are the current typed projection of workflow truth. They must not become a
bag of convenient state with unclear ownership.

Every fact has a provenance class.

## Provenance Classes

### Runtime Projections

Owned by the kernel/control plane.

Examples:

```text
claimable effects
running effects
active turns
available agents
```

Runtime projections are derived from events, effects, dependencies, runs,
leases, and control metadata. They are exposed through status, scheduler queries,
and trace output, not automatically persisted as durable facts. A projection only
becomes a fact when the kernel appends a documented event/fact pair such as
`agent.turn.completed` or `human.ask.created`.

### Rule-Recorded Facts

Produced by source rules using `record`.

Example:

```whipplescript
record ReviewedWork {
  turn turn
  review review
}
```

These facts are committed atomically with the rule step that produced them.

### Effect Completion Facts

Derived from terminal effect events.

Examples:

```text
agent.turn.completed
loft.claim.succeeded
schema.coerce.succeeded
coerce.succeeded
human.ask.created
human.answer.received
```

`coerce.succeeded` is a current compatibility projection for the coerce
backend. `schema.coerce.succeeded` is the target semantic projection.

Core effect contracts define their completion fact schemas.

### External Projection Facts

Projected from external kernels or systems through registered capabilities.

Examples:

```text
loft.readyIssue
loft.unfinishedIssue
loft.conflict
```

Loft remains the source of truth for Loft facts. WhippleScript may cache or
project them for rule matching, but the provenance must remain visible.

### Package/Provider Projection Facts

Projected by package capabilities or provider outputs.

Examples:

```text
memory.queryResult
repo.resourceReservation
github.prStatus
```

Packages may register schemas for these facts. Providers may not write them
directly into an instance. Provider observations enter WhippleScript through
effects/events and kernel-mediated projection.

## Required Fact Metadata

Every stored fact records:

```text
fact_id
instance_id
name
key
value_json
schema_id
provenance_class
source_event_id?
source_rule?
source_effect_id?
source_run_id?
external_system?
external_id?
correlation_id?
created_at
updated_at
```

## Replay

Replay must be able to explain every fact:

```text
event log + program version + rule commits + external projection snapshots
  -> fact projection
```

Facts whose source system is external should record the observed external
version, command id, cursor, or snapshot hash when available.

## Rule Permissions

User rules may:

- read facts from any provenance class allowed by policy
- record rule-owned facts
- consume or update rule-owned facts, when the language supports consumption
- react to effect completion facts

User rules may not:

- directly record runtime facts
- directly record package/provider projection facts
- directly mutate provider-owned facts
- forge effect completion facts

To change an external system, rules enqueue effects.
