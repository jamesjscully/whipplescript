# `std.agent`: the agent-provider boundary package

Status: concrete package design 2026-07-04 (std-package campaign, Process step 6).
Governed by [`std-package-ecosystem-shape.md`](std-package-ecosystem-shape.md)
(M1-M8, E1-E7 bind this design). Grounded in DR-0009 (agent package), DR-0015
(feature semantics), DR-0017 (Claude provider package), DR-0004 (provider
capability disclosure), and
[`owned-harness-tool-surface.md`](owned-harness-tool-surface.md). This document
also designs the thin provider packages **`std.agent.codex`** and
**`std.agent.claude`**; **`std.agent.pi`** is deferred name-reserved (see
Deferred With Cause).

## Framing

**`std.agent` is a boundary/taxonomy package, not a harness bundle** (ecosystem
shape "Semantic domains vs provider catalogs"). Core owns what an agent turn
*is*; `std.agent` owns the portable vocabulary for *who runs it and what they
can truthfully do*: provider kinds, the profile-preset table, the feature-class
taxonomy, and the capability-report schema its provider sub-packages fill in.

Why it belongs: three provider adapters plus the owned harness already exist
with real behavioral differences (cancellation, sessions, tool policy), and
today those differences live as scattered string convention — hard-matched
preset names in each adapter (kernel/pi_rpc.rs:107-135), hidden default tool
policy inside sidecar JavaScript (scripts/claude-agent-sdk-sidecar.mjs:182-186),
a compiled-in closed provider-kind check (parser/lib.rs:6038-6043), and a
catalog advertising Claude cancellation DR-0017 says not to advertise
(kernel/provider.rs:746). The package's job is to make each of those claims a
centralized, inspectable, truthful piece of data.

## What core keeps (NOT in this package)

Per DR-0009 "Construct Graph Contract", unchanged: `agent` declaration grammar,
`AgentRef` typing, `tell` syntax, capacity/readiness, the `agent.tell` effect
and its `core_effect` lowering, the canonical `agent.turn.started/completed/
failed/timed_out/cancelled` lifecycle facts, record-once replay, cancellation
state, evidence storage, and the IFC engine. The agent declaration remains a
core parser item; per the ecosystem shape's "Construct grammar" decision this
package registers no new construct grammar at all — the easiest M1 case:
authorization data only.

Also NOT here: model-credential providers (`openai`/`anthropic` coerce
credentials, cli/coerce_runtime.rs:44-68) — that plane belongs to the
`std.coercion` concrete design, which owns the shared-codex-credential-layer
reconciliation note. The owned harness's tool surface and governance gating
stay specified in [`owned-harness-tool-surface.md`](owned-harness-tool-surface.md);
this package consumes that surface, it does not respecify it. Skills/context
injection is deferred (below), not silently claimed. std.test remains dropped.

## Surface

### Declarations

The shipped agent declaration (parser/lib.rs:327-348) plus ONE additive field:

```whip
use std.agent
use std.agent.claude

agent reviewer {
  provider claude
  profile repo-reader
  capacity 2
  requires [session.resume]        // NEW: portable feature requirement (DR-0015)
                                   // — illustrative: errors at check time until
                                   // a provider report states the class supported
  skills ["code-reviewer"]         // parses today; attachment deferred (below)
}
```

`requires [<feature.class>...]` takes feature classes from the DR-0015 taxonomy
and is validated against the selected provider's accepted feature report: a
required class the report cannot truthfully state as supported is an `error`;
a `probed`-vs-`compiled` source mismatch is a `warning` (DR-0015 severity
rule). The example above deliberately shows the error's bite: session features
are deferred in this very design (session ids are evidence-only single-turn
today), so `requires [session.resume]` is a check `error` on every shipped
provider — by design, not a gap — until a provider package ships the class as
supported. The soft-deprecated `harness` keyword (parser/lib.rs:252-256) stays
supported as advanced endpoint routing per DR-0009 "Provider Binding And Route
Vocabulary"; it is not part of this package's contract.

### Effect operations (exact kinds)

`std.agent` contributes exactly one effect contract, already attributed to it
today (parser/lib.rs:3380-3389):

