# Construct Lowering Preservation

Status: draft formal baseline

This document defines the next proof layer after construct graph acceptance.
The construct graph answers whether package composition is statically valid.
Lowering preservation answers whether that accepted graph becomes ordinary core
IR without losing or inventing meaning.

The target chain is:

```text
accepted program
  -> platform-owned lowering
  -> ordinary core IR
  -> runtime lifecycle
```

Lowering is not a package runtime. It is a compiler phase owned by the platform.
Packages may declare construct instances, interfaces, contracts, and allowed
lowering classes. They may not execute code during lowering or choose a
different lowering based on runtime state.

## Lowered IR Report

The compiler should emit a lowered IR report for every accepted program it
lowers. This report is the concrete artifact that connects the construct graph
to executable core IR:

```text
graph_id
accepted_program_digest
lowerer_version
package_lock_digest
source_digest
node_lowerings[]
  node_id
  lowering_class
  produced core object refs
  preserved source span refs
  preserved resource refs
  preserved capability refs
  preserved version refs
  preserved cardinality refs
  preserved provenance refs
  preserved terminal binding refs
edge_lowerings[]
  required_port_id
  provided_port_id
  core relation ref
  produced core object refs
  preserved type/resource/capability/version/span/cardinality/provenance refs
core_objects[]
  object kind fact|event|signal_source|schedule|effect|rule|dependency|
    projection|assertion|diagnostic
  object id
  owner kind node|edge|dependency
  owner ref
  runtime entrypoint fact_record|event_record|signal_source_template|
    schedule_template|effect_graph_template|rule_template|
    effect_dependency_template|event_projection|assertion_check|
    diagnostic_record
  entrypoint refs, when the runtime entrypoint needs them
    dependency objects require upstream_effect, predicate, downstream_effect
    rule objects require rule, fact, graph
    event-source objects require event
    schedule objects require schedule
    assertion objects require assertion
  source span
  resource/capability/version refs, if applicable; each ref set must be unique
diagnostics[]
  code
  severity
  node/edge/core object refs
  source span
```

The draft JSON Schema for this report lives at
`report-schemas/lowered_ir_report_v0.schema.json`.

Vocabulary note: the target package design calls author-declared outside inputs
`signal`s and shared source declarations `signal_source` / `clock_source`. The
signal half of this rename has landed (Stage P1a): report schemas and the
generated Maude bridges now use `signal_source` / `signal_emit` /
`signal_source_template`. The clock half still uses the legacy
`schedule_emitter` / `schedule` vocabulary for the `timer.wait` slice pending the
P1b source-declaration restructure (see the clock note in the legacy → target
map). The Maude model operator vocabulary (camelCase `eventSourceLowering`,
`coreEventSourceKind`, …) was intentionally left unchanged; the bridges
translate the renamed wire values to those model operators.

The concrete lowered-IR validator accepts only runtime entrypoints that the
compiler lowerer and generated runtime-handoff bridge can represent. The
current executable vocabulary is `fact_record`, `event_record`,
`effect_graph_template`, `signal_source_template`, `schedule_template`,
`rule_template`, `effect_dependency_template`, `event_projection`,
`assertion_check`, and `diagnostic_record`. Event, projection, and diagnostic
objects require explicit `entrypoint_refs` matching the runtime handoff shape:
`event`, `event` plus `fact`, and `rule` respectively. This admission support
does not add source syntax that emits durable event occurrences; source-level
signal admission remains runtime-owned.

