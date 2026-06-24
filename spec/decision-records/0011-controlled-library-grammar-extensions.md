# 0011: Controlled Library Grammar Extensions

Status: accepted baseline; broader platform extension classes remain tentative

## Decision

WhippleScript should support a constrained way for libraries to add
native-feeling declaration forms without becoming parser plugins or a macro
system.

The model is:

```text
library registers a declaration form
core parses it with a fixed extension meta-grammar
checker normalizes it into a typed construct graph
graph acceptance validates ports, resources, cardinality, versions, and
  capabilities
platform-owned lowering classes lower accepted nodes into core IR
runtime sees only ordinary facts, effects, events, providers, and evidence
```

Libraries may extend the language's domain vocabulary. They may not extend
control-flow semantics, rule scheduling, retry semantics, durable execution
boundaries, terminal status semantics, cross-run storage semantics, or runtime
authority. If a package needs behavior outside an accepted extension class, the
platform must either reject it, move the semantic primitive into core, or define
and model a new platform-owned extension class before packages can use it.

## Scope

This design applies to library-owned declaration forms such as:

```whip
tracker backlog {
  provider github
}

memory pool project {
  provider local
  search hybrid
}

source clock as daily_triage {
  every weekday at 09:00
  emit triage.tick { scheduled_at tick.scheduled_at }
}
```

These examples are source vocabulary over platform semantics. For example,
`schedule` may be a concise standard-package declaration for recurrence policy
over core time/event primitives; it is not permission for packages to redefine
rule scheduling or invent a second clock.

It does not allow arbitrary statement, expression, or rule syntax:

```text
no custom if/while/retry/after semantics
no parser callbacks
no arbitrary AST transforms
no provider calls during parsing, checking, or lowering
no hidden context injection
no hidden retries, writes, durable boundaries, or cross-run storage mutation
```

## Extension Shape

A grammar-extending library should register data, not executable parser code.
The extension form should fit one of the platform-owned construct families:

```text
construct family
keyword
name field
body fields
allowed field kinds
required/provided typed interfaces
diagnostic labels
lowering class
version and contract metadata
```

Allowed field kinds should be drawn from a small core set:

```text
Ident
String
Number
Bool
Duration
TypeRef
ProviderRef
CapabilityRef
EventRef
EffectRef
Expression<T>
Predicate<T>
List<T>
Record<T>
Enum literal
```

The core parser owns the syntax for these fields. Libraries choose from the
field kinds and declare how they compose; they do not provide a new grammar
engine.

Current implementation baseline: first-class package manifests and lockfiles
are accepted for library/capability/provider manifests. Old plugin-shaped
manifests are not part of the package contract. Accepted package contracts are
merged into `whipplescript.contract_registry.v0`; generic `call
package.capability` contracts record `capability.call` as the core effect kind
and the target package capability as the required authority. Manifests may also
register construct forms in the contract registry. `metadata_only` forms are
tooling metadata. The accepted executable form class is currently
`lowering_target: capability_call` for rule-body `effect_operation` syntax that
names a declared `target_capability` with a matching `capability.call` effect
contract. The first concrete form is memory
`recall from <pool> for <query> as <binding>`, authorized by the locked memory
package and lowered to `memory.query`.

The compiler records construct-use metadata in `construct_graph` and
`lowered_ir_report` artifacts. Runtime commands enforce
`validation: runtime_boundary` for locked package capability outputs: invalid
provider output fails the effect before `capability.call.succeeded` or any
typed success projection can be derived.

## Lowering Contract

The durable effect/event/fact layer is the ABI. Extended declarations must lower
into ordinary core constructs:

```text
named schemas
fact definitions
event declarations
effect contracts
provider binding requirements
capability requirements
projections
diagnostics metadata
```

Lowering must be deterministic from:

```text
source text
package manifest
package lockfile
operator-independent library metadata
```

Lowering must not depend on:

```text
provider availability
credentials
filesystem state outside locked package inputs
network calls
current time
environment variables
runtime facts or events
```

## Typed Effect Contract Registry

The `coerce` / `decide` audit shows that controlled grammar extension is not
enough by itself. A library-owned declaration often introduces effects whose
success bindings, terminal facts, input materialization, provider execution, and
revision compatibility must be understood by static analysis and runtime.

The library system therefore needs a typed effect contract registry. Each
library effect contract should describe:

```text
effect kind
contract version
source forms that construct it
input materialization schema
success payload type
failure / timeout / cancellation payload types, if specialized
terminal fact names
fact schema_id policy
required capabilities
dependency predicates it supports
provider kind / provider capability requirements
worker execution strategy
evidence redaction policy
revision compatibility summary
```

Core still owns the effect lifecycle. The contract supplies metadata for
analysis and provider dispatch; it does not define new run statuses or new
dependency semantics.

The registry should be explicit and normalized during source analysis:

```text
library metadata + lockfile -> accepted contract registry
source declaration + accepted contract -> lowered core IR
lowered core IR + operator binding -> runnable effects
```

Accepted registry entries should be finite, versioned, and inspectable. They
are not live runtime facts and should not require reinterpreting arbitrary
library code during replay. This is the main tradeoff: the more expressive the
registry becomes, the more native libraries can feel; the smaller and more
declarative it remains, the easier it is to preserve deterministic lowering,
revision checks, and provider interoperability.

## Typed Provider Validation Boundary

Provider execution is external nondeterminism. A provider may claim an effect,
call a model or service, and report raw output, but the runtime must validate
that output against the locked typed effect contract before projecting typed
facts or success bindings.

The boundary should behave like this:

```text
running effect + registered contract + provider raw output
  -> valid typed value -> completed effect + typed fact projection
  -> invalid value     -> failed effect + validation diagnostic
```

Provider-side validation may improve errors or reduce wasted work, but it is
not the authority for durable state. The runtime boundary is. This keeps bad
JSON, stale schemas, provider bugs, model drift, and mismatched contract
versions from silently becoming facts.

Design choices:

```text
raw provider output may be retained in execution metadata
only validated projections enter the fact model
repair/coercion should be another explicit effect, not hidden validation logic
streaming output may report observations, but typed facts require complete
  validated values unless an incremental typed-output contract exists
existing facts remain tied to the contract version that validated them
```

## Concrete Audit: `coerce` / `decide`

Today, typed schema coercion is built into several layers:

```text
parser AST: CoerceDecl, BodyEffectKind::Coerce, BodyEffectKind::Decide
IR: IrCoerce plus IrEffectKind::Coerce
semantic context: coerce output and parameter maps
type checking: arity, argument types, after-success binding type
CLI lowering: coerce input JSON and fixture payloads
worker dispatch: match effect.kind == "coerce"
kernel: run_coerce, evidence, terminal diagnostic, and derived facts
store policy: coerce capability schema, provider binding, profile grants
provider layer: coerce HTTP client and coerce provider capability
```

Those implementation names reflect the current coerce-backed path. The target
conceptual contract is `schema.coerce`: a durable typed coercion effect whose
backend may be coerce or another schema-coercion engine.

This does not mean `coerce` / `decide` must remain core forever. It means a
library-capable replacement must expose the specific interfaces those built-ins
currently rely on:

```text
typed effect declarations with parameters and output types
generic success-binding typing for `after effect succeeds`
generic argument validation against parameter types
generic effect input materialization from source expressions
generic terminal fact projection from provider results
generic provider dispatch for registered effect kinds
generic runtime validation for typed provider output values
analysis-summary entries for generated effect contracts
```

The core obligation is not "understand coerce" and not "own workflow decisions."
The core obligation is to preserve type-safe, replay-safe external
nondeterminism at the schema-coercion boundary.

`decide` is a useful stress test. It is anonymous typed effect syntax inside a
rule body. If rule-body extension remains out of scope, `decide` should either
remain a first-party built-in convenience or lower through a small core generic
typed-effect call form. It should not be the feature that opens arbitrary
statement grammar to libraries.

## Core Versus Library Test

A construct should stay core only when it changes an invariant that cannot be
represented by the library contracts above.

Keep in core:

```text
rule scheduling and fact matching
after dependency predicates and terminal status taxonomy
finite-union case analysis and exhaustiveness
type reflection / schema materialization for source types
JSON validation against WhippleScript IR types
durable effect/run/evidence/artifact lifecycle
provider binding and capability/profile enforcement
replay and revision compatibility checks
```

Move to libraries when expressible as data:

```text
domain declarations
effect kinds and typed input/output contracts
provider-specific request construction
provider capability reports
terminal fact names and payload schemas
profile presets and capability names
prompt/context templates
fixtures and deterministic test payloads
```