```text
effect kind:            agent.tell
required capability id: agent.turn        (shipped id; grandfathered id != kind)
output:                 AgentTurn handle; terminals via agent.turn.* facts
lowering class:         core_effect (core-owned; the contract row is package data)
```

The shipped contract's capability id is `agent.turn` (parser/lib.rs:3385,
merged into required_capabilities at parser/lib.rs:3592, snapshot-asserted at
cli/main.rs:40957). M3's capability-id-equals-effect-kind pattern is the norm
for new contracts; `agent.turn` is documented here as the one grandfathered
exception — this package adopts the shipped id verbatim rather than renaming
it, so the M3 lock-time check (contract required_capabilities ⊆ manifest
capabilities[]) passes against the shipped contract with zero code churn.

No renames: `agent.tell`, `agent.turn`, and `agent.turn.*` are not in the M4
set. The
turn-grant vocabulary the owned harness enforces (`tracker.*` ops, file ops,
`command.run`; cli/harness_tools.rs:2062-2160) is M3's third plane
(turn-grants ⊆ store-bound) — enforcement data consumed here, owned by its
home specs (tracker/files/script designs).

### Profile presets: the 7-row table as package data

The canonical preset list pinned in DR-0009 "Profiles And Capabilities":

```text
repo-reader | repo-writer | internet-research | issue-triager |
human-review | release-operator | no-repo
```

becomes a TABLE in the `std.agent` manifest's `profiles` section: one row per
preset with a fully explicit authority expansion — owned-harness tool policy
(the boolean vector at cli/harness_tools.rs:408-470), native tool-policy
translation hints per provider class, and the capability list the preset
grants. Adapters and the sidecar receive computed policy from this table; the
sidecar's compiled-in `allowedTools`/`disallowedTools` defaults
(sidecar.mjs:182-186) are deleted — the sidecar becomes policy-free transport.
Two shipped dishonesty holes this closes: unknown profile falls through to
`permissive()` in the owned harness (cli/harness_tools.rs:465-468) — becomes
fail-closed (recoverable policy block); and `issue-triager` is canonical but
unmapped there (absent from cli/harness_tools.rs:430-464), so it silently gets
permissive today.

### Capability reports (minimal DR-0015 surface)

Schema `whipplescript.agent_feature_report.v0`, published per provider kind.
v1 is minimal: the DR-0015 feature-class list verbatim, each entry carrying

```text
class:    e.g. turn.cancel, session.resume, skill.attach
support:  native | emulated | request_only | unsupported | unknown   (DR-0004 + DR-0017 vocab)
source:   compiled | probed        (probed-vs-compiled honesty)
native_name + dispatch mechanism   (when support != unsupported/unknown)
```