Current implementation status: `check --json` and `compile --json` emit
`lowered_ir_report` beside `construct_graph`. The executable slice covers
package-backed `capability_call` construct nodes, compiler-owned `core_effect`
built-in nodes, metadata effect-contract nodes, capability-resolution edges,
source effect dependencies, and compiler-owned source-admission nodes for legacy
typed `event` declarations (target `signal` declarations), compiler-owned
schedule nodes for `timer.wait`,
compiler-owned assertion-check nodes for top-level source assertions,
compiler-owned rule-template nodes for rule triggers, and checker-owned
projection-read metadata nodes for rule guards and assertions. Ordinary
effect-operation nodes lower to `effect` core objects with the
`effect_graph_template` entrypoint, while `timer.wait` lowers to a `schedule`
core object with the `schedule_template` entrypoint. Metadata effect-contract
nodes lower to no runtime object, capability-resolution edges preserve static
relations without emitting runtime objects, source effect dependencies lower to
`dependency` core objects with the `effect_dependency_template` entrypoint,
source-admission nodes lower to `signal_source` core objects with the
`signal_source_template` entrypoint, assertion-check nodes lower to `assertion`
core objects with the `assertion_check` entrypoint, and rule-template nodes
lower to `rule` core objects with the `rule_template` entrypoint plus one
`fact` core object with the `fact_record` entrypoint for each fact schema the
rule body can record.
These fact objects are static rule-owned fact-record templates, not durable
fact occurrences committed at compile time. Projection-read metadata nodes
lower to zero runtime objects; they preserve deterministic read dependencies
for checker/tooling visibility and must not be confused with event/fact
projection records. Construct graph output declarations are validated against
this same core object kind/runtime entrypoint vocabulary before lowering
preservation is attempted, so stale graph terms such as `effect_object` or
`kernel.graph_commit` cannot enter the generated bridge as accepted output
evidence. The report is validated for graph ownership, duplicate
core-object IDs, exactly-one node/edge/dependency lowering coverage, exactly-one
core object ownership inside the covered construct-graph slice, and entrypoint
refs required by fact-record, dependency, event-source, schedule, assertion-check,
and rule templates. The formal bridge keeps source lowering
classes distinct from runtime entrypoints: for example,
`assertion_check` maps to the Maude lowering class `assertionCheckLowering`,
while the emitted core assertion object uses the runtime entrypoint
`assertionCheck`. Likewise, `rule_template` maps to `ruleTemplateLowering`,
while emitted rule objects use `ruleTemplate`. The generated formal bridge
derives node class lifecycle acceptance from each artifact node's lowering
class, construct family, emitted core-object inventory, and runtime entrypoint
mapping; it no longer asserts `nodeClassLifecycleAccepted` or
`nodeClassOutputAccepted` as raw facts. The bridge also consumes each core
object's `runtime_entrypoint` and proves runtime handoff for the covered slice
without using a broad runtime-lifecycle evidence shortcut.

Accepted lowered reports include validator-owned `derived_facts` for the same
covered slice: node/edge/dependency lowering coverage, node lifecycle inputs,
node output compatibility, node/edge/dependency preservation inputs, core
object owner and runtime-entrypoint validation, graph-wide lowering-member
uniqueness, graph-wide deterministic lowering identity, report completeness,
ownership uniqueness, no-runtime-input evidence, and runtime-boundary
inventory.
Lifecycle input evidence includes an aggregate predicate plus per-field
predicates for lowering class, construct family, lifecycle profile, produced
core objects, produced object kinds, and produced runtime entrypoints. A
generated bridge may use those facts to derive `nodeClassLifecycleAccepted` and
`nodeClassOutputAccepted`; it may not replace them with untraced assumptions
about the lowering class.
Node output compatibility means every node-owned emitted core object has an
object kind and runtime entrypoint allowed by that accepted construct graph
node's `allowed_core_object_kinds` and `allowed_runtime_entrypoints`. Output
compatibility evidence includes the allowed graph vocabulary and produced output
inventory, split into allowed object-kind, allowed runtime-entrypoint, produced
core-object, produced object-kind, and produced runtime-entrypoint components.
Node, edge, and dependency preservation evidence includes an aggregate preservation
predicate plus per-field validator predicates backed by the corresponding
lowered report field, not by bridge-only assertions. Node components cover
lowering class, produced core objects, source span, resource, capability,
version, cardinality, provenance, and terminal binding. Edge components cover
required/provided port identity, core relation, produced core objects, type,
resource, capability, version, span, cardinality, and provenance. Dependency
components cover produced core objects, effect endpoints, predicate, span, and
provenance. These facts are emitted only when the lowered report has no
diagnostics, so generated formal checks can consume a trace produced by the
concrete validator rather than relying on hand-written premises.
Edge lowering identity is the full `(required_port_id, provided_port_id)` pair.
The generated bridge must not index edge lowerings only by required port,
because `many` and `named-many` required ports can legally resolve multiple
providers through the same required port. Generated model-search descriptions
for edge lowering preservation include both endpoint IDs for the same reason:
human-readable ledger rows must stay distinguishable when aggregate resolution
produces more than one edge from the same required port.
Generated node preservation obligations likewise enumerate explicit edge refs
with `(required node, required port, provider node, provided port)` identity.
They do not lower node requirements to a required-port-only list, because that
would allow aggregate ports to collapse multiple provider edges before the
formal preservation check sees them.

