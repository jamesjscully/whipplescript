# Statechart Workflow Implementation Plan

Status: working plan

This plan sequences the work so formal modeling pressures the design before the
runtime hardens, without making formal verification a constant blocker during
early scaffolding.

## Phase 0: Proposal Specification

Goal: write enough specification to align on product shape and semantics.

Deliverables:

- architecture document
- product surface and CLI document
- authoring format sketch
- workflow IR sketch
- runtime semantics document
- verification strategy
- explicit existing-system reuse boundary
- concrete spec implementation workflow example
- initial implementation plan

Exit criteria:

- the team can explain the runtime boundary in one paragraph
- the team can list every trusted component
- the team can identify what user-authored workflow source can and cannot do
- the team can explain which old Armature concepts are kept, dropped, or
  reframed
- the example workflow feels simpler than the equivalent script

## Phase 1: Hand-Written Formal Model

Goal: pressure-test the semantics before implementation.

This phase should model the language semantics, not the whole product.

Model:

```text
bounded work items
workflow states
finished events
idle observation events
coerce run classification as nondeterministic output
coerce next-action selection as nondeterministic output
worker and quality active counters
work item statuses
capability facts
failure visibility
human review visibility
```

Do not model:

```text
actual LLM behavior
actual prompts
actual resource contents beyond bounded abstractions
full compatibility adapter/runtime behavior
full un-tie session behavior
Git state
networking
```

Initial properties:

```text
active_workers <= max_workers
active_quality <= max_quality
no duplicate active work item
started work is always visible
failed work is completed, blocked, failed, or human_review
undeclared capabilities cannot be invoked
unknown agents cannot be targeted
idle unfinished work cannot be silently ignored forever in the model's bounded horizon
```

Recommended sequence:

1. Write a compact TLA+/Apalache-style model or equivalent transition-system
   model for the example workflow using the native DSL/IR semantics.
2. Install/provision the selected checker in CI or developer tooling.
3. Run bounded checks and collect counterexamples.
4. Revise the workflow semantics and specs.
5. Reevaluate whether Maude should be added immediately as an executable
   rewriting-semantics model.
6. If Maude is warranted, model the same WorkflowIR/runtime semantics and use
   it to pressure handler lookup, event ordering, raised events, and effect
   commit behavior.
7. Re-express the same model in Veil or write a small Veil companion model.
8. Pin the Veil/Lake dependency version before treating Veil output as more
   than an exploratory artifact.
9. Decide which formal backend should be the first generated target.

Exit criteria:

- at least one hand-written model exists
- the model finds or rules out the expected obvious failure modes
- the specs have been revised based on model feedback
- the Maude decision is documented with rationale
- each important invariant is assigned to static validation, property tests,
  TLA+/Apalache, optional Maude, future Veil work, runtime enforcement, or
  adapter contract tests
- the chosen IR still appears lowerable to Veil or another formal backend

## Phase 2: Runtime Skeleton

Goal: create the smallest executable workflow stack with fake or narrow
adapters.

Deliverables:

- workflow crate/module skeleton
- native `.armature` parser for the restricted statechart DSL, using `logos`
  for lexing and `rowan` for a lossless syntax tree
- typed AST/lowering pass from rowan syntax tree to WorkflowIR
- IR structs
- static validator shell
- golden fixture tests:

```text
examples/workflows/minimal.armature -> expected WorkflowIr JSON
examples/workflows/minimal.armature -> expected validation report
synthetic event sequence -> expected status projection
```

- SQLite-backed durable workflow state store
- SQLite-backed event queue, transition log, and effect log
- single-instance interpreter loop
- manifest-driven fake adapters
- CLI commands:

```text
armature validate <file>
armature emit <file> --event <event> --payload <json>
armature run <file>
armature status [workflow]
armature events [workflow]
armature log [workflow]
armature build <file>
armature check <file> [--adapter-manifest <manifest>]
```

Implementation guidance:

- do not build an in-memory state machine and add persistence later; the first
  interpreter slice should append durable records
- implement only the minimal grammar needed for `minimal.armature`; do not add
  general-purpose programming constructs while building the parser