Compiled claims ship in the provider package manifest; `whip doctor
--providers` (main.rs:555-647) renders them and may upgrade entries to
`source: probed` from deterministic local probes. The honesty rule (mirroring
M8's audit): a report may not state `native` for behavior never live-validated
— exactly the shipped `turn.cancel` bug: the compiled catalog advertises
Claude cancellation as `cooperative_request` (kernel/provider.rs:746) while
DR-0017 "Cancellation should remain conservative" mandates
`unsupported | request_only | unknown` until live validation. The fix spans
two planes: the compiled catalog drops the advertised depth to
`CancellationDepth::None` in the shipped enum (slice 2), and Claude's feature
REPORT — where the DR-0017 triple actually lives — states `turn.cancel:
unknown` when that report ships (slice 7, over slice 5's schema).

### Open provider registry

DR-0009 resolved the registry OPEN ("Provider catalog openness"); the code is
still closed: `is_supported_harness_kind` hardcodes the kind set
(parser/lib.rs:6038-6043) and the agent `provider` clause checks against it
(parser/lib.rs:6108-6120). This design replaces both with registry-driven
validation: the set of known provider kinds = kinds contributed by the
`providers` sections of embedded std manifests plus locked third-party
manifests (M5). Diagnostics per the M5 graduated ladder: kind contributed by
an imported/embedded package → valid; contributed but the package not
imported → advisory missing-import lint (v1); contributed by no known
manifest → `error` naming it an unknown provider (missing package).

### Operator CLI

One v1 item: the existing `whip agent` subcommand is an unrelated IFC-check
REPL (main.rs:12044); per ecosystem shape "Names" it is renamed
**`whip infoflow`** (one-way, no alias), freeing the `whip agent`/`whip
agents` namespace. The DR-0009 CLI suite is deferred; `whip doctor
--providers` remains the operator door.

## Providers (M2 seam classification)

| provider kind | seam (M2) | home |
| --- | --- | --- |
| `owned` | HTTP sans-IO step machine (BrokeredTurnMachine, kernel/harness_loop.rs) | **`std.agent` itself** |
| `fixture`, `native-fixture` | deterministic in-process reference providers | `std.agent` (DR-0009 "Fixture And Test Providers") |
| `command` | subprocess adapter (native-only; DO counterpart = DO tracker Phase 8) | `std.agent` |
| `codex` | native adapter trait behind cargo feature `codex` (kernel/Cargo.toml) | `std.agent.codex` |
| `claude` | native adapter trait behind cargo feature `claude` + Node sidecar | `std.agent.claude` |
| `pi` | native adapter trait (always built today, kernel/pi_rpc.rs) | `std.agent.pi` — deferred |

**Taxonomy position of `owned`:** `provider owned` is the DEFAULT agent path
(DR-0024) and lives in `std.agent` as its reference provider row — not a
sub-package. It is the boundary package's own answer to "what does a fully
governed turn look like": its report can be all-`native` because whip brokers
every tool. Its model backends (OpenAI Responses / Anthropic Messages / Codex
OAuth, cli/coerce_runtime.rs:101-105) are a credential-plane axis shared with
`std.coercion`, not provider kinds.

**Thin provider packages** (`std.agent.codex`, `std.agent.claude`) are data
over compiled adapters: each contributes its provider kind, its compiled
feature report, and its rows in the profile-translation table. Adapter code
stays a cargo-feature-gated kernel module (kernel/Cargo.toml `[features]`);
the package's embedded manifest is included only when its feature is compiled
in, so manifest presence and adapter presence agree by construction
(drift-tested). A binary without feature `codex` genuinely does not know
provider kind `codex` — the registry reports it as a missing package, which is
the truth. Provider expectations: codex = App Server JSON-RPC over child stdio
(kernel/codex_app_server.rs:222-322), explicit-profile-required; claude = SDK
sidecar per DR-0017 "Native Surface", request-only cancel resolving to the
`uncertain` terminal when no terminal is observed; both preserve session
identity as evidence (single-turn today).

## Manifest (M5 contributions)

`std.agent` (embedded, catalog-privileged, validated by the third-party
pipeline):

```text
libraries:        std.agent (standard: true)
effect_contracts: agent.tell (required capability agent.turn; output AgentTurn)
constructs:       none (agent/tell are core grammar; nothing to authorize)
capabilities:     agent.turn (grandfathered id != kind; see Effect operations)
providers:        owned, fixture, native-fixture, command
profiles:         the 7-preset table (rows carry the explicit expansions)
```

`std.agent.codex` / `std.agent.claude` (embedded iff the cargo feature is
compiled): `libraries` (depending on std.agent), `providers` (codex / claude),
the compiled feature report, and their profile-translation rows. No
capabilities, contracts, or constructs. Import bite follows the M5 ladder:
advisory missing-import lint only. Importing any of these packages grants zero
runtime authority — authority still flows through provider bindings,
credentials_ref, profile allowlists (main.rs:21505-21527), and effect
capabilities.

## Static checks (M8: all tier-2 hand-coded core checks, named here)

None meet the rule of three for the generic tier; each is a core check this
spec names as its owner:

1. **Provider-kind-known** (registry-driven; replaces parser/lib.rs:6038-6043)
   with the missing-package/missing-import diagnostic split above.
2. **`requires [feature.class]` vs accepted feature report** — unreportable
   required class = error; probed/compiled mismatch = warning.
3. **Preset-known** — `profile` naming neither a table preset nor a registered
   profile policy is a check-time warning and a runtime recoverable block
   (kills the permissive fallback, cli/harness_tools.rs:468).
4. **Manifest/adapter drift test** — embedded provider manifest present iff
   the adapter feature is compiled; feature-report classes ⊆ the DR-0015
   taxonomy (unknown class = manifest validation error).

## Information-flow face (DR-0029 posture)

- An agent turn is the canonical two-way crossing: prompt/context egress to
  the provider + low-integrity result ingress. That labeling is core IFC.
- These packages export **no `@tool`**, so per DR-0029 "Contract shape" they
  carry no `information_flow` section: absent = no IFC claim, consumer
  fail-closed defaults govern. X3 (no package-asserted authority) holds by
  construction — manifests contribute kinds/reports/profile rows: visibility
  and policy data, never grants.
- The enforcement door stays the owned harness's governance envelope
  (enforce_turn_access_governance; grants coverage at
  cli/harness_tools.rs:2135-2160). The profile table narrows what that door
  exposes; it can never widen it (M3 monotone narrowing: contract caps ⊆
  store-bound caps ⊇ turn grants).
- Feature reports and preset expansions are public metadata (no labels
  cross); probed entries derive from local process probes, not workflow data.

## Dependencies

On core: agent/tell grammar and lifecycle, the embedded-manifest pipeline
(substrate slice S6 in the ecosystem shape's "Candidate build order"), and the
registry-driven validation hook. `std.agent.codex`/`.claude` depend on
`std.agent` — the first exerciser of a package-to-package dependency edge. The
capability-report *pattern* (support: native/emulated/unsupported) is shared
with std.messaging and std.tracker per DR-0004; each package owns its own
report schema — no premature shared framework.

## Spec amendments

1. **spec/agent-harness.md, "Provider Configuration"** (and the terminology
   section): provider-kind validity is registry-derived from package manifests
   (not a compiled-in set); preset tool policy is defined by the `std.agent`
   profile table — the DR-0009 "Harness spec reconciliation" item, executed.
2. **spec/decision-records/0015-agent-harness-feature-semantics.md, "What Is
   Standardized"**: v1 ships the minimal report (class/support/source/native
   name/dispatch); the full per-entry field list (versions, headless flags,
   event mappings) moves to the probed-report re-entry, not required of v1
   manifests.
3. **spec/decision-records/0009-agent-package.md, "CLI And Operations"**: the
   `whip providers|agents|skills` suite is registered deferred; the freed
   `whip agent` namespace comes from the `whip infoflow` rename.

## v1 implementation slices (each independently gateable; S6 sequencing noted per slice)

1. **`whip infoflow` rename.** Rename the IFC REPL (main.rs:12044), update
   tests/docs; no alias. Gate: suite green; `whip agent` errors with a pointer.
2. **`turn.cancel` honesty fix (catalog plane).** The Claude catalog entry
   stops advertising unvalidated cancellation:
   `CancellationDepth::CooperativeRequest` → `CancellationDepth::None`
   (kernel/provider.rs:746).
   The shipped enum (provider.rs:87-129) has no `unknown` variant and its
   values participate in ranked `allows()` comparison inside
   validate_provider_binding, so the catalog fix uses the shipped vocabulary —
   DR-0017's `unsupported | request_only | unknown` triple is feature-REPORT
   vocabulary and lands with the report schema (slice 5) and Claude's compiled
   report (slice 7). Gate: catalog/doctor tests assert Claude advertises no
   cancellation depth; validate_provider_binding rejects a binding requesting
   `cooperative_request` from Claude; DR-0017 conformance test added.
3. **Open provider registry.** Registry-derived kind set replacing
   parser/lib.rs:6038-6043 + :6108-6120; error/lint diagnostic split. Gate:
   fixture manifest contributes a kind and validates; unknown kind = error
   naming the missing package; negative Maude fixture in
   package-contract.maude (unknown provider kind never authorized). Sequenced
   with substrate slice S6 (embedded manifests).
4. **Profile table as package data.** 7 rows with explicit expansions; owned
   harness + adapters + sidecar consume computed policy; sidecar defaults
   deleted; unknown-preset fail-closed; `issue-triager` mapped. Gate:
   table-vs-harness-policy drift test; sidecar smoke asserts explicit policy
   in the request; permissive-fallback regression test inverted. Sequenced
   with substrate slice S6 for the manifest home; pre-S6 staging allowed —
   the table may land first as compiled data consumed by the harness/adapters,
   folded into the embedded manifest when S6 lands.
5. **Minimal capability reports.** Report schema + compiled reports for
   owned/fixture/command in std.agent's manifest; doctor renders `source`.
   Gate: schema validation test; owned report all-`native`; fixture report
   deterministic. Sequenced with substrate slice S6 for the manifest home;
   same pre-S6 staging as slice 4 (reports as compiled data first).
6. **`requires [feature.class]`.** Parser field (AgentField::Requires),
   taxonomy-membership check, validation against the selected provider's
   report. Gate: error on unreportable class; warning on probed mismatch; .ir
   snapshot updated once.
7. **Thin provider packages.** std.agent.codex + std.agent.claude embedded
   manifests (feature-conditional) with kinds, reports, translation rows;
   drift tests per Static checks item 4. Sequenced with substrate slice S6
   (embedded manifests are the substrate). Gate: build matrix with features
   on/off; kind resolution flips accordingly; Claude report states
   `turn.cancel: unknown` per DR-0017 (report half of slice 2).

## Deferred with cause

- **`std.agent.pi` (name reserved).** Cause: `pi_variant` has no
  lock/provenance/discovery design (zero parser/IR/runtime hits) — ecosystem
  shape "Merge/split/defer/drop verdicts". Re-entry: pi_variant locking per
  DR-0015 "Provider-Specific Semantics" + its "Next Validation Work" probes.
- **Skills wiring.** Declared skills parse and persist but dispatch sends
  `skill_names: &[]` (main.rs:21302); the provenance schema is an open DR-0009
  question. Re-entry: `skill.attach` feature class + the spec/skills.md
  ownership question, as its own slice.
- **Probed/versioned report artifacts + report hash in evidence.** Cause:
  requires live-provider probes and an evidence-schema decision. Re-entry:
  DR-0015 "Next Validation Work"; the upgrade is additive (`source: probed`).
- **Session features (`session.resume/fork`, `turn.steer/follow_up`,
  compaction).** Cause: session ids are evidence-only single-turn today; no
  consumer demand. Re-entry: classes already reserved in the taxonomy; each
  becomes a provider-package slice when a workflow requires it.
- **Operator CLI suite (`whip providers|agents|skills`).** Cause: doctor
  covers current demand. Re-entry: after slices 3-5, the data exists to render.
- **Enterprise brokers; `native.command.dispatch` escape hatch.** Cause: no
  demand; DR-0015 marks dispatch provider-specific-only. Re-entry: a real
  broker/tenant requirement.
- **Web search tool.** Not this package's item — canonical home is
  [`owned-harness-tool-surface.md`](owned-harness-tool-surface.md) "Open
  items"; the network-tool gate closed 2026-07-07 (designs accepted:
  web-search + web-fetch design notes); remaining work is build.

## Open naming-boundary questions

1. **`permissive` preset**: shipped in the owned harness
  (cli/harness_tools.rs:454) but absent from the canonical 7. Recommend an
  operator/dev-only table row marked non-canonical, never valid hosted —
  Jack's call alongside the profile-table slice.
2. **`command` provider kind**: kept in std.agent (generic subprocess bridge);
  could split if an external-command ecosystem appears.
3. **`harness` keyword retirement**: stays soft-deprecated; provider-config-only
  remains DR-0009's open question, untouched by v1.

## Verdict

**Build.** All v1 slices are data-centralization and honesty fixes over
shipped, tested mechanisms — no new execution machinery, one additive grammar
field, zero renames of durable kinds. The package earns its existence by
deleting four scattered-policy/dishonesty holes (closed kind set, sidecar JS
policy, permissive fallback, overstated cancel) and by being the first
exerciser of manifest-contributed provider kinds and a package-to-package
dependency edge — both load-bearing for every later third-party provider.