Core object metadata refs are first-class evidence, not loose annotations.
When `resource_refs`, `capability_refs`, or `version_refs` are present on a
core object, the concrete validator and schema require them to be arrays of
unique non-empty refs. Graph-wide ownership evidence also cites the owner kind
and owner ref for each object, so a generated bridge cannot collapse node,
edge, and dependency ownership into an ambiguous owner string.

The generated lowered-IR bridge is fail-closed with respect to that trace:
before emitting Maude obligations the dedicated lowered-IR report schema rejects
duplicate node, edge, and dependency lowering witness refs, and the report
schemas reject duplicate derived-fact `input_refs`. The bridge also rejects
duplicate logical lowered members before set-based trace comparison: node
lowering refs, implicit edge lowering refs, dependency lowering refs, and core
object IDs must be unique in the emitted artifact. It independently rebuilds
the node, edge, and dependency lowering membership sets from the accepted
construct graph, requires exact coverage, and rebuilds the produced core-object
	owner map from `produced_core_object_refs` before accepting each object's
declared `owner_kind` and `owner_ref`. It also validates the required
`entrypoint_refs` payloads for each supported runtime entrypoint before
emitting Maude obligations, including their expected values when those values
are determined by the accepted graph and lowered core-object identity: fact
records require fact/schema refs,
dependency templates require upstream/predicate/downstream refs, source-admission
templates currently require event refs, schedule templates require schedule refs, rule
templates require rule/fact/graph refs, and assertion checks require assertion
refs. The validator-owned core object entrypoint facts cite those concrete
`entrypoint_refs` key/value witnesses, so the generated bridge rejects stale
or weakened entrypoint trace before emitting Maude obligations. The bridge then verifies
that every node, edge, dependency, core object, and graph-wide lowering
predicate promised by the current `lowered_ir_validator` vocabulary is present
exactly once in `derived_facts` with the exact expected unique `input_refs` set,
and that no unknown validator-owned predicates are present. This includes both
aggregate and per-field node, edge, and dependency preservation evidence. The
bridge still derives the concrete Maude premises from the lowered report
inventory, but it will not accept a diagnostic-free report that has lost,
duplicated, weakened, padded, or stale validator trace for coverage, output
compatibility, preservation inputs, lifecycle inputs, ownership, entrypoints, or
graph-level lowering boundary evidence. It also rejects stale artifact pairs:
the lowered report's `graph_id`, `source_digest`, and `package_lock_digest`
must match the construct graph it is paired with before any generated
obligation can run. The
generated lowered-IR bridge also admits that construct graph through the
construct-graph bridge's full validator-owned trace and inventory checks, so
lowering preservation obligations cannot be generated from a graph whose
construct evidence would fail graph-only admission. The
standalone artifact-report validator applies the same lowered-IR trace and
artifact-identity admission checks without emitting Maude, so report/schema CI
can catch stale or spoofed lowering evidence before formal search generation.

Every emitted core object must be accounted for by a node, edge, or source
effect dependency in the accepted graph. The report must be deterministic for
the same source, package lock, accepted graph, and platform lowerer version.

Core object accounting is positive coverage, not an absence claim. A lowered
report is covered only when every emitted core object appears in exactly one
node, edge, or dependency lowering entry, carries an allowed core object kind,
names a valid runtime entrypoint class, and names an owner from the accepted
graph. Ownership is not just positive coverage: a core object with two owners is
invalid even if both owners are accepted graph elements. This includes
node-vs-node, node-vs-edge, edge-vs-edge, and dependency ownership duplicates.
An extra effect, dependency, projection, diagnostic object, or any run/lifecycle
state with no accepted graph owner is a lowering failure. A run is not a lowered
core object; worker runs are created by the runtime after claimability, policy,
and provider binding.

