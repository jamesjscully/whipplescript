# `std.tracker` package design: issues, claims, the shipped backlog

Status: concrete package design 2026-07-04 (std-package campaign, Process step 6).
Constitution: [`std-package-ecosystem-shape.md`](std-package-ecosystem-shape.md) — decisions M1–M8/E1–E7 bind this design.
Substrate: [`decision-records/0002-work-tracker-package.md`](decision-records/0002-work-tracker-package.md) (the tracker semantics ADR)
and [`work-queues.md`](work-queues.md) (superseded queue formulation) — this document
does not restate them; it packages what shipped and records the seam to the rebuild.

## Package contract

- **Core functionality.** A workspace-scoped durable work backlog: a `tracker`
  resource declaration, issue filing, deterministic readiness projected into
  instance facts, branchable claim/renew/release with terminal auto-release,
  finish, and a direct CLI so humans and agents can operate the backlog without
  a workflow (DR-0002 "CLI Requirements").
- **Why it belongs.** The dispatch loop is shipped and load-bearing today under
  queue nouns: decl (parser/lib.rs:17875), four durable effect kinds with
  contracts already attributed to library_id `std.tracker` (parser/lib.rs:3470-3510),
  readiness sugar over projected facts (parser/lib.rs:12114; kernel/rule_pass.rs:290-374),
  the `WorkItemStore` with atomic claim CAS (store/items.rs:43-251), the
  backend-agnostic `WorkItems` trait implemented native + DO
  (store/items.rs:258-327; host-do/do_store.rs:6333-6497), the generic kernel
  handler (kernel/effect_handlers.rs:1276-1444), terminal auto-release
  (kernel/rule_pass.rs:270-283), and reserved-word privilege rows for
  claim/renew/release → std.tracker (core/lib.rs:561-582). What is missing is
  exactly identity and honesty: tracker nouns, capability ids on the contracts
  (all four pass EMPTY required_capabilities today), a parsing `renew`, an
  exercised privilege mechanism (zero exercisers), and a CLI that can claim.
- **NOT in this package.** The event-sourced provider rebuild (deferred, see
  "The rebuild seam"); generic leases/ledgers/counters (`std.coord` — a
  separate lease mechanism by standing decision, see std-coord.md "The
  four-mechanisms resolution"); effect lifecycle, `after` branching, instance
  facts, and terminal auto-release enforcement (core invariants per E7 —
  release_holder_resources_on_terminal spans queue claims and coordination
  leases from one core hook, kernel/rule_pass.rs:270-283, and this package may
  not opt out); agent-turn todo tools' harness policy (owned-harness surface;
  this package owns only their store and vocabulary); boards, sprints, time
  tracking, PM UI (DR-0002 "Rationale").
- **Target feature set (v1).** The post-rename surface (substrate slice C) +
  `tracker.*` capability ids on all contracts (M3) + `renew` parsing and
  executing with optional claim TTL + claim/renew/release privilege rows
  exercised through the embedded manifest and built-in registrations + minimal
  `whip issue` CLI covering the lifecycle the language can express.
- **Dependencies.** Core: effect lifecycle + idempotency, sum-type
  exhaustiveness, the `WorkItems` store trait, worker-pass projection, IFC
  engine, terminal auto-release hook. Substrate slices: S3 (rename C — this
  design specifies its target surface), S4 (capability-namespace invariants),
  S6 (embedded manifests — the privilege/manifest slice rides it). Packages:
  none required; composes with std.agent (claim gates agent turns) and
  std.coord (sibling claim surface, kept separate) without depending on them.
- **Provider expectations.** One provider in v1: `builtin`, store-plane (see
  "Providers"). External trackers must declare claim strength when un-deferred
  (DR-0002 "External Providers").
- **Open naming-boundary questions.** Listed at the end.
- **Verdict.** KEEP as its own semantic-domain package (E2/E6). v1 = identity +
  rename + honesty on the shipped row-store; the ADR-0002 event-sourced rebuild
  is split IN TIME, not in package (E6), behind a documented seam.

## The rebuild seam (E6, load-bearing)

