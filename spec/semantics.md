# Semantics

Status: draft

This document defines the target mathematical model.

## Facts

Let:

```text
Sort      = named type
FactName  = identifier
Fact      = FactName(Value...)
FactSet   = finite set or multiset of facts
```

Whippletree should default to set semantics for derived facts. Runtime facts that
represent unique lifecycle records carry stable IDs. If true multiplicity is
needed, it must be represented explicitly with IDs rather than accidental
duplicates.

Example facts:

```text
work(id: "T1", status: open, goal: "Add parser")
agent(worker, profile: repo-writer)
available(worker)
turn(id: "turn-1", agent: worker, work: "T1", status: running)
```

## Events

An event is an append-only external or derived observation:

```text
Event = (id, type, payload, time, causation, correlation)
```

Events are never edited. Rules may append derived events, but most durable
workflow state should be represented as facts.

## Rules

A rule is:

```text
r = (name, trigger, reads, consumes, produces, effect_graph, guards)
```

Where:

- `trigger` optionally restricts the rule to a new event or clock.
- `reads` are facts required but not removed.
- `consumes` are facts removed or advanced by the rewrite.
- `produces` are facts inserted by the rewrite.
- `effect_graph` is a finite graph of durable outbox commands and dependency
  edges.
- `guards` are pure predicates over matched values.

A rule step is atomic:

```text
(L, F, Q, D, C) -> (L', F', Q', D', C')
```

No external effect runs during the step. Effects are appended to `Q`.
Dependency edges are appended to `D`.

Rule commits are scoped to one instance. The control plane may run many
instances concurrently, but it serializes commits within an instance.

## Rewriting

A pure rewrite changes facts only:

```text
F' = (F - consumes) union produces
```

An effectful rewrite also appends a durable effect graph:

```text
Q' = Q append effect nodes
D' = D append dependency edges
```

If a rewrite fails validation, no part of it commits.

## Fixpoint

After accepting an event, the runtime may apply enabled pure rules until a
fixed point:

```text
while exists enabled pure rule that produces a new fact:
  apply it
```

Effectful rules are not applied in an unbounded internal loop unless the
compiler proves a bounded productive measure. The conservative v0 rule is:

```text
an effectful rule may fire at most once per triggering event/correlation unless
it consumes a unique fact
```

## External Recursion

Long-running loops are allowed when each cycle crosses an external boundary:

```text
agent completed turn -> rule enqueues next tell -> harness runs agent
```

Internal effect recursion is rejected:

```text
ready(work) => ready(work) + tell(worker, work)
```

because it can enqueue unbounded effects without waiting for the world to
change.

## Control-Plane State

The mathematical runtime model is per instance. The full control plane state is
a finite map:

```text
Instances : InstanceId -> (L, F, Q, D, C)
```

plus immutable program versions and shared capability/profile bindings.

Cross-instance coordination is not part of the first language semantics. It
must go through explicit external capabilities or future shared facts.