## Preservation Obligation

For every accepted construct graph, lowering must preserve:

```text
node identity
edge identity
source spans
port kind
payload type
resource identity
contract version
cardinality
capability requirements
causal provenance
terminal output binding
diagnostic references
```

The core obligation is:

```text
accepted edge R -> P in the construct graph
  => corresponding core IR dependency or binding exists
  => the core relation carries the same type/resource/version/capability/span
```

An accepted program is not lowerable merely because `graphAccepted` holds. It is
lowerable only when the accepted-program adequacy contract holds and each node
lowers through a declared platform lowering class.

## Lowering Class Lifecycle Contract

Each lowering class must have a platform-owned catalog entry before any package
construct can use it or any compiler-owned node can enter the generated
lowering bridge. The catalog entry declares:

```text
accepted construct families
whether package manifests may target the lowering
allowed emitted core object kinds
allowed runtime entrypoint classes
authority profile: none, capability-scoped, signal-admission
  (currently named event-admission in reports), or projection
output validation requirements
determinism and contract pinning
no runtime inputs during lowering
no hidden authority
no package-owned scheduler
no package-owned lifecycle state
no direct fact writes
no direct rule firing
```

## Lowering Class Catalog

This is the single normative list of lowering classes. `construct-grammar.md`
and `construct-graph-calculus.md` reference this table rather than restating it.
It mirrors `PLATFORM_CONSTRUCT_CATALOG` in `crates/whipplescript-core/src/lib.rs`
and the report schemas; those are the source of truth and this table must track
them.

Two facts govern the whole table:

- Exactly two classes are package-authorable: `metadata_only` and
  `capability_call`. The other ten are compiler/platform-owned. The package
  manifest `lowering_target` enum is therefore `["metadata_only",
  "capability_call"]` and nothing else.
- There are two metadata classes. `metadata` is compiler-owned and attaches to
  `effect_contract` and `projection_read` nodes. `metadata_only` is the
  package-authorable declaration-block class. They are not the same class. Both
  emit no runtime object.

| Lowering class | Authorable | Construct family | Authority profile | Lifecycle profile | Emitted core object |
| --- | --- | --- | --- | --- | --- |
| `metadata` | no | `effect_contract`, `projection_read` | none | none | — |
| `metadata_only` | yes | `declaration_block` | none | none | — |
| `capability_call` | yes | `effect_operation` | capability-scoped | `effect_graph` / `typed_effect_graph` | effect |
| `typed_effect_call` | no | `effect_operation` | capability-scoped | `typed_effect_graph` | effect |
| `resource_effect` | no | `effect_operation` | capability-scoped | `resource_effect_graph` | effect |
| `core_effect` | no | `effect_operation` | capability-scoped | `effect_graph` / `typed_effect_graph` | effect |
| `signal_emit` | no | `effect_operation` | signal-admission | `event_record` | event |
| `signal_source` | no | `source_declaration` (target) | signal-admission | `signal_source_template` | signal-source admission |
| `clock_source` | no | `source_declaration` (target) | signal-admission | `clock_source_template` (new) | clock-source admission |
| `rule_template` | no | `rule` | none | `rule_template` | rule (+ fact-record templates) |
| `projection_view` | no | `projection_read` | projection-source | `event_projection` | projection |
| `assertion_check` | no | `assertion` | none | `assertion_check` | assertion |

`core_effect` is the compiler-owned class for built-in rule-body effect
operations (`tell`, `ask`, and similar) that are not package capability calls.
It is live in the catalog and must appear in every taxonomy list; earlier drafts
omitted it.

### Restructure: source declarations

`signal_source` and `clock_source` are bound to a new `source_declaration`
construct family for top-level `source … as …` and `source clock …` blocks.
This is a structural change, not a rename: the live slice binds the equivalent
lowerings to the legacy `event_source` declaration family and to the
`timer.wait` `effect_operation`. In the target:

- a `signal <name> { <fields> }` declaration is a typed schema only. It lowers
  through `metadata` (provides `Signal<T>`, emits no runtime object).
- a `source … { observe; emit }` block lowers through `signal_source`.
- a `source clock … { … }` block lowers through `clock_source`.

Backwards compatibility is not preserved. The formal models, report schemas, and
generated Maude bridges adopt the target names first; the compiler emitter is
then made to conform.

### Legacy → target name map

The signal half of the rename has **landed** (Stage P1a). There is no
clock/schedule rename: `clock_source` is added net-new in P1b and `timer.wait`
keeps its `schedule_*` vocabulary — see the clock note below.

| Former name | Current/target | Status |
| --- | --- | --- |
| `event` declaration | `signal` declaration | landed (source syntax was already `signal`) |
| lowering class `event_source` | `signal_source` | landed |
| lowering class `event_emit` | `signal_emit` | landed |
| core object kind `event_source` | `signal_source` | landed |
| lifecycle profile / runtime entrypoint `event_source_template` | `signal_source_template` | landed |
| `schedule_emitter` / `schedule` / `schedule_template` | unchanged (stay with `timer.wait`) | no rename |
| (new) `clock_source` / `clock_source_template` | added in P1b for `source clock` | additive |

The signal-half rename is wire-only: the code constants, report-schema enums, and
the values the Maude bridges read were renamed. The Maude **model** operator
vocabulary (camelCase `eventSourceLowering`, `coreEventSourceKind`, …) is a
separate axis and was intentionally left unchanged; the bridges translate the
renamed wire values to those model operators. Renaming the model operators is a
deferred, optional follow-up.

**Clock note (P1b — decided: coexist).** `schedule_emitter` is *not* renamed to
`clock_source`; they are distinct constructs that coexist. `schedule_emitter` is
the lowering for `timer.wait` — a one-shot, rule-body `effect_operation` that
completes an effect and releases a dependent in the same instance. `clock_source`
is a top-level `source clock { … }` recurrence source (`source_declaration`
family, not yet implemented) whose runtime admits a durable **signal fact** on a
calendar schedule and fires no rule directly. They differ at every layer (family,
trigger, output, lifecycle), so:

- `timer.wait` keeps the `schedule_emitter` / `schedule` / `schedule_template`
  vocabulary unchanged.
- `clock_source` is **net-new** with its own vocabulary (`clock_source` object
  kind, `clock_source_template` entrypoint). No `schedule`→`clock` rename occurs.

P1b is therefore purely additive (new `source_declaration` family + `source`
block parser + `clock_source`), not a rename.

The **event-sourcing substrate does not rename**: the core object kind `event`
(a durable occurrence), the runtime entrypoint `event_record`, and the
lifecycle profile / runtime entrypoint `event_projection` stay as-is. A rule
that emits a signal still lowers to a durable `event` via `event_record` —
"signal" is the author/admission concept, "event" is the durable substrate.
The execution sequence is Stages P1a (landed) and P1b (pending) in
[`implementation-plan.md`](implementation-plan.md).

### Not lowering classes

- **Resource declarations** (`tracker {}`, `file store {}`, `channel {}`) lower
  through `metadata_only`. The resource is a registry/capability identity, not a
  lowering-emitted runtime object; resource state is ordinary facts and resource
  mutations are effects, and durable resource lifecycles (leases, claims) are
  kernel-owned. There is no `resource_declaration` lowering class.
- **Turn-access grants** (`with access to <resource> { … }`) are
  authority-narrowing metadata on the agent-turn effect, expressed as required
  ports (`Resource`, per-granted `Operation`, `Capability`) on the `tell` node
  and lowered as bounded sub-authority fields on the `agent.tell` effect.
  In-turn tool invocations are recorded as evidence, not durable child effects.
  There is no `agent_turn_grant` lowering class.

### Namespace convention

