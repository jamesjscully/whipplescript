# Statechart Workflow Verification

Status: design sketch

Whippletree workflows should be designed so their semantics can be validated both
statically and with transition-system verification. Verification starts after
the native `.whip` source has been parsed, lowered, and validated; formal
backends consume WorkflowIR, not raw source text.

The verification strategy should optimize for early bug discovery. TLA+ and
Apalache are the first modeling tools because they should produce useful bounded
counterexamples quickly. Maude is a candidate executable-semantics tool if the
transition semantics need sharper pressure after that first model. Veil remains
the preferred proof-oriented target once the semantics are stable enough to
justify deeper integration.

The workflow author should not need to write TLA+, Maude, or Veil.

## Verification Layers

Verification happens in three layers, plus one early design checkpoint.

### 0. Hand-Written Semantic Model

Before implementation begins in earnest, the team should write a small
hand-authored transition-system model of the intended semantics.

This model is not generated from workflow IR. It is a design tool.

It should model the spec implementation workflow with:

- bounded work items
- workflow control states
- `finished` events
- `idle` observation events
- coerce outputs as nondeterministic schema-valid choices
- active worker and quality counters
- visible work item statuses
- capability facts
- blocked and human-review states

It should not model:

- prompt text
- LLM behavior
- BAML HTTP server behavior
- real files
- Git state
- network behavior
- the full compatibility adapter/runtime implementation
- the full un-tie runtime

Candidate tools:

```text
TLA+/Apalache for fast bounded counterexamples
Maude for executable rewriting semantics and reference interpreter pressure
Veil for proof-oriented transition-system modeling in Lean after semantics stabilize
K if the DSL becomes a larger formal language; deferred for now
```

The first model should optimize for useful counterexamples, not elegance. If it
is hard to model the semantics, the language design is probably too vague.

### Maude Reevaluation Checkpoint

Immediately after the first hand-written TLA+/Apalache model is checked, the
team should decide whether to add a Maude model before the runtime skeleton
continues.

Add Maude if any of these are true:

- hierarchical handler lookup is ambiguous
- effect commit semantics are hard to express clearly in TLA+
- event queue ordering or internal raised events need executable small-step
  pressure
- the team wants a reference semantics for interpreter conformance tests
- counterexamples suggest the written semantics and intended runtime behavior
  are drifting apart

Defer Maude if the initial model exposes the important design bugs and the
remaining semantics are straightforward enough to test with IR fixtures and
property tests.

Any Maude model should target WorkflowIR and runtime semantics, not raw parser
syntax. It can become a reference executable semantics later, but it should not
gate the first runtime skeleton unless the initial model exposes real semantic
ambiguity.

### 1. Static Validation

Static validation runs for every workflow before it can execute.

It checks:

- all states exist
- all transitions target valid states
- all agents are declared
- all actions are known
- all capabilities are declared
- all coerce functions have valid BAML-compatible schemas
- rowan parse tree was produced without fatal syntax errors
- typed AST lowering produced valid WorkflowIR
- all referenced contract files exist
- all immediate transition cycles are rejected or proven bounded
- all failure-prone calls have a failure path or default blocked behavior
- all concurrency limits are finite
- all durable state writes target declared data paths

This layer should catch authoring mistakes quickly.

Static validation should verify as many source-level invariants as possible
before runtime. Parser and lowering correctness are not formal-methods targets
initially, but they should be covered with golden tests, property tests, and
eventually fuzzing. If the compiler cannot model or validate a construct, it
should fail closed with a diagnostic instead of treating the construct as
implicitly safe.

### 2. Runtime Enforcement

Runtime enforcement is mandatory even if static validation passes.

The runtime checks before every effect:

- capability availability
- contract compatibility
- concurrency bounds
- idempotency keys
- target existence
- input schema validity
- current-state compatibility

The runtime also checks expression invariants after a transition reaches a
stable state. A violated or non-boolean expression invariant fails the event and
rolls back in-memory interpreter state before durable state is saved.

This is the security boundary. Verification improves confidence, but runtime
checks remain authoritative.

### 3. Generated Transition-System Checks

For high-value workflows, Whippletree should generate a formal transition-system
model from the validated IR.

The generated model should include:

- finite workflow states
- declared events
- data fields relevant to invariants
- active invocation counters
- capability declarations
- bounded work item abstractions
- coerce outputs as nondeterministic values constrained by schema
- Whippletree expression primitives as pure deterministic operations over bounded
  typed values

The generated model should not include:

- prompt text
- model weights
- BAML HTTP protocol behavior
- arbitrary logs
- arbitrary file contents
- host-language implementation details

Current generated TLA+ support includes a first version of this coerce
abstraction: each declared coerce function gets a finite output set derived from
enum, literal, bool, null, union, ref, and record-discriminant schemas. Coerce
calls become nondeterministic updates to a per-function output map, and
`CoerceType` checks that every stored output remains inside the declared
abstraction. The same generated TLA+ model tracks the last abstract effect label
and checks `DeclaredEffectType`, which is the first generated effect-surface
invariant. Ordinary effects such as `send`, `askHuman`, `raise`, and capability
calls are emitted as stuttering effect observations; bounded native `start`
effects stay coupled to active-counter updates. Generated Maude currently lists the same
coerce output spaces as comments while its executable rewriting model remains
focused on control-state and active-invocation invariants.

