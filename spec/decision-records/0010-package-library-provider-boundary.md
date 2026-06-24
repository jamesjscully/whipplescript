# 0010: Package, Library, Provider, and Kernel Boundary

Status: accepted baseline

## Decision

WhippleScript should define extension through a formal four-layer boundary:

```text
kernel    deterministic rule/effect runtime and extension ABI
library   analyzable source surface imported by workflows
provider  executable implementation behind durable effects
package   installable distribution unit that registers libraries and providers
```

This keeps the language open without letting extension code redefine
orchestration semantics.

## Boundary Model

The kernel owns facts, rules, durable effects, event logs, evidence, capability
checks, provider binding, replay, and terminal status. It is the part every
program depends on and every provider must respect.

A library contributes source-level meaning:

```text
types and schemas
declarations and reusable patterns
effect contracts
capability names and requirements
projection definitions
diagnostics and examples
skills and context bundle metadata
```

Libraries are imported by source. They may introduce names and analyzable
contracts, but importing a library does not execute provider code and does not
grant authority.

A provider implements one or more effect contracts:

```text
validate provider config
advertise provider capabilities
claim eligible effects
run external work
emit observations, evidence, artifacts, and terminal outcomes
honor cancellation and lease semantics
```

Providers are configured by the operator or deployment environment. They are
not selected by prompt text, hidden package code, or ambient credentials.

A package is the installable unit:

```text
package manifest
one or more libraries
one or more providers
skills / prompts / examples / docs
script manifests
native binaries or sidecars
schema migrations or local stores, when explicitly declared
```

The older word "plugin" should not name a fourth extension layer. It may remain
temporarily in implementation names for the runtime provider-registration
substrate, but author-facing design should use package, library, and provider.

## Import, Binding, And Authority

`use` imports library surface. It may make declarations, types, effects,
profiles, and provider names type-checkable:

```whip
use std.agent
use std.tracker
```

It must not grant execution authority. A program can mention `provider codex`
or `provider github` only after the relevant package surface is available, but
the runtime may execute those providers only when operator configuration binds
an installed provider endpoint with sufficient capability, profile, credential,
workspace, and artifact policy.

The useful split is:

```text
source import       names and contracts are available
provider binding    a concrete endpoint can run selected effects
capability grant    the endpoint is allowed to perform a class of action
profile policy      the endpoint is allowed to run under an authority bundle
effect requirement  this individual effect asks for concrete capabilities
```

Diagnostics should keep these failure modes distinct:

```text
missing package/library
unknown provider name
provider installed but not configured
provider config invalid
provider lacks required capability
provider refuses requested profile
capability not bound for this program
profile does not allow capability
```

## Provider Registry Contract

Provider catalogs should be registry data, not parser constants. A provider
registration should describe at least:

```text
provider id and aliases
provider kind / family
implemented effect kinds
configuration schema
supported surfaces or transports
supported profile presets
supported effect capabilities
streaming event kinds
cancellation depths
artifact support
health checks
credential requirements
redaction and evidence policy
```

The compiler may use the package registry or lockfile for source diagnostics,
but provider openness should not require editing the parser. Unknown providers
should be diagnosable as missing package, missing registration, or missing
operator binding depending on where resolution fails.

## Effect Contract ABI

The durable effect is the ABI between libraries and providers. An effect
contract should specify:

```text
effect kind
input schema
output / terminal schema
required capability fields
idempotency key rules
dependency and readiness semantics
cancellation semantics
observation event names
evidence and artifact expectations
retry / timeout posture
provider capability requirements
```

Providers may be very different internally, but they must report through the
contract's ordinary lifecycle. They may not mutate facts directly, bypass the
event log, complete work outside the run/effect lifecycle, or silently inject
context or credentials.

Effect contracts should live in an explicit contract registry. The registry is
compile-time and lockfile-pinned data, not executable library code. It should
answer:

```text
which source forms construct this effect
which core IR constructors lowering may emit
which input and output schemas are in force
which capability and provider requirements apply
which typed facts may be projected after validation
which contract version validates existing durable facts
which migrations can reinterpret or retire old contracts
```

This registry is the static meaning of a library import. It lets the checker,
revision analyzer, provider binder, and replay tools agree on what an effect
means without trusting provider implementation details.