ADR-0002's state model (commands → accepted events → projections; immutable tx
log; disposable index; lease/status split; conflicts, heads, state tokens) is
DEFERRED to its own later model-first campaign — the ADR is status:proposed and
it was the campaign's largest unforced slice (constitution "Merge/split/
defer/drop verdicts"). Two things are contracts that SURVIVE the rebuild, so v1
work on them is not throwaway:

1. **The `WorkItems` trait** (store/items.rs:258-327) — the sans-IO,
   object-safe seam already implemented three ways (WorkItemStore,
   NativeStores facade at store/native_stores.rs:633, DO DoSqliteStore at
   host-do/do_store.rs:6333-6497, verified against real SQLite). The rebuilt
   event-sourced provider plugs in BEHIND this trait; abandoning its shape
   re-does the DO port for nothing.
2. **This package contract** — surface grammar, effect-kind strings,
   capability ids, fact names, CLI verbs, manifest rows. The rebuild swaps the
   provider implementation; it does not get a second rename.

Everything v1 deliberately does NOT fix (combined claim/status write, thin
issue model, write-once rows) is recorded under "Deferred with cause" as the
rebuild's backlog, each with the deviation it cures.

## Surface (post-rename; substrate slice C delivers the rename, this section defines its target)

Slice C (constitution "Renames", slice C) is substrate, not a slice of this
design. Its acceptance target, defined here:

```whip
tracker <name> { provider builtin }
```

Declaration renamed from `queue <name> { tracker builtin }` (parser/lib.rs:17875);
the field becomes a block-internal `provider` clause per the channel precedent
(parser/lib.rs:17923) and DR-0002 "Readiness". Family `declaration_block`,
lowering `metadata_only`.

Effect operations, with EXACT effect-kind strings and capability ids (M3:
capability id == effect kind for 1:1 operations, matching the store's
default-required-capability rule by construction, store/lib.rs:6873-6877):

| operation | grammar | effect kind = capability id | outcomes |
|---|---|---|---|
| file | `file issue into <tracker> { title .. body .. labels .. metadata .. } as b` | `tracker.file` | filed / failed |
| claim | `claim <issue> [ttl <duration>] as c` | `tracker.claim` | claimed / already_claimed{holder} / not_found |
| renew (NEW) | `renew <claim> [as b]` | `tracker.renew` | renewed{expires_at?} / not_held |
| release | `release <claim>` | `tracker.release` | released / failed |
| finish | `finish <issue> { summary <expr> } [as b]` | `tracker.finish` | finished / failed |

Readiness sugar stays core-owned projection-read metadata until
`projection_view` is un-deferred (E4):

```whip
when <tracker> has ready issue as issue
```

over the projected fact `tracker.issue.ready` (renamed from
`queue.item.ready`). Ready = status `open` AND no active claim (unclaimed, or
claim past its `expires_at`).

Statuses (full decided set, M4 slice C): `open` / `in_progress` / `closed` /
`canceled` / `archived` (renamed from open/in_progress/done/cancelled,
store/items.rs:25). `archived` is reachable only via `whip issue archive` in
v1 (no source verb); archived issues are never ready and are excluded from
default listings.

Facts: `tracker.<op>.completed` / `tracker.<op>.failed` with the DR-0032
EffectError base (`after claim fails as f` keeps binding typed failures,
kernel/effect_handlers.rs:1451). Typed claim output schema `TrackerClaim`
(renamed from `QueueClaim`).

**Renew/TTL semantics (the one new mechanism).** A claim without `ttl` behaves
exactly as today: no expiry, terminal auto-release is the only recovery
(kernel/rule_pass.rs:270-283 — claims carry no TTL). A claim with
`ttl <duration>` records `expires_at`; `ready` and `claim` lazily release
expired claims in the same transaction (the coordination-store expiry pattern,
store/coordination.rs:95-410), so the issue re-projects as ready
(updated_at-salted keys already re-project released items,
kernel/rule_pass.rs:290-374). `renew` is a holder-checked CAS extending
`expires_at` by the claim's ttl; renewing an untimed claim succeeds as a
recorded heartbeat (`expires_at` stays null). `not_held` covers missing,
expired-and-reclaimed, and other-holder — normal typed failures per DR-0002
"Source Operations". The `WorkItems` trait gains `renew_claim` plus
expiry-aware `ready`/`claim`; all three implementations port.