Because generated models do not yet include workflow data, model generation
fails closed when the IR contains expression invariants. Diagnostics should
explain the specific unsupported surface, for example that workflow data is not
included in the generated model yet. The runtime still enforces those expression
invariants after transitions; generated verification must add data abstraction
before it can claim coverage for them.

## Veil Strategy

Veil is a preferred long-term verification target if its ergonomics are
acceptable. Veil 2.0 is currently documented as a pre-release, so generated
Veil support should stay exploratory until the dependency/version story is
pinned. The compiler can lower workflow IR to a Veil module:

```text
Whippletree workflow state       -> Veil mutable state
events                        -> Veil actions
transitions                   -> guarded action bodies
agent/resource counters       -> mutable relations/functions
contracts/capabilities        -> immutable theory components
workflow invariants           -> Veil safety/invariant declarations
coerce result enum/schema space -> nondeterministic choices
```

Example conceptual lowering:

```text
when selection.action == "StartWorker" {
  start(worker, work_item: selection.work_item_id)
  transition supervising
}
```

becomes a transition that nondeterministically chooses a valid
`WorkSelection`, then checks that starting a worker preserves:

```text
active_worker_count <= max_worker_count
worker_capabilities_subset_of_contract
work_item_not_already_active
```

The important point is that Veil does not need to know what the prompt "means".
It only needs the set of structured outputs that each coerce function is allowed
to return.

## Generated Verification Timing

Generated verification should not be a production gate during the first runtime
skeleton. The intended sequence is:

```text
1. hand-write a formal model of the semantics from the native DSL/IR design
2. revise the specs based on counterexamples
3. implement the parser, typed lowering, and runtime skeleton against the
   revised IR
4. generate formal models from the implemented IR subset, not raw source
5. expose check/prove commands
6. make verification gates optional
7. require gates only for stable or enterprise workflows
```

This gives the project formal-methods backpressure without allowing early tool
friction to dominate scaffolding.

## Built-In Invariants

The workflow system should provide named invariants that can be checked for
every workflow.

Initial set:

```text
agentCapabilitiesRespected
declaredAgentsOnly
declaredEffectsOnly
maxActiveRespected
terminalInvocationsObserved
failedEffectsAreDurable
blockedWorkIsVisible
noSilentEventDrop
noUnboundedInternalLoop
```

These invariants should be enforced partly by construction, partly by static
validation, and partly by generated verification.

The generated model should include every workflow-relevant invariant that is
decidable over the bounded abstraction. Anything excluded from the model should
be documented with the layer responsible for it: static validation, runtime
enforcement, adapter contract testing, property tests, or future proof work.
Generated TLA+ and Maude artifacts include built-in invariant coverage comments
for every built-in invariant declared by the workflow, so the artifact is
explicit about whether the obligation is generated, static, runtime-enforced, or
adapter/policy-enforced.

Current generated TLA+ and Maude models include declared agent `maxActive`
limits, bounded active invocation counters, start increments, completion
decrements, and max-active safety checks. The models intentionally treat
completion as a bounded abstraction over processed `finished` events rather than
modeling external agent execution.

## User-Defined Invariants

Users may define additional invariants over workflow state:

```whippletree
invariant retryCountWithinBound {
  assert data.retryCount <= 3
}
```

The first implementation supports the same expression subset used by guards and
assignments, with one `assert <expr>` statement per invariant block. The system
should reject invariants it cannot check instead of pretending they are
verified.

## Liveness

The initial target is safety, not full liveness.

Instead of promising "every item eventually completes", the system should first
model obligations as safety-visible state:

```text
if work is started, it must remain visible as active, completed, blocked,
failed, or human_review
```

This avoids hiding stuck work while leaving room for future liveness checking.

## Verification Modes

Recommended modes:

```text
whip validate <file>
  Fast static validation.

whip check <file> [--adapter-manifest <manifest>]
  Static validation plus bounded model checking.

whip prove <file>
  Generate Veil and run stronger invariant checks.

whip emit-model <file> --target veil [--adapter-manifest <manifest>]
  Produce the generated Veil module for inspection.

whip emit-model <file> --target maude [--adapter-manifest <manifest>]
  Produce the generated Maude module for executable semantics inspection if
  Maude support is enabled.
```

The product should not require all users to understand Veil. Veil is an expert
backend and diagnostic artifact.

## Gate Levels

Workflow verification gates should be configurable:

```text
none      allow rapid prototyping
validate  require static validation
check     require bounded model checks
prove     require stronger backend-specific proof/check results
```

Early local development should default to `validate` or `none`. Enterprise or
stable workflow publication may require `check` or `prove` once diagnostics are
good enough for ordinary users and coding agents to act on.