`signal_source`, `schedule_template`, `rule_template`, `event_projection`, and
`assertion_check` name values in more than one axis (lowering class, lifecycle
profile, core object kind, runtime entrypoint). Prose must qualify the axis:
write "lifecycle profile `schedule_template`", "runtime entrypoint
`assertion_check`", "core object kind `signal_source`". Bare tokens are ambiguous
and must not be used.

### Lifecycle profile detail

The class categories, restated as lifecycle/authority profiles:

```text
metadata, metadata_only -> no core output, no runtime authority
capability_call, typed_effect_call, resource_effect, core_effect
                        -> effect object, effect graph template,
                           capability-scoped, runtime-boundary validation
signal_emit             -> event occurrence object, durable event record,
                           typed payload, kernel/runtime-owned admission
signal_source           -> signal-source object, admission template only,
                           typed payload, kernel/runtime-owned admission
                           (renamed from event_source; landed)
clock_source            -> clock-source object, admission template only,
                           typed payload, runtime-owned clock admission
                           (target; live slice is schedule_emitter/timer.wait
                           pending the P1b restructure)
rule_template           -> rule object and rule-owned fact-record templates,
                           no runtime authority
projection_view         -> projection object, event/fact projection record,
                           typed projection source
assertion_check         -> assertion object, assertion check template,
                           no runtime authority
```

A class name is not enough. The checker must reject a class if its family,
authorability, authority profile, emitted core object kind, runtime entrypoint,
or forbidden behavior facts are missing or mismatched. Future classes are inert
until the platform catalog declares their lifecycle/static/authority profile and
the formal model admits that profile shape. Compiler artifact admission rejects
any lowering class/lifecycle-profile pair outside the admitted catalog before
generated formal bridge obligations are emitted.

## Forbidden Lowering Effects

Lowering must reject any construct that would:

```text
drop a required edge
drop or rewrite source spans
widen a resource identity
change a port's cardinality
change a contract version
introduce an undeclared capability
emit an unaccounted fact/event/effect/rule/dependency/projection
emit the same core object from more than one node, edge, or dependency lowering
emit a run, claim, provider run, terminal status, cancellation request,
  cancellation ack, retry record, lease recovery state, or provider evidence
create package-owned rule scheduling
create package-owned retry, claim, cancellation, terminal status, or replay
complete work outside the core effect/run lifecycle
depend on runtime provider output, clock state, queue state, memory retrieval,
  ambient credentials, filesystem order, or network state during lowering
```

The lowering model is allowed to be conservative. A construct that cannot prove
preservation should fail package acceptance or compile, even if it might work at
runtime.

## Core IR Boundary

Lowering can emit only ordinary core objects and lifecycle entrypoint
templates:

```text
facts
events
signal-source admission templates
clock-source admission templates
effects
rules
effect dependency edges
projections
assertions
diagnostics
```

Package-specific semantics must be represented as typed contracts and core IR
metadata, not as new runtime transition systems. Runtime lifecycle behavior is
validated in the lifecycle model, but lowering must already prove that packages
cannot bypass that lifecycle.

## Runtime Handoff Boundary

Lowered core IR is not runtime state. It is the accepted program's deterministic
entry into runtime-owned state machines.

The handoff classes are:

```text
fact object        -> durable fact record
event object       -> durable event record
signal_source object -> signal-source admission template
schedule object    -> clock-source admission template (legacy `schedule_emitter`; P1b)
effect object      -> effect graph template
dependency object  -> effect dependency graph template
rule object        -> rule template
projection object  -> event/fact projection record
assertion object   -> assertion check template
diagnostic object  -> diagnostic record
```

Effect objects hand off as graph templates. Only the kernel rule-commit path
may enqueue the corresponding outbox effect. Dependency objects hand off as
existing kernel dependency graph templates: the upstream effect can be queued,
the downstream effect starts blocked, and scheduler release happens only after
the upstream lifecycle satisfies the declared predicate.

Event objects enter the event/fact projection lifecycle as concrete event
occurrences. Signal-source and clock-source objects register admission
templates; they do not append event occurrences during lowering or handoff.
Clock templates depend on runtime-owned clock admission before a signal fact can
exist.
Projection objects enter the projection lifecycle. Rule, fact, assertion, and
diagnostic objects enter the existing kernel terms without package-owned
transitions.