**T3 BUILT 2026-07-15 (post-campaign Wave 2).** The full renew + claim-TTL
surface shipped as one slice: the `TrackerRenew` IrEffectKind (every
exhaustive match + the claim-`ttl` clause round-trip through flow_expand),
`claim <issue> [ttl <duration>] as c` threading an `expires` into `claim_item`
(now `(item, actor, expires)` across all three `WorkItems` impls), and
`renew <claim>` binding-typed disambiguation — a renew naming a `claim … as
<b>` binding lowers to `tracker.renew` (heartbeat: re-affirms the holder's
lease), a renew naming an `acquire … as <b>` binding stays `lease.renew`,
mirroring the shipped `release` split; a renew naming neither is a check
error. `whip issue claim --ttl` / `renew --ttl` set/extend a finite deadline
(holder-checked, monotonic). The manifest's `tracker.renew` contract row
folds against the parser-compiled one. Deferred with cause: source `renew`
is heartbeat-only (finite extension via the CLI `--ttl`; the source grammar
has no duration), `renew` keeps its required `as` binding (inherited from the
shared lease-renew parser), and DO claim-`ttl` is inert (the DO clock stub
`"now"` doesn't parse, so a ttl falls back to an untimed claim — the same
stub the DO's coordination wait-deadline uses; renew heartbeat + untimed
claims work). The historical disposition below records the pre-T3 state.

**Renew disposition as of 2026-07-14 (T4 landed, T3 still open — SUPERSEDED
by the T3 BUILT note above).** The store
plane of T3 already shipped with the A+blockers rebuild: `WorkItems::renew_claim`
(holder-checked, monotonic, heartbeat-on-untimed) and expiry-aware
`ready`/`claim` exist across all three implementations, model-verified by
tracker-lease.maude, and `whip issue renew` drives them from the CLI. What T3
still owes is the SURFACE: the `tracker.renew` effect kind + contract, the
claim `ttl` clause, `renew <claim>` lowering, and the CLI `--ttl` flags. Until
then the language-level `renew` verb parses ONLY as coord's lease renewal
(`renew <acquire-binding>` → `lease.renew`, BodyEffectKind::LeaseRenew; the
compile-time check rejects a renew naming anything but a same-rule acquire) —
this is the spec-sanctioned pre-T3 state, not a blessed permanent split: the
`renew` reserved-keyword privilege row already points at std.tracker, the
embedded manifest seeds the `tracker.renew` capability/provider/binding trio
and its construct row (sans contract, the std.coord `lease.renew` precedent),
and T3 resolves the verb by binding-typed disambiguation (a renew naming a
claim binding lowers to `tracker.renew`, mirroring the shipped `release`
split). Landing a `tracker.renew` effect kind WITHOUT the claim-TTL half was
rejected: with no way to create a timed claim, the kind could only ever
heartbeat — a decorative effect kind, exactly the pretense this package
forbids. T3 stays one slice.

Slice C blast-radius items this design depends on (S3 scope, per M4: kind-map
dedup first; store-open guard errs loudly on pending legacy `queue.*` effects;
idempotency keys rekey): projection fact rename; harness todo-tool status
rendering remap (done/cancelled currently both render "completed",
harness_tools.rs:1221-1240 — becomes closed/canceled); examples rebind
`claim item as lease` → `claim issue as active_claim`
(queue-worker-with-review.whip:54, multi-agent-bounded-concurrency.whip:29,
package-memory.whip:21; design-tracker "Current Notes" demands the handle
noun); `given tracker <name> issue` harness clause already uses target
vocabulary (control_plane.rs:7411) and is untouched.

## Providers (M2 seam classification)

