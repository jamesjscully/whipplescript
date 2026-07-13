# 0009: Agent Package

Status: proposed

## Decision

`std.agent` is the standard package for agent provider bindings, profile
presets, skills/context injection, and provider health/capability discovery.
It follows the general package/library/provider boundary in
[`0010-package-library-provider-boundary.md`](0010-package-library-provider-boundary.md).

The core keeps the agent execution contract:

```text
agent declarations
AgentRef types
tell effects
agent capacity/readiness
canonical agent.turn.* lifecycle (terminal facts) and in-turn evidence
provider run state, evidence, and cancellation semantics
record-once replay (recorded terminal + evidence; never re-invoke the provider)
capability/profile enforcement
```

Agent-turn replay is record-once: the turn records its terminal outcome and
evidence durably, and replay reads the recorded facts and never re-invokes the
provider. See Determinism And Replay in
[`../admission-and-idempotency.md`](../admission-and-idempotency.md).

`std.agent` supplies the shared agent-provider contract and operator-facing
vocabulary:

```text
profile presets
skill/context bundle resolution
provider feature taxonomy
provider health checks
provider capability reports
```

Concrete first-party harnesses live in provider-specific packages:

```text
std.agent.codex
std.agent.claude
std.agent.fixture
```

The split is intentional. Codex slash commands/plugins/hooks/subagents and
Claude Agent SDK slash commands/skills/hooks/plugins/subagents are not
equivalent semantics. The shared package defines the boundary and report
shape; each provider package maps its native feature surface into that shape.
See [`0015-agent-harness-feature-semantics.md`](0015-agent-harness-feature-semantics.md).

`std.agent.fixture` is a deterministic provider package for exercising the
ordinary agent-turn lifecycle. It is not evidence for a `std.test` package; test
and eval tooling remain outside the standard runtime package ecosystem.

## Construct Graph Contract

`std.agent` is deliberately not the owner of the core agent-turn construct. The
construct graph should model this split:

```text
agent declarations, AgentRef values, tell effects, capacity/readiness, and
agent.turn.* lifecycle are core constructs and core runtime lifecycle.

std.agent contributes provider kinds, profile presets, provider config schemas,
skill/context metadata, and capability reports that those core constructs can
reference.
```

Minimum source-facing surfaces:

```text
agent <name> { provider <provider>; profile <profile>; ... }
tell <agent> ... as <binding>
provider/profile/skill metadata supplied by std.agent packages
```

Graph meaning:

```text
agent declaration
  owner: core syntax, with package-visible provider/profile references
  provides: Value<AgentRef> or Resource<agent-name, Agent>
  requires: ProviderKind<P> and Profile<profile> when validation is strict
  lowering class: rule/projection metadata or no executable output for the
    declaration itself; no provider run is created by the declaration

tell operation
  owner: core effect_operation
  requires: Value<AgentRef>
  requires: Capability<agent.tell> plus source-declared capabilities
  may require: Value<ContextArtifact> or other explicit context values
  provides: EffectHandle<AgentTurn>
  lowering class: core_effect, not a std.agent package lowering

provider/profile/skill catalog entries
  owner: std.agent package/library registry
  family: provider_declaration or metadata declaration
  provides: ProviderKind, Profile, Capability, ContextArtifact metadata
  lowering class: metadata
```

Importing `std.agent` can make first-party provider names, profile presets, and
skill metadata visible to source analysis. It must not authorize provider
execution. Runtime provider authority still flows through provider bindings,
profile allowlists, credentials, workspace policy, and effect capabilities.

This record should therefore avoid treating Codex, Claude, fixture, or
broker integrations as alternate `tell` semantics. They are provider
implementations behind the core `agent.tell` effect and canonical
`agent.turn.*` observations. The construct graph can reference their
`ProviderKind` and capability surfaces, but lowering cannot emit provider runs,
sessions, tool approvals, cancellation acknowledgements, or terminal outcomes.

