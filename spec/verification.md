# Verification Strategy

Status: draft

Armature needs two related but separate verification tracks:

1. Validate the architecture before and after implementation.
2. Statically analyze user Armature programs as a product feature.

The same semantics should feed both tracks, but the tools should not be forced
into one shape.

## Tool Roles

### Maude: Rule Kernel And Program Semantics

Maude is the primary tool for the Armature language kernel. Maude system modules
specify rewrite theories, and rewrite rules represent local state transitions.
That matches Armature's core model:

```text
facts + events + effect queue + dependency edges + rewrite rules
```

Use Maude for:

```text
rule commits
effect graph dependency behavior
claimability
completion events
bounded searches for bad rule cycles
generated per-program counterexample checks
```

Maude should be the first formal target generated from typed Armature IR.

### TLA+/Apalache: Durable Runtime Lifecycles

TLA+ is the right fit for the control-plane lifecycle because the hard bugs are
asynchronous and temporal:

```text
event log append
projection catch-up
effect claim
lease expiry
worker crash
retry
idempotency
recovery
late completion
pause/resume/cancel
```

TLA+ specifications are useful design artifacts for asynchronous systems, and
Apalache provides bounded/symbolic checking for TLA+ transition systems. Use
TLA+/Apalache for architecture validation, not for every user workflow in v0.

### Veil/Lean: Later Assurance Layer

Veil is a Lean-embedded framework for specifying, testing, and proving safety
properties of state transition systems. It provides model checking and SMT-style
automation with Lean available when automation falls short.

That is powerful, but too heavy for the critical path while the kernel is still
moving. Keep Veil as a later hardening target for stable safety invariants:

```text
dependency safety
idempotent recovery
trace conformance
capability enforcement facts
```

Do not require Veil for v0 development gates.

## Architecture Validation

Architecture validation checks whether the runtime design is sound independent
of any particular user program.

### Maude Kernel Model

Represent runtime state as a Maude configuration:

```maude
< events  : EventLog
  facts   : FactSet
  effects : EffectQueue
  deps    : EffectDependencies
  clock   : Time
  control : RuntimeMeta >
```

Each Armature rule lowers to one or more Maude rewrite rules. External systems
are modeled nondeterministically:

```text
effect requested -> effect completed | failed | timed_out
blocked effect -> claimable effect, when dependency predicates are satisfied
```

Initial checks:

- a dependent effect cannot run before its dependency predicate is satisfied
- dependency failure blocks downstream success-only effects instead of running
  them
- source order never creates ordering
- capacity cannot go negative
- no internal rewrite sequence can enqueue unbounded effects
- stuck states are explainable as waiting for external events, human input,
  dependency satisfaction, policy, or unavailable capacity

### TLA+/Apalache Control-Plane Model

Model control-plane actions:

```text
AppendEvent
DeriveFacts
CommitRule
EnqueueEffectGraph
SatisfyDependency
ClaimEffect
StartRun
CompleteRun
FailRun
ExpireLease
RecoverLease
PauseInstance
ResumeInstance
CancelInstance
```

Initial safety invariants:

- every run references an existing effect
- no effect has more than one successful terminal completion
- no provider run starts unless the effect is claimable
- no claimable effect has unsatisfied dependencies
- retry reuses effect identity unless the program creates a new attempt
- paused instances do not commit new effectful rewrites
- recovery does not reorder the per-instance event log
- projections are derivable from the log and committed rule steps

Initial liveness/fairness goals, checked only after safety stabilizes:

- under fair workers, claimable effects eventually run or become terminal
- expired leases eventually recover or become terminal
- unprocessed events eventually reach the projection cursor

## Runtime Trace Conformance

The implementation should emit enough evidence to replay its behavior against
the abstract lifecycle model.

Trace checker input:

```text
events
facts
effects
effect_dependencies
runs
leases
evidence
diagnostics
```

Trace conformance should reject:

- effect claimed while dependency unsatisfied
- completion for unknown effect
- duplicate terminal completion
- run started after cancellation
- dependency output used before success
- blocked_by_dependency for an effect whose dependency edge listens for failure
- event sequence gaps inside one instance

This is the bridge between formal specs and the Rust implementation.

## Static Analysis Of Armature Programs

Compiler checks should be fast, local, and explainable:

```text
type checking
read/write/consume sets
effect graph validation
output binding scopes
rule dependency graph
recursion strata
idempotency key derivability
capability/profile requirements
capacity/resource constraints
```

Generated Maude checks should be optional at first:

```sh
armature check workflow.armature --model-search
```

They should provide bounded counterexamples for unsafe orchestration patterns,
not require authors to understand Maude.

Do not generate TLA+ per user program in v0. TLA+ models the runtime/control
plane; Maude models user program behavior.

## Scope Boundary

Verification does not prove:

- prompt correctness
- external agent correctness
- filesystem correctness
- semantic truth of model classifications
- correctness of Docket or Thoth internals

It proves orchestration-kernel properties under typed contracts for external
results.

## Source Notes

The strategy is based on:

- Maude documentation describing rewrite theories and system modules as local
  state-transition systems:
  <https://maude.cs.uiuc.edu/maude1/manual/maude-manual-html/maude-manual_13.html>
- TLA+ guidance emphasizing asynchronous systems, atomicity choice, and
  specification as a way to reveal design errors:
  <https://lamport.org/pubs/lamport-spec-tla-plus.pdf>
- Apalache documentation describing TLA+ specs as transition systems and
  symbolic/bounded model checking:
  <https://apalache-mc.org/>
- Veil documentation describing Lean-embedded transition-system verification:
  <https://veil.dev/docs/>

## Implementation Path

1. Hand-write the Maude effect-graph kernel model.
2. Hand-write the TLA+ control-plane lifecycle model.
3. Add a lightweight trace-conformance contract to the runtime-store and
   observability specs.
4. Encode Ralph loop and Docket-claim-before-agent-turn examples.
5. Add generated Maude from typed rule IR once the parser/IR exists.
6. Reevaluate Veil after the kernel semantics stop moving.