- **`builtin` — store-plane.** Not one of M2's three I/O seams: tracker verbs
  are typed builtin effect kinds executed by the generic kernel handler over
  the held `RuntimeKernel<S: WorkItems>` (kernel/effect_handlers.rs:1276-1444),
  the same class as std.coord's handler. Already DO-portable
  (host-do/do_store.rs:6333-6497). No `CapabilityProvider` projection is
  involved — these are not `capability.call` fixtures. Registry rows
  contributed by the manifest are admission/audit, not dispatch, consistent
  with constitution "Ground truth" item 1.
- **External trackers (github/linear/jira) — DEFERRED.** When un-deferred they
  are M2 class 1 (HTTP sans-IO step machine, the CoerceStepMachine precedent)
  with a declared claim-strength (strong / best-effort / advisory /
  unsupported) per DR-0002 "External Providers", so readiness projections
  expose uncertainty instead of pretending atomic leases.

## Manifest (M5 contribution)

std.tracker becomes an embedded manifest (rides S6), validated by the same
pipeline as third-party manifests — the std-tracker.json privilege-acceptance
test (cli/main.rs:37468) anticipated exactly this. It contributes:

- **libraries**: `std.tracker`, standard:true.
- **effect_contracts**: the five kinds above; `required_capabilities = [kind]`
  each (today all four shipped contracts pass EMPTY lists,
  parser/lib.rs:3470-3510); typed output `TrackerClaim` on `tracker.claim`.
  The S4 lock-time check (contract caps ⊆ manifest capabilities[]) holds by
  construction. *(Shipped shape 2026-07-14: FOUR contract rows —
  `tracker.renew` rides sans contract until T3 lands its effect kind; it
  ships only the capability/provider/binding trio plus its construct row,
  the std.coord `lease.renew` precedent, spec/std-coord.md "Deferred with
  cause". A `tracker.renew` contract row without a parser partner would be
  manifest-only pretense; the drift test pins the absence.)*