Skill/context injection must also be explicit in graph terms:

```text
declared skill metadata may provide ContextArtifact metadata
worker dispatch may attach bounded context according to the accepted source and
  package lock
run evidence records the concrete skill ids, versions, and refs
```

A skill does not satisfy a required capability and does not create hidden
`Value<ContextArtifact>` for arbitrary source operations. If a workflow wants
memory, repo briefs, or other context as typed values, those values must come
from accepted package effects and explicit graph edges.

## Existing Shape

The current implementation already has the right split in most places:

```text
source agent -> durable agent.tell effect
worker -> provider selection
provider adapter -> agent.turn.* observations
kernel -> events, facts, runs, evidence, terminal status
```

Source-level `agent` declarations carry:

```text
provider
harness keyword, for advanced named endpoints (soft-deprecated)
profile
capacity
capabilities
skills
```

Provider config carries:

```text
provider_id
provider_kind
surface
credentials_ref
profile_ids
default_model
workspace_policy
timeout_ms
cancellation_depth
artifact_policy
health_checks
extra provider options
```

Native provider lifecycle normalization maps Codex and Claude events into
the canonical rule-matchable lifecycle facts `agent.turn.started`,
`agent.turn.completed`, `agent.turn.failed`, `agent.turn.timed_out`, and
`agent.turn.cancelled`. In-turn `streamed`, `tool_requested`, and
`artifact_captured` observations are normalized into **evidence**, not
rule-matchable facts (Proposal A in
[`../admission-and-idempotency.md`](../admission-and-idempotency.md)).

## Boundary

`std.agent` owns:

```text
provider kinds and adapter surfaces
provider config schemas
provider capability discovery
provider health checks
profile preset definitions
skill resolution and prompt/context bundles
native provider policy translation
adapter-specific evidence summaries
enterprise broker integration
```

It does not own:

```text
agent syntax
tell syntax
AgentRef typing
capacity readiness semantics
effect/run lifecycle
event log semantics
terminal status taxonomy
the capability registry itself
repo resource policy
memory retrieval policy
```

This boundary matters because agent turns are core orchestration primitives.
Provider packages can add execution surfaces, but they cannot redefine what an
agent turn is.

## Provider Binding And Route Vocabulary

The ordinary authoring model should be:

```whip
use std.agent

agent implementer {
  provider codex
  profile "repo-writer"
  capacity 2
  capabilities ["agent.tell", "repo.write"]
  skills ["whipplescript-author"]
}
```

`provider` is the normal source-level route. It should name a logical provider
binding or provider family. Operator config decides the concrete surface,
credentials, model, workspace policy, and artifact policy.

Named endpoints should still use the `provider` clause. If one provider family
needs multiple configured endpoints in the same program, operator/package config
should expose those endpoints as distinct provider bindings:

```whip
agent implementer {
  provider coder
  profile "repo-writer"
}
```

The word "harness" carries three distinct meanings; this record uses each
deliberately and does not conflate them (the harness layer spec keeps the same
disambiguation):

```text
harness layer    the runtime execution layer that runs agent.tell effects
                 (see agent-harness.md)
harness keyword  a soft-deprecated source keyword for advanced named endpoints
native harness   the provider's own agent harness (Codex/Claude)
```

The `harness` keyword is an implementation and compatibility term for advanced
named endpoint routing, not a package authoring keyword. New package specs
should route agents through ordinary `provider`-binding clauses rather than ad
hoc header syntax. References to a provider's own runner are "native harness".

Models must not choose routes from prompt text. Dynamic routing should use
typed source data such as `AgentRef<codex | claude>`, enums, literal
fields, or tracker assignment fields.

## Profiles And Capabilities

Profiles are authority bundles. Capabilities are effect-level requirements.
They should remain separate:

```text
profile: what authority/policy this agent runs under
capability: what this specific effect requires
provider config profile_ids: which profiles this configured endpoint accepts
```