The runtime has a separate typed-provider validation boundary. A provider may
finish external work and return raw output, but that output does not become a
typed fact until the runtime validates it against the locked effect contract.
Invalid output should fail the effect with a validation diagnostic. Provider
prevalidation is allowed as an optimization, but the runtime boundary is the
authority that admits data into durable facts.

The tradeoff is intentional:

```text
registry too coarse    compiler cannot audit or revise library effects
registry too broad     the registry becomes a macro system
boundary too lax       providers can smuggle invalid durable state
boundary too strict    useful provider output may require explicit repair effects
```

The preferred point is a small registry of declarative contracts plus a strict
typed-output admission boundary. Repair, coercion, or fallback behavior should
be represented as explicit effects rather than hidden validation side effects.

Implementation status: the compiler exposes `whipplescript.contract_registry.v0`
in JSON `check` and `compile` reports. Built-in surfaces are normalized into
standard `std.*` library/effect contracts. A package import must resolve
through the package lock before package-owned declaration forms are accepted.

The CLI now accepts first-class package manifests with separated `libraries`,
`capabilities`, `providers`, `profiles`, and `bindings`:

```sh
whip package check examples/packages/memory.json
whip package lock --output whip.lock examples/packages/memory.json
whip --json compile --package-lock whip.lock examples/package-memory.whip
```

`package lock` pins each manifest by exact SHA-256. `check`, `compile`, `run`,
`dev`, and `worker` can load the lock. During source analysis, a locked package
registration supplies the library version and explicit effect contracts such as
`memory.query`. During runtime commands, locked manifests are registered into
the store before policy checks so package capabilities, profiles, bindings, and
provider registrations are visible.

The next package-management step is the local project workflow specified in
[`../package-management.md`](../package-management.md): `whip.packages.json`
declares project package intent, `whip package sync` validates that intent and
writes a portable `whip.lock`, and source/runtime commands discover that lock by
default. This is a package-set and lock-management layer, not a registry,
resolver, installer, publisher, or provider-authority mechanism.

`whip package catalog` emits the compiler-owned
`whipplescript.platform_construct_catalog.v0` object directly. `whip package
check --json`, `whip --json check`, and `whip --json compile` now emit a
`whipplescript.package_contract.v0` artifact. The package contract contains
locked manifest summaries, the same platform construct catalog, the
normalized contract registry, diagnostics, and a `package_contract_digest` over
that body. Construct graph artifacts cite the same digest, and `whip
verify-report` rejects stale package contracts or graphs checked against a
different package contract.

This is intentionally still centered on `capability.call`. `call memory.query`
lowers to the core `capability.call` effect and requires the package authority
`memory.recall`. A locked package may also authorize a fixed rule-body
declaration form with `lowering_target: capability_call`; the initial memory
example is `recall from <pool> for <query> as <binding>`, which lowers to the declared
`target_capability` `memory.recall`. A locked package effect contract may
declare `output_schema` and `validation: runtime_boundary`; worker execution
validates the provider payload against that locked schema before deriving
`capability.call.succeeded`. Invalid output fails the run and derives
`capability.call.failed` with a provider-output-validation diagnostic instead
of admitting bad state.

The current schema fragment is deliberately small and deterministic:

```text
string scalar names such as string, int, number, bool, null, json, any
object maps with required declared fields
single-item arrays for homogeneous lists
```

This is enough to enforce the package/provider admission boundary for generic
capability calls. Richer WhippleScript type references and declaration-lowering
extensions are separate work under the controlled library-extension design.

Package acceptance is also conservative in this phase. `whip package check` and
`whip package lock` reject:

```text
duplicate package ids or package names in one loaded package set or lock
duplicate manifest identities for libraries, capabilities, providers, profiles,
  bindings, effect contracts, or constructs
ambiguous libraries that declare both effect_contracts and the legacy effects
  alias
duplicate string-set entries in manifest fields such as required_capabilities,
  provider_kinds, projected_facts, source_forms, or allowed_capabilities
effect contracts whose effect_kind is not capability.call
effect contracts whose id is not a declared package capability
required_capabilities that are not declared package capabilities
providers that implement undeclared capabilities
profiles that allow undeclared capabilities, except wildcard "*"
bindings whose capability is undeclared or whose provider kind does not
  implement that capability
declaration forms that use reserved core keywords
declaration forms whose scope or field kinds are outside the fixed metadata
  vocabulary
metadata_only declaration forms that also declare target_capability
capability_call declaration forms outside rule_body scope
capability_call declaration forms without a declared target_capability and
  matching capability.call effect contract
declaration forms whose lowering_target is outside the accepted set
```