- **constructs**: `tracker` (declaration_block / metadata_only) + the five
  effect operations (effect_operation / `typed_effect_call` — see "Static
  checks" for the privilege-tuple correction). The ready-issue projection is
  documented core-owned sugar, not a manifest row (`projection_read` is not
  package-authorable; E4 defers projection_view).
- **capabilities**: `tracker.file`, `tracker.claim`, `tracker.renew`,
  `tracker.release`, `tracker.finish`.
- **providers**: `builtin` (store-plane, WorkItems seam).
- **profiles**: one `tracker` profile allowlisting the five capability ids
  (the coerce seed-row precedent, migrations/0001_runtime_store.sql:377-429).

Import bite per E5: advisory missing-import lint in v1 (`use std.tracker`),
hard requirement deferred with the registered lint→error escalation.

**Privilege exercise.** The reserved-word privilege rows for
claim/renew/release → std.tracker (core/lib.rs:561-582) currently grant
lowering `capability_call`, which no shipped or planned tracker lowering uses
— tracker verbs are typed dedicated kinds, i.e. `typed_effect_call` (the
std.files promotion class, E4; DR-0002 "Construct Graph Contract" names
typed_effect_call as acceptable). v1 corrects the tuples to
`typed_effect_call`, updates the acceptance test, and the embedded manifest's
claim/renew/release constructs become the mechanism's first real exercisers
(today: shipped, zero exercisers). The lock-exemption re-key and its mandatory
std-construct-authorization.maude re-model ride S6 (constitution
"Cross-cutting registered items" item 2); extending the exemption beyond
`capability.call` contracts to typed_effect_call is part of that re-model, not
a second model.

## Static checks (M8 tier assignment)

- **Tier 1 (catalog-flag generic — rule of three already satisfied):**
  mandatory `as` binding on claim; exhaustive claim-outcome handling
  (constitution M8 names "tracker claim outcomes" in the generic set). The
  renew outcome pair (renewed / not_held) joins the same generic engine, not a
  bespoke checker.
- **Tier 2 (hand-coded core checks, named here as owning spec):**
  (a) `renew`/`release` bindings must trace to a `claim ... as <binding>` in
  the same rule — extends the existing release disambiguation; note that
  mechanism is a textual body scan today (kernel/rule_lowering.rs:2141-2163)
  and must become structural (binding-table) when the renew arm touches it, not
  via a framework; (b) ready-issue names a declared tracker (shipped,
  parser/lib.rs:12114); (c) `ttl` clause takes a duration literal.
- **Honesty note for the close-out audit:** the manifest's static-guarantee
  flags must map exactly to (a)–(c) plus the tier-1 rows; no decorative flags.

## Information-flow face (DR-0029 posture)

- **The backlog is a cross-workflow carriage plane — declared here, NOT yet
  enforced by the IFC engine.** Issue payloads filed by one instance surface
  as `tracker.issue.ready` facts inside others (kernel/rule_pass.rs:290-374).
  The intended posture is the one std.coord's `shared` resources already have
  (E-COORD carriage, models/maude/infoflow-coord-carriage.maude), but coord's
  posture is engine-enforced and the tracker plane's is not:
  `is_coordination_effect` (cli/ifc.rs:818) matches only
  LeaseAcquire/LedgerAppend/CounterConsume, `rule_read_resources` never
  classifies a claim or a `has ready issue` read, so today a rule consuming
  agent-filed backlog payloads carries TOP integrity, and no tracker infoflow
  Maude model exists. Carriage classification for the tracker plane is
  deferred with cause (entry 11 below); the tracker plane is workspace-scoped
  and nothing crosses a workspace boundary in v1, which bounds the exposure
  until that slice lands.
- **Provenance is stamped at both doors.** `filed_by` records run identity
  whether an issue arrives via effect or via CLI ("two doors, one stamp",
  cli/main.rs:28995-29120). The `whip issue` CLI keeps this invariant for
  every mutating subcommand.
- **Turn grants are governance-checked.** `with access to tracker { ... }`
  grants on agent turns are validated against the verified governance envelope
  before tool exposure (harness_tools.rs:2135-2160); per DR-0029 and M3,
  `use std.tracker` grants no authority — authority = contract capability ids
  (compile plane), manifest/store rows (admission plane), and turn grants
  (harness plane), monotonically narrowing.
- **No new membrane door.** Tracker effects are store operations, not egress.
  Cross-workspace tracker sync arrives only with external providers and is an
  egress surface to be designed then (deferred).

## v1 implementation slices

Each slice is independently gateable under the per-piece review discipline.

- **T2 — capability ids on contracts** (after S3, with S4). Set
  `required_capabilities = [kind]` on all five `std.tracker` contracts;
  manifest capabilities[] rows match; S4 subset check green. Tests:
  contract-registry snapshot asserting the five ids; an S4 fixture proving a
  contract cap absent from the manifest fails the lock. Teeth in v1 =
  compile-plane attribution + S4 invariant + turn-grant narrowing; runtime
  admission for builtin kinds stays core-gated and is documented as such (no
  pretense of registry dispatch). *(2026-07-14, superseded at T4: once the
  embedded manifest seeds capability/provider/binding rows at store init,
  keeping the builtin `tracker.*` exemption would have been the pretense —
  so T4 deleted it from the NATIVE policy gate (store/lib.rs
  `policy_block_on`), mirroring std.coord slice 4: an unbound tracker kind
  blocks as blocked_by_capability, and the embedded rows make it pass by
  construction. The DO mirror keeps its exemption until the DO bootstrap
  registers packages — intentional divergence, documented at both sites.)*
- **T3 — renew end-to-end + claim TTL** (after S3; model FIRST per house
  discipline). Model: a small claim-lifecycle model (claim/renew/expire/
  release/terminal) with coverage AND bite — properties: an expired claim
  never blocks readiness; only the holder renews; renew extends monotonically;
  terminal releases everything held. Today's coverage rides only the generic
  effect-lifecycle models (models/tla/ControlPlaneLifecycle.tla:81-103); this
  adds the first tracker-specific artifact. Then: `renew` grammar + `ttl`
  clause; `IrEffectKind` + contract for `tracker.renew`; `WorkItems::renew_claim`
  + expiry-aware ready/claim across all three implementations (WorkItemStore,
  NativeStores, DoSqliteStore, with rusqlite-backed DoSql tests per the DO
  port pattern); handler outcomes; tier-1 exhaustiveness extended. Tests:
  store-unit renew/expiry/CAS races; e2e claim-ttl-expire-reproject; e2e
  renew-by-non-holder types as not_held; the terminal-release regression
  (control_plane.rs:5598) stays green.
- **T4 — privilege exercise + embedded manifest** (after S3 and S6). Built-in
  ConstructRegistrations for the six constructs owned by standard:true
  std.tracker; privilege tuples corrected capability_call → typed_effect_call
  (core/lib.rs:561-582) with the acceptance test updated (cli/main.rs:37468);
  embedded std.tracker manifest lands through the S6 pipeline. Tests: manifest
  validation acceptance; construct-use authorization through the re-keyed
  exemption; a drift test pinning manifest contracts against compiled-in
  registrations. Model: rides the S6 std-construct-authorization.maude
  re-model (no separate artifact).
- **T5 — `whip issue` CLI** (after S3; the `renew` subcommand AND the `--ttl`
  flags after T3 — the TTL mechanism, expires_at + expiry-aware ready/claim,
  exists only once T3 lands, so a T5 landed before T3 ships without `--ttl`
  and gains the flags with T3). Replace `whip items list|add|show`
  (cli/main.rs:28995-29120) one-way with
  `whip issue new|list|show|ready|claim|renew|release|finish|archive`, all
  `--json`; every mutating command returns the full issue row (DR-0002 "CLI
  Requirements": no show-after-claim round trip); `--ttl` on claim/renew (T3-
  gated, with their tests); actor identity defaults to the run-identity stamp
  with `--actor` override.
  Closes the shipped gap that no human or agent can claim/release/finish from
  the CLI at all. Tests: CLI integration incl. CLI-vs-workflow claim
  contention (already_claimed), archive excluded from ready and default list,
  provenance stamped through the CLI door.

## Deferred with cause

1. **Event-sourced rebuild** (command/event/projection split, tx log,
   disposable index, conflicts, heads, state tokens — the whole ADR-0002
   "Local Provider" model). Cause: no forcing demand; ADR status:proposed;
   the campaign's largest unforced slice (E6). Re-entry: its own model-first
   campaign behind the surviving `WorkItems` seam + this package contract.
2. **Lease/status split.** `claim` still writes `status = in_progress` and
   `claimed_by` in one row update (store/items.rs:181-188) — the combined
   behavior DR-0002 "Leases And Lifecycle" forbids. Cause: the split IS the
   rebuild's state model; forcing it onto the row store rebuilds half the ADR
   without its event log. Recorded as an interim deviation; re-entry: rebuild.
3. **Issue-model enrichment** (priority, assignee distinct from claim,
   defer_until/due_at, relations/blocked_by, comments, evidence, state_token,
   opaque-id/alias split — WS-N as primary id contradicts "Portable Issue
   Model"). Cause: honest only atop events + conflict machinery; DR-0002
   "Risks" warns against a too-large first implementation. Re-entry: rebuild.
4. **Missing ops** (note, attach evidence, set field, label mutation post-
   filing, dep add, cancel, reopen, fail-with-note). Cause: write-once row
   store. Re-entry: rebuild; cancel/reopen may land earlier as small
   demand-driven row-store slices if orchestration needs them first.
5. **External providers + claim-strength capability report.** Cause: zero
   demand; the M2 HTTP seam exists when needed. Re-entry: DR-0002 "External
   Providers".
6. **Local HTTP API** (DR-0002 "Local API"). Cause: no consumer — harnesses
   use the CLI. Re-entry: first daemon/editor consumer.
7. **Discovery + conformance fixtures** (`whip issue capabilities|schema`).
   Cause: exactly one provider. Re-entry: second provider.
8. **Readiness dimensions beyond open+unclaimed/unexpired** (deferral,
   blocking, conflicts, lease overlay). Cause: derived from the deferred
   relation/conflict model. Re-entry: rebuild.
9. **`tracker.ready` capability.** Cause: readiness is a projection, not an
   effect; no operation to attach it to. Re-entry: projection_view
   un-deferral (E4).
10. **`whip tracker` admin alias.** ADR open question; `whip issue` suffices.
    Re-entry: operator demand.
11. **IFC carriage classification for the tracker plane.** Extending the
    E-COORD posture to tracker kinds: classify `tracker.*` effects and
    `has ready issue` reads in the IFC engine (`is_coordination_effect` +
    `rule_read_resources`, cli/ifc.rs) so backlog payloads carry filed-at
    labels instead of TOP integrity, with its own Maude carriage model
    (infoflow-tracker-carriage, sibling of infoflow-coord-carriage.maude).
    Cause: a new IFC-engine + model slice outside v1's identity/rename/honesty
    scope; the plane is workspace-scoped in v1 so no cross-workspace
    laundering path exists yet. Re-entry: a dedicated model-first slice,
    forced at latest by external providers or any cross-workspace tracker
    door (which MUST NOT open before this lands).

## Spec amendments

1. **decision-records/0002-work-tracker-package.md** — add an "Interim status
   (2026-07)" note under "Status": identity + rename land NOW on the row-store
   provider per ecosystem-shape E6; the event-sourced rebuild is deferred to
   its own campaign; the `WorkItems` trait and this package contract survive
   the rebuild; the combined claim/status write and WS-N-as-id are recorded
   interim deviations from "Leases And Lifecycle" and "Portable Issue Model".
2. **work-queues.md** — extend the existing superseded banner to point at this
   document as the current surface once slice C lands; queue-noun examples are
   replaced, not aliased (DR-0002 "Replacement Of Current Queue Store").
3. **capability-registry.md**, tracker grant section (the turn-grant rows at
   spec/capability-registry.md:103-111) — reconcile the documented seeded
   grant-plane capabilities (`tracker.file`/`tracker.claim`/`tracker.finish`/
   `tracker.release` plus umbrella `tracker.update`/`tracker.write`) with the
   five effect-contract capability ids (M3): the per-op grants already mirror
   the effect vocabulary and the `update_todo` claim/finish/release
   per-transition mapping stays current; document that `update`/`write` are
   grant-plane umbrellas with no effect-contract counterpart and that
   `tracker.renew` arrives effect-plane-only with T3 (grant op pending the
   naming-boundary question below); todo-tool status rendering follows the
   post-rename status set.

## Open naming-boundary questions

- **Turn-grant verb set vs effect capability ids.** Owned-harness tracker
  grants already largely mirror the effect vocabulary: the accepted grant ops
  are `file`/`add`, `claim`, `finish`/`complete`/`close`, `release`/`reopen`,
  plus the umbrella `update` (all update transitions) and `write` (all tracker
  mutations) (turn_tool_access_from_input, harness_tools.rs:2098-2110), and
  capability-registry.md:103-111 documents the seeded `tracker.claim`/
  `tracker.finish`/`tracker.release` capabilities with `update_todo` requiring
  claim/finish/release per status transition. Against the five effect caps
  (file/claim/renew/release/finish): `update`/`write` are grant-plane umbrellas
  with no effect-plane counterpart, and `renew` is the only effect cap with no
  grant counterpart. M3 keeps the planes separate mechanisms in one namespace;
  what is flagged for Jack with the capability-registry amendment is narrower
  than a from-scratch mapping table: whether the umbrella `update`/`write`
  grants survive alongside the per-op grants, whether the aliases
  (`add`/`complete`/`close`/`reopen`) stay, and whether `renew` gets a grant op
  when T3 lands.
- **`ttl` keyword choice.** `claim <issue> ttl 30m` avoids overloading `for`
  (used by recall/consume) and `until` (used by timer/lease `until ttl`);
  alternatives were considered and rejected for ambiguity, but the final
  keyword is a small decision batched for review.
- **`finish` vs `close`.** v1 keeps the shipped `finish` verb (terminal status
  `closed`); DR-0002 lists both `finish issue / close issue`. Whether `close`
  ever becomes a distinct source verb (close-without-summary) is deferred to
  the rebuild's op set.
