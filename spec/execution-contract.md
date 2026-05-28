# Execution Contract

Status: draft

This document defines how rule commits, effects, dependencies, completions, and
follow-up execution work.

The key rule:

```text
rules enqueue durable effect graphs; effects never run inline
```

An effect graph is still part of the outbox model. It is not a second workflow
language and not a hidden callback system.

## Rule Commit

A rule commit is atomic. It may:

```text
consume/update facts
produce facts
append derived events
enqueue an effect graph
record diagnostics/evidence
advance the instance cursor
```

Provider execution is never part of the rule commit.

If validation fails, no facts, events, effects, evidence, or diagnostics from
that rule commit are persisted.

Fact records and effect graph nodes produced by the same rule commit share
correlation metadata. This lets later rules match typed relationships such as
"worker completed turn for Docket issue" without depending on prompt text.

## Effect Graph

An effect graph is a finite set of effect nodes and dependency edges:

```text
EffectGraph = (Nodes, Edges)
Node        = durable effect record
Edge        = dependency from one effect node to another
```

Each node has its own idempotency key, input schema, output schema, capability
requirements, timeout, retry policy, and artifact policy.

Edges are dependency predicates over upstream effect lifecycle:

```text
succeeds
fails
completes
```

The first implementation should not support arbitrary boolean dependency
expressions inside the graph. Joins and rich branching belong in rules over
facts/events.

## Source Order

Source order does not imply effect ordering.

This is unordered:

```armature
=> {
  docket.note "Starting work"
  tell worker "Implement the issue"
}
```

If ordering matters, the source must express it:

```armature
=> {
  claim issue with docket as claim

  after claim succeeds {
    tell worker """
    Implement {{ claim.issue.title }}
    """
  }
}
```

Lowering:

```text
effect e1 = docket.claim(issue)
effect e2 = agent.tell(worker, prompt)
edge e1 --succeeds--> e2
```

The `after` block is durable dependency sugar. It does not preserve a stack
frame or execute arbitrary code later.

## Scheduling

An effect is claimable only when:

```text
all dependency predicates are satisfied
policy accepts the requested capability/profile
retry/backoff policy allows execution
capacity is available, if applicable
```

Conceptual statuses:

```text
queued
blocked_by_dependency
blocked_by_policy
claimable
claimed
running
completed
failed
timed_out
cancelled
```

The store may implement `claimable` as a query over `queued` effects plus
dependencies and policy decisions. The status view should still expose the
distinction.

If an upstream dependency fails and no downstream edge listens for `fails` or
`completes`, the dependent effect becomes `blocked_by_dependency`. It is not a
provider failure because the provider never ran.

## Output Binding

Named effect bindings expose typed outputs only after the relevant dependency
predicate is satisfied.

Allowed:

```armature
claim issue with docket as claim
after claim succeeds {
  tell worker "{{ claim.issue.title }}"
}
```

Rejected:

```armature
claim issue with docket as claim
tell worker "{{ claim.issue.title }}"
```

The compiler should report that `claim.issue.title` is only available after the
claim effect succeeds.

Effect bindings are typed by the effect contract:

```text
effect input type
success output type
failure output type
timeout output type
cancel output type
```

Inside `after effect succeeds`, the binding has the effect's success output
type. Inside `after effect fails`, it has the failure output type. Inside
`after effect completes`, it has a tagged union of all terminal output types.

The lifecycle predicates are generic. The payload shape is effect-specific.
For example, `after claim succeeds` uses the generic success predicate, but the
fields available on `claim` come from the `docket.claim` success contract.

## Branching

Effect graphs may branch:

```armature
coerce classifyWork(result.summary) as classification

after classification succeeds {
  docket.note "Classification: {{ classification.status }}"
  tell reviewer "Review this result"
}
```

Both downstream effects become independently claimable when the upstream effect
succeeds. There is no implicit transaction across provider effects.

## Joins

Effect graph joins are not part of v0.

To wait for multiple external results, let completions produce facts/events and
write a normal rule:

```armature
rule synthesize
  when research result from alpha
  when research result from beta
  when synthesizer is available
=> {
  tell synthesizer "Synthesize both findings."
}
```

This keeps coordination visible in the rule/fact model instead of burying it in
a mini workflow language inside the outbox.

## Completion Events And Facts

Every terminal effect outcome appends an event:

```text
effect.completed
effect.failed
effect.timed_out
effect.cancelled
effect.blocked_by_dependency
effect.blocked_by_policy
```

The runtime derives standard lifecycle facts from these events.

Core effects may also define typed completion facts:

```text
agent.turn.completed
docket.claim.succeeded
docket.claim.failed
baml.coerce.succeeded
baml.coerce.failed
human.answer.received
```

Domain-specific facts should be produced by rules unless a core effect contract
explicitly defines them.

## Idempotency

Every effect node has a stable idempotency key derived from:

```text
instance_id
program_version
rule_name
trigger_event_id or consumed_fact_keys
effect_path_in_graph
normalized_input_hash
resolved_dependency_output_hashes, if used
```

Dependency satisfaction is scheduling state, not effect identity. Retries reuse
the same effect identity unless the source rule explicitly creates a new
attempt.

## Formal Model

The execution contract extends the runtime state with dependency edges:

```text
R = (L, F, Q, D, C)
```

Where:

- `L` is the append-only event log.
- `F` is the fact set.
- `Q` is the durable effect outbox.
- `D` is the durable effect-dependency relation.
- `C` is control metadata.

A rule step may append nodes to `Q` and edges to `D`. A provider step may occur
only for a claimable node. A completion step appends an event to `L`, which
rules may later consume.