Lowering must not materialize runs, claims, terminal statuses, cancellation
requests or acknowledgements, retry records, lease recovery state, provider
evidence, provider-run markers, or any worker-owned lifecycle state. Provider
boundaries remain runtime-owned: workers and the runtime store create run
records, provider evidence, and terminal events.

## Current Formal Coverage

The Maude model `models/maude/construct-lowering.maude` captures a first finite
abstraction:

```text
acceptedProgram is required before lowering preservation can hold
node lowering requires platform ownership and core-only output
node lowering rejects extra capabilities and package schedulers
edge lowering requires a corresponding core relation
edge lowering preserves type, resource, capability, version, span, cardinality,
  and provenance
graph lowering requires every node and required edge to be preserved
graph lowering requires every source effect dependency to be preserved
graph lowering requires deterministic lowered IR reports
graph lowering requires every emitted core object to have exactly one owner
graph lowering rejects duplicate core-object ownership and runtime inputs
duplicate node-vs-node, node-vs-edge, edge-vs-edge, and dependency ownership
  normalizes to checker-visible inconsistency before coverage can account for
  the object
node lowering rejects package-owned lifecycle behavior beyond scheduling
negative cases cover missing core relations, resource/version/span/terminal
  preservation, report completeness, deterministic lowering, object coverage,
  runtime inputs, extra capabilities, package schedulers, and package lifecycle
  semantics
```

The runtime handoff model has no aggregate lifecycle-boundary shortcut. Both
hand-written fixtures and the generated compiler-output bridge must provide the
individual `noLowered...` runtime-state claims. The compiler validator derives
those claims from the lowered IR report/object inventory, requires
checker-owned evidence for deterministic lowering, report completeness, and
no-runtime-inputs during lowering, rejects runtime-owned object kinds and
entrypoints directly, and proves `lifecycleHandoffOk` from those derived claims
plus each object's concrete runtime entrypoint.

The Maude model `models/maude/lowering-class-lifecycle.maude` captures the
extension/lowering-class layer between graph acceptance and lowering
preservation:

```text
lowering classes are accepted only with a complete lifecycle profile
metadata emits no executable IR
capability_call, typed_effect_call, and resource_effect emit effect graph
  templates and require capability scoping plus runtime-boundary validation
signal_emit emits `event` core objects through `event_record` entrypoints with
  typed payloads and kernel/runtime-owned admission
signal_source and clock_source emit admission templates, not event occurrences
  (signal_source is live; clock_source is still serialized as schedule_emitter
  pending P1b)
projection_view emits `projection` core objects through `event_projection`
  entrypoints, not direct fact records
node acceptance bridges class lifecycle profiles into existing lowering
  obligations: platform lowering, core-only output, allowed output class, no
  extra capability, no package scheduler, and no package lifecycle
negative cases cover unknown classes, missing capability scoping, missing
  output validation, missing event admission ownership, direct fact output,
  metadata emitting effects, rule templates masquerading as event emissions,
  assertion checks emitting effects, object/entrypoint mismatch, family
  mismatch, direct rule firing, package scheduler authority, run-state output, and hidden
  authority
```

The Maude model `models/maude/lowering-runtime-handoff.maude` captures the
handoff from lowered core IR into existing runtime lifecycle shapes:

```text
lowering preservation alone is not sufficient for runtime handoff safety
every emitted object must enter through an allowed runtime entrypoint
effects enter as graph templates and are queued only by kernel graph commit
dependencies enter as dependency graph templates with blocked downstream work
events and projections enter the event/fact projection lifecycle
run, claim, terminal, cancellation, retry/lease, and provider-run state cannot
  be materialized by lowering
provider execution remains runtime-owned
generated compiler-output checks prove object-level handoff for every emitted
  core object entrypoint and aggregate lifecycle handoff without package-owned
  provider/run state
```

This is intentionally not yet a full compiler proof. The next pass should align
more construct families and full runtime handoff evidence with the concrete
compiler's emitted normalized graph artifact and lowered IR report.
