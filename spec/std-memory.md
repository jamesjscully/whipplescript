# `std.memory`: memory pools, explicit recall/learn, turn-scoped memory grants

Status: concrete package design 2026-07-04 (std-package campaign, design tracker
Process step 6).
Constitution: [`std-package-ecosystem-shape.md`](std-package-ecosystem-shape.md) —
decisions M1–M8/E1–E7 bind this design.
Design baseline: [DR-0008](decision-records/0008-memory-package.md) (accepted),
amended below where the constitution supersedes it.
[`memory-plugin.md`](memory-plugin.md) remains historical background.

## Framing and core functionality

**Memory is workflow-controlled context, never ambient model state.** A memory
pool is a named durable place; `recall` reads a bounded context bundle out of
it, `learn` submits material into it, `curate` maintains it, and turn-scoped
`with access to` grants expose bounded memory tools to an agent for one turn
(DR-0008 "Decision"). Nothing moves without an explicit source-level operation
or grant.

std.memory is also **the package-mechanism worked example**: it is the only
inventory row that already exists as a real manifest
(std/manifests/memory.json, exercised at cli/main.rs:36212-36255 and
examples/package-memory.whip), and it is the constitution's **forcing case for
the CapabilityProvider seam** (M2): `capability.call` today settles via a
fabricated fixture value (kernel/effect_handlers.rs:1883-1887), and fixture +
local backend is the genuine two-provider demand that makes the registry
dispatch-honest.

## Why it belongs as a package

Memory is provider-pluggable domain vocabulary, not a lifecycle invariant (E7):
core owns the effect lifecycle, turn-grant composition point, artifacts,
evidence, and IFC; retrieval strategy, storage, curation policy, and the memory
CLI are exactly the things a package owns (DR-0008 "Core/Package Boundary").
It is a **semantic domain** package (E2) and an **authoring surface** package
(E3).

## What is NOT in the package

- The agent-turn lifecycle, `with context` passing, and the turn-grant
  composition point — core (DR-0008 "Core/Package Boundary").
- The CapabilityProvider seam and `capability_bound` promotion — core substrate
  (constitution slice S5), std.memory is its first customer, not its owner.
- Ambient/hidden memory, an always-on context injector, a vector-database API
  as language surface, embeddings/chunking/index maintenance as workflow
  concepts (DR-0008 "Anti-Goals"). `search hybrid`-style intent clauses are
  deferred with the policy grammar (below).
- The versioned-workspace "knows" plane and any `std.knowledge` projection
  engine (DR-0008 open question; registered boundary below).

## Surface

Grammar additions are **core parser arms authorized post-parse** (M1), exactly
the shipped recall/send pattern (parser/body.rs:1853-1985). `use std.memory`
gets the advisory missing-import lint per the M5 ladder (not hard-off; only
std.script is hard-off).

### Pool declaration (new parser arm)

```whip
memory pool project_memory {
  context limit 8
}
```