For typed schema coercion, this suggests:

```text
core owns typed external effect semantics, coerce/decide syntax, and runtime
  typed-output admission
std.coercion owns schema-coercion backend/toolchain contracts, generated or
  bound artifacts, provider config metadata, fixtures, and evidence shape
coerce backend owns coerce-specific declarations, generated coerce artifacts, clients,
  SAP/parse diagnostics, and coerce-specific evidence fields
```

## Acceptance Contract

A library declaration extension is accepted only if the checker can validate
these contracts.

Interface graph contract:

```text
every construct instance declares provided and required typed interface ports
ports carry kind, resource/name identity, type parameters, phase, owner, and
  locked contract version
resource-qualified composition resolves by resource identity before payload type
bare composition is accepted only when one compatible candidate is in scope
operation inputs consume typed `Value<T>` ports from expressions, projections,
  or validated terminal outputs
event sources produce ordinary durable events/facts/effects, not direct rule
  execution
runtime-only provider output cannot satisfy `Value<T>` or `TerminalOutput<T>`
  until validated at the locked runtime boundary
```

This interface graph is a compile-time and analysis artifact. It is not a
second runtime. Lowering still emits ordinary core IR, and runtime authority
still comes from capability/profile/provider binding.

Parse contract:

```text
extension keyword is namespace-owned or otherwise unambiguous
the declaration form has a deterministic parse
the form cannot capture core syntax accidentally
missing, duplicate, and unknown fields have specified diagnostics
```

Type contract:

```text
all fields have declared types
expressions and predicates type-check in an explicit context
generated schemas are finite, named, and inspectable
generated names are namespace-stable
typed effect contracts declare parameter and result types
```

Lowering contract:

```text
lowering emits only allowed core IR constructors
lowering is deterministic and side-effect-free
generated dependencies and readiness are explicit
generated terminal outcomes use core terminal status taxonomy
diagnostics retain source spans from the extension declaration
generated effect contracts appear in the program analysis summary
```

Authority contract:

```text
importing the library grants no runtime authority
generated effects declare required capabilities
provider references resolve through the provider registry
provider bindings are operator/deployment configuration
credentials are never available during parse, check, or lowering
provider dispatch is selected from registered effect contracts, not prompt text
```

Interoperability contract:

```text
generated facts, effects, events, and projections are visible to ordinary source
other libraries can depend on generated core names
status, evidence, and artifacts use ordinary runtime records
extension-specific metadata is optional debugging context, not hidden state
success bindings and terminal facts type-check through the same schema index as
ordinary source
```

Version contract:

```text
extension form has a version
accepted package versions are locked
breaking changes require an explicit package-lock update and source update
accepted source lowers only under the matching locked contract
```

## Enforcement Model

The extension system should have staged acceptance. A package does not get all
possible extension power merely by being installed.

The stages are platform-owned extension classes, not per-library privilege
levels. A package is accepted only when every declared feature fits an already
accepted class. If a useful package feature does not fit, the response is to
reject it for now or change the platform contract first; it is not to approve
that package with bespoke semantics.

Level 1: contract-only package

```text
register libraries, capabilities, providers, profiles, bindings
register effect contracts for existing core effect kinds
use generic source forms such as call package.capability
declare input_schema, output_schema, validation, and required_capabilities
```

This level is enforced by global validators:

```text
JSON schema and manifest shape checks
namespace, uniqueness, and reference checks
package manifests and locks have unique package/library/effect/construct/provider/profile/binding identities
package_contract artifact digest binds manifest summaries, platform construct
  catalog, and normalized contract registry before construct graph checking
effect_kind is known to core
current generic-call packages use only effect_kind = capability.call
required_capabilities refer to declared package capabilities
providers, profiles, and bindings refer to declared package capabilities
bindings target a provider kind registered for the bound capability
metadata-only declaration forms use accepted scopes, field kinds, and
non-reserved keywords
capability_call declaration forms are rule-body scoped, name a declared
target_capability, and lower only to a matching capability.call contract
validation: runtime_boundary has an output schema
package lock pins the exact manifest hash
compile resolves package imports only through the package lock
worker validates locked output before success facts are projected
kernel/store enforce leases, idempotency, profiles, bindings, and capabilities
```