- preserve comments, whitespace, malformed tokens, and source spans in the rowan
  tree even when lowering fails
- add property tests for parser/lowering invariants as soon as the AST shape is
  stable enough to make them valuable
- validate every `serde_json::Value` against `WorkflowIr` or `AdapterManifest`
  schemas before use
- consume adapter manifests before real adapters exist, including fake `coerce`,
  `start`, `send`, `askHuman`, and capability operation support
- keep effect dispatch idempotency in the first skeleton
- record transitions even if adapters are fake
- do not add arbitrary scripting escape hatches

Exit criteria:

- `minimal.armature` can be parsed and lowered into golden IR
- validator output is snapshot-tested
- the interpreter can process synthetic events
- event, transition, effect, and current state records are persisted in SQLite
- status explains current state and recent transitions
- validator catches unknown states, agents, actions, and capabilities

## Phase 3: Real Adapters

Goal: connect the skeleton to useful runtime systems.

Deliverables:

- un-tie thread/session adapter
- BAML execution adapter
- scoped plan/state file adapter
- human review adapter or event bridge
- legacy Armature event/run adapter only if it clearly serves the new workflow
  model

Exit criteria:

- a workflow can start work or message a declared agent target
- a workflow can observe a real completion event from at least one adapter
- BAML calls are recorded with input, raw output, parsed output, and failures
- adapter failures become durable workflow state

## Phase 4: Generated Formal Models

Goal: generate model artifacts from the same IR the interpreter executes.

Deliverables:

- formal model generator for the implemented subset
- generated model fixtures for the spec implementation workflow
- CLI command:

```text
armature emit-model <file> --target <target> [--adapter-manifest <manifest>]
```

Target choices:

```text
tla
apalache
maude
veil
```

The first target should be whichever gives the fastest useful counterexamples.
Maude is a candidate reference-semantics target if executable rewriting exposes
bugs more directly than a state-space checker. Veil remains the preferred
long-term proof-oriented target if the ergonomics are acceptable.

Current implementation note: the first generated targets are TLA+ and a small
Maude rewriting model over the validated IR's state-transition abstraction.
Generated TLA+ and Maude currently include declared agent `maxActive` limits,
active invocation counters, start increments, completion decrements, and
max-active safety checks. Generated TLA+ also includes finite `coerce` output
spaces for enum/literal/bool/null/ref-record discriminants, nondeterministic
coerce transitions, and a `CoerceType` invariant over the function output map.
It also tracks the last abstract effect label with a `DeclaredEffectType`
invariant as the first generated effect-surface check. Ordinary effects such as
`send`, `askHuman`, `raise`, and capability calls are modeled as stuttering
observations; bounded `start` effects remain counter-updating actions.
Generated Maude records the same finite coerce spaces as model comments while
keeping the current rewriting abstraction focused on control state and active
invocation counters. The runtime enforces the same max-active limit before
dispatching `start` effects. Generated artifacts annotate declared built-in
invariants with their current coverage layer, and model emission fails closed
for expression invariants until workflow data is represented in the formal
abstraction. The hand-written TLA+/Maude models remain the stronger
specification pressure until generated models include workflow data and deeper
effect/capability invariants.

Exit criteria:

- generated model includes states, events, counters, coerce output spaces, and
  selected invariants
- generated model is emitted from validated WorkflowIR, not raw `.armature`
  source
- generated model agrees with the hand-written model on core properties
- model generation failures produce actionable diagnostics
- build artifacts include validated adapter-manifest and policy-document bundles
  when supplied, so effect/event schemas, required capabilities, and authority
  assumptions are reproducible with the generated IR and models

## Phase 5: Validation Commands

Goal: expose verification as product commands without making it mandatory.

Deliverables:

```text
armature validate <file>
armature validate-adapter <manifest>
armature validate-policy <policy>
armature check <file> [--adapter-manifest <manifest>]
armature prove <file>
armature run <file>
armature status [workflow]
armature emit-model <file> --target <target> [--adapter-manifest <manifest>]
armature emit-config <file> --target <target> [--adapter-manifest <manifest>]
```

Expected behavior:

- `validate` is fast and local
- `check` runs bounded model checks when tooling is installed
- `prove` runs stronger backend-specific verification when available
- until a proof backend is implemented, `prove` validates workflow, adapter, and
  policy contracts before returning a clear unavailable result
- missing formal tools produce clear installation/configuration diagnostics

Exit criteria:

- users can inspect generated artifacts
- CI can run `validate` without heavyweight dependencies
- advanced users can opt into `check` or `prove`

## Phase 6: Optional Gates

Goal: allow teams to require validation at appropriate boundaries.

Gate levels:

```text
none
validate
check
prove
```

Possible enforcement points:

```text
before workflow start
before workflow publish
before enterprise/stable workflow enablement
in CI
```

Exit criteria:

- gates are configurable per workspace or workflow
- gates can be disabled during early development
- stable/enterprise mode can require at least static validation

## Phase 7: Product Hardening

Goal: make the system practical for nontechnical and enterprise users.

Deliverables:

- workflow status in `armature overview`
- readable diagnostics for agents and humans
- schema/version migration story
- workflow templates
- companion skill updates
- example workflows
- enterprise capability policy examples
- documentation for common stuck states

Exit criteria:

- a nontechnical user can inspect why a workflow is waiting
- a coding agent can repair a workflow from diagnostics without reading runtime
  internals
- capability violations are explained in terms of contracts and agents
- the spec implementation workflow works end to end in a real repo

Current implementation note: `armature overview` now renders validation health
plus the durable status projection as a compact current-state, queued-event,
active-invocation, latest-transition, latest-effect, and recent-failure summary.
It is intentionally derived from the same projection as `status` so the first
product-facing view does not introduce a second runtime state model. Invalid
source that cannot lower to IR still returns validation diagnostics with no
runtime status.

Current implementation note: CLI validation diagnostics are now carried through
shared error handling, so commands outside `validate` can report concrete
workflow diagnostics instead of only returning a generic validation failure.
Parser diagnostics now attach source locations for common current-token errors;
CLI-loaded workflows preserve the actual file path in those locations.
Validator diagnostics now use declaration and executable-step spans for common
static errors such as invalid `maxActive`, undeclared effect targets, bad
transitions, bad raises, bad assignments, and invalid expression paths or calls.
Adapter-manifest workflow diagnostics also point at the effect step or handler
that requested unsupported adapter authority. CLI-loaded workflows also preserve
`workflow.source_path` in emitted IR and build artifacts.

Current implementation note: `check` and `emit-model` accept
`--adapter-manifest` and `--policy`, and validate adapter-backed workflow
effects before emitting or checking the formal abstraction. The generated models
still consume only WorkflowIR, but the CLI no longer lets explicitly supplied
adapter or policy contracts be silently ignored.

Current implementation note: the first policy document shape is JSON with
`mode`, `allowed_capabilities`, and `denied_capabilities`. Exact denied
capabilities are errors in every mode. Unknown capabilities warn in local mode
and become errors under stricter modes according to effect category and
write-like capability names. The manifest dispatcher enforces supplied policy
documents again at runtime before dispatching adapter-backed effects. `build`
writes supplied policy documents to `policy-documents.json` beside adapter
manifests and generated model artifacts.

Current implementation note: the new companion coding-agent skill lives at
`skills/armature-statechart`. It replaces the legacy v0.3 runtime skill for the
new product surface and teaches agents to author restricted `.armature`
statecharts, keep external authority behind adapter manifests, use typed
`coerce` decisions instead of unconstrained control scripts, inspect durable
state with `overview`, and repair common lifecycle/capability failures.

## First Implementation Slice

The first slice should be deliberately narrow:

```text
one workflow instance
synthetic events
Armature DSL -> IR lowering for minimal.armature
golden IR fixture
static validator subset with golden validation report
SQLite durable event queue
SQLite append-only transition/effect logs
fake send/start/coerce effects
status view
adapter manifests for fake effects
```

This lets us validate the semantics and runtime loop before real adapters.

The checked formal model should not block parser/IR/validator work, but it
should block deeper interpreter semantics beyond this first synthetic-event
slice.