This prevents the lockfile from becoming a promise that the generic runtime
cannot honor. Later controlled-extension classes can relax this only through
platform-owned design work: the compiler/runtime contract, report schemas,
tests, and formal model must be updated before packages can rely on another
lowering target or effect kind. A package-specific proof is not an ordinary
path to new semantics.

Packages may register declaration forms in the contract registry. Metadata-only
forms are reported in `whipplescript.contract_registry.v0` for tooling and
compatibility checks. The narrow executable `capability_call` class grants no
new runtime authority: source files can use an accepted form only with a package
lock that imports the owning package, and the lowered effect remains the core
`capability.call` with the package capability requirement visible to static
analysis and provider policy.

## Standard Package Privilege

First-party standard packages may be bundled, enabled by default, and given
concise syntax when the construct is central to orchestration and needs static
analysis. That privilege should be narrow.

Examples:

```text
std.agent    core syntax for agent/tell, package-owned provider catalog
std.tracker  tracker declarations and ready-work projections
std.memory   memory effects and context bundle construction
std.time     clock sources and recurrence policy over core time/effect primitives
std.ingress  delivery providers for typed signal declarations
```

Third-party packages should primarily extend through ordinary libraries,
effect contracts, provider registrations, and skills. They should not add
arbitrary grammar, control-flow semantics, retry semantics, durable execution
boundaries, or hidden cross-run storage behavior.

If libraries are allowed to add native-feeling declaration forms, they should do
so through the stricter controlled-extension contract in
[`0011-controlled-library-grammar-extensions.md`](0011-controlled-library-grammar-extensions.md),
not through parser plugins or source macros.

## Resolution Pipeline

The intended pipeline is:

```text
1. load package manifests and lockfile
2. import library surfaces requested by source
3. type-check source names, schemas, effects, profiles, and capabilities
4. lower workflows into durable facts/effects/events
5. bind provider endpoints from operator config
6. apply capability, profile, workspace, credential, and capacity policy
7. run providers through the effect lifecycle
8. record observations, evidence, artifacts, and terminal status
```

Steps 1-4 are source analysis. Steps 5-8 are runtime authority and execution.
Mixing these steps is what makes plugin systems confusing and unsafe.

The implementation may still call the runtime registration tables or helper
methods "plugin" while this transition is underway. Those names should be read
as compatibility/implementation debt. They do not imply that package code can
extend parsing, checking, lowering, scheduling, terminal statuses, or provider
authority outside the package/library/provider boundary.

## Manifest Shape

The package manifest should separate surfaces rather than using one broad
"plugin" bucket:

```text
libraries      source modules and exported names
providers      executable endpoints and capability reports
skills         context bundles and activation guidance
scripts        pinned exec capabilities
schemas        persisted provider/package stores
commands       CLI extensions, if allowed
docs/examples  author and operator help
```

This separation lets a package expose `std.agent` authoring syntax without
automatically enabling every installed agent provider, or expose a provider
without requiring its source library to be imported by every workflow.

## Consequences

- Package imports and provider authority are different operations.
- Provider catalogs become extensible without parser edits.
- Standard libraries can remain bundled while still having crisp ownership.
- The checker can produce better errors for missing source surface versus
  missing runtime authority.
- Existing "plugin" docs should be revised toward package/library/provider
  terminology.
- Current hard-coded provider validation should move toward registry-backed
  validation or opaque provider ids with later package/provider diagnostics.

## Risks

- If package imports imply authority, workflows become hard to audit.
- If provider names stay parser constants, first-party packages are not truly
  packages and third-party providers remain second-class.
- If providers can invent lifecycle semantics, replay and status reporting
  fracture.
- If every standard package gets custom grammar, the extension model becomes a
  macro system instead of a durable orchestration language.

## Open Questions

- Should package manifests be locked per project, per deployment, or both?
- Should source require `use std.agent` for `provider codex`, or should unknown
  provider names remain syntactically valid until package resolution?
- Which standard packages are implicitly available in v0?
- How should CLI extensions be sandboxed and named?
- What is the minimum stable provider capability schema for v0?