Library authors interact with this level through a manifest, examples, fixtures,
and ordinary CLI checks:

```sh
whip package check package.json
whip package lock --output whip.lock package.json
whip --json check --package-lock whip.lock workflow.whip
whip dev workflow.whip --package-lock whip.lock
```

No hand-written proof should be required for this level. The author supplies a
machine-readable contract; the CLI produces acceptance or diagnostics.
Conformance tests and golden examples are useful package quality signals, but
the safety proof comes from the global validator and runtime boundary.

Level 2: platform-owned declaration/lowering classes

```text
register a declaration form through the fixed extension meta-grammar
declare fields and field types
declare the allowed executable lowering target
emit only allowed core IR constructors through declarative lowering metadata
register generated schemas, events, effects, projections, and diagnostics
```

This level needs all Level 1 checks plus extension-specific validation:

```text
installed extension keywords are unambiguous
the declaration has deterministic parsing
all fields type-check in an explicit context
generated names are namespace-stable
generated effect contracts and capability requirements are explicit
lowering is deterministic from source plus lockfile metadata
lowering emits no forbidden core constructors
diagnostics retain source spans
revision analysis can compare generated contract versions
```

This still should not require a library author to write a theorem by hand. The
platform models the accepted lowering class once. Package acceptance then
produces a verification artifact for that class: normalized extension metadata,
generated contract registry entries, golden lowering cases, negative fixture
cases, and model-checkable facts over the small extension calculus.

Level 3: platform-owned native implementation

```text
first-party code that cannot yet be expressed declaratively may implement an
accepted platform extension class
the implementation must emit the same normalized contracts and lowered core IR
the implementation must pass the same conformance suite as declarative classes
```

This is a platform implementation technique, not an ordinary package power. It
does not allow third-party packages to run custom lowering, redefine semantics,
or earn authority by being imported.

## Non-Extensible Semantics

The following remain core/platform-owned even when exposed through a standard
package surface:

```text
rule scheduling and fact matching
after predicates and terminal status taxonomy
retry, timeout, cancellation, lease, and idempotency semantics
durable effect/run/evidence/artifact lifecycle
cross-run storage admission into facts
provider binding and capability/profile enforcement
event-log replay and revision compatibility
```

Libraries may name policies or request effects in these areas only through
accepted contracts. They may not implement hidden retries, hidden writes,
inline provider execution, direct fact mutation, or new control flow.

## Modeling Strategy

Most safety properties should be modeled once for all libraries:

```text
imports do not grant authority
locks make accepted contracts stable
accepted declarations lower deterministically
lowered programs contain only core facts, events, effects, rules, and metadata
providers cannot create facts except through validated terminal outcomes
provider success cannot project a value that violates the locked output schema
revision cannot silently reinterpret facts under a different contract version
```

Per-library modeling is not an ordinary acceptance path. A package that only
registers `capability.call` contracts should not need a bespoke model; the
global model plus manifest validation and runtime-output validation are the
contract. If a package cannot be reduced to an already modeled extension class,
the platform must either reject it, move the needed semantic primitive into
core, or define a new platform extension class and model that class globally
before packages can use it.

Standard-package declarations such as `tracker`, `memory`, or `schedule` should
contribute fixtures, golden lowerings, and generated registry facts showing that
they fit the accepted class. They should not receive bespoke semantics merely
because they are first-party.

The proof obligation therefore belongs mostly to the platform:

```text
global proof: each platform extension class preserves core invariants
package proof artifact: this package's metadata satisfies an accepted class
manual proof: reserved for changes to core semantics or extension classes
```

This makes proof part of the platform acceptance story without making ordinary
package authors formal-methods authors.

## Standard And Third-Party Libraries

First-party standard libraries may use concise declaration forms when the domain
is central to orchestration and benefits from static analysis. Examples include
tracker, repo, memory, schedule, ingress, and agent declarations.

Third-party libraries should be able to use the same mechanism, but the default
bar should be stricter:

```text
no arbitrary grammar
declarative extension metadata only
no native lowering code unless explicitly trusted
conformance tests required
capability and provider requirements explicit
```

This keeps standard libraries from becoming magical one-offs while still
protecting the language from extension fragmentation.

## Implementation Baseline

The accepted baseline is intentionally narrower than the full thought
experiment:

```text
first-class package manifests and lockfiles
contract registry in check/compile/package reports
package capability/provider/profile/binding consistency validation
locked package output validation at the runtime boundary
metadata_only declaration forms for tooling
capability_call declaration forms for fixed rule-body syntax
memory recall as the first executable package-owned form
Maude model coverage for metadata declarations and capability_call lowering
JSON schema, docs, examples, and CLI tests for the accepted baseline
```

The baseline keeps hard runtime semantics in core. The executable declaration
class only lowers to `capability.call`; it does not let libraries define custom
control flow, new run states, direct fact writes, or provider execution during
checking.

Work that remains outside this accepted baseline:

```text
additional executable declaration classes
non-capability.call package effect kinds
registry-driven replacement of more hard-coded IrEffectKind knowledge
unifying body AST and line-scanner effect analysis around typed statement nodes
making success-binding payload types data-driven
making worker dispatch consult registered effect providers
including effect contracts in analysis summaries used by revision checks
porting selected built-in capabilities into standard packages
per-library conformance suites for new extension classes
formal proof obligations beyond the existing Maude acceptance model
```

These are not latent powers that packages can opt into today. Each item needs a
separate platform design, implementation, tests, and model coverage before any
package may rely on it. Only after those later boundaries are accepted should a
built-in such as `coerce` be considered for extraction into `std.coercion`; even
then, coerce would remain a backend under the schema-coercion package, not the
semantic package itself.

## Formal Verification Direction

The acceptance contract should become executable. At minimum:

```text
parser ambiguity tests for installed extension sets
golden lowering tests from extension source to core IR
property tests for deterministic lowering
negative tests for hidden authority and forbidden IR constructors
namespace hygiene tests
capability/profile requirement tests
replay equivalence tests over lowered IR
diagnostic source-span tests
lockfile/version mismatch tests
effect-contract compatibility tests in revision analysis
provider-dispatch tests for registered effect kinds
typed provider-output validation tests
contract-version mismatch tests
```

For the most important invariants, use model/property tests over a small
extension calculus:

```text
extension declarations cannot create effects without capability metadata
extension lowering cannot create facts except through core events/effects/rules
two accepted libraries cannot make the same source parse two ways
accepted source plus lockfile always lowers to the same IR
runtime behavior depends on lowered IR, not extension implementation details
lowering requires an accepted locked contract registry entry
provider success cannot create a typed fact whose value violates its contract
revision cannot silently reinterpret active typed facts through changed
library-generated schemas
```

The formal baseline is now split across the construct and lowering models:

```text
models/maude/construct-grammar.maude
models/maude/construct-graph.maude
models/maude/construct-lowering.maude
models/maude/lowering-class-lifecycle.maude
models/maude/lowering-runtime-handoff.maude
```

These models check that accepted source forms resolve through typed ports,
resource identity, cardinality, capability closure, deterministic lowering
classes, core-object ownership, and runtime handoff boundaries. The older
`models/maude/package-contract.maude` remains useful as a small contract-registry
slice, but library specs should now start from the construct graph calculus.
The kernel later commits accepted effect graph templates to queued effects, and
typed provider output is validated against the locked contract only for
kernel-owned provider runs before any typed fact is projected.

The proof target is not "libraries are correct." It is narrower and stronger:
accepted library extensions preserve WhippleScript's core guarantees.

## Consequences

- Libraries can feel native without owning the parser.
- The language can gain durable domain declarations without becoming a macro
  system.
- Standard-library syntax and third-party extension use the same conceptual
  mechanism.
- Implementation must include registry, lockfile, parse, lowering, diagnostics,
  and conformance-test work before porting core features.

## Risks

- If lowering becomes arbitrary executable code, this becomes a macro system.
- If extension syntax can appear in statements or expressions, grammar
  interactions will become hard to reason about.
- If generated IR is not inspectable, interoperability fails.
- If standard libraries bypass the contract, third-party libraries become
  second-class and the boundary loses credibility.

## Open Questions

- Should third-party extensions be restricted to namespaced keywords, or may
  they register bare keywords after dependency resolution?
- Can declarative lowering templates express all standard library needs, or do
  some first-party packages need trusted native lowering?
- What is the exact core IR constructor set exposed to library lowering?
- How should generated names be displayed in diagnostics and status output?
- Which existing built-ins should be tested first against this contract?
