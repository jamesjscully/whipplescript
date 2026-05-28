# Fact Provenance

Status: draft

Facts are the current typed projection of workflow truth. They must not become a
bag of convenient state with unclear ownership.

Every fact has a provenance class.

## Provenance Classes

### Runtime Facts

Owned by the kernel/control plane.

Examples:

```text
instance.started
agent.available
effect.claimable
effect.running
turn.active
```

Runtime facts are derived from events, effects, dependencies, runs, leases, and
control metadata. User rules may read them but should not record them directly.

### Rule-Recorded Facts

Produced by source rules using `record`.

Example:

```armature
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
docket.claim.succeeded
baml.coerce.succeeded
human.answer.received
```

Core effect contracts define their completion fact schemas.

### External Projection Facts

Projected from external kernels or systems through registered capabilities.

Examples:

```text
docket.readyIssue
docket.unfinishedIssue
docket.conflict
```

Docket remains the source of truth for Docket facts. Armature may cache or
project them for rule matching, but the provenance must remain visible.

### Plugin Projection Facts

Projected by plugin capabilities.

Examples:

```text
memory.queryResult
thoth.resourceLease
github.prStatus
```

Plugins may register schemas for these facts. They may not write them directly
into an instance. Plugin observations enter Armature through effects/events and
kernel-mediated projection.

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
- directly record plugin projection facts
- directly mutate Docket/Thoth/GitHub facts
- forge effect completion facts

To change an external system, rules enqueue effects.
