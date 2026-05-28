# Static Analysis

Status: draft

The compiler should reject programs whose orchestration behavior cannot be
bounded or explained by the restricted rule model.

## Required Analyses

### Type Checking

Every fact, event, guard, template reference, and effect payload must be typed.
Unknown fields are errors. Optional fields must be handled before use.

Type checking uses [type-system.md](type-system.md) as the source of truth for
boundary types, allowed operations, optional/null handling, media references,
and BAML lowerability.

Fact production through `record` must be checked against a known class schema.
Conversational facts supplied by core integrations must lower to typed fact
queries before later compiler phases.

### Read/Write Sets

For each rule, compute:

```text
reads(r)
consumes(r)
produces(r)
effect_nodes(r)
effect_edges(r)
correlates(r)
```

These sets drive conflict checks, cycle analysis, and diagnostics.

### Rule Dependency Graph

Build a graph:

```text
r1 -> r2 when r1 produces a fact that r2 can consume or read
```

Classify strongly connected components:

- pure monotonic recursion: allowed
- recursion through external event or clock: allowed with rate limits
- effectful internal recursion: rejected unless explicitly proven bounded
- negation recursion: rejected unless stratified
- aggregate recursion: rejected unless stratified

### Effect Safety

An effectful rule must either:

- consume or advance a unique fact, or
- be keyed to a unique triggering event/correlation, or
- declare an explicit bounded `choose up to N` shape.

The compiler should reject rules that preserve their trigger while emitting new
effects.

### Effect Graph Validation

For each rule, validate the produced effect graph:

- the graph is finite and acyclic
- source order does not imply dependency
- dependency predicates are limited to `succeeds`, `fails`, and `completes`
- downstream effects can reference upstream outputs only in scopes where the
  matching dependency predicate guarantees availability
- blocked downstream effects are distinguishable from provider failures
- joins are expressed through rules over completion facts, not hidden graph
  callbacks
- each effect node has its own stable idempotency key

### Constraint Preservation

Declared and built-in invariants should be checked against every rule:

- one running turn per work item unless explicitly allowed
- capacity never goes negative
- effects have stable idempotency keys
- completed work cannot become running again without a retry/reopen fact
- work cannot be accepted without a successful review when review is required

The first implementation can combine syntactic checks with generated Maude
searches for bounded counterexamples.

## Diagnostic Standard

Errors should explain:

1. what rule is unsafe
2. what cycle or invariant is involved
3. what effect could repeat or what fact could become inconsistent
4. the smallest source change that would make the intent explicit

The target quality bar is Gleam-style helpfulness: strict where the distinction
matters, generous where the compiler can safely infer intent.
