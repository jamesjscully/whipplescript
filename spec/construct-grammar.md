# Construct Grammar And Library Interfaces

Status: draft baseline

This document sketches the library-extension model WhippleScript should aim
for. It is not locked design. The purpose is to define the abstraction layer
between "packages cannot extend syntax" and "packages can be full parser
plugins."

The target is controlled declarative lowering:

```text
package declares construct instances
core owns construct families and parsing shapes
checker validates typed interfaces between constructs
lowering emits ordinary core IR
runtime executes only ordinary facts, events, effects, rules, runs, and evidence
```

Another way to say this: libraries contribute a typed construct graph. Core
owns the allowed node kinds and edge kinds. The compiler validates the graph
and lowers it into ordinary WhippleScript IR.

The source-facing construct grammar should be read together with
[`construct-graph-calculus.md`](construct-graph-calculus.md). This document
describes the package authoring surface and construct families; the calculus
document defines the normalized graph objects and acceptance invariants that
make package interoperability statically checkable.

Diagnostic behavior should be read together with
[`error-handling.md`](error-handling.md): package-owned constructs provide
labels, examples, and fix templates, but the platform owns diagnostic codes,
source spans, rendering, and construct graph resolution traces.

## Registry Layers

The construct grammar does not replace the runtime provider registry. The current
runtime already has a legacy registration substrate: manifests register
capability schemas, effect providers, profiles, and capability bindings in the
store. First-class package manifests feed that same runtime machinery today.

The intended split is:

```text
construct/library registry
  compile-time source meaning
  construct instances
  provided/required interfaces
  effect contracts
  lowering classes
  package-lock-pinned source contracts

runtime provider registry
  runtime authority and execution
  capability schemas
  effect providers
  profiles
  capability bindings
  provider config and implementation metadata

package
  distribution unit that may contain both registry layers
```

The compile-time registry answers "what does this source mean and what may it
lower to?" The runtime provider registry answers "is this lowered effect allowed
to run, and which provider can run it?" Importing a library or accepting a
construct must not imply runtime authority.

## Core Design

The library system has closed and open parts.

Closed by core/platform:

```text
construct families
composition points
field kinds
lowering classes
runtime lifecycle semantics
rule scheduling semantics
terminal status taxonomy
capability/profile enforcement
event-log and replay invariants
```

Open to packages:

```text
construct instances
keywords and names
provided interfaces
required interfaces
effect contracts
provider bindings
schemas
diagnostic labels
examples and fixtures
```

Libraries may declare new domain vocabulary over core orchestration
primitives. They may not replace the orchestration model.

Diagnostic labels are part of the package contract, not an escape hatch.
Packages may describe their constructs, fields, resources, ports,
capabilities, provider features, examples, and common fix templates. They may
not provide arbitrary parser code, custom diagnostic renderers, or opaque
compiler errors. The platform renders package diagnostics through the shared
diagnostic model.

That means a package can say:

```text
this is a resource declaration
this is a projection usable in a `when`
this is an operation that enqueues a durable effect
this is a signal source
this is a provider binding
this is a typed output projection
```

It cannot say:

```text
here is arbitrary grammar
here is parser code
here is custom rule scheduling
here is a new run status
here is hidden retry behavior
here is direct fact mutation
here is provider execution during checking or lowering
```

## Problem

WhippleScript needs libraries that can feel native:

```whip
use std.tracker
use std.memory
use std.files
use std.time
use std.agent

tracker backlog { provider github }
memory pool project_memory { provider builtin }
file store project_files {
  provider local
  root "./"
  allow read ["docs/**"]
}

signal triage.tick { scheduled_at time }

source clock as morning_triage {
  every weekday at 09:00
  timezone "America/New_York"
  missed coalesce

  observe as tick
  emit triage.tick {
    scheduled_at tick.scheduled_at
  }
}

rule triage
  when backlog has ready issue as issue
=> {
  recall from project_memory for issue as context

  after context succeeds as memory {
    read markdown from project_files at "docs/issue-template.md" as template

    after template succeeds as issue_template {
      tell coder
        with context memory
        with context issue_template
        with access to project_memory {
          recall for issue
          learn for issue
        }
        with access to project_files {
          read ["docs/**"]
        }
      "Work the issue."
    }
  }
}
```