Current string profiles such as `repo-reader` and `repo-writer` are useful, but
the package should eventually publish named presets instead of relying on
informal string convention.

This is the **canonical first-party preset list**. It is pinned here once; the
harness layer and the provider decision records (0016/0017) reference this
list rather than restating their own preset sets:

```text
repo-reader
repo-writer
internet-research
issue-triager
human-review
release-operator
no-repo
```

Each preset's authority expansion is fully explicit — there is no implicit
capability grant (see "Implicit native repo.read" below). Preset expansion must
be visible in diagnostics and status output, using the one severity enum
`error | warning | info | hint`. A blocked effect should be able to explain
whether it failed because:

```text
the source agent lacks the requested capability
the selected provider endpoint does not expose the capability
the endpoint refuses the requested profile
the workspace/artifact policy blocks the run
```

## Skills And Context Bundles

`skills` should be treated as explicit context attachments, not hidden model
behavior. `std.agent` should resolve skill names to prompt/context bundles,
record what was attached, and preserve provenance in run metadata or evidence.

Skill injection should be deterministic from source plus installed packages:

```text
agent declaration names requested skills
package registry resolves skill metadata
worker attaches bounded skill context to the turn
evidence records skill ids, versions, and paths or package ids
```

Skills may describe when the model should use a capability, but they do not
grant authority. Authority still comes from provider config, profiles, and
capabilities.

## Provider Capability Reports

The package should make provider capabilities discoverable as data:

```text
provider kind
surface
protocol version
session identity fields
stream event kinds
tool policy
cancellation depths
artifact manifest support
health checks
auth requirements
supported profiles
supported effect capabilities
```

This supports `whip doctor`, hosted deployment checks, and source validation.
It also keeps provider differences honest: Codex app-server, Claude Agent SDK,
command harnesses, and enterprise brokers do not have identical
streaming, cancellation, artifact, approval, or credential behavior.

## Native Providers

Codex and Claude should remain native/sidecar providers, not plain
`exec` wrappers. They need:

```text
streaming observations
session and turn identities
tool/approval event normalization
artifact and diff capture
cooperative or native cancellation
provider-specific health checks
credential/session lifecycle management
redacted evidence summaries
```

The runtime should continue to normalize provider-specific events into
canonical `agent.turn.*` observations while preserving redacted provider
metadata for debugging.

## Fixture And Test Providers

The fixture provider belongs in `std.agent` because it is the reference provider
for workflow authoring and tests. It should remain deterministic and local, and
it should exercise the same durable lifecycle as real providers.

Test-only provider behavior should be explicit:

```text
forced success
forced failure
forced timeout
forced cancellation
scripted streamed observations
scripted artifact capture
```

This lets users validate policies before running real agents.

## Implementation Audit

The existing codebase already has several of the right primitives:

```text
agent IR with provider, harness, profile, capacity, capabilities, and skills
AgentRef typing and dynamic tell target validation
durable policy blocks for missing profiles, capabilities, and capacity
provider binding config and capability reports
native lifecycle normalization into agent.turn.* facts
run metadata, evidence, cancellation, and status JSON
skill registration, attachment, and evidence tables
```

The `std.agent` split should close these gaps before the package boundary is
considered real:

```text
Provider catalog openness (resolved: OPEN registry):
  the provider registry is OPEN. Core does NOT hard-code provider kinds.
  `provider codex` is a known kind because `use std.agent` / the provider
  package (`std.agent.codex`) registers it; an unimported provider id is an
  opaque id that source validation reports as an unknown provider (missing
  package), not a hard-coded enum violation. Core accepts opaque provider ids
  and defers catalog validation to the package-populated provider registry.
  Source validation currently hard-codes provider and harness kinds; that must
  be replaced by registry-driven validation.

Declared skill attachment:
  agent skills are parsed and persisted, but normal worker dispatch does not yet
  resolve declared skills into turn context. The package must connect source
  declarations, installed skill metadata, bounded context injection, and evidence.

Provider profile allowlists:
  provider config already has profile_ids, but runtime selection must enforce
  them as endpoint allowlists. A non-empty list should reject mismatched effect
  profiles before provider launch.

Profile preset translation:
  the store enforces registered profile policy, while native adapters still carry
  provider-specific string matches for repo-reader and repo-writer. Preset
  definitions and provider policy translation should live behind one package
  contract instead of scattered string convention.

Implicit native repo.read (resolved):
  native provider requests currently add repo.read when no repo or command
  capability is explicit. This implicit grant is removed. Authority must be
  fully explicit: `repo.read` is granted only by a preset whose expansion lists
  it (e.g. `repo-reader`) or by an explicit capability on the agent/effect.
  Preset expansion is visible in diagnostics; no capability is added silently.

Missing profile compatibility:
  some native request builders default a missing profile to repo-reader. Decide
  whether this remains a compatibility fallback or becomes a uniform policy
  block.

Harness spec reconciliation:
  older specs still teach harnesses as the primary concept. They should be
  revised after this decision so provider remains the ordinary authoring
  vocabulary and harness remains advanced endpoint routing.
```

## CLI And Operations

`std.agent` should own or extend operator commands for:

```text
whip providers
whip providers doctor
whip providers capabilities
whip agents
whip agents status
whip agents profiles
whip skills list
whip skills show
```

Existing `doctor` and provider-config validation are the beginning of this
surface. The future CLI should make it easy to answer:

```text
which agents are declared?
which provider endpoint will this agent use?
which profiles and capabilities are available?
which skills will be attached?
can this provider cancel, stream, capture artifacts, and run in this workspace?
```

## Fit With Existing Runtime

No new rule semantics are required. `std.agent` builds on existing effects and
the canonical rule-matchable lifecycle facts:

```text
agent.tell
agent.turn.started
agent.turn.completed
agent.turn.failed
agent.turn.timed_out
agent.turn.cancelled
```

`streamed`, `tool_requested`, and `artifact_captured` are recorded as evidence
on the turn, not as rule-matchable facts (Proposal A).

Provider selection metadata should remain visible on effects, runs, and
evidence. It should include the source provider or harness, selected provider
id, provider kind, surface, and redacted config posture.

## Consequences

- `agent`, `tell`, `AgentRef`, capacity, and `agent.turn.*` stay core.
- `std.agent` becomes the shared contract for profile presets, feature reports,
  skill/context attachment, health checks, and provider capability discovery.
- Codex, Claude, fixture, command, and broker bindings live in provider
  packages that implement the `std.agent` contract.
- `harness` remains supported but should be documented as advanced endpoint
  routing, not the main concept.
- Profiles become package-visible authority presets rather than scattered
  string lore.
- Skill injection becomes explicit, versioned, and auditable.
- Provider capability differences become inspectable instead of tribal
  knowledge.

## Risks

- If `std.agent` tries to own core agent semantics, it will fracture replay and
  readiness.
- If profiles stay only informal strings, policy errors will remain hard to
  explain.
- If skills are injected invisibly, users cannot audit why an agent behaved a
  certain way.
- If all providers are treated as equivalent, cancellation/artifact/recovery
  behavior will be misleading.

## Open Questions

- Should profile presets remain strings, or should source eventually support a
  first-class profile declaration/import syntax?
- (Resolved) The provider registry is OPEN: core accepts opaque provider ids
  and provider validation reports missing packages later. `provider codex` is
  known because `use std.agent` / the provider package registers it; core does
  not hard-code provider kinds. See "Provider catalog openness" above.
- Should `harness` remain source syntax long term, or become provider-config
  only once direct provider bindings are expressive enough?
- What is the exact schema for skill provenance on an agent turn?
- How should enterprise broker providers advertise per-tenant policy without
  leaking provider-specific concepts into source?
