# Construct Graph Calculus

Status: draft formal baseline

This document isolates the construct graph from concrete WhippleScript syntax
and package domains. The goal is a small static calculus that can be modeled
once, then reused by standard and third-party packages.

The construct grammar answers:

```text
can packages expose native-feeling source forms?
```

The construct graph calculus answers:

```text
after parsing and package resolution, is this package composition statically
well formed?
```

The intended pipeline is:

```text
source + package lock
  -> construct instances
  -> typed construct graph
  -> accepted graph
  -> accepted program
  -> ordinary core IR
```

The graph is compile-time and analysis data. It is not a second runtime.

## Graph Objects

A construct graph contains:

```text
nodes       construct instances from core, std libraries, or packages
ports       provided or required typed interfaces on those nodes
edges       explicit resolution from required port to provided port
versions    locked contract versions on nodes and ports
lowering    declared platform-owned lowering class and allowed output kind
authority   declared capabilities required by lowered effects
diagnostics source spans and package ownership for errors
```

A node is not accepted merely because it exists. A node is accepted only when
all of its ports, lowering obligations, and authority obligations pass the
platform checks.

The normative list of lowering classes and their families, authority profiles,
lifecycle profiles, and emitted core objects lives in the lowering-class catalog
in [`construct-lowering-preservation.md`](construct-lowering-preservation.md#lowering-class-catalog).
This document references that catalog rather than restating it. The signal half
of the rename has landed: the live report vocabulary is now `signal_source` /
`signal_emit` / `signal_source_template`. The clock/schedule half
(`schedule`, `schedule_emitter`, `schedule_template`) is still live under its
legacy names pending the P1b source-declaration restructure; the catalog holds
the legacy → target map.

## Normalized Graph Artifact

The compiler should be able to emit a normalized graph artifact for every
checked program. The artifact is the implementation object that the formal model
talks about. It should contain:

```text
graph_id
platform_version
package_lock_digest
source_digest
nodes[]
  node_id
  construct_id
  construct_family
  lowering_class
  lifecycle_profile
  owner package/library
  source span
  required_ports[]
  produced_ports[]
  required_capabilities[]
  lowered_effect_capabilities[]
  lowering_output_kind
  allowed_core_object_kinds[]    # lowered-IR object kind vocabulary
  allowed_runtime_entrypoints[]  # lowered-IR runtime entrypoint vocabulary
ports[]
  port_id
  owner node_id
  direction required|produced
  kind
  type
  phase
  resource identity, if any
  contract version
  cardinality
  source span
edges[]
  required_port_id
  provider_node_id
  provided_port_id
  order index, if the required port cardinality is many or named-many
  resource key, if the required port cardinality is named-many
  resolution reason
  source/import/alias/resource binding evidence
derived_facts[]
  predicate
  owner subsystem
  input artifact refs
  diagnostic span
diagnostics[]
  code
  severity
  package/node/port/edge refs
  source span
```

The draft JSON Schema for this artifact lives at
`report-schemas/construct_graph_v0.schema.json`.

The artifact must be canonical enough that the same source, package lock, and
platform version produce byte-for-byte stable graph identity and stable
node/port/edge IDs. Human-readable diagnostic wording can evolve, but diagnostic
codes and structural refs should remain stable.

Current implementation status: `check --json` emits this normalized graph
artifact and the report schema validates the nested artifact against
`report-schemas/construct_graph_v0.schema.json`. The executable slice covers
locked package-backed `capability.call` constructs, package effect-contract
metadata nodes, compiler-owned built-in effect-operation nodes, and source
effect dependencies. `timer.wait` effect-operation nodes advertise
`schedule`/`schedule_template` output vocabulary and lower to runtime-owned
clock admission templates rather than generic effect graph templates. In the
target package vocabulary, typed `signal` declarations and `source ... as ...`
blocks produce signal-source and clock-source admission nodes. The signal-source
rename has landed: the generated report vocabulary now uses `signal_source`
object names. The clock path still uses the legacy `schedule` object name (the
`timer.wait` slice above) until the P1b source-declaration restructure adds a
`source clock` block. These source nodes lower to admission templates rather
than durable event occurrences. Top-level source
assertions are emitted as
compiler-owned assertion-check nodes. Rule triggers are emitted as
compiler-owned rule-template nodes keyed by rule name and trigger index; rule
nodes that can record facts advertise both `rule`/`rule_template` and
`fact`/`fact_record` output vocabulary, with the concrete fact schemas listed
in node metadata. Guard and assertion projection queries are emitted as
checker-owned projection-read metadata nodes; these nodes expose deterministic
read dependencies to tooling but lower to no runtime object. Compiler-owned
`model_search.guard_source` and `model_search.terminal_branch_source` derived
facts anchor generated checker source spans for guard and terminal-branch
searches without adding runtime objects or acceptance predicates. It validates
the artifact for node/port
ownership, edge endpoint compatibility, type, phase, version, resource
identity, ID uniqueness, cardinality semantics, effect-dependency endpoints,
dependency predicates, and node output vocabulary. Ordinary effect-operation
nodes advertise the lowered vocabulary `effect` + `effect_graph_template`;
metadata nodes that emit no runtime object advertise no allowed core object
kind. Stale or pre-lowered names such as `effect_object`,
`effect_graph_template` as an object kind, or `kernel.graph_commit` as an
entrypoint are rejected before validator facts are emitted. Accepted graphs also carry
`construct_graph_validator` derived facts for those covered validator
predicates, so the artifact distinguishes package/lowering facts from
checker-owned graph acceptance facts.

`scripts/check-formal-models.sh` generates a package lock, checks and compiles
`examples/package-memory.whip`, emits a digest-bearing `package_contract`
artifact, and lowers the emitted construct graph into a Maude module with
`scripts/construct-graph-to-maude.py`. It also checks the core-only
`examples/event-bridge.whip` construct graph, exercising generated
acceptance obligations for signal-source declarations, rule templates, assertion
checks, projection-read metadata nodes, metadata nodes, and core effects. The
generated modules prove each emitted node is accepted, each emitted edge is
accepted, the graph aggregates from accepted nodes, and the admitted graph
reaches `acceptedProgram` through compiler-emitted adequacy predicates checked
by bridge admission. Those predicates cover source/lock determinism, closed
registries, validator fact accountability/consistency, namespace and construct
identity stability, authority scoping, phase/cardinality/version discipline,
complete diagnostics, and declared lowering/lifecycle boundaries. The
generated obligations use generic list-shaped graph and port
facts, so the bridge is no longer tied to small hand-written arities. The
bridge consumes the embedded artifact from either single-entry `check_report_v0`
arrays, explicitly selected multi-entry check-report entries via
`--entry-index`, or `compile_report_v0` objects.
Before emitting Maude, the bridge validates the input against
`check_report_v0` or `compile_report_v0`, and the embedded construct graph against
`construct_graph_v0`. The dedicated construct graph schema rejects duplicate
node inventory refs, declared interface entries, capability/output refs, edge
evidence refs, dependency evidence refs, and derived-fact `input_refs`, so
malformed graph and trace arrays fail before bridge-specific admission. The
bridge also rejects duplicate logical graph-member refs before set-based trace
comparison: node IDs, port IDs, implicit edge refs
`required_port_id -> provided_port_id`, and effect dependency refs must be
unique in the emitted artifact. It then admission-checks the concrete
node, port, edge, and dependency inventory before trace comparison: node port
refs must name ports owned by that node with the right direction, declared
interfaces must be satisfied by concrete same-direction ports with compatible
phase coverage and cardinality, and catalog-declared lowering interface
requirements must be present on nodes whose lowering class appears in the
verifier platform catalog. Edge endpoints must name existing
required/produced ports and provider nodes, the provider node must own the
provided port, edge endpoint
kind/type/phase/version/resource compatibility must hold, and effect
dependency endpoints and predicates must be valid. The bridge then checks the
`derived_facts` trace for the graph, node profile, declared interface,
capability, output, node/port consistency, port profile, edge compatibility,
required-port cardinality, effect dependency, and graph acceptance predicates it expects from
`construct_graph_validator`. These predicates must be owned by the
`construct_graph_validator` subsystem; same-named facts from another subsystem
do not satisfy bridge admission. The validator-owned predicate vocabulary must
exactly match the current checker slice: unknown validator-owned predicates are
rejected, each required predicate must appear exactly once, and its `input_refs`
must exactly match the concrete graph, node, construct profile, interface,
capability, output, port, edge, dependency, and cardinality witnesses that each
predicate claims to validate. Edge compatibility facts cite labeled required
and provided port metadata witnesses for owner, direction, kind, type, phase,
contract version, cardinality, resource identity, resolution reason, and
resolution evidence; endpoint IDs alone are not enough evidence for
`validator.edge.kind_compatible`, `type_compatible`, `phase_compatible`,
`version_compatible`, or `resource_compatible`. Missing, duplicated, weakened,
padded, spoofed, or stale validator certificates are rejected before Maude
obligations are emitted.
Port profile facts separately certify each emitted port's owner, direction,
kind, type, phase, contract version, cardinality, and resource identity, so the
generated `port(...)` Maude premises are tied to validator-owned evidence even
when a port is not used by an edge.
The graph-wide acceptance predicate is an inventory certificate: it must cite
the graph ID, node IDs, port IDs, edge refs, and effect dependency refs that the
generated artifact searches use. Removing an edge or dependency ref from that
fact makes bridge admission fail even if the per-edge/per-dependency facts are
still present. After schema validation has established unique graph arrays and
unique `input_refs`, and after direct bridge checks have established unique
logical graph-member refs, the graph-wide uniqueness and acceptance facts are
compared against the exact ref sets. Adding an unrelated bogus ref is rejected
by both the standalone artifact-report validator and the Maude bridge admission
path like any other stale trace evidence.
This means generated construct-graph Maude premises are admitted from either
validator-owned trace evidence or direct schema/contract/inventory checks,
rather than from raw report inventory alone. It prevents a malformed or
diagnostic-free graph from entering the generated formal path after losing,
spoofing, or weakening the validator facts that the artifact format currently
promises. Broader accepted-program adequacy remains a separate layer above
graph acceptance.
The explicit verifier platform catalog is also an admitted premise. Bridge
generation rejects catalogs with duplicate scalar vocabulary, duplicate family
or lowering IDs, unresolved lowering families/scopes, non-authorable package
lowering targets, malformed lifecycle/static/authority profiles, or lowering
interface requirements that are not declared as platform interface kinds before
those catalog entries can define package construct admission.
`scripts/platform-catalog-to-maude.py` also lowers the emitted catalog itself
into `lowering-class-lifecycle.maude` obligations. Each catalog lowering must
derive the model's static-safety and authority acceptance. Catalog-level
acceptance deliberately stops there: concrete output object kinds and runtime
entrypoints are proven from each lowered IR report, where the emitted objects
are visible.

The same check report now carries a `lowered_ir_report`. The generated
lowering bridge (`scripts/lowered-ir-to-maude.py`) proves edge preservation,
node lowering preservation, core-object coverage, graph lowering boundary
evidence, graph lowering aggregation, and runtime lifecycle handoff for the
covered package-backed `capability_call`, compiler-owned `core_effect`, and
effect-dependency lowering slice, plus compiler-owned source admission
templates, `schedule_template` entries for `timer.wait`, `assertion_check`
templates, static `rule_template` entries, rule-owned `fact_record` templates,
and checker-owned projection-read metadata nodes with no emitted runtime
objects.
For node lowering, the generated bridge now requires aggregate and per-field
validator evidence for lifecycle inputs and output compatibility, then derives
`nodeUsesLoweringClass`, `nodeConstructFamily`, class-family allowance, static
safety, and class authority facts from the admitted platform catalog rather than
from a bridge-local class table. It emits node class output and
object/entrypoint facts from the lowered report. It then lets
`lowering-class-lifecycle.maude` derive class acceptance. It does not assert
`nodeClassLifecycleAccepted` or `nodeClassOutputAccepted` directly.
The lowered IR validator requires every graph node, interface edge, and effect
dependency to have exactly one lowering entry before the report is accepted.
The generated runtime handoff obligation is derived from the lowered report's
concrete `runtime_entrypoint` inventory and individual
no-lowered-runtime-state claims, not from an aggregate runtime-lifecycle
evidence shortcut. The hand-written construct-graph, lowering, and handoff
fixtures remain targeted model examples for negative cases and edge conditions,
but they use the same explicit boundary facts as generated bridge obligations.

Like the construct-graph bridge, the lowered-IR bridge schema-validates the
check or compile report, construct graph, and lowered IR report before emitting
Maude; duplicate derived-fact `input_refs` are schema-invalid at that boundary.
It admits the embedded construct graph through the same construct-graph bridge
trace and inventory checks before any lowering obligation is emitted, so direct
lowered-IR bridge runs cannot bypass stale construct graph validator evidence
or spoofed graph inventory. It then rejects graph/lowered pairs whose
`graph_id`, `source_digest`, or `package_lock_digest` differ, then
admission-checks the lowered report's concrete `derived_facts` trace. It requires
`lowered_ir_validator`-owned predicates for node/edge/dependency lowering,
node/edge/dependency preservation inputs, node lifecycle inputs, core object
ownership and runtime entrypoints, graph-wide lowering-member uniqueness,
graph-wide coverage, deterministic lowering identity, report completeness,
owner uniqueness, no-runtime-input evidence, and runtime-boundary inventory.
Same-named facts owned by another subsystem are ignored for bridge admission.
The bridge rejects duplicate logical lowered members before set-based trace
comparison: node lowerings, implicit edge lowerings, dependency lowerings, and
core object IDs must be unique in the report. The validator-owned predicate
vocabulary must exactly match the current lowering checker slice: unknown
validator-owned predicates are rejected, each required predicate must appear
exactly once, and its unique `input_refs` set must exactly match the concrete
graph ID, lowering entries, preserved refs, produced core objects, owners,
object kinds, and runtime entrypoints needed by the generated obligation.
Missing, duplicated, weakened, padded, or stale validator certificates are
rejected before Maude obligations are emitted.

The lowered-IR validator and generated bridge now admit event, projection, and
diagnostic handoff objects only when they include the explicit entrypoint refs
required by the runtime handoff model: `event`, `event` plus `fact`, and
`rule`. Lowered IR
reports now include
`lowered_ir_validator` facts for covered lowering coverage, lifecycle inputs,
preservation inputs, core object ownership/entrypoints, ownership uniqueness,
deterministic lowering identity, report completeness, no-runtime-input
evidence, and runtime-boundary inventory. Until the remaining coverage exists,
the formal model is an executable acceptance and lowering-preservation proof
for the covered artifact shape, plus a runtime-handoff proof for the covered
emitted entrypoint shape.

## Adequacy Contract

The formal model is only useful if the compiler and runtime agree on what the
modeled facts mean. The adequacy contract is the bridge between the concrete
implementation and the abstract construct graph:

```text
accepted program =
  same source + same package lock produce the same construct graph
  + graph acceptance succeeds
  + every graph predicate is derived by the platform checker
  + lowering uses only declared platform-owned lowering classes
  + runtime executes only ordinary core lifecycle objects
```

Package manifests, package code, providers, and local configuration may supply
inputs to the checker, but they do not get to assert acceptance predicates. A
package cannot make itself accepted by declaring that its ports are compatible,
that resolution is unique, that a capability is authorized, or that its lowering
is deterministic. Those facts are owned by the compiler, checker, package lock,
profile binding, and platform registries.

Package manifests are admitted through the closed
`whipplescript.package_manifest.v0` object shape before they can contribute to a
package lock or package contract. Package locks are admitted through the closed
`whipplescript.package_lock.v0` object shape before they can authorize package
constructs for a source program. Unknown fields, missing required fields, and
schema-invalid scalar, array, or digest fields are rejected rather than ignored
or defaulted, while manifest schema/config fragments remain opaque package data
until their declared validation boundary.

The accepted-program chain is:

```text
admit closed package manifests and produce a package lock
parse/check source under a package lock
emit a normalized package_contract artifact
emit a normalized construct graph artifact
admit reports only after package_contract/contract_registry spine checks and digest links
prove package-contract acceptance/lowering from the emitted package_contract artifact
prove package construct-grammar acceptance/lowering from the emitted package_contract artifact
derive acceptance facts from the artifact and locked registries
check the graph acceptance rules
check the adequacy invariants
lower accepted graph nodes through platform lowering classes
execute only ordinary core facts, events, effects, rules, runs, and projections
```

If the implementation cannot emit the normalized graph for a program, the formal
model has no claim over that program. If it emits the graph but not a derivation
trace for a predicate class, that class can be checked structurally but still
has not proven the full adequacy chain.

## Test Scenario Validation

User-facing `test` declarations are validated against the accepted program, but
they are not part of the production construct graph. A test scenario may
reference graph members, package projections, package resources, fixture
outcomes, and diagnostics; it must not add graph nodes, ports, edges, lowering
objects, or runtime lifecycle semantics to the accepted program.

The validation relation is separate:

```text
accepted workflow graph
package contract registry
fixture outcome registry
test scenario source
  -> accepted test scenario
```

Required checks:

```text
given signal       names a declared signal/source shape and validates payload
given resource     names a declared package resource fixture surface
stub outcome       names a runtime-facing surface and supported fixture outcome
run condition      names a supported deterministic runner stop condition
expect projection  resolves to one package/core projection under imports
expect effect      names a core or package effect surface visible to the test
expect diagnostic  names a stable diagnostic code
risk modifier      is legal for the referenced fixture surface class
```

Forbidden behavior:

```text
test scenario changes accepted program graph identity
test scenario contributes package construct nodes
test scenario emits deployable rules, facts, effects, runs, or lowering objects
test scenario grants provider authority
test scenario uses real providers by default
stub targets metadata-only constructs
package-specific fixture aliases lack platform outcome mapping
```

`whip test` may build an internal scenario execution graph for the isolated
fixture runtime, but that graph is a test-run artifact. It is not admitted as
the compiled workflow's accepted construct graph and must not affect `check`,
`compile`, `run`, `worker`, replay, or deployment identities.

## Fact Derivation Accountability

Every acceptance fact needs a single owner:

```text
nodeFamilyAccepted        platform construct registry
nodeLoweringAccepted      platform construct/lowering registry
nodeShapeUnique           parser shape registry and source parse
nodeFactsConsistent       normalized artifact validator
nodeNeeds                 package construct contract under the package lock
nodeProduces              package construct contract under the package lock
providesPort              package construct contract under the package lock
producedPortAllowed       platform construct/lowering registry
port                      locked contract schema and type checker
resolvesPort              resolver after imports, aliases, and resource binding
uniqueResolution          resolver cardinality check
uniqueResolvedPort        resolver witness for the single chosen provider
resolutionFactsConsistent normalized artifact validator
kindCompatible            construct interface registry
typeAssignable            type checker
phaseCompatible           phase checker
versionCompatible         package lock and compatibility policy
resourceCompatible        resource resolver and resource identity checker
declaredCapability        package/provider contract plus active profile binding
loweredEffectRequires     platform lowering class
singleLoweringOutput      lowering planner
allowedLoweringOutput     platform lowering registry
noHiddenNodeBehavior      platform construct/lowering audit
diagnosticsComplete       checker diagnostic audit
checkerFactsConsistent    normalized artifact validator
```

The checker should reject a graph if any fact has no accountable derivation or
if two subsystems claim incompatible derivations for the same fact.

The normalized artifact validator is responsible for contradiction checks before
it certifies `nodeFactsConsistent`, `resolutionFactsConsistent`, or
`checkerFactsConsistent`. Examples include:

```text
noCapabilityRequirements and nodeRequiresCapability on the same node
nodeProduces0 and providesPort on the same node
uniqueResolution and resolutionAbsent on the same required port
uniqueResolution with more than one resolvesPort fact for the same required port
uniqueResolvedPort that does not match the only visible resolvesPort fact
singleLoweringOutput with multiple lowered output kinds
exactly-one resolution with more than one visible provider
many or named-many resolution without deterministic ordering
named-many resolution without stable resource keys
```

## Ports

Each port has:

```text
kind        Resource, Projection, Signal, SignalSource, Value, EffectHandle,
            TerminalOutput, Capability, ProviderKind, Operation
type        payload, input, or output type
phase       compile, runtime, or both
resource    optional named resource identity
version     locked contract version
cardinality exactly-one, optional-one, named-many, or many
owner       package/library and node id
span        source location for diagnostics
```

Ports are the only way package surfaces compose. A package cannot satisfy a
requirement by side effect, provider availability, ambient credentials, hidden
context injection, or runtime-only data.

## Cardinality

Cardinality is part of the interface contract, not a UI hint:

```text
exactly-one   resolver must find one visible compatible provider
optional-one  resolver may find zero or one visible compatible provider
many          resolver may find zero or more compatible providers; ordering is
              explicit and deterministic
named-many    resolver may find zero or more compatible providers keyed by a
              stable resource/name identity
```

For `exactly-one` and the present-provider branch of `optional-one`, the
normalized graph carries both the cardinality fact and a
`uniqueResolvedPort` witness tying the required port to the single selected
provider. `uniqueResolution` alone is not enough to accept an arbitrary
`resolvesPort` fact.

For `many` and `named-many`, lowering must preserve the chosen aggregation
semantics. If a consuming construct depends on ordering, the ordering must be
declared by the interface, not inherited from filesystem, manifest, hash map, or
import order.

The validator-owned cardinality fact must cite the required port, every
selected edge, and the edge-level aggregate metadata that makes the selection
deterministic. `many` and `named-many` cite `order_index` witnesses; `named-many`
also cites `resource_key` witnesses. The generated checker bridge rejects
schema-valid artifacts with missing, duplicate, or non-contiguous aggregate
ordering, scalar edges that carry aggregate metadata, `many` edges that carry
resource keys, or `named-many` edges without unique resource keys before it
emits Maude facts.

## Namespace And Import Stability

Source forms exported by packages must be namespace-owned or resolved through an
explicit import/alias. Bare forms are allowed only when the active package set
makes them unambiguous.

Reserved bare keywords are stricter. A package construct may use a reserved
keyword only when the platform construct catalog grants that library the keyword
for the exact construct family, scope, and lowering class. The construct graph
validator treats that privilege as part of static acceptance; third-party
packages cannot acquire core-looking verbs by declaring them in their own
manifest.

Adding a package can do only one of three things:

```text
leave existing resolved edges unchanged
make a previously bare source form ambiguous and produce a diagnostic
make newly imported/aliased forms available
```

It must not silently redirect an existing edge to a different provider, change
which construct family parses a source form, or change the resource identity of a
previously resolved edge.

## Edge Acceptance

An edge from required port `R` to provided port `P` is accepted only when all of
these checks hold:

```text
kind compatibility       P is allowed to satisfy R's port kind
type assignability       P.type is assignable to R.type
phase compatibility      runtime-only P cannot satisfy compile-time R
version compatibility    P.version is compatible with R.version
resource compatibility   named-resource ports refer to the same resource
cardinality discipline   exactly-one requirements resolve to one provider
visibility               P is visible through imports, aliases, and scope
```

The graph should report ambiguity instead of choosing among candidates. Adding
an unrelated package may introduce a diagnostic if it exports a conflicting
bare form, but it must not silently change which provider a prior edge uses.

## Diagnostic Provenance

Construct graph diagnostics are part of graph acceptance. A rejected graph must
be explainable in source-facing package vocabulary, not only as failed graph
predicates. The shared diagnostic contract is
[`error-handling.md`](error-handling.md).

For every rejected required port, the validator should emit a resolution trace:

```text
required port ref
source construct ref and source span
expected kind/type/phase/version/resource/cardinality
visible candidate provider ports
rejection reason for each candidate
selected edge refs, if any
final diagnostic code
package-provided labels used for rendering
```

The default user-facing diagnostic should name the construct, resource, and
package concept:

```text
`recall` needs a memory pool named `project_memory`
```

The JSON report and `--explain` output may include graph refs, accepted facts,
candidate lists, and rejected predicate detail:

```text
node memory.recall#n42
required port memory.pool exactly-one resource=project_memory
candidates considered: 0
```

Package contracts may provide labels, examples, docs anchors, and fix templates
for these diagnostics. They may not assert acceptance, fabricate spans, or
override platform rendering. A package construct without enough diagnostic
metadata for its construct family may be rejected by package-contract
validation before it contributes graph nodes.

## Node Acceptance

A node is accepted only when:

```text
its construct family is platform-accepted
its lowering class is platform-accepted for that family
its parser shape is deterministic and unambiguous
every required port is satisfied according to its cardinality
all required capabilities are declared by the package/library contract
all lowered effects expose their required capabilities
all produced ports are allowed for that construct family and lowering class
the declared lowering output kind is allowed and unique
no hidden runtime behavior is declared or implied
no direct rule firing or fact mutation is introduced
```

The "unique lowering output" check is the static form of deterministic
lowering. Given the same source and lockfile, a node must lower to one ordinary
core IR shape.

## Graph Acceptance

A graph is accepted only when every reachable node needed by the program is
accepted. Extra visible nodes from unrelated packages may exist, but they
matter only if they are referenced or if their exported source shape creates an
ambiguity at a composition point.

Accepted graph invariants:

```text
No missing requirements.
No ambiguous exactly-one or optional-one requirements.
No runtime-to-compile dependency.
No type-unsafe value flow.
No resource-identity confusion.
No incompatible contract-version edge.
No hidden capability in lowered effects.
No forbidden lowering output.
No produced port outside the construct family and lowering class contract.
No package-defined control flow, rule scheduling, or terminal status.
No source meaning change from unrelated packages except explicit ambiguity.
No lifecycle profile mismatch between lowering class, emitted core object kind,
  and runtime entrypoint.
```

Accepted program invariants:

```text
Source/lock determinism.
No ambient registry input outside the package lock and active platform version.
Import and namespace stability.
Construct identity stability across check, compile, trace, replay, diagnostics.
Fact derivation accountability for every acceptance predicate.
No authority amplification through package composition.
Resource identity preservation across ports, lowering, runtime, and replay.
Strict compile/runtime phase separation.
Explicit cardinality semantics for exactly-one, optional-one, many, and
named-many ports.
Version and replay pinning for existing runs.
Diagnostic completeness for every rejection path.
Lowering boundary declaration before executable IR is emitted.
Runtime lifecycle boundary declaration before effects or events can run.
```

## Relationship To Lowering

Graph acceptance is necessary but not sufficient for executable semantics.
After graph acceptance, each node lowers through a platform-owned lowering
class. A lowering class is accepted only if it has a platform-declared
lifecycle profile:

```text
accepted construct families
allowed emitted core object kinds
allowed runtime entrypoint classes
authority profile: none, capability-scoped, signal-admission, or projection
output validation requirements
no runtime inputs during lowering
no hidden authority
no package-owned scheduling
no package-owned lifecycle state
no direct fact writes
no direct rule firing
```

Lowering must preserve:

```text
typed input/output information
resource identity
source spans
capability requirements
contract versions
causal provenance
```

The lowering model should prove that accepted graph edges become ordinary core
IR dependencies and that no package can smuggle unmodeled lifecycle behavior
through lowering.

The lowering preservation proof should be stated over normalized graph artifacts,
not over concrete syntax. Its central obligation is:

```text
accepted graph edge in the construct graph
  -> corresponding typed dependency/resource/capability/version relation in core IR
```

The proof should fail if lowering drops a source span, widens a resource,
changes a cardinality, introduces a new capability, changes a contract version,
creates package-owned rule scheduling, or emits a runtime object that cannot be
explained by the accepted graph and the node's accepted lowering-class
lifecycle profile.

## Relationship To Runtime Lifecycle

Runtime lifecycle remains outside the construct graph. The graph may say an
operation produces `EffectHandle<O>`, but runtime must still:

```text
authorize the effect through provider/profile/capability binding
claim and run the effect through the ordinary effect lifecycle
validate provider output against the locked contract
project TerminalOutput<O> only after validation succeeds
record durable events/facts/effects through the event log
replay the same validated outputs under the same contract version
```

Lifecycle modeling should be separate from graph acceptance. Mixing the two
would make packages look more powerful than they are.

The runtime boundary proof should show that packages can request ordinary core
operations but cannot redefine how those operations progress. In particular,
packages may not introduce their own claim protocol, retry state, terminal
outcome lattice, cancellation semantics, replay behavior, or status projection
outside the core lifecycle.

## Current Formal Coverage

The Maude model `models/maude/construct-graph.maude` captures this draft
calculus at a finite abstraction level. It currently checks:

```text
accepted edge derivation from kind/type/phase/version/resource compatibility
explicit port cardinality for exactly-one, optional-one, many, and named-many
per-edge unique-resolution witnesses for exactly-one and optional-one
optional-one zero-provider satisfaction
many-provider deterministic ordering evidence
named-many stable resource key evidence
node acceptance for finite required/produced port lists, with small fixed-arity
  fixtures retained as model examples
graph acceptance from accepted nodes
capability closure for lowered effects
unique allowed lowering output
produced ports allowed by construct family and lowering class
accepted-program adequacy as a distinct layer above graph acceptance
node, resolution, and checker fact consistency as explicit validator evidence
signal and clock sources not directly firing rules
compositionality for unrelated visible package nodes
diagnostic adequacy hooks for rejected graph paths
test scenario validation as a separate non-production relation
negative cases for missing, ambiguous, wrong-type, wrong-phase,
  wrong-version, wrong-resource, hidden-capability, and forbidden-lowering
  graphs
negative cases for many resolution without ordering evidence, named-many
  resolution without resource keys, contradictory unique-resolution facts, and
  missing consistency evidence
negative cases where graph acceptance is not enough because adequacy facts,
  lowering boundaries, lifecycle boundaries, or the accepted graph itself are
  missing
diagnostic negative cases where rejection paths lack checker-owned diagnostic
  evidence or where diagnostic-complete facts are package-supplied
test negative cases where fixture outcomes are missing, unsupported, aliased
  without a platform mapping, or attached to metadata-only constructs
```

The model is intentionally not a parser model and not a package implementation
model. Concrete tracker, repo, memory, schedule, ingress, agent, script, and
notification semantics need separate lowering and lifecycle models.

## Finite Collection Modeling

The construct-graph model now has generic finite-list facts for generated
artifacts:

```text
nodeNeeds(N, RequiredPorts)
nodeProduces(N, ProducedPorts)
graphNodes(G, Nodes)
```

Generated construct-graph checks use those list-shaped facts, while the older
`nodeNeeds0`, `nodeNeeds1`, `graphNodes2`, and similar facts remain as small
hand-written fixtures for edge cases.

Construct lowering and lifecycle checks now accept generated list facts for
required node ports, graph emissions, node-owned core objects,
edge/dependency-owned core objects, lowering-class output profiles, and
node-class output projections. Construct grammar checks accept list-shaped
operation input requirements:

```text
nodeNeeds(N, RequiredPorts)
graphEmits(G, CoreObjects)
nodeCoreObjects(N, CoreObjects)
edgeCoreObjects(N, RequiredPort, CoreObjects)
dependencyCoreObjects(Dependency, CoreObjects)
classCoreOutputs(L, OutputProfile)
nodeClassOutput(N, L, OutputProfile)
operationNeeds(C, InputTypes)
```

The old fixed-arity required-port, node-output, edge/dependency, and operation
input facts remain only for compact hand-written fixtures.

This remains acceptable scaffolding while lowering and lifecycle invariants are
still moving. It keeps searches small, makes expected solution counts stable,
and avoids introducing broader list/set machinery before the modeled obligations
are settled.

The long-term target is to keep cardinality and collection data in emitted
artifacts, not rule shape in the Maude model. As the checker contract
stabilizes, define generic predicates over those collections:

```text
every required port is satisfied
every graph node is accepted
every emitted core object is accounted for
every class output matches an allowed runtime entrypoint
every cardinality contract is preserved by lowering
```

The migration should be driven by generated fixtures from real compiler/checker
artifacts, so Maude checks the same normalized facts the implementation emits.
Do not make this abstraction move merely for elegance: recursive collection
predicates and AC-multiset rewriting can make Maude searches harder to debug.
The useful endpoint is a finite normalized graph model where collection
cardinality is explicit data and the proof obligations are generic.