But package authors should not get arbitrary parser extension, arbitrary AST
transforms, hidden runtime behavior, or package-specific control flow. A user
should still be writing one language, with familiar WhippleScript shapes and
static analysis that composes across packages.

The missing layer is a construct grammar: a core-owned grammar of authoring
construct families plus typed interfaces between construct instances.

## Shared Authoring Abstractions

Package syntax should compose through a small set of author-facing abstractions.
These are not separate parser plugins; they are recurring uses of the construct
families below.

```text
declared resource
  named durable or external resource surface
  resource <name> { provider <provider>; policy clauses }
  examples: tracker, memory pool, file store, channel, lease, ledger, counter

source declaration
  named outside-observation source with an observation binding and explicit
  signal emission mapping
  examples: source clock, source http, source file, source interaction

effect operation
  durable rule-body operation that returns an effect handle and uses ordinary
  after-branch lifecycle

projection
  typed read-side view usable in rule conditions without mutating state

turn access grant
  declarative `tell` clause that narrows one agent turn's access to a declared
  resource; grants are not executable statements

provider capability report
  machine-readable feature and authority metadata used by static checks,
  runtime dispatch, status, and evidence

typed signal admission
  core-owned validation boundary where an outside or cross-instance observation
  becomes a durable typed signal fact

test scenario
  core-owned non-runtime source artifact that validates `given`, `stub`, `run`,
  and `expect` clauses against workflows and package fixture outcomes

tooling metadata
  declarative package metadata for diagnostics, editor completions/hovers,
  code actions, and deterministic fixture outcomes; it never changes runtime
  semantics
```

The common declaration style is a block-internal `provider` clause, not bespoke
header syntax. For example, packages should prefer:

```whip
channel release_room {
  provider local
  destination "release"
}
```

over package-specific header syntax.

Runtime authority is never implied by syntax acceptance. A resource declaration,
source declaration, or package import does not grant credentials, filesystem
access, model memory, process execution, messaging access, or provider-native
tools. Effective authority is the intersection of package contract, resource
policy, provider capability report, runtime capability binding, and profile
policy.

## Construct Families

A construct family is a core-defined syntactic and semantic shape. Packages
instantiate families; they do not invent families. The exact list should stay
small. Adding a construct family is a platform change requiring specs,
validator changes, tests, and model coverage.