Family `declaration_block`, lowering class `metadata_only`, provides
`Resource<MemoryPool>`. v1 clauses: `context limit <n>` (optional recall
packing budget) only — **v1 pools are provider-less**. There is no
`provider <ident>` clause: provider selection is binding-owned (M2 — the
promoted `capability_bound` returns the BINDING's provider + config_json), so
a pool-level provider clause that nothing reads or reconciles against the
binding would be exactly the decorative-clause dishonesty the constitution's
ground truth 1 exists to kill. Deferred with cause below.

`context limit` is **not provider config** (providers take config exclusively
from the binding's `config_json`): it lowers into the `capability.call` effect
INPUT alongside `pool` and `query`, and providers read it from the effect
input like any other operation argument. The DR-0008 policy
clauses `learning`, `retention`, `capacity`, `curation { ... }`, `search` are
deferred with the curation-policy grammar (below). Unknown clauses are rejected
(file-store precedent, parser/lib.rs:18003-18057). Today no pool declaration
exists at all — the phrase appears only in an error string (body.rs:1856).

### Effect operations

Every memory operation lowers to effect kind **`capability.call`** (lowering
class `capability_call`), settling facts **`capability.call.completed`** /
**`capability.call.failed`** (kernel/effect_handlers.rs:1870-1881), carrying a
`target_capability`. The `typed_effect_call` promotion stays a recorded future
step (DR-0008 "Construct Graph Contract"); files (E4) is the precedent to copy
when taken.

| construct | form | target capability | output |
|---|---|---|---|
| recall | `recall <pool> for <query> as <b>` — SHIPPED (body.rs:1853-1903; lowering rule_lowering.rs:2933, flows flow_expand.rs:1245) | `memory.query` | `MemoryContext` |
| learn | `learn from <source> into <pool> [for <subject>] [{ note <expr> }] as <b>` — new arm | `memory.write` | `MemoryLearnResult` |
| curate | `curate <pool> [{ reason <expr> }] as <b>` — new arm | `memory.curate` | `MemoryCurationResult` |

Capability ids (M3, one namespace): **`memory.query`**, **`memory.write`** —
KEPT verbatim per E1 (no `memory.recall`/`memory.learn` renames) — plus
**`memory.curate`** (additive, new authority, not a rename; flagged below).
Memory rides the generic `capability.call` kind, so the enforced runtime plane
is fed by the contract's `required_capabilities` (== `target_capability` by
construction) and the manifest `capabilities[]` rows; the M3 lock-time subset
check (contract caps ⊆ manifest caps) holds by construction.

`MemoryContext` (v1 minimum, replacing the fixture-shaped `{summary, target}`
output schema in the manifest): `summary` (packed bundle text), `pool`,
`entries[]` of `{memory_id, text, created_at, provenance}`. Selection detail
(candidates, scores, budget, strategy) is **evidence, not output** — the
source-visible/evidence split stays a registered open question (DR-0008 "Open
Questions").

### Turn grants

The grant grammar is SHIPPED including per-op subjects —
`with access to project_memory { recall for issue  learn for issue }` parses
and carries `operation`/`target` (body.rs test at :3606-3627). Validation of
memory-pool grants was deliberately left to this design (parser/lib.rs:4019-4025),
and the grant is **inert at turn time**: `turn_tool_access_from_input` maps
only file/command/tracker ops and drops recall/learn through `_ => {}`
(harness_tools.rs:2093-2112). This design replaces that arm:

- `TurnMemoryAccess { pools: [{pool, recall, learn}] }` joins `TurnToolAccess`.
- Owned harness exposes two bounded tools per granted pool: **`recall_memory`**
  and **`learn_memory`** (verb-first, matching `list_todos`/`add_todo`,
  harness_tools.rs:41-43), calling the same CapabilityProvider as the workflow
  operations and recorded as turn tool-call evidence (tracker-tool precedent —
  no child durable effects in v1; deferred below).
- Governance: memory pools join file/command/tracker in
  `enforce_turn_access_governance` — an envelope that does not govern the pool
  resource rejects the grant (harness_tools.rs:2135-2163).
- Native adapters (codex/claude) get no memory tools in v1; providers own
  exposure mechanics per DR-0008, and only the owned harness has the tool seam
  today. This is made loud, not silent: static check 4 (below) warns on a
  memory grant whose target statically resolves to a native adapter, so the
  native path does not recreate the inert-grant behavior MEM-5 eliminates on
  the owned path.

## Providers (M2 seam classification)

The execution seam is the **new CapabilityProvider host-projection trait**
(constitution M2, substrate S5): it replaces the fixture else-branch at
kernel/effect_handlers.rs:1883-1887, keeps the
validate→complete_run→derive-fact tail unchanged (the `CapabilityContract`
projection, effect_handlers.rs:1797-1801), and is selected registry-honestly by
the promoted `capability_bound` returning the binding's provider + config_json
(store/lib.rs:6928).

- **`fixture`** — today's fabricated output becomes a provider NAMED `fixture`
  (M2). Deterministic, bound under the test harness; every existing
  capability.call test stays green under it.
- **`local`** — the REAL provider: workspace-scoped SQLite behind a new
  sans-IO **`MemoryStore` trait** (the Coordination/FileStore/WorkItems seam
  pattern, store/coordination.rs:431-660, store/files.rs:20-58; default path
  `.whipplescript/memory.sqlite`, env `WHIPPLESCRIPT_MEMORY_STORE`). Retrieval
  v1 is deliberately boring: FTS5 lexical match + pool scoping + recency, per
  memory-plugin.md "Retrieval". Entries carry provenance columns (source
  instance/effect/run, author actor, created_at) per memory-plugin.md
  "Provenance". `memory.curate` = dedupe + prune strategies inside the store.
- Remote/HTTP memory services are a later provider on the **HTTP sans-IO
  step-machine** seam class (M2 class 1); DO-plane memory rides the
  `MemoryStore` trait port and belongs in the DO tracker (M7), not here.
  As of this design the DO tracker carries **no** memory row — adding the
  MemoryStore-over-DoSql port row (Phase 8-adjacent) to
  spec/durable-object-runtime-tracker.md is an explicit MEM-3 deliverable,
  not a claim of existing registration.

Provider expectations: implement CapabilityProvider for the three capabilities;
settle only through the effect lifecycle (no fact writes outside settle,
DR-0008 "Anti-Goals"); fail with the DR-0032 EffectError base; emit recall
evidence explaining query material, candidates, selection, and budget (DR-0008
"Events, Evidence, And Projections"); take config exclusively from the
binding's `config_json`.

## Manifest (M5)

The embedded std.memory manifest evolves std/manifests/memory.json —
validated by the same pipeline as third-party manifests, catalog-privileged:

- identity: `package_id`/`name` → **`std.memory`** (E1; currently
  `package-memory`/`memory`).
- `libraries[]`: library `std.memory` with effect_contracts `memory.query`,
  `memory.write`, `memory.curate` (effect_kind `capability.call`,
  provider_kinds `["memory-provider"]`, output schemas per the surface above).
- `constructs[]`: `memory.pool` (declaration_block/metadata_only),
  `memory.recall` (source_form corrected to the shipped
  `recall <pool> for <query> as <binding>` — the `recall from` drift dies here),
  `memory.learn`, `memory.curate` (effect_operation/capability_call).
- `capabilities[]`: the three ids.
- `providers[]`: `fixture` and `local`, provider_kind `memory-provider`.
- `profiles[]`: `memory-user` allowing all three capabilities.
- `bindings[]`: global default → `local`; the test harness binds `fixture`.

`recall` uses no reserved bare word beyond its own keyword; no privilege-tuple
rows are needed (core/lib.rs:554-582 unchanged).

## Static checks (M8)

Tier 2 (hand-coded core checks, named here as this package's demands — core
implements and enforces per E7):

1. `recall`/`learn`/`curate` must name a **declared memory pool** (the
   declared-channel precedent); undeclared pool = check error with the pool's
   declaration form in the hint.
2. Memory-pool grant validation: a `with access to <pool>` block on a declared
   pool accepts only `recall`/`learn` ops; memory ops on non-pool resources are
   rejected (closes the deliberate deferral at parser/lib.rs:4019-4025).
3. Pool declaration: unknown clauses rejected — including `provider`, with a
   hint that provider selection is binding-owned (manifest `bindings[]`);
   `context limit` must be a positive integer.
4. Memory-grant exposure (MEM-5): a `with access to <pool>` grant on a tell
   whose target agent statically resolves to a native adapter (codex/claude)
   is a **check-time warning** — the grant would be inert at turn time, since
   only the owned harness has the tool seam in v1. This deliberately narrows
   DR-0008's "checker must reject a memory grant if the target
   agent/profile/provider cannot expose a turn-scoped memory tool" from
   hard-reject to warning (Spec amendment 5 below); hard rejection re-enters
   with provider capability reports (DR-0015).

Tier 1 (generic, already in the rule-of-three set): binding-required — `recall`
already enforces `as` at parse (body.rs:1873-1880); new arms do the same.
No exhaustive-outcome check: memory ops settle through the generic
succeeds/`fails as` surface.

## Information-flow face

Cite: [DR-0029](decision-records/0029-cross-package-information-flow.md)
(cross-package information flow) + DR-0027/0028 posture.

- **Pools are governed resources.** A pool enters the label map like a channel:
  fail-closed untrusted bottom until governance grants a label (channel
  precedent, ifc.rs:1701-1715). The envelope grant form is
  **`grant memory <pool> <level>`** — the memory mirror of
  `grant channel <name> -> <provider:dest> <level>`, minus the destination
  (pools have no external endpoint) — and it lands in slice MEM-3 alongside
  the carriage extension, so pools are labelable from v1 rather than stuck at
  bottom. `learn` is a write crossing into the pool;
  `recall` is a read crossing out of it.
- **Integrity carriage.** Learned entries record author provenance; entries
  written from agent turns carry agent-turn integrity. Recall output is the
  **opaque join** of selected entries (DR-0029 X2 / DR-0030 whole-result v1) —
  no finer signature is claimed.
- **Cross-instance carriage.** Pools are workspace-scoped shared state, the
  same posture as `shared` coordination resources (E-COORD carriage,
  ifc.rs:683-838, models/maude/infoflow-coord-carriage.maude). The carriage
  model is extended for memory in slice MEM-3.
- **Turn grants** are checked against the verified governance envelope
  (harness_tools.rs:2135-2163, extended to pool resources).
- **Package boundary.** std.memory exports no `@tool` in v1, so no
  `information_flow` contract entry is required (DR-0029: absent = no claim,
  consumer fail-closed defaults). Any future memory `@tool` declares its pool
  crossings in `surface`/`required_crossings`.
- **New-reader caution.** A pool that ingests labeled evidence/trace content
  becomes a new reader of that content; the evidence-plane no-new-readers rule
  (experimentation-subsystem research note, "evidence-plane IFC") applies to
  any future evidence→memory ingestion path. Registered, not designed.

## Dependencies

- **Substrate S5** (CapabilityProvider seam + `capability_bound` promotion) —
  hard prerequisite; std.memory is the forcing case and first customer.
- **Substrate S6** (embedded std manifests + import lint) for the manifest
  identity move; MEM-1..MEM-5 can land before S6 against the existing
  example-manifest lock path.
- Core: effect lifecycle, `CapabilityContract` projection, turn-grant
  composition point (shipped), owned-harness tool surface (DR-0024).
- Other packages: none. std.agent providers are downstream consumers of the
  grant surface, not dependencies.

## Target feature set

v1 (sliced below): pool declaration; `recall` (shipped grammar) and `learn`
against real `local` + named `fixture` providers; `curate` with provider-default
strategies; harness grant wiring with governance; embedded manifest; minimal
operator projections `whip memory pools` / `whip memory entries <pool>`
(coordination-CLI precedent, cli/main.rs:29668-29717).

Designed but not v1: reviewed learning + keep/forget lifecycle, curation policy
grammar, recall block form, subject-scoped grant enforcement, typed_effect_call
promotion, remote providers — all in "Deferred with cause".

## v1 implementation slices

Each independently gateable under the per-piece review discipline.

- **MEM-1 — pool declaration + declared-pool checks.** Parser arm, AST/IR,
  fmt, .ir snapshot; static checks 1–3; manifest `memory.pool` construct row.
  Tests: parse/lower/fmt round-trip; check errors for undeclared pool,
  rejected `provider` clause (binding-owned hint), unknown clause,
  non-positive `context limit`; existing recall fixtures gain a pool decl.
- **MEM-2 — memory contracts over fixture-as-named-provider (rides S5).**
  memory.query/memory.write dispatch through the promoted `capability_bound`
  to provider `fixture`. Tests: existing capability e2e green under the named
  row; binding-selects-provider test; unbound capability still blocks via
  `policy_block_on` (store/lib.rs:6423).
- **MEM-3 — `local` provider.** `MemoryStore` trait + SQLite/FTS5 impl +
  CapabilityProvider for query/write; `MemoryContext` output + manifest schema
  update; recall evidence (query, candidates, selected, budget); `whip memory
  pools|entries`; the `grant memory <pool> <level>` envelope grant form (pool
  label entry, channel-grant precedent); register the DO-plane
  MemoryStore-over-DoSql port row (Phase 8-adjacent) in
  spec/durable-object-runtime-tracker.md. Tests: store units (retrieval, pool
  scoping, provenance stamps); e2e recall→`tell ... with context`;
  learn→recall round-trip; contract output validation failure path;
  label-grant fixture (ungranted pool stays fail-closed bottom, granted pool
  carries its level). Models: memory capability rows
  added to models/maude/package-contract.maude coverage; E-COORD-style
  carriage extension to infoflow-coord-carriage.maude (or a sibling
  infoflow-memory-carriage.maude) with a bite fixture.
- **MEM-4 — `learn from ... into` arm.** Parse/lower to capability.call →
  memory.write with provenance fields from `from`/`for`. Tests: parse/lower;
  e2e under local + fixture; `fails as` binds the DR-0032 base.
- **MEM-5 — harness grant wiring.** Replace the inert arm
  (harness_tools.rs:2093-2112) with `TurnMemoryAccess`; `recall_memory`/
  `learn_memory` tools gated per grant; governance-envelope extension; static
  check 4 (inert-grant warning on native-adapter targets).
  Tests: harness units (deny-all default, per-op exposure, ungoverned-pool
  rejection); e2e owned turn recalls via granted tool against `local`;
  warning fixture for a memory grant on a native-adapter tell.
- **MEM-6 — `curate`.** Parser arm; `memory.curate` capability + contract +
  manifest rows; dedupe/prune in `local`; applied-changes evidence.
  Tests: parse/lower; e2e curate reduces duplicates; re-curate idempotent.
- **MEM-7 — embedded manifest identity (rides S6).** memory.json →
  embedded `std.memory` manifest; advisory import lint; drift test
  manifest-vs-registered-contracts. Tests: manifest validation, lock-exemption
  behavior unchanged, `use std.memory` lint fires.

## Deferred with cause

- **Reviewed learning + keep/forget + proposal projections** (`learning
  reviewed`, `when <pool> has proposed memory`). Cause: needs a projection
  surface; projection vocabulary is deferred campaign-wide (E4:
  `projection_view` unproduced by decision). Re-entry: queue-style worker-pass
  fact projection (kernel/rule_pass.rs:290-374 precedent) or DR-0021 nouns
  when built; `keep`/`forget` land with it (DR-0008 "Curation").
- **Curation policy grammar** (`curation {}`, `retention`, `capacity`,
  `learning`, `search` clauses). Cause: smallest-useful-policy-grammar is an
  open DR-0008 question; v1 curate strategies are provider defaults. Re-entry:
  with reviewed learning, as additive pool clauses.
- **Recall block form** (`{ query ... context limit N }`). Cause: single-query
  shipped form suffices; additive parse later. Re-entry: on author demand.
- **Subject-scoped grant enforcement** (`recall for issue` narrowing). Cause:
  grammar already carries the subject (body.rs test :3606) and v1 passes it as
  a retrieval hint, but enforcement needs a subject-identity story. Re-entry:
  with the MemoryContext schema/evidence work.
- **Child durable effects for turn-tool memory use** (DR-0008 "agent grant use
  → child recall/learn effects"). Cause: the tracker-tool precedent records
  tool calls as turn evidence, not child effects. Re-entry: when evidence-grade
  per-tool audit is demanded.
- **typed_effect_call promotion.** Cause: capability_call is the live enforced
  path; promotion buys nothing until the files precedent (E4) is proven.
  Re-entry: recorded in DR-0008 "Construct Graph Contract".
- **Remote/HTTP providers; DO-plane memory.** Cause: M7 — DO package layer
  lives in the DO tracker; `MemoryStore` is the port seam. Re-entry: the DO
  tracker Phase 8-adjacent row that MEM-3 registers (not yet present in the
  tracker as of this design).
- **Pool-level `provider` clause.** Cause: provider selection is binding-owned
  (M2, registry-honest via the promoted `capability_bound`); a pool clause
  nothing reads or reconciles is a decorative clause (constitution ground
  truth 1), and its enforcement story (pool-scoped binding override vs static
  consistency check against `bindings[]`) is undesigned. Re-entry: if per-pool
  provider overrides are demanded, as a pool-scoped binding row consulted by
  `capability_bound` — never as an unenforced bare identifier.
- **Full `whip memory` CLI** (learn/keep/forget/curate/explain, DR-0008
  "Operator Surface"). Cause: mutation CLI without the proposal lifecycle
  invites unaudited writes. Re-entry: with reviewed learning.
- **std.knowledge / versioned-workspace knows-plane boundary.** Cause: owned by
  the versioned-workspace research note. Re-entry: registered open question.

## Spec amendments

1. **DR-0008 "Capability Surface"**: capability ids become `memory.query`,
   `memory.write`, `memory.curate` — E1 keeps query/write; the
   `memory.recall/learn/keep/forget` id list is superseded (keep/forget ids
   return with their lifecycle, named then).
2. **DR-0008 "Workflow-Managed Recall" + std/manifests/memory.json
   `source_forms` + construct-grammar.md worked example**: recall's canonical
   form is the shipped `recall <pool> for <query> as <binding>` (no `from`) —
   closes the naming drift recorded in the state survey.
3. **std/manifests/memory.json identity**: `package-memory`/`memory` →
   `std.memory`; the file becomes the embedded-manifest seed (M5, slice MEM-7).
4. **spec/capability-registry.md turn-grant vocabulary**: add memory-pool grant
   ops (`recall`, `learn`) beside the tracker/file/command rows (:103-111).
5. **DR-0008 "Turn Access Grant"**: the requirement that "the checker must
   reject a memory grant if the target agent/profile/provider cannot expose a
   turn-scoped memory tool" is narrowed for v1 to a check-time **warning** on
   statically-resolvable native-adapter targets (static check 4) — the owned
   harness is the only tool seam, and provider exposure capability is not yet
   a queryable fact. Hard rejection returns with provider capability reports
   (DR-0015).

## Open naming/boundary questions

- `memory.curate` as a new capability id (additive under M3) — small, but new
  authority vocabulary: flag for Jack with slice MEM-6.
- Evidence→memory ingestion vs the evidence-plane no-new-readers rule (above) —
  boundary owned jointly with the experimentation subsystem note.
- `MemoryContext` source-visible detail vs evidence-only (DR-0008 open
  question) — settle inside MEM-3 review.

## Verdict

**Keep, as designed here.** std.memory ships v1 as the campaign's
package-mechanism worked example: pool declarations + recall/learn/curate over
the CapabilityProvider seam with `fixture` and `local` as honest registry-bound
providers, turn grants finally live in the owned harness, and the embedded
manifest as the first std package through the third-party validation path.