The closed set of platform construct **families** is defined by
`PLATFORM_CONSTRUCT_CATALOG`. The author-facing **shapes** below are uses of
those families. The `Lowering Classes` column names target classes; the full
per-class contract (authorability, authority profile, lifecycle profile, emitted
core object) is normative in
[`construct-lowering-preservation.md`](construct-lowering-preservation.md#lowering-class-catalog).
Do not duplicate that catalog here.

| Author-facing shape | Source Shape | Platform family | Lowering Classes | Example |
| --- | --- | --- | --- | --- |
| resource declaration | `<noun> [qualifier] <name> { <clauses> }` | `declaration_block` | `metadata_only` | `memory pool project_memory { ... }` |
| signal declaration | `signal <dotted.name> { <fields> }` | `signal_source` (current; target `signal` in P1b) | `metadata` | `signal triage.tick { scheduled_at time }` |
| source declaration | `source <provider> as <name> { <clauses> }` | `source_declaration` (target) | `signal_source`, `clock_source` | `source clock as daily_triage { ... }` |
| rule projection | `when <source> has <projection> as <binding>` | `projection_read` | `metadata`, `projection_view` | `when backlog has ready issue as issue` |
| effect operation | core-owned verb/preposition slots ending in `as <binding>` | `effect_operation` | `capability_call`, `typed_effect_call`, `resource_effect`, `core_effect`, `signal_emit` | `recall from project_memory for issue as context` |
| signal emit | `emit signal <signal> to <target> { <fields> } as <binding>` | `effect_operation` | `signal_emit` | `emit signal deploy.finished to peer.id { ... } as sent` |
| resource operation | core-shaped resource verbs such as `acquire`, `release`, `append`, `consume` | `effect_operation` | `resource_effect` | `claim issue as active_claim` |
| turn-access grant | `with access to <resource> { <grant clauses> }` or `with access to { <resource> { <grant clauses> } ... }` | (not a family) metadata on the `tell`/`invoke` effect | none — see below | `with access to project_memory { recall for issue }` |
| provider declaration | provider/profile/binding clauses or blocks | `declaration_block` | `metadata_only` | `provider github` |
| rule | `rule <name> when ... => { ... }` | `rule` | `rule_template` | `rule triage when ... => { ... }` |
| assertion | top-level `assert <predicate>` | `assertion` | `assertion_check` | `assert facts count Issue is 1` |
| policy clause | bounded named clauses inside declaration blocks | (consumed by parent) | consumed by parent construct | `search hybrid { bm25 0.25 }` |
| test scenario | `test <name> { given ... stub ... run ... expect ... }` | `test_scenario` (core-owned, non-runtime) | none; test-only validation | `test "failed CI gets triaged" { ... }` |

Turn-access grants are not a lowering class. A `with access to <resource> { … }`
clause, or grouped `with access to { <resource> { … } ... }` shorthand, is
authority-narrowing metadata on the agent-turn or workflow-invoke effect: it is
validated as required ports (`Resource`, per-granted `Operation`, `Capability`)
on the effect node and lowered as bounded sub-authority fields on the effect.
In-turn tool invocations are recorded as evidence, not durable child
effects. Resource declarations are likewise not their own lowering class; they
lower through `metadata_only` and the resource surface comes from the runtime
provider registry.

The live executable baseline supports first-class package manifests, the two
package-authorable lowering classes (`metadata_only`, `capability_call`), and a
set of compiler-owned classes (`metadata`, `typed_effect_call`,
`resource_effect`, `core_effect`, `signal_emit`, `signal_source`, `clock_source`,
`rule_template`, `projection_view`, `assertion_check`). The `source_declaration`
family and the `signal`/`source`/`clock` restructure are decided but not yet
implemented; see the catalog's restructure note.

`test_scenario` is core-owned and not package-authorable. It parses and
typechecks against accepted workflow/package surfaces, but it does not emit
deployable workflow IR, construct graph runtime nodes, rules, facts, effects, or
provider runs. It is a source artifact for `whip check`, `whip test`, and LSP
tooling.

## Construct Instances

A construct instance is a package-declared use of a family.

Example:

```text
id: memory.recall
family: effect_operation
keyword: recall
shape: recall from <pool: MemoryPoolRef> for <query: Expr> as <binding: EffectHandle>
lowering_class: capability_call
target_capability: memory.recall
```

File I/O is another example:

```text
id: files.read
family: effect_operation
keyword: read
shape: read <format> from <store: FileStoreRef> at <path: Expr> as <binding: EffectHandle>
lowering_class: typed_effect_call
target_capability: files.read
```

The package chooses names, field types, clauses, required/provided interfaces,
and an accepted lowering class. The core parser still owns the concrete syntax
for identifiers, expressions, blocks, bindings, clauses, and paths.

## Test Scenarios And Fixture Metadata

Workflow tests are source artifacts over accepted workflows. They are validated
by the compiler and run by `whip test`, but they do not participate in the
ordinary accepted program graph.

```text
test scenario source
  -> parse/typecheck test clauses
  -> validate referenced workflow and package surfaces
  -> validate package fixture outcomes
  -> run isolated deterministic fixture runtime
  -> assert projections/effects/diagnostics
```

Package fixture metadata is part of the package contract:

```text
surface id
surface class
fixture outcome names
deterministic fixture response shapes
diagnostic codes for failure/denial outcomes
projection changes exposed to expectations
terminal/retryable/branchable classification
```

This metadata is tooling data. It never grants authority, changes source
meaning, introduces a provider boundary, or defines new lifecycle behavior.
Packages may add domain aliases for fixture outcomes only when each alias maps
to a platform-owned risk utility class.

## Interface Vocabulary

Every construct instance declares what it provides and requires. The interface
vocabulary must be finite and typed so package surfaces compose by contract
rather than convention.

| Interface | Meaning | Phase |
| --- | --- | --- |
| `Name` | exported source name or namespace | compile |
| `Type` | WhippleScript type or package schema reference | compile |
| `Schema` | JSON/schema fragment for boundary validation | compile/runtime |
| `Resource` | named durable or external resource surface | compile/runtime |
| `Projection<T>` | source condition/view that binds `T` | compile/runtime |
| `Signal<T>` | declared outside-input schema with payload type `T` | compile/runtime |
| `SignalSource<T>` | configured source that can admit facts matching `Signal<T>` | runtime |
| `EffectContract<I, O>` | durable effect input/output contract | compile/runtime |
| `Operation<I, O>` | source operation that produces an effect handle | compile |
| `Capability` | authority required to perform an effect | compile/runtime |
| `ProviderKind` | provider family that can implement contracts | compile/runtime |
| `Profile` | capability/policy bundle | runtime |
| `Binding` | operator or package binding from capability to provider | runtime |
| `EffectHandle<O>` | rule-body handle for terminal dependency branches | compile/runtime |
| `TerminalOutput<O>` | value available in `after handle succeeds as x` | compile/runtime |
| `ContextArtifact` | auditable bounded context material | runtime |
| `Diagnostic` | source or runtime diagnostic metadata | compile/runtime |

Required interfaces can refer to provided interfaces from the same package,
another imported package, standard library, or core. Package locks pin the
accepted interface graph.

## Typed Interface Graph

The checker should normalize package surfaces into a typed interface graph.
Construct instances are nodes. Provided and required interfaces are typed ports
on those nodes. Composition is legal only when each required port is satisfied
by compatible provided ports according to its declared cardinality.

An interface port has these fields:

```text
interface kind     Resource, Projection, Signal, SignalSource, Operation, etc.
name or resource   stable source identity when the interface is named
type parameters    payload/input/output types such as Issue or MemoryContext
phase              compile, runtime, or compile/runtime
cardinality        exactly-one, optional-one, many, or named-many
owner              package/library id and construct instance id
contract version   locked package contract version
source span        for diagnostics
```

Identity matters. `github_backlog: Resource<Tracker<Issue>>` and
`linear_backlog: Resource<Tracker<Issue>>` are distinct resources even when
they share a family and payload type. A rule projection such as:

```whip
when github_backlog has ready issue as issue
```

does not ask for any `Projection<Issue>` in the program. It asks for the
projection interface attached to the named `github_backlog` resource. Bare
forms are allowed only when scope resolution leaves one compatible candidate.
Otherwise the checker must require a namespace, alias, or resource-qualified
form.

Type compatibility should be structural where WhippleScript types are
structural and nominal where a package declares a nominal schema. The important
point is that compatibility is checked by the core type system, not by package
convention. A port requiring `Issue` may be satisfied by a value whose type is
assignable to `Issue`; it may not be satisfied by an unrelated
`MemoryContext`, raw JSON, provider output, or an unvalidated schema claim.

The useful interface constructors are:

```text
Resource<Name, T>
Projection<ResourceName, T>
Signal<T>
SignalSource<T>
Operation<I, O>
Capability<C>
ProviderKind<P>
EffectHandle<O>
TerminalOutput<O>
Value<T>
```

`Value<T>` is the ordinary expression/binding side of the graph. It is how
typed data from one package feeds another package's operation. For example,
`learn from turn into project_memory for issue { ... }` consumes
`Value<AgentTurn>` for provenance and `Value<Issue>` for subject, while
`tell coder with context memory ...` consumes an explicit
`Value<MemoryContext>` from a successful memory recall.
Likewise, `read markdown from project_files at path as doc` consumes
`Resource<FileStore>` and `Value<string>` for the path, then provides an
effect handle whose terminal output is a typed document value or artifact.

The checker should reject at least these graph errors:

```text
missing required interface
ambiguous provider for an exactly-one requirement
wrong resource identity even if the payload type matches
wrong payload type or missing assignability proof
runtime-only provider output used before runtime-boundary validation
unversioned or unlocked interface used by source lowering
hidden capability not listed in the operation/effect contract
signal source that tries to fire rules directly instead of admitting signal facts
package construct that provides an interface outside its accepted family/class
```

This graph is not a new runtime substrate. It is the static explanation for how
package syntax composes before lowering. Lowering still emits ordinary core IR,
and runtime authority still comes from the package/provider registry.

## Composition Rules

Composition rules define which provided interfaces may satisfy which
composition points. This is the interoperability layer.

Baseline rules:

```text
declaration_block may provide Resource R
rule_projection requires Projection<T> and provides a rule binding of T
effect_operation requires input I and Capability C, then provides EffectHandle<O>
after succeeds consumes EffectHandle<O> and provides TerminalOutput<O>
signal_emit_operation requires Signal<T>, field values assignable to T, and delivery authority
source_declaration requires ProviderKind<P>, observation mapping, and Signal<T>;
  provides a runtime-admitted SignalSource<T>
provider binding requires Capability C and ProviderKind P that implements C
profile grants may satisfy required Capability C when policy allows C
```

More precise composition points:

```text
Resource-qualified projection:
  Resource<R, SourceT> + Projection<R, T> -> rule binding Value<T>

After-success branch:
  EffectHandle<O> + validated terminal success -> TerminalOutput<O> + Value<O>

Operation input:
  Operation<I, O> + Value<I> + Capability<C> -> EffectHandle<O>

Signal condition:
  Signal<T> + admitted durable signal payload assignable to T -> rule binding Value<T>

Signal source:
  SignalSource<T> -> admitted durable signal facts only; never direct rule fire

Provider binding:
  Capability<C> + ProviderKind<P implements C> + operator binding -> runtime authority
```

Important consequences:

```text
an effect operation cannot hide an unlisted capability
a projection cannot bind an unknown or untyped output
a clock source cannot fire rules directly; it can only admit signal facts
a memory operation cannot inject context into an agent turn unless the source
  explicitly composes its output into that turn
a provider cannot project typed facts until runtime output validation succeeds
```

Composition points are core-owned. A package cannot add a new kind of hole in
the language where arbitrary syntax or behavior can be inserted.

## Missing Checks To Add

The current implementation only checks the first slice of this model:
package-lock-pinned `capability.call` contracts, the fixed memory `recall`
operation, package input/output schema-fragment admission for the supported
`Value<T>` subset, top-level required input-field coverage for
`capability_call` construct registrations, package construct/effect vocabulary
validation against the platform construct catalog during report admission, and
package construct target/effect required-capability consistency plus
package-construct keyword uniqueness plus platform-owned reserved-keyword
privileges during report admission, and
runtime-boundary output validation. The richer interface graph requires
additional checks before tracker, schedule, ingress, repo, memory, agent, and
notification packages can all compose safely.

Needed source/package checks:

```text
normalize provided/required interfaces into graph ports
resolve resource-qualified projections by resource identity
reject ambiguous bare keywords, projections, operations, and provider names
check source operation input expressions against `Value<T>` requirements
  (the package schema subset and top-level construct/input-field coverage are
  admitted at lock time; expression typing for arbitrary package operation
  fields still needs checker support)
check terminal-output types before feeding another package's operation
check source declarations against declared `Signal<T>` schemas
check that signal and clock sources lower only to admission templates and
  ordinary metadata, not concrete signal/event occurrences
check named-many resources by source name instead of by family alone
preserve owner package and locked contract version on every graph edge
surface missing library, missing interface, missing provider, and missing
  runtime authority as distinct diagnostics
```

Needed runtime checks:

```text
provider raw output validates before `TerminalOutput<O>` is projected
provider bindings authorize all required capabilities on lowered effects
source providers admit durable signal facts rather than invoking rules
replay reconstructs the same typed graph outputs from validated durable records
```

## Lowering Classes

A lowering class is a platform-owned declarative translation from a construct
family instance to core IR. Every lowering class must define:

```text
accepted construct families
whether packages may target it
preconditions
required fields and interfaces
allowed generated IR constructors
declared lifecycle profiles
declared authority profile
capability and provider requirements
input materialization
terminal output behavior
dependency behavior
diagnostic obligations
revision compatibility rules
forbidden behavior
model/property tests
```

### `capability_call`

Current accepted executable lowering class.

Accepted families:

```text
effect_operation
```

Preconditions:

```text
construct has a target_capability
target_capability is declared by the package
target_capability has a matching EffectContract<I, O>
effect contract lowers to core effect kind capability.call
required capabilities are declared by the package
provider/profile/binding policy can authorize the capability at runtime
if O is consumed, output_schema is declared and runtime-boundary validated
```

Generated IR:

```text
effect kind: capability.call
target: target_capability
input: materialized source fields
required_capabilities: [target_capability] plus declared requirements
binding: EffectHandle<O>
construct-use metadata: construct id, keyword, family, lowering class, target_capability
```

Terminal behavior:

```text
provider success + valid output -> capability.call.succeeded
provider success + invalid output -> capability.call.failed with validation diagnostic
provider failure -> capability.call.failed
after handle succeeds as x binds O from validated output
```

Forbidden:

```text
direct fact writes
inline provider calls
custom terminal statuses
hidden retries
hidden context injection
extra capabilities not declared in the contract
```

Model obligations:

```text
accepted construct lowers to an effect graph template; only the kernel commits
that template to a queued effect
lowered effect carries required capability metadata
provider success cannot project output that violates the locked schema
source without an accepted lock entry cannot lower
```

### Other Classes

The normative list of lowering classes — authorability, families, authority
profiles, lifecycle profiles, and emitted core objects — lives in
[`construct-lowering-preservation.md`](construct-lowering-preservation.md#lowering-class-catalog).
Do not duplicate the catalog here.

Live today (12 classes): `metadata`, `metadata_only`, `capability_call`,
`typed_effect_call`, `resource_effect`, `core_effect`, `signal_emit` (renamed
from `event_emit`; landed), `signal_source` (renamed from `event_source`;
landed), `clock_source` (still live as `schedule_emitter`/`timer.wait` pending
P1b), `rule_template`, `projection_view`, and `assertion_check`.
Only `metadata_only` and `capability_call` are package-authorable; the rest are
compiler/platform-owned.

Decided target restructure (not yet implemented): `signal_source` and
`clock_source` move to a new `source_declaration` family for top-level `source …`
blocks; a `signal <name> { … }` declaration lowers through `metadata` as a typed
schema only; and the legacy `event_*` / `schedule_emitter` report vocabulary is
renamed to `signal_*` / `clock_*`. Backwards compatibility is not preserved; the
formal models and report schemas adopt the target names first.

There is no `resource_declaration` class (resource blocks lower through
`metadata_only`) and no `agent_turn_grant` class (turn grants are metadata on the
agent-turn effect).

Adding a genuinely new lowering class needs a separate platform design, validator
implementation, report-schema updates, tests, and model coverage before packages
can use it.

## Syntax Philosophy

Package syntax should be assembled from WhippleScript's existing visual
patterns:

```text
noun name { clauses }
signal dotted.name { fields }
source provider as name { clauses }
verb/preposition slots ending in as binding
when source has projection as binding
after handle succeeds as value { ... }
emit signal dotted.name to target { fields } as binding
with access to resource { grant clauses }
```

Packages should prefer domain words in these positions, not new punctuation or
new control-flow forms. This keeps source readable and lets different packages
compose without creating several dialects.

Standard libraries may be bundled and privileged in naming, but they should not
be semantically magical. Third-party packages should use the same construct
families and lowering classes. If a standard library needs special power, that
is evidence that the construct system is missing a platform concept.

## Manifest Sketch

A future construct manifest could look like this:

```json
{
  "constructs": [
    {
      "id": "memory.recall",
      "family": "effect_operation",
      "keyword": "recall",
      "shape": {
        "form": "recall from <pool> for <query> as <binding>",
        "fields": [
          {"name": "pool", "kind": "resource_ref", "interface": "MemoryPool"},
          {"name": "query", "kind": "expression"},
          {"name": "binding", "kind": "effect_handle"}
        ]
      },
      "requires": [
        {"kind": "Resource", "name": "MemoryPool"},
        {"kind": "Capability", "name": "memory.recall"}
      ],
      "provides": [
        {"kind": "EffectHandle", "output": "MemoryContext"}
      ],
      "lowering": {
        "class": "capability_call",
        "target_capability": "memory.recall"
      }
    }
  ]
}
```

This is a design sketch. The current implementation uses
`constructs` with explicit `construct_family` plus
`lowering_target: "capability_call"` as the first small slice of this model.

## Worked Example: Memory Recall

Package manifest construct:

```text
construct id: memory.recall
family: effect_operation
keyword: recall
requires:
  Resource MemoryPool
  Capability memory.recall
provides:
  EffectHandle<MemoryContext>
lowering:
  class capability_call
  target_capability memory.recall
```

Source:

```whip
use std.memory

memory pool project_memory {
  provider builtin
}

rule implement
  when backlog has ready issue as issue
=> {
  recall from project_memory for issue as context

  after context succeeds as memory {
    tell coder with context memory "..."
  }
}
```

Resolved interfaces:

```text
project_memory provides Resource<MemoryPool>
issue provides input expression value
memory.recall provides EffectContract<Issue, MemoryContext>
recall consumes Resource<MemoryPool>, input expression, Capability memory.recall
recall provides EffectHandle<MemoryContext>
after context succeeds consumes EffectHandle<MemoryContext>
memory binding has TerminalOutput<MemoryContext>
tell consumes explicit context value supplied by the author
```

Lowered effect sketch:

```text
kind: capability.call
target: memory.recall
input:
  pool: project_memory
  query: <materialized issue expression>
required_capabilities:
  memory.recall
binding:
  context: EffectHandle<MemoryContext>
metadata:
  construct: memory.recall
  source_form: recall
```

Runtime:

```text
provider returns raw value
runtime validates raw value against locked MemoryContext schema
valid value derives capability.call.succeeded
after context succeeds as memory binds validated MemoryContext
invalid value derives capability.call.failed and no MemoryContext binding
```

## Worked Example: Tracker

Tracker can be described as declaration, projection, and operation instances:

```text
tracker declaration:
  family: declaration_block
  keyword: tracker
  provides: Resource<Tracker>
  requires: tracker provider binding

ready issue projection:
  family: rule_projection
  shape: when <tracker> has ready issue as <binding>
  requires: Resource<Tracker>
  provides: Projection<Issue>

claim operation:
  family: resource_operation or effect_operation
  shape: claim <issue> as <claim_handle>
  requires: Issue binding, tracker.claim capability
  provides: EffectHandle<TrackerClaim>
```

The tracker package owns issue vocabulary. Core still owns rule matching,
effect handles, after branches, lower-level leases if they are core resources,
and terminal status.

## Worked Example: Schedule

Schedule is the stress test for semantic discipline.

```text
clock source declaration:
  family: source_declaration
  keyword/provider: source clock
  requires: Signal<T>, clock observation schema, explicit mapping, time policy fields
  provides: SignalSource<T>
  lowering_class: clock_source
```

A clock observation must become an ordinary durable signal fact visible to the
runtime and replay tools. The schedule package may describe recurrence policy
over core time primitives; it must not redefine rule scheduling or fire rules
directly.

## Negative Examples

Rejected: custom retry statement.

```whip
retry context up to 3 times with backoff 5m
```

Reason: retry semantics are core/platform-owned. A future policy construct may
declare retry posture, but hidden retries outside the effect lifecycle are not
allowed.

Rejected: hidden memory injection.

```text
memory package automatically attaches search results to every agent turn
```

Reason: memory movement into agent context must be explicit and auditable.

Rejected: schedule directly fires a rule.

```text
source clock as morning_triage { run rule triage every weekday at 09:00 }
```

Reason: clock sources may admit durable signal facts. Rule scheduling remains
core-owned.

Rejected: projection with unknown output type.

```text
when backlog has ready issue as issue
```

Reason: valid only if `ready issue` resolves to `Projection<Issue>` and `Issue`
is declared in the locked interface graph.

Rejected: operation requiring unbound capability.

```whip
recall from cloud_memory for issue as context
```

Reason: the lowered effect must expose a declared capability, and runtime
policy must bind that capability to an authorized provider before execution.

Rejected or conflicted: two imported packages claim the same bare operation
shape.

```text
package A: recall from <pool> for <expr> as <binding>
package B: recall from <source> for <expr> as <binding>
```

Reason: construct matching must be unambiguous. Resolution may require
namespaced forms or an explicit import alias.

## Package Author Contract

A package author declares construct instances in manifest data:

```text
construct id
family
keyword
shape fields
allowed clauses
provided interfaces
required interfaces
lowering class
target contracts/capabilities
version
diagnostic labels
editor metadata
fixture outcome metadata
fixtures
```

The CLI validates:

```text
family is accepted by the platform
keyword and shape are unambiguous
reserved bare keywords are authorized by the platform catalog for this package,
  construct family, scope, and lowering class
fields use core-owned field kinds
required interfaces resolve
provided interfaces do not conflict
lowering class accepts this family
lowering class requirements are satisfied
generated names are stable
capabilities are declared and bound by policy
outputs are validated at the runtime boundary
runtime-facing surfaces declare required fixture outcomes by surface class
fixture outcomes map to deterministic test-provider behavior
lockfile pins the accepted contract
```

Package authors should provide examples and conformance fixtures. They should
not provide parser callbacks, arbitrary lowering code, or hand-written proofs
for ordinary packages.

## Verification

Verification should happen at three levels.

Platform model:

```text
construct families preserve parser determinism
accepted lowering classes preserve core invariants
lowered IR contains only ordinary core constructs
imports do not grant runtime authority
provider success cannot project invalid typed facts
lockfiles make source meaning stable
compile-time construct acceptance does not grant runtime provider authority
lowered effects require runtime package/provider/profile/binding authorization
```

Package acceptance:

```text
manifest schema validation
interface resolution
namespace and ambiguity checks
lowering-class conformance checks
golden lowering fixtures
negative fixtures for missing requirements and conflicts
runtime output validation tests
```

Program checking:

```text
source imports resolve through lockfile
construct uses resolve to accepted instances
composition points match provided/required interfaces
lowered effects expose required capabilities
after bindings and projections type-check
diagnostics retain source spans
```

Most packages should rely on the platform model plus package acceptance. A
package that needs a construct family or lowering class that does not exist is
not accepted until the platform adds and models that class.

## Migration Plan

Current implementation:

```text
first-class package manifest
package lock
contract registry
runtime registration tables and package manifest registration APIs
generic capability.call effect contracts
machine-readable platform construct catalog for accepted families and lowerings
standalone `whip package catalog` command for the current platform catalog
finite construct interface vocabulary in package manifests and reports
package-authorable lowerings distinct from compiler/internal lowerings
package check validation for construct interface vocabulary and lowering obligations
constructs with explicit construct_family
recall -> memory.recall via lowering_target: capability_call
runtime output validation for locked package capability calls
check/compile reports with package_contract, construct_graph, lowered_ir_report,
  platform catalog, and verified artifact envelopes
generated Maude bridges from emitted reports for package contracts, construct
  grammar, construct graph acceptance, lowering preservation, and runtime
  handoff
core-owned construct graph/lowering coverage for signal sources, clock sources,
  rules, assertions, projection reads, effect dependencies, and package
  capability calls
core-owned test_scenario validation and package fixture_outcomes metadata for
  deterministic user workflow tests
artifact admission and schema fixtures that reject stale, spoofed, padded,
  duplicated, and malformed report evidence
```

`whip package catalog` emits the current platform construct catalog directly.
`whip package check --json` embeds the same catalog as
`platform_construct_catalog`, and the package manifest schema is tested against
the same core catalog so transport enums cannot drift silently.
The executable Maude construct-grammar bridge now consumes the emitted
`package_contract` artifact for the current memory package. It proves the
capability-only `memory.recall` construct shape, where the package operation
requires only the target capability interface and represents fields such as
`pool` and `query` as operation fields rather than separate graph interfaces.
The hand-written Maude fixtures still cover interface-qualified capability calls
where an additional resource/projection interface must be provided before the
construct can be accepted.

Current limits:

```text
author-facing package syntax is still narrow: generic `call` plus the explicit
  `recall from <pool> for <query> as <binding>` memory surface
package-authorable lowerings are limited to `metadata_only` and
  `capability_call`
signal-source/clock-source/rule/assertion/projection/turn-grant lowering is
  compiler-owned, not yet package-authorable
standard library surfaces such as tracker, files, schedule, ingress, repo,
  memory, and agent are not yet ported as packages beyond the checked memory
  recall slice
parser internals still need to converge on typed construct uses instead of
  construct-specific parser branches where practical
```

Next implementation steps:

```text
generalize the recall parser path into a reusable package keyword parser for
  effect_operation + capability_call constructs
add golden package fixtures for every newly admitted package surface
add negative package-resolution fixtures for ambiguous keywords, missing
  interfaces, missing package locks, and missing provider authority
keep widening generated report bridges before adding new authorable lowerings
define projection_view for tracker ready-work projections
define signal_source / clock_source for ingress and time
define resource_effect for tracker/coordination operations if needed
port one standard-library surface at a time
```

## Open Questions

- What is the minimal construct family set for `std.tracker`, `std.memory`,
  `std.files`, `std.time`, `std.ingress`, and `std.agent`?
- Should third-party packages be allowed to register unreserved bare keywords,
  or should they use namespaced keywords unless bundled as standard packages?
- How much lowering should be declarative templates versus first-party native
  implementation that emits the same verification artifacts?
- What is the canonical JSON schema for provided/required projections,
  resources, effects, events, and capabilities?
- How should generated names appear in source diagnostics and runtime status?
- How should core-owned `coerce` and `decide` expose typed-effect interfaces to
  packages without making model-provider or schema-coercion backend semantics
  package-authorable?
